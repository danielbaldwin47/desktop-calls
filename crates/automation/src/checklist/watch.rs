//! Watches the app's control interface and keeps a timeline: every state it
//! published, with the moment its change signal arrived. Steps find the first
//! state that satisfies a condition, so a latency is measured between the
//! signals themselves, however late the runner looks.

use std::collections::{BTreeMap, HashMap};
use std::hash::{DefaultHasher, Hash as _, Hasher as _};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex, PoisonError};
use std::thread;
use std::time::Instant;

use futures_util::StreamExt as _;
use zbus::fdo::PropertiesProxy;
use zbus::message::Type;
use zbus::names::InterfaceName;
use zbus::{Connection, MatchRule, MessageStream, OwnedVariant};

use super::sitting::Event;

/// The app's control interface, as `CODING_STANDARDS.md` names it.
pub const BUS_NAME: &str = "io.github.danielbaldwin47.DesktopCalls";
pub const PATH: &str = "/io/github/danielbaldwin47/DesktopCalls";
pub const INTERFACE: &str = "io.github.danielbaldwin47.DesktopCalls1";

/// What the control interface publishes. Phone numbers are read but never
/// kept: only whether the number is withheld, and a fingerprint that tells
/// whether two Calls had the same number.
#[derive(Clone, Debug, Default)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "each bool is one of the control interface's boolean properties, read as published"
)]
pub struct AppState {
    pub connection_state: String,
    pub phone_name: String,
    pub calls: Vec<AppCall>,
    pub audio_location: String,
    pub ringing: bool,
    pub do_not_disturb: bool,
    pub muted: bool,
    pub codec: String,
    pub wideband: bool,
    pub surfaces: BTreeMap<String, BTreeMap<String, String>>,
    /// Every `MissedCall` signal since watching began, oldest first.
    pub missed_calls: Vec<AppMissedCall>,
}

/// One `MissedCall` signal: a Missed Call notified.
#[derive(Clone, Debug)]
pub struct AppMissedCall {
    /// The Call's id.
    pub call: u32,
    pub withheld: bool,
    pub number: Fingerprint,
}

/// One Call as the control interface lists it.
#[derive(Clone, Debug)]
pub struct AppCall {
    pub id: u32,
    pub direction: String,
    pub state: String,
    pub number: Fingerprint,
}

/// A phone number's fingerprint: its national number, hashed, and its
/// country code, if it was written with one. It stays in memory and is
/// never written anywhere, so the number itself isn't kept.
///
/// Two fingerprints are [the same number](Fingerprint::is) when their whole
/// national numbers match and so do their country codes, if both have one:
/// `07700 900999` and `+44 7700 900999`, the same number written two ways,
/// match; `+44 8700 900999` and `+33 7700 900999` don't.
#[derive(Clone, Copy, Debug)]
pub struct Fingerprint {
    national: u64,
    country: Option<u16>,
}

impl Fingerprint {
    pub fn of(number: &str) -> Self {
        let trimmed = number.trim_start();
        let digits: Vec<u8> = number
            .bytes()
            .filter(u8::is_ascii_digit)
            .map(|digit| digit - b'0')
            .collect();
        let mut hasher = DefaultHasher::new();
        if digits.is_empty() {
            number.hash(&mut hasher);
            return Self {
                national: hasher.finish(),
                country: None,
            };
        }
        // `+44 …` or `0044 …`: a country code, then the national number.
        // `07700 …`: the national number after its trunk prefix, 0.
        let international = if trimmed.starts_with('+') {
            Some(&digits[..])
        } else {
            digits.strip_prefix(&[0, 0][..])
        };
        let (country, national) = match international {
            Some(international) => {
                let length = country_code_length(international).min(international.len());
                let (country, national) = international.split_at(length);
                let country = country
                    .iter()
                    .fold(0_u16, |code, digit| code * 10 + u16::from(*digit));
                (Some(country), national)
            }
            None => (None, digits.strip_prefix(&[0][..]).unwrap_or(&digits)),
        };
        national.hash(&mut hasher);
        Self {
            national: hasher.finish(),
            country,
        }
    }

