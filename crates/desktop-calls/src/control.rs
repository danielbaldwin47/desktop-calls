//! The control interface: the app's one seam on the session bus, for the
//! CLI, tests, the keybinding and the Indicator.
//!
//! It publishes the Call state machine's state as properties with change
//! signals, each state before the surfaces that follow from it (see
//! [`Published`]), turns method calls into user intents for the state machine,
//! signals failures as `Error(kind, message)` with the Messages text, and
//! signals Missed Calls as `MissedCall`. Bus name, object path and interface
//! are in `CODING_STANDARDS.md`.

use std::collections::BTreeMap;
use std::sync::{Mutex, MutexGuard, PoisonError};

use desktop_calls_core::matching::{self, DialerRow, DialerRowKind};
use desktop_calls_core::messages::{self, Failure};
use desktop_calls_core::{
    AudioLocation, CallId, Caller, ContactsState, Input, MissedCallNotice, State,
};
use futures_channel::mpsc::UnboundedSender;
use futures_channel::oneshot;
use zbus::object_server::{InterfaceRef, SignalEmitter};
use zbus::{Connection, DBusError, interface};

use crate::logging::EventKind;

pub const BUS_NAME: &str = "io.github.danielbaldwin47.DesktopCalls";
pub const PATH: &str = "/io/github/danielbaldwin47/DesktopCalls";
/// The prefix of the D-Bus error names for [`Refused`] requests.
pub const ERROR_PREFIX: &str = "io.github.danielbaldwin47.DesktopCalls1.Error.";

/// Each shown surface's key content, by surface name (`"banner"`,
/// `"call-window"`), as the surfaces report it.
pub type ShownSurfaces = BTreeMap<String, BTreeMap<String, String>>;

/// Where a Dial request's outcome goes: the failure that kept the Call from
/// being placed, or `None` once the Phone has reported the Call.
pub type DialVerdict = oneshot::Sender<Option<Failure>>;

/// A request to Call a number or `tel:` link, as the user gave it, with
/// where its outcome goes.
pub type DialRequest = (String, DialVerdict);

/// A request the Desktop refuses, as the D-Bus error a method returns: the
/// failure kind as the error name, and the Messages sentence.
#[derive(Debug, DBusError)]
#[zbus(prefix = "io.github.danielbaldwin47.DesktopCalls1.Error")]
pub enum Refused {
    NoLiveCall(String),
    MuteUnavailable(String),
    InvalidTones(String),
    /// `HoldAndAnswer` or `EndAndAnswer` with no Waiting Call.
    NoWaitingCall(String),
    /// `Swap` with no Call on hold.
    NoHeldCall(String),
    /// `HoldAndAnswer` while the user's own Call is still being placed.
    NotConnectedYet(String),
    /// `Swap` while a Call is on hold and a Waiting Call rings: the Phone
    /// would answer it instead. `HangUp` refused for the same reason reports
    /// the `Error` signal instead, as it returns nothing.
    WaitingCallFirst(String),
    /// `HoldAndAnswer` or `Answer` while another Call is on hold already.
    AlreadyHolding(String),
    /// `HoldAndAnswer` or `EndAndAnswer` while the only other Call is on
    /// hold.
    NothingToHoldOrEnd(String),
    /// A Call the user asked to place wasn't; the sentence says why.
    CallNotPlaced(String),

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
            Failure::NoWaitingCall => Self::NoWaitingCall(text),
            Failure::NoHeldCall => Self::NoHeldCall(text),
            Failure::NotConnectedYet => Self::NotConnectedYet(text),
            Failure::WaitingCallFirst => Self::WaitingCallFirst(text),
            Failure::AlreadyHolding => Self::AlreadyHolding(text),
            Failure::NothingToHoldOrEnd => Self::NothingToHoldOrEnd(text),
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

            _ => Self::Failed(text),
        }
    }
}

/// What the control interface publishes, and where its intents go.
///
/// The published state sits behind locks of its own, so publishing needs
/// only shared access to the interface: a `Dial` call holds that until the
/// Phone reports its Call, and publishing must go on meanwhile.
pub struct ControlInterface {
    state: Mutex<State>,
    surfaces: Mutex<ShownSurfaces>,
    intents: UnboundedSender<Input>,
    dials: UnboundedSender<DialRequest>,
}

