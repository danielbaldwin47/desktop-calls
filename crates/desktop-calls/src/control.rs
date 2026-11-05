//! The control interface: the app's one seam on the session bus, for the
//! CLI, tests, the keybinding and the Indicator.
//!
//! It publishes the Call state machine's state as properties with change
//! signals, turns method calls into user intents for the state machine, and
//! signals failures as `Error(kind, message)` with the Messages text. Bus name,
//! object path and interface are in `CODING_STANDARDS.md`.

use std::collections::BTreeMap;

use desktop_calls_core::matching::{self, DialerRow, DialerRowKind};
use desktop_calls_core::messages::{self, Failure};
use desktop_calls_core::{AudioLocation, CallId, Caller, ContactsState, Input, State};
use futures_channel::mpsc::UnboundedSender;
use futures_channel::oneshot;
use zbus::object_server::{InterfaceRef, SignalEmitter};
use zbus::{Connection, DBusError, interface};

pub const BUS_NAME: &str = "io.github.danielbaldwin47.DesktopCalls";
pub const PATH: &str = "/io/github/danielbaldwin47/DesktopCalls";
/// The prefix of the D-Bus error names for [`Refused`] requests.
pub const ERROR_PREFIX: &str = "io.github.danielbaldwin47.DesktopCalls1.Error.";

/// Each shown surface's key content, by surface name (`"banner"`,
/// `"call-window"`), as the surfaces report it.
pub type ShownSurfaces = BTreeMap<String, BTreeMap<String, String>>;

/// A request to Call a number or `tel:` link, as the user gave it, with
/// where the verdict goes: the failure that refused it, or `None` once the
/// Phone has been asked to dial.
pub type DialRequest = (String, oneshot::Sender<Option<Failure>>);

/// A request the Desktop refuses, as the D-Bus error a method returns: the
/// failure kind as the error name, and the Messages sentence.
#[derive(Debug, DBusError)]
#[zbus(prefix = "io.github.danielbaldwin47.DesktopCalls1.Error")]
pub enum Refused {
    NoLiveCall(String),
    MuteUnavailable(String),
    InvalidTones(String),
    /// A Call the user asked to place wasn't; the sentence says why.
    CallNotPlaced(String),
    /// The Desktop can't move the audio to the Phone itself yet; the
    /// sentence says how to do it on the Phone.
    HandoffToPhoneUnavailable(String),
    /// `MoveAudio` was given something other than `"desktop"` or `"phone"`.
    UnknownAudioLocation(String),
    Failed(String),
}

impl From<Failure> for Refused {
    fn from(failure: Failure) -> Self {
        let text = messages::failure(failure).text.to_owned();
        match failure {
            Failure::NoLiveCall => Self::NoLiveCall(text),
            Failure::MuteUnavailable => Self::MuteUnavailable(text),
            Failure::InvalidTones => Self::InvalidTones(text),
            Failure::Undialable
            | Failure::PhoneNotSetUp
            | Failure::PhoneConnecting
            | Failure::PhoneOutOfRange
            | Failure::CallInProgress
            | Failure::NumberRefused
            | Failure::PhoneBusy
            | Failure::DialNotSupported
            | Failure::NoSignal
            | Failure::NetworkError
            | Failure::DialFailed => Self::CallNotPlaced(text),
            Failure::HandoffToPhoneUnavailable => Self::HandoffToPhoneUnavailable(text),
            _ => Self::Failed(text),
        }
    }
}

/// What the control interface publishes, and where its intents go.
pub struct ControlInterface {
    state: State,
    surfaces: ShownSurfaces,
    intents: UnboundedSender<Input>,
    dials: UnboundedSender<DialRequest>,
}

#[interface(interface = "io.github.danielbaldwin47.DesktopCalls1")]
impl ControlInterface {
    /// `"not-set-up"`, `"connecting"`, `"connected"` or `"disconnected"`.
    #[zbus(property)]
    fn connection_state(&self) -> String {
        self.state.connection_state().as_str().to_owned()
    }

    /// The Phone's name and Bluetooth address, or two empty strings while no
    /// Phone is set up.
    #[zbus(property)]
    fn phone(&self) -> (String, String) {
        self.state
            .phone()
            .map(|phone| (phone.name.clone(), phone.address.clone()))
            .unwrap_or_default()
    }

