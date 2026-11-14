//! What the app reports through its control interface, as scenarios read it.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, PoisonError};
use std::thread;

use futures_util::StreamExt as _;
use zbus::Connection;
use zbus::MatchRule;
use zbus::MessageStream;
use zbus::fdo::PropertiesProxy;
use zbus::message::Type;
use zbus::names::InterfaceName;

use super::simulated_phone::PhoneIdentity;
use super::{APP_BUS_NAME, APP_INTERFACE, APP_PATH};

/// The `Surfaces` property: each shown surface (`"banner"`,
/// `"call-window"`) with its key content, as the surface shows it.
pub type ShownSurfaces = BTreeMap<String, BTreeMap<String, String>>;

/// The app's state, as its control interface publishes it.
#[derive(Clone, Debug, PartialEq, Eq)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "each bool is one of the control interface's boolean properties, read as published"
)]
pub struct AppState {
    /// `ConnectionState`: `"not-set-up"`, `"connecting"`, `"connected"`,
    /// `"disconnected"` or `"pairing-lost"`.
    pub connection_state: String,
    /// `Phone`, or `None` while it is empty.
    pub phone: Option<PhoneIdentity>,
    /// `Calls`, oldest first.
    pub calls: Vec<AppCall>,
    /// `AudioLocation`: `"desktop"`, `"phone"`, or `""` while there is no Call.
    pub audio_location: String,
    /// `Ringing`.
    pub ringing: bool,
    /// `Muted`: whether the Desktop has muted its microphone.
    pub muted: bool,
    /// `DoNotDisturb`: whether the Desktop's Do Not Disturb is on.
    pub do_not_disturb: bool,
    /// `Codec`'s name: `"cvsd"`, `"msbc"`, `"lc3-swb"`, or `""` while the
    /// Desktop has negotiated none.
    pub codec: String,
    /// `Codec`'s wideband flag ("HD").
    pub wideband: bool,
    /// `Surfaces`: each shown surface (`"banner"`, `"call-window"`) with its
    /// key content, as the surface shows it.
    pub surfaces: BTreeMap<String, BTreeMap<String, String>>,
    /// `ContactsState`.
    pub contacts: AppContactsState,
    /// `Problems`, in the order the app lists them.
    pub problems: Vec<AppProblem>,
    /// `CallsOff`'s state: `"on"` or `"off"`.
    pub calls_off: String,
}

/// The app's `ContactsState`: how far Contacts and Call History are synced.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AppContactsState {
    /// `"syncing"`, `"ready"`, `"permission-needed"`, `"denied"` or
    /// `"unavailable"`.
    pub state: String,
    /// How many Contacts the Desktop holds.
    pub contacts: u32,
    /// How many Call History entries the Desktop holds.
    pub call_history: u32,
    /// While `"permission-needed"`: when the Phone's prompt times out, in
    /// milliseconds since the Unix epoch; otherwise 0.
    pub deadline_ms: u64,
    /// While `"unavailable"`: why, such as `"prompt-timed-out"`; otherwise
    /// empty.
    pub reason: String,
}

/// One entry of the app's `Problems`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AppProblem {
    /// The problem's kind, such as `"contacts-denied"`.
    pub kind: String,
    /// The sentence shown to the user.
    pub message: String,
    /// What the user can do, or empty.
    pub action: String,
}

/// One entry of the app's Call History.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AppCallHistoryEntry {
    /// `"incoming"`, `"outgoing"` or `"missed"`.
    pub kind: String,
    /// The other party's number as the Phone reports it, or `""` if withheld.
    pub number: String,
    /// The Contact's name, or `""`.
    pub display_name: String,
    /// The number's label, such as `"mobile"`, or `""`.
    pub number_label: String,
    /// When, in milliseconds since the Unix epoch, or 0 if the Phone didn't say.
    pub at_ms: u64,
}

/// One row of the Dialer, as the app's `Search` method returns it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AppDialerRow {
    /// `"contact"`, `"call-history"` or `"call-number"` (the number typed).
    pub kind: String,
    /// For a Call History row, `"incoming"`, `"outgoing"` or `"missed"`;
    /// otherwise `""`.
    pub call_kind: String,
    /// The Contact's name, or the name the Phone gave, or `""`.
    pub name: String,
    /// The number's label, such as `"mobile"`, or `""`.
    pub number_label: String,
    /// The number Enter dials, or `""` if it was withheld or can't be dialled.
    pub number: String,
    /// The number grouped for reading, or `""` if withheld.
    pub shown_number: String,
    /// For a Call History row, when, in milliseconds since the Unix epoch;
    /// otherwise 0.
    pub at_ms: u64,
}

impl AppState {
    /// The key content of the surface named `surface`, if it is shown.
    #[must_use]
    pub fn surface(&self, surface: &str) -> Option<&BTreeMap<String, String>> {
        self.surfaces.get(surface)
    }
}