#[interface(interface = "io.github.danielbaldwin47.DesktopCalls1")]
impl ControlInterface {
    /// `"not-set-up"`, `"connecting"`, `"connected"`, `"disconnected"` or
    /// `"pairing-lost"`.
    #[zbus(property)]
    fn connection_state(&self) -> String {
        self.state().connection_state().as_str().to_owned()
    }

    /// The Phone's name and Bluetooth address, or two empty strings while no
    /// Phone is set up.
    #[zbus(property)]
    fn phone(&self) -> (String, String) {
        self.state()
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
        let state = self.state();
        state
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
                    state
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
        self.state()
            .audio_location()
            .map_or("", |location| location.as_str())
            .to_owned()
    }

    /// Whether the Desktop is ringing.
    #[zbus(property)]
    fn ringing(&self) -> bool {
        self.state().ringing()
    }

    /// Whether the Desktop has muted its microphone for the Call.
    #[zbus(property)]
    fn muted(&self) -> bool {
        self.state().muted()
    }

    /// Whether the Desktop's Do Not Disturb is on. False on a Desktop
    /// without Omarchy's.
    #[zbus(property)]
    fn do_not_disturb(&self) -> bool {
        self.state().do_not_disturb()
    }

    /// The codec the Desktop's Call audio negotiated (`"cvsd"`, `"msbc"`,
    /// `"lc3-swb"`, `"other"`, or empty) and whether it is wideband ("HD").
    #[zbus(property)]
    fn codec(&self) -> (String, bool) {
        self.state()
            .codec()
            .map_or((String::new(), false), |codec| {
                (codec.as_str().to_owned(), codec.is_wideband())
            })
    }

    /// The surfaces shown now, each with its key content as shown.
    #[zbus(property)]
    fn surfaces(&self) -> ShownSurfaces {
        lock(&self.surfaces).clone()
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
        contacts_state(&self.state())
    }

    /// What is wrong that the user can put right: kind (such as
    /// `"contacts-denied"`), the sentence to show, and the next action, or
    /// empty.
    #[zbus(property)]
    fn problems(&self) -> Vec<(String, String, String)> {
        problems(&self.state())
    }