    /// The Phone's Calls, oldest first: id, direction (`"incoming"`,
    /// `"outgoing"`, `"unknown"`), state (`"ringing"`, `"waiting"`,
    /// `"dialing"`, `"alerting"`, `"active"`, `"held"`), number (empty if
    /// withheld), display name (the Contact's name, or empty), and when the
    /// Call connected in milliseconds since the Unix epoch (0 until then).
    #[zbus(property)]
    fn calls(&self) -> Vec<(u32, String, String, String, String, u64)> {
        self.state
            .calls()
            .iter()
            .map(|call| {
                (
                    call.id().0,
                    call.direction().as_str().to_owned(),
                    call.state().as_str().to_owned(),
                    match call.caller() {
                        Caller::Number(number) => number.clone(),
                        Caller::Withheld => String::new(),
                    },
                    self.state
                        .contact(call.caller())
                        .map(|contact| contact.name)
                        .unwrap_or_default(),
                    call.active_since().map_or(0, |since| since.0),
                )
            })
            .collect()
    }

    /// `"desktop"` or `"phone"`, or empty while there is no Call.
    #[zbus(property)]
    fn audio_location(&self) -> String {
        self.state
            .audio_location()
            .map_or("", |location| location.as_str())
            .to_owned()
    }

    /// Whether the Desktop is ringing.
    #[zbus(property)]
    fn ringing(&self) -> bool {
        self.state.ringing()
    }

    /// Whether the Desktop has muted its microphone for the Call.
    #[zbus(property)]
    fn muted(&self) -> bool {
        self.state.muted()
    }

    /// The codec the Desktop's Call audio negotiated (`"cvsd"`, `"msbc"`,
    /// `"lc3-swb"`, `"other"`, or empty) and whether it is wideband ("HD").
    #[zbus(property)]
    fn codec(&self) -> (String, bool) {
        self.state.codec().map_or((String::new(), false), |codec| {
            (codec.as_str().to_owned(), codec.is_wideband())
        })
    }

    /// The surfaces shown now, each with its key content as shown.
    #[zbus(property)]
    fn surfaces(&self) -> ShownSurfaces {
        self.surfaces.clone()
    }

    /// How far Contacts and Call History are synced from the Phone: the
    /// state (`"syncing"`, `"ready"`, `"permission-needed"`, `"denied"`,
    /// `"unavailable"`), how many Contacts and Call History entries the
    /// Desktop holds, while permission is needed when the Phone's prompt
    /// times out in milliseconds since the Unix epoch (otherwise 0), and
    /// while unavailable why (`"not-synced"`, `"no-contacts-service"`,
    /// `"prompt-timed-out"`, `"failed"`; otherwise empty).
    #[zbus(property)]
    fn contacts_state(&self) -> (String, u32, u32, u64, String) {
        contacts_state(&self.state)
    }

    /// What is wrong that the user can put right: kind (such as
    /// `"contacts-denied"`), the sentence to show, and the next action, or
    /// empty.
    #[zbus(property)]
    fn problems(&self) -> Vec<(String, String, String)> {
        problems(&self.state)
    }

    /// The Call History, newest first: kind (`"incoming"`, `"outgoing"`,
    /// `"missed"`), number (empty if withheld), the Contact's name or the
    /// name the Phone gave (or empty), the number's label (or empty), and
    /// when in milliseconds since the Unix epoch (0 if the Phone didn't
    /// say).
    fn get_call_history(&self) -> Vec<(String, String, String, String, u64)> {
        self.state
            .call_history()
            .entries()
            .iter()
            .map(|entry| {
                let contact = self.state.contact(&entry.caller);
                let name = contact
                    .as_ref()
                    .map(|contact| contact.name.clone())
                    .filter(|name| !name.is_empty())
                    .unwrap_or_else(|| entry.name.clone());
                (
                    entry.kind.as_str().to_owned(),
                    match &entry.caller {
                        Caller::Number(number) => number.clone(),
                        Caller::Withheld => String::new(),
                    },
                    name,
                    contact.map_or_else(String::new, |contact| {
                        messages::number_label(&contact.label)
                    }),
                    entry.at.map_or(0, |at| at.0),
                )
            })
            .collect()
    }