    /// Whether `other` is the same number: the whole national number
    /// matches, and so does the country code where both have one.
    pub fn is(&self, other: &Self) -> bool {
        self.national == other.national
            && match (self.country, other.country) {
                (Some(mine), Some(theirs)) => mine == theirs,
                _ => true,
            }
    }
}

/// How many digits the country code at the start of `international` has,
/// by ITU-T E.164: `1` and `7` have one, the codes listed here two, and
/// every other code three. No code is the start of another.
fn country_code_length(international: &[u8]) -> usize {
    const TWO_DIGITS: &[u8] = &[
        20, 27, 30, 31, 32, 33, 34, 36, 39, 40, 41, 43, 44, 45, 46, 47, 48, 49, 51, 52, 53, 54, 55,
        56, 57, 58, 60, 61, 62, 63, 64, 65, 66, 81, 82, 84, 86, 90, 91, 92, 93, 94, 95, 98,
    ];
    match international {
        [1 | 7, ..] => 1,
        [first, second, ..] if TWO_DIGITS.contains(&(first * 10 + second)) => 2,
        _ => 3,
    }
}

impl AppState {
    /// Whether `surface` (`"banner"`, `"call-window"`) is shown.
    pub fn shows(&self, surface: &str) -> bool {
        self.surfaces.contains_key(surface)
    }

    /// Whether `surface` is shown and has keyboard focus.
    pub fn focused(&self, surface: &str) -> bool {
        self.surfaces
            .get(surface)
            .and_then(|content| content.get("focused"))
            .is_some_and(|focused| focused == "true")
    }

    /// The first incoming Call that is ringing, if any.
    pub fn ringing_call(&self) -> Option<&AppCall> {
        self.calls
            .iter()
            .find(|call| call.direction == "incoming" && call.state == "ringing")
    }

    /// The Call with this id, if the Phone still has it.
    pub fn call(&self, id: u32) -> Option<&AppCall> {
        self.calls.iter().find(|call| call.id == id)
    }

    /// The Missed Call notified for the Call with this id, if it was.
    pub fn missed_call(&self, id: u32) -> Option<&AppMissedCall> {
        self.missed_calls.iter().find(|missed| missed.call == id)
    }

    /// The Dialer's selected row, as it reads, while the Dialer is open.
    pub fn dialer_selection(&self) -> Option<&str> {
        let dialer = self.surfaces.get("dialer")?;
        let selected = dialer.get("selected")?;
        dialer.get(&format!("row-{selected}")).map(String::as_str)
    }

    fn apply(&mut self, name: &str, value: OwnedVariant) -> zbus::Result<()> {
        match name {
            "ConnectionState" => self.connection_state = String::try_from(value)?,
            "Phone" => self.phone_name = <(String, String)>::try_from(value)?.0,
            "Calls" => {
                self.calls = <Vec<(u32, String, String, String, String, u64)>>::try_from(value)?
                    .into_iter()
                    .map(|(id, direction, state, number, ..)| AppCall {
                        id,
                        direction,
                        state,
                        number: Fingerprint::of(&number),
                    })
                    .collect();
            }
            "AudioLocation" => self.audio_location = String::try_from(value)?,
            "Ringing" => self.ringing = bool::try_from(value)?,
            "DoNotDisturb" => self.do_not_disturb = bool::try_from(value)?,
            "Muted" => self.muted = bool::try_from(value)?,
            "Codec" => (self.codec, self.wideband) = <(String, bool)>::try_from(value)?,
            "Surfaces" => self.surfaces = BTreeMap::try_from(value)?,
            _ => {}
        }
        Ok(())
    }
}

/// Every state the app has published since watching began, oldest first.
/// It never empty: the first entry is the state when watching began.
pub type Timeline = Arc<Mutex<Vec<(Instant, AppState)>>>;