    /// The Call History, newest first: kind (`"incoming"`, `"outgoing"`,
    /// `"missed"`), number (empty if withheld), the Contact's name or the
    /// name the Phone gave (or empty), the number's label (or empty), and
    /// when in milliseconds since the Unix epoch (0 if the Phone didn't
    /// say).
    fn get_call_history(&self) -> Vec<(String, String, String, String, u64)> {
        let state = self.state();
        state
            .call_history()
            .entries()
            .iter()
            .map(|entry| {
                let contact = state.contact(&entry.caller);
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
        let state = self.state();
        matching::search(query, state.contacts(), state.call_history())
            .iter()
            .map(search_row)
            .collect()
    }

    /// Opens the Setup Window, to pair a Phone, pair it again or switch to
    /// another, as the Indicator's "Connect your phone…" does.
    fn open_connect_window(&self) {
        let _ = self.intents.unbounded_send(Input::OpenSetupWindow);
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

    /// Puts the live Call on hold and answers the Waiting Call, its audio on
    /// the Desktop. It returns once the Phone is asked; the Calls' states
    /// change when the Phone has done it, and `Error` says so if it refuses.
    /// Fails with the reason while no Call is waiting.
    fn hold_and_answer(&self) -> Result<(), Refused> {
        self.pass_on(Input::HoldAndAnswer)
    }

    /// Ends the live Call and answers the Waiting Call, as `HoldAndAnswer`
    /// does otherwise.
    fn end_and_answer(&self) -> Result<(), Refused> {
        self.pass_on(Input::EndAndAnswer)
    }

    /// Puts the live Call on hold and makes the Held Call live again; with
    /// only a Held Call, makes it live again. It returns once the Phone is
    /// asked, and `Error` says so if it refuses. Fails with the reason while
    /// no Call is on hold.
    fn swap(&self) -> Result<(), Refused> {
        self.pass_on(Input::Swap)
    }

    /// Calls `number`, a number as typed or a `tel:` link, with the audio on
    /// the Desktop. Replies once the Phone reports the Call (dialing,
    /// alerting or further on), or with `Error.CallNotPlaced` and the
    /// sentence the Call Window shows if the Call can't be placed now, the
    /// Phone refuses it, or the Phone doesn't report it in time (15 s). The
    /// Call Window and the `Error` signal show the same failure.
    async fn dial(&self, number: String) -> Result<(), Refused> {
        let (verdict, outcome) = oneshot::channel();
        // The decide loop only stops with the service.
        let _ = self.dials.unbounded_send((number, verdict));
        match outcome.await {
            Ok(None) => Ok(()),
            Ok(Some(failure)) => Err(failure.into()),
            // The service is stopping, so the Call won't be followed.
            Err(oneshot::Canceled) => Err(Failure::DialFailed.into()),
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
    /// the reason while there is no live Call.
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

    /// A Missed Call was notified: the Call's id, the caller's number (empty
    /// if withheld), the Contact's name (or empty), how many Missed Calls
    /// from this caller the notification now tells of, and when the Call
    /// started ringing, in milliseconds since the Unix epoch.
    #[zbus(signal)]
    async fn missed_call(
        emitter: &SignalEmitter<'_>,
        call_id: u32,
        number: &str,
        display_name: &str,
        count: u32,
        rang_at_ms: u64,
    ) -> zbus::Result<()>;
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

/// Takes `lock`. A panic while it was held left nothing half-written: every
/// write is one whole replacement.
fn lock<T>(lock: &Mutex<T>) -> MutexGuard<'_, T> {
    lock.lock().unwrap_or_else(PoisonError::into_inner)
}

impl ControlInterface {
    /// The state published now.
    fn state(&self) -> MutexGuard<'_, State> {
        lock(&self.state)
    }

    /// Sends `intent` to the Call state machine, unless the state published
    /// now refuses it.
    fn pass_on(&self, intent: Input) -> Result<(), Refused> {
        let refusal = self.state().refusal(&intent);
        if let Some(failure) = refusal {
            return Err(failure.into());
        }
        let _ = self.intents.unbounded_send(intent);
        Ok(())
    }
}

/// Proof that the control interface has published a state of the Call
/// state machine: its change signals are sent. Rendering the surfaces and
/// the Indicator takes one, and only [`Publisher::publish`] makes one, so
/// nothing that follows from a state can show before the state is
/// published. A client following the control interface therefore sees each
/// Call change before the surface changes it causes, such as a Call answered
/// on the Phone before its Banner closing (ticket #107).
#[must_use = "the surfaces render only once their state is published"]
pub struct Published(());

/// Logs a change the control interface couldn't signal.
pub fn publish_failed(error: &zbus::Error) {
    tracing::warn!(
        event_kind = %EventKind::PublishFailed,
        error_kind = "publish-failed",
        detail = %error,
        "Couldn't publish the new state on the control interface"
    );
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
            state: Mutex::new(State::default()),
            surfaces: Mutex::new(ShownSurfaces::new()),
            intents,
            dials,
        };
        server.at(PATH, control).await?;
        let interface = server.interface::<_, ControlInterface>(PATH).await?;
        session.request_name(BUS_NAME).await?;
        Ok(Self { interface })
    }

    /// Publishes `state`, signalling each property that changed, and hands
    /// back the proof that rendering what follows from it needs. A signal
    /// that can't be sent is logged: the surfaces still show.
    pub async fn publish(&self, state: &State) -> Published {
        if let Err(error) = self.signal_changes(state).await {
            publish_failed(&error);
        }
        Published(())
    }

    async fn signal_changes(&self, state: &State) -> zbus::Result<()> {
        let emitter = self.interface.signal_emitter();
        let published = self.interface.get().await;
        let old = std::mem::replace(&mut *published.state(), state.clone());
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
        if old.do_not_disturb() != state.do_not_disturb() {
            published.do_not_disturb_changed(emitter).await?;
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
        let published = self.interface.get().await;
        let changed = {
            let mut shown = lock(&published.surfaces);
            let changed = *shown != surfaces;
            *shown = surfaces;
            changed
        };
        if changed {
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

    /// Signals the Missed Call `notice` tells of as `MissedCall`.
    pub async fn missed_call(&self, notice: &MissedCallNotice) -> zbus::Result<()> {
        let number = match &notice.caller {
            Caller::Number(number) => number.as_str(),
            Caller::Withheld => "",
        };
        let name = notice
            .contact
            .as_ref()
            .map_or("", |contact| contact.name.as_str());
        ControlInterface::missed_call(
            self.interface.signal_emitter(),
            notice.call.0,
            number,
            name,
            notice.count,
            notice.at.0,
        )
        .await
    }
}