/// One Call, as the app's `Calls` property lists it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AppCall {
    /// The app-assigned stable Call id that `Answer`, `Decline` and `HangUp` take.
    pub id: u32,
    /// `"incoming"`, `"outgoing"` or `"unknown"`.
    pub direction: String,
    /// `"ringing"`, `"waiting"`, `"dialing"`, `"alerting"`, `"active"` or `"held"`.
    pub state: String,
    /// The other party's number as the Phone reports it, or `""` if withheld.
    pub number: String,
    /// The Contact's name, or `""`.
    pub display_name: String,
    /// When the Call connected, in milliseconds since the Unix epoch, or 0.
    pub active_since_ms: u64,
}

/// One `Error` signal from the app, or the error a method of its control
/// interface returned.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AppError {
    /// The failure kind, such as `"answer-failed"`; for a method's error,
    /// the D-Bus error name.
    pub kind: String,
    /// The sentence shown to the user.
    pub message: String,
}

/// One `MissedCall` signal from the app.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MissedCall {
    /// The latest Missed Call's id.
    pub call: u32,
    /// The caller's number, or `""` if withheld.
    pub number: String,
    /// The Contact's name, or `""`.
    pub display_name: String,
    /// How many Missed Calls from this caller the notification tells of.
    pub count: u32,
    /// When the latest one started ringing, in milliseconds since the Unix
    /// epoch.
    pub at_ms: u64,
}

pub(super) async fn read_app_state(session: &Connection) -> zbus::Result<AppState> {
    // One GetAll, so the properties come from the same moment.
    let properties = PropertiesProxy::builder(session)
        .destination(APP_BUS_NAME)
        .path(APP_PATH)
        .build()
        .await?;
    let all = properties
        .get_all(InterfaceName::try_from(APP_INTERFACE)?)
        .await?;
    let property = |name: &str| {
        all.get(name)
            .ok_or_else(|| zbus::Error::Failure(format!("no {name} property")))?
            .try_clone()
    };
    let connection_state = String::try_from(property("ConnectionState")?)?;
    let (name, address) = <(String, String)>::try_from(property("Phone")?)?;
    let calls = read_calls(property("Calls")?)?;
    let (codec, wideband) = <(String, bool)>::try_from(property("Codec")?)?;
    let surfaces = <BTreeMap<String, BTreeMap<String, String>>>::try_from(property("Surfaces")?)?;
    let (state, contacts, call_history, deadline_ms, reason) =
        <(String, u32, u32, u64, String)>::try_from(property("ContactsState")?)?;
    let problems = <Vec<(String, String, String)>>::try_from(property("Problems")?)?
        .into_iter()
        .map(|(kind, message, action)| AppProblem {
            kind,
            message,
            action,
        })
        .collect();
    let (calls_off, _until, _pending) = <(String, u64, bool)>::try_from(property("CallsOff")?)?;
    Ok(AppState {
        connection_state,
        phone: (!address.is_empty()).then_some(PhoneIdentity { name, address }),
        calls,
        audio_location: String::try_from(property("AudioLocation")?)?,
        ringing: bool::try_from(property("Ringing")?)?,
        muted: bool::try_from(property("Muted")?)?,
        do_not_disturb: bool::try_from(property("DoNotDisturb")?)?,
        codec,
        wideband,
        surfaces,
        contacts: AppContactsState {
            state,
            contacts,
            call_history,
            deadline_ms,
            reason,
        },
        problems,
        calls_off,
    })
}

/// Reads a value of the `Calls` property.
fn read_calls(value: zbus::OwnedVariant) -> zbus::Result<Vec<AppCall>> {
    Ok(
        <Vec<(u32, String, String, String, String, u64)>>::try_from(value)?
            .into_iter()
            .map(
                |(id, direction, state, number, display_name, active_since_ms)| AppCall {
                    id,
                    direction,
                    state,
                    number,
                    display_name,
                    active_since_ms,
                },
            )
            .collect(),
    )
}

/// Collects every `Error` signal the app sends on `session`, from now on.
pub(super) fn collect_app_errors(session: &Connection) -> Arc<Mutex<Vec<AppError>>> {
    collect_signal(session, "Error", |message| {
        let (kind, message) = message.body().deserialize::<(String, String)>().ok()?;
        Some(AppError { kind, message })
    })
}

/// Collects every `MissedCall` signal the app sends on `session`, from now
/// on.
pub(super) fn collect_missed_calls(session: &Connection) -> Arc<Mutex<Vec<MissedCall>>> {
    collect_signal(session, "MissedCall", |message| {
        let (call, number, display_name, count, at_ms) = message
            .body()
            .deserialize::<(u32, String, String, u32, u64)>()
            .ok()?;
        Some(MissedCall {
            call,
            number,
            display_name,
            count,
            at_ms,
        })
    })
}