    /// The Dialer's rows for `query` (what the user typed), best first, at
    /// most 50: kind (`"contact"`, `"call-history"`, or `"call-number"`
    /// for the number typed); for a Call History row its kind
    /// (`"incoming"`, `"outgoing"`, `"missed"`, otherwise empty); the
    /// Contact's name, or the name the Phone gave (or empty); the number's
    /// label (or empty); the number Enter dials (empty if withheld or not
    /// dialable); the number grouped for reading (empty if withheld); and
    /// for a Call History row when, in milliseconds since the Unix epoch
    /// (otherwise 0). With nothing typed, the rows are the Call History,
    /// newest first.
    fn search(&self, query: &str) -> Vec<SearchRow> {
        matching::search(query, self.state.contacts(), self.state.call_history())
            .iter()
            .map(search_row)
            .collect()
    }

    /// Answers the ringing Call with this id on the Desktop.
    fn answer(&self, call: u32) {
        let _ = self.intents.unbounded_send(Input::Answer(CallId(call)));
    }

    /// Declines the ringing Call with this id.
    fn decline(&self, call: u32) {
        let _ = self.intents.unbounded_send(Input::Decline(CallId(call)));
    }

    /// Hangs up the Call with this id.
    fn hang_up(&self, call: u32) {
        let _ = self.intents.unbounded_send(Input::HangUp(CallId(call)));
    }

    /// Calls `number`, a number as typed or a `tel:` link, with the audio on
    /// the Desktop. Replies once the Phone has been asked to dial, or with
    /// `Error.CallNotPlaced` and the sentence to show if the Call can't be
    /// placed now. Failures the Phone reports later go to the Call Window and
    /// the `Error` signal.
    async fn dial(&self, number: String) -> Result<(), Refused> {
        let (verdict, refusal) = oneshot::channel();
        // The decide loop only stops with the service.
        let _ = self.dials.unbounded_send((number, verdict));
        match refusal.await {
            Ok(Some(failure)) => Err(failure.into()),
            Ok(None) | Err(oneshot::Canceled) => Ok(()),
        }
    }

    /// Mutes (`true`) or unmutes the Desktop's microphone. Mute works only
    /// while the Call's audio is on the Desktop; otherwise this fails with
    /// the reason.
    fn set_muted(&self, muted: bool) -> Result<(), Refused> {
        self.pass_on(Input::SetMuted(muted))
    }

    /// Moves the Call's audio to `location`, `"desktop"` or `"phone"`: a
    /// Handoff. It returns once the Phone is asked; `AudioLocation` changes
    /// when the audio arrives, and `Error` says so if it doesn't. Fails with
    /// the reason while there is no live Call, and with the instruction for
    /// the Phone's own audio button for `"phone"`, until the Desktop can
    /// close the audio link itself.
    fn move_audio(&self, location: &str) -> Result<(), Refused> {
        let location = AudioLocation::from_name(location).ok_or_else(|| {
            Refused::UnknownAudioLocation(messages::unknown_audio_location().to_owned())
        })?;
        self.pass_on(Input::MoveAudio(location))
    }

    /// Plays keypad tones to the other party: digits, `*` and `#`, in order.
    /// Spaces, dashes, dots and brackets are left out; anything else fails.
    fn send_tones(&self, tones: String) -> Result<(), Refused> {
        self.pass_on(Input::SendTones(tones))
    }

    /// What the Desktop Calls keybinding does: gives keyboard focus to the
    /// Incoming Call Banner while a Call rings, otherwise to the Call Window
    /// while a Call is live, otherwise opens the Dialer. It never answers,
    /// declines or ends a Call.
    fn activate(&self) {
        let _ = self.intents.unbounded_send(Input::Activate);
    }

    /// Opens the Dialer afresh with `query` typed (empty for the Call
    /// History), selecting the Call History row of the Call with id
    /// `select_call_id` if there is one (0 for none).
    fn open_dialer(&self, query: String, select_call_id: u32) {
        let _ = self.intents.unbounded_send(Input::OpenDialer {
            query,
            select_call: (select_call_id != 0).then_some(CallId(select_call_id)),
        });
    }