/// Why watching couldn't start.
#[derive(Debug)]
pub enum WatchError {
    /// Nothing owns the control interface's bus name.
    NotRunning,
    /// The bus or the app answered in a way the runner doesn't understand.
    Unexpected(String),
}

/// Starts watching the control interface on `bus`. Each change is added to
/// the returned timeline, and announced on `events`.
pub fn start(bus: &Connection, events: Sender<Event>) -> Result<Timeline, WatchError> {
    let (stream, first) = zbus::block_on(subscribe(bus)).map_err(|error| match &error {
        zbus::Error::FDO(fdo)
            if matches!(
                **fdo,
                zbus::fdo::Error::ServiceUnknown(_) | zbus::fdo::Error::NameHasNoOwner(_)
            ) =>
        {
            WatchError::NotRunning
        }
        _ => WatchError::Unexpected(error.to_string()),
    })?;
    let timeline: Timeline = Arc::new(Mutex::new(vec![(Instant::now(), first)]));
    let kept = timeline.clone();
    thread::spawn(move || zbus::block_on(follow(stream, &kept, &events)));
    Ok(timeline)
}

/// Subscribes to the control interface's signals (its `PropertiesChanged`
/// and `MissedCall`), then reads it whole, so no change slips between the
/// two.
async fn subscribe(bus: &Connection) -> zbus::Result<(MessageStream, AppState)> {
    let rule = MatchRule::builder()
        .msg_type(Type::Signal)
        .path(PATH)
        .build()?;
    let stream = MessageStream::for_match_rule(rule, bus, None).await?;
    let properties = PropertiesProxy::builder(bus)
        .destination(BUS_NAME)
        .path(PATH)
        .build()
        .await?;
    let all = properties
        .get_all(InterfaceName::try_from(INTERFACE)?)
        .await?;
    let mut state = AppState::default();
    for (name, value) in &all {
        state.apply(name, value.try_clone()?)?;
    }
    Ok((stream, state))
}

async fn follow(mut stream: MessageStream, timeline: &Timeline, events: &Sender<Event>) {
    while let Some(Ok(message)) = stream.next().await {
        let arrived = Instant::now();
        let Some(change) = Change::from(&message) else {
            continue;
        };
        let mut timeline = timeline.lock().unwrap_or_else(PoisonError::into_inner);
        let mut state = timeline
            .last()
            .map(|(_, state)| state.clone())
            .unwrap_or_default();
        match change {
            Change::Properties(changed) => {
                for (name, value) in changed {
                    // A value the runner can't read leaves that property as
                    // it was; the step that needs it then says what it saw.
                    let _ = state.apply(&name, value);
                }
            }
            Change::MissedCall(missed) => state.missed_calls.push(missed),
        }
        timeline.push((arrived, state));
        drop(timeline);
        if events.send(Event::Changed).is_err() {
            return;
        }
    }
}

/// A signal from the control interface that changes what the runner sees.
enum Change {
    Properties(HashMap<String, OwnedVariant>),
    MissedCall(AppMissedCall),
}

impl Change {
    fn from(message: &zbus::Message) -> Option<Self> {
        let header = message.header();
        match header.member().map(zbus::names::MemberName::as_str) {
            Some("PropertiesChanged") => {
                let (interface, changed, _) = message
                    .body()
                    .deserialize::<(String, HashMap<String, OwnedVariant>, Vec<String>)>()
                    .ok()?;
                (interface == INTERFACE).then_some(Self::Properties(changed))
            }
            Some("MissedCall")
                if header.interface().map(zbus::names::InterfaceName::as_str)
                    == Some(INTERFACE) =>
            {
                let (call, number, ..) = message
                    .body()
                    .deserialize::<(u32, String, String, u32, u64)>()
                    .ok()?;
                Some(Self::MissedCall(AppMissedCall {
                    call,
                    withheld: number.is_empty(),
                    number: Fingerprint::of(&number),
                }))
            }
            _ => None,
        }
    }
}