/// Collects every signal `member` of the app's control interface on
/// `session`, from now on, as `read` reads it.
fn collect_signal<T: Send + 'static>(
    session: &Connection,
    member: &'static str,
    read: fn(&zbus::Message) -> Option<T>,
) -> Arc<Mutex<Vec<T>>> {
    let signals = Arc::new(Mutex::new(Vec::new()));
    let stream = zbus::block_on(async {
        let rule = MatchRule::builder()
            .msg_type(Type::Signal)
            .path(APP_PATH)
            .interface(APP_INTERFACE)
            .member(member)
            .build()?;
        MessageStream::for_match_rule(rule, session, None).await
    })
    .unwrap_or_else(|error| panic!("subscribe to the app's {member} signal: {error}"));
    let collected = signals.clone();
    thread::spawn(move || {
        zbus::block_on(async move {
            let mut stream = stream;
            while let Some(Ok(message)) = stream.next().await {
                if let Some(signal) = read(&message) {
                    collected
                        .lock()
                        .unwrap_or_else(PoisonError::into_inner)
                        .push(signal);
                }
            }
        });
    });
    signals
}

/// What the app had published just after one of its signals: its `Calls`,
/// `Surfaces` and `ConnectionState` as the last `PropertiesChanged` carrying
/// them left them, and every `Error` and `MissedCall` signal sent so far.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Published {
    /// `Calls`, oldest first.
    pub calls: Vec<AppCall>,
    /// `Surfaces`: each shown surface with its key content.
    pub surfaces: ShownSurfaces,
    /// `ConnectionState`.
    pub connection_state: String,
    /// Every `Error` signal sent so far, oldest first.
    pub errors: Vec<AppError>,
    /// Every `MissedCall` signal sent so far, oldest first.
    pub missed_calls: Vec<MissedCall>,
}

impl Published {
    /// What the app has published before it sends anything: no Calls, no
    /// surfaces, and a Phone that isn't set up.
    #[must_use]
    pub fn at_start() -> Self {
        Self {
            connection_state: "not-set-up".to_owned(),
            ..Self::default()
        }
    }
}

/// Collects, from now on, what the app has published after each of its
/// `PropertiesChanged` signals that carries `Calls`, `Surfaces` or
/// `ConnectionState`, and after each `Error` and `MissedCall` signal, in the
/// order the app sent them. One subscription gets them all, so the bus's
/// order is kept.
pub(super) fn collect_published(session: &Connection) -> Arc<Mutex<Vec<Published>>> {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let stream = zbus::block_on(async {
        let rule = MatchRule::builder()
            .msg_type(Type::Signal)
            .path(APP_PATH)
            .build()?;
        MessageStream::for_match_rule(rule, session, None).await
    })
    .expect("subscribe to the app's signals");
    let collected = seen.clone();
    thread::spawn(move || {
        zbus::block_on(async move {
            let mut stream = stream;
            let mut published = Published::at_start();
            while let Some(Ok(message)) = stream.next().await {
                if !publish(&mut published, &message) {
                    continue;
                }
                collected
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .push(published.clone());
            }
        });
    });
    seen
}

/// Applies the app's signal `message` to `published`. Returns whether it
/// changed anything that `Published` records.
fn publish(published: &mut Published, message: &zbus::Message) -> bool {
    let header = message.header();
    let (Some(interface), Some(member)) = (header.interface(), header.member()) else {
        return false;
    };
    if interface.as_str() == APP_INTERFACE {
        match member.as_str() {
            "Error" => {
                let Ok((kind, message)) = message.body().deserialize::<(String, String)>() else {
                    return false;
                };
                published.errors.push(AppError { kind, message });
                true
            }
            "MissedCall" => {
                let Ok((call, number, display_name, count, at_ms)) =
                    message
                        .body()
                        .deserialize::<(u32, String, String, u32, u64)>()
                else {
                    return false;
                };
                published.missed_calls.push(MissedCall {
                    call,
                    number,
                    display_name,
                    count,
                    at_ms,
                });
                true
            }
            _ => false,
        }
    } else if interface.as_str() == "org.freedesktop.DBus.Properties"
        && member.as_str() == "PropertiesChanged"
    {
        let Ok((_, mut changed, _)) = message.body().deserialize::<(
            String,
            std::collections::HashMap<String, zbus::OwnedVariant>,
            Vec<String>,
        )>() else {
            return false;
        };
        let calls = changed
            .remove("Calls")
            .and_then(|calls| read_calls(calls).ok());
        let surfaces = changed
            .remove("Surfaces")
            .and_then(|surfaces| ShownSurfaces::try_from(surfaces).ok());
        let connection_state = changed
            .remove("ConnectionState")
            .and_then(|state| String::try_from(state).ok());
        let carried = calls.is_some() || surfaces.is_some() || connection_state.is_some();
        if let Some(calls) = calls {
            published.calls = calls;
        }
        if let Some(surfaces) = surfaces {
            published.surfaces = surfaces;
        }
        if let Some(connection_state) = connection_state {
            published.connection_state = connection_state;
        }
        carried
    } else {
        false
    }
}