    /// Something the user asked for failed: the failure kind and the
    /// sentence to show.
    #[zbus(signal)]
    async fn error(emitter: &SignalEmitter<'_>, kind: &str, message: &str) -> zbus::Result<()>;
}

/// One row of `Search`'s reply.
type SearchRow = (String, String, String, String, String, String, u64);

fn search_row(row: &DialerRow) -> SearchRow {
    (
        row.kind.as_str().to_owned(),
        match row.kind {
            DialerRowKind::CallHistory(kind) => kind.as_str().to_owned(),
            DialerRowKind::Contact | DialerRowKind::CallNumber => String::new(),
        },
        row.name.clone(),
        row.label
            .as_ref()
            .map_or_else(String::new, messages::number_label),
        row.dial().unwrap_or_default(),
        row.shown_number.clone(),
        row.at.map_or(0, |at| at.0),
    )
}

fn contacts_state(state: &State) -> (String, u32, u32, u64, String) {
    let contacts_state = state.contacts_state();
    let count = |count: usize| u32::try_from(count).unwrap_or(u32::MAX);
    (
        contacts_state.as_str().to_owned(),
        count(state.contacts().directory().len()),
        count(state.call_history().entries().len()),
        match contacts_state {
            ContactsState::PermissionNeeded { deadline } => deadline.0,
            _ => 0,
        },
        match contacts_state {
            ContactsState::Unavailable(reason) => reason.as_str().to_owned(),
            _ => String::new(),
        },
    )
}

fn problems(state: &State) -> Vec<(String, String, String)> {
    state
        .problems()
        .into_iter()
        .map(|problem| {
            let message = messages::problem(problem.kind);
            (
                problem.kind.as_str().to_owned(),
                message.text.to_owned(),
                match message.next_action {
                    Some(messages::NextAction::Command(command)) => command.to_owned(),
                    None => String::new(),
                },
            )
        })
        .collect()
}

impl ControlInterface {
    /// Sends `intent` to the Call state machine, unless the state published
    /// now refuses it.
    fn pass_on(&self, intent: Input) -> Result<(), Refused> {
        if let Some(failure) = self.state.refusal(&intent) {
            return Err(failure.into());
        }
        let _ = self.intents.unbounded_send(intent);
        Ok(())
    }
}

/// Publishes new state on the control interface.
#[derive(Clone)]
pub struct Publisher {
    interface: InterfaceRef<ControlInterface>,
}

impl Publisher {
    /// Serves the control interface on `session`, sending its intents to
    /// `intents` and its Dial requests to `dials`, and claims its bus name.
    pub async fn serve(
        session: &Connection,
        intents: UnboundedSender<Input>,
        dials: UnboundedSender<DialRequest>,
    ) -> zbus::Result<Self> {
        let server = session.object_server();
        let control = ControlInterface {
            state: State::default(),
            surfaces: ShownSurfaces::new(),
            intents,
            dials,
        };
        server.at(PATH, control).await?;
        let interface = server.interface::<_, ControlInterface>(PATH).await?;
        session.request_name(BUS_NAME).await?;
        Ok(Self { interface })
    }

    /// Publishes `state`, signalling each property that changed.
    pub async fn publish(&self, state: &State) -> zbus::Result<()> {
        let emitter = self.interface.signal_emitter();
        let mut published = self.interface.get_mut().await;
        let old = std::mem::replace(&mut published.state, state.clone());
        if old.connection_state() != state.connection_state() {
            published.connection_state_changed(emitter).await?;
        }
        if old.phone() != state.phone() {
            published.phone_changed(emitter).await?;
        }
        // A Call's display name changes when Contacts do.
        if old.calls() != state.calls() || old.contacts() != state.contacts() {
            published.calls_changed(emitter).await?;
        }
        if old.audio_location() != state.audio_location() {
            published.audio_location_changed(emitter).await?;
        }
        if old.ringing() != state.ringing() {
            published.ringing_changed(emitter).await?;
        }
        if old.muted() != state.muted() {
            published.muted_changed(emitter).await?;
        }
        if old.codec() != state.codec() {
            published.codec_changed(emitter).await?;
        }
        if contacts_state(&old) != contacts_state(state) {
            published.contacts_state_changed(emitter).await?;
        }
        if old.problems() != state.problems() {
            published.problems_changed(emitter).await?;
        }
        Ok(())
    }

    /// Publishes what the surfaces show now.
    pub async fn publish_surfaces(&self, surfaces: ShownSurfaces) -> zbus::Result<()> {
        let emitter = self.interface.signal_emitter();
        let mut published = self.interface.get_mut().await;
        if published.surfaces != surfaces {
            published.surfaces = surfaces;
            published.surfaces_changed(emitter).await?;
        }
        Ok(())
    }

    /// Signals `failure` as `Error(kind, message)`.
    pub async fn report_error(&self, failure: Failure) -> zbus::Result<()> {
        let message = messages::failure(failure);
        ControlInterface::error(
            self.interface.signal_emitter(),
            failure.as_str(),
            message.text,
        )
        .await
    }
}
