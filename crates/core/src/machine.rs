//! The Call state machine: one function from (state, input) to (new state,
//! effects). Every policy decision lives here.

use crate::call::{
    AudioLocation, Call, CallId, CallState, Caller, Codec, Direction, Request, Timestamp, Transport,
};
use crate::connection::{ConnectionState, Phone};
use crate::contacts::{CallHistory, Contacts, ContactsState, SyncFailure, Unavailable};
use crate::matching::ContactMatch;
use crate::messages::{Failure, ProblemKind};

/// How long the Call Window shows "Call ended" before it closes.
const CALL_ENDED_MS: u32 = 2_000;

/// The hands-free microphone gain to restore when the Phone's own value is
/// unusable: 15, the top of the hands-free range (0 to 15), which is also
/// PipeWire's default (`bluez5.hfp-hf.default-mic-volume`).
const DEFAULT_MICROPHONE_VOLUME: u8 = 15;
/// The top of the hands-free gain range.
const MAX_MICROPHONE_VOLUME: u8 = 15;

/// Something that happened, as the shell adapters report it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Input {
    /// A paired Phone that can carry Calls exists on the Desktop.
    PhonePaired(Phone),
    /// The paired Phone with this address is gone.
    PhoneUnpaired { address: String },
    /// The Phone with this address is connected for Calls.
    PhoneConnected { address: String },
    /// The Phone with this address is no longer connected for Calls.
    PhoneDisconnected { address: String },
    /// The Phone has a Call it didn't have before.
    CallAdded {
        call: CallId,
        state: CallState,
        caller: Caller,
        at: Timestamp,
    },
    /// A Call moved on in its life.
    CallChanged {
        call: CallId,
        state: CallState,
        at: Timestamp,
    },
    /// A Call is over, however it ended.
    CallEnded { call: CallId },
    /// The Call audio link between Phone and Desktop changed.
    TransportChanged(Transport),
    /// The audio link's codec changed; `None` is the Phone's 0, "no link".
    CodecChanged(Option<Codec>),
    /// The Phone reported the hands-free microphone gain, 0 to 15. PipeWire
    /// reports 255 (its -1, sent as a byte) before the Phone has said.
    MicrophoneVolumeChanged(u8),
    /// The user asked to answer this Call on the Desktop.
    Answer(CallId),
    /// The user asked to decline this ringing Call.
    Decline(CallId),
    /// The user asked to hang up this Call.
    HangUp(CallId),
    /// The user asked to mute (`true`) or unmute the Desktop's microphone.
    SetMuted(bool),
    /// The user typed or pasted these keypad tones.
    SendTones(String),
    /// The user pressed the Desktop Calls keybinding: do the most useful
    /// thing for what is happening now.
    Activate,
    /// The user let go of the surface that has keyboard focus (Escape).
    ReleaseFocus,
    /// The Phone refused, or never answered, this command.
    PhoneCommandFailed(PhoneCommand),
    /// A timer started by [`Effect::StartTimer`] ran out.
    TimerFired(Timer),
    /// Contacts and Call History cached from an earlier sync were read.
    ContactsLoaded {
        contacts: Contacts,
        call_history: CallHistory,
    },
    /// A sync finished with Contacts and Call History.
    ContactsSynced {
        contacts: Contacts,
        call_history: CallHistory,
    },
    /// A refresh of the Call History finished.
    CallHistoryRefreshed(CallHistory),
    /// The Phone is asking whether to share Contacts, until `deadline`.
    ContactsPermissionNeeded { deadline: Timestamp },
    /// A sync, or a refresh, ended without Contacts.
    ContactsSyncFailed(SyncFailure),
}

/// What the Phone link can be asked to do with Calls.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PhoneCommand {
    /// Answer this ringing Call.
    Answer(CallId),
    /// Hang up this Call; for a ringing Call, that declines it.
    HangUp(CallId),
    /// Bring the live Call's audio to the Desktop.
    MoveAudioToDesktop,
    /// Keep the next Call's audio on the Phone unless the Desktop asks for it.
    KeepAudioOnPhone,
    /// Set the hands-free microphone gain: 0 mutes.
    SetMicrophoneVolume(u8),
    /// Play these keypad tones (digits, `*` and `#`) to the other party, in
    /// order.
    SendTones(String),
}

/// A timer the state machine can ask for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Timer {
    /// Close the Call Window that shows this ended Call.
    CloseCallWindow(CallId),
}

/// Something the shell adapters must do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Effect {
    /// Follow the Phone with this address: report when it connects and
    /// disconnects for Calls.
    AttachPhone { address: String },
    /// Stop following the Phone.
    DetachPhone,
    /// Ask the Phone to do something. The Phone commands of one step run in
    /// order, and stop at the first that fails.
    Phone(PhoneCommand),
    /// Play the Desktop ringtone until told to stop.
    StartRinging,
    /// Stop the Desktop ringtone.
    StopRinging,
    /// Report [`Input::TimerFired`] with `timer` after `after_ms`.
    StartTimer { timer: Timer, after_ms: u32 },
    /// Tell the user something went wrong.
    ReportError(Failure),
    /// Sync Contacts and Call History from the attached Phone.
    SyncContacts,
    /// Pull the Phone's Call History again.
    RefreshCallHistory,
    /// Delete the Contacts and Call History held for the attached Phone,
    /// on disk too.
    ForgetContacts,
}

/// What the Desktop surfaces show, worked out from the state.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Surfaces {
    pub banner: Option<BannerView>,
    pub call_window: Option<CallWindowView>,
}

/// The Incoming Call Banner's content.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BannerView {
    pub call: CallId,
    pub caller: Caller,
    /// The Banner has keyboard focus, so A answers and D declines.
    pub focused: bool,
    /// The Contact the caller is, if Contacts say.
    pub contact: Option<ContactMatch>,
}

/// The Call Window's content.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CallWindowView {
    pub call: CallId,
    pub caller: Caller,
    /// The Contact the other party is, if Contacts say.
    pub contact: Option<ContactMatch>,
    pub phase: CallWindowPhase,
    /// The Call Window has keyboard focus.
    pub focused: bool,
}

/// Where the Call in the Call Window is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CallWindowPhase {
    /// Answered or placed, and not yet connected.
    Connecting,
    /// Connected: the timer runs from `active_since`.
    Live {
        active_since: Timestamp,
        audio_location: AudioLocation,
        wideband: bool,
        mute: Mute,
    },
    /// Over; the window closes shortly.
    Ended,
}

/// The Call Window's Mute control.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mute {
    /// The microphone is on; Mute mutes it.
    Off,
    /// The microphone is muted; Mute unmutes it.
    On,
    /// Mute works only while the Audio Location is the Desktop.
    Unavailable,
}

/// Whether the Desktop muted the microphone, and the gain to put back.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Muting {
    #[default]
    Off,
    On {
        restore: u8,
    },
}

/// Which surface the user gave keyboard focus with the keybinding. Each
/// surface takes focus only when asked, and gives it up when it closes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Focus {
    #[default]
    None,
    Banner,
    CallWindow,
}

/// An ended Call the Call Window still shows.
#[derive(Clone, Debug, PartialEq, Eq)]
struct EndedCall {
    id: CallId,
    caller: Caller,
}

/// Everything the Call state machine knows.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct State {
    phone: Option<Phone>,
    connection: ConnectionState,
    calls: Vec<Call>,
    transport: Transport,
    /// The Phone's latest codec report, whatever the transport is doing.
    reported_codec: Option<Codec>,
    /// The codec the Desktop's audio negotiated during the current Calls.
    codec: Option<Codec>,
    ended: Option<EndedCall>,
    /// The Phone's latest report of its hands-free microphone gain.
    microphone_volume: Option<u8>,
    muting: Muting,
    focus: Focus,
    contacts_sync: ContactsState,
    contacts: Contacts,
    call_history: CallHistory,
}

/// Something wrong that the user can put right, shown until it is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Problem {
    pub kind: ProblemKind,
}

impl State {
    /// The Phone the Desktop works with, once one is paired.
    #[must_use]
    pub fn phone(&self) -> Option<&Phone> {
        self.phone.as_ref()
    }

    #[must_use]
    pub fn connection_state(&self) -> &ConnectionState {
        &self.connection
    }

    /// The Phone's Calls, oldest first.
    #[must_use]
    pub fn calls(&self) -> &[Call] {
        &self.calls
    }

    /// Whether the Desktop ringtone should be playing: a Call rings here and
    /// no other Call is live, so a waiting Call never rings over a live one.
    #[must_use]
    pub fn ringing(&self) -> bool {
        self.calls.iter().any(Call::rings_here)
            && !self.calls.iter().any(|call| call.state.blocks_ringing())
    }

    /// Where the Calls' audio is, while there are any.
    #[must_use]
    pub fn audio_location(&self) -> Option<AudioLocation> {
        if self.calls.is_empty() {
            None
        } else if self.transport == Transport::Active {
            Some(AudioLocation::Desktop)
        } else {
            Some(AudioLocation::Phone)
        }
    }

    /// The codec the Desktop's Call audio negotiated, kept for as long as
    /// there are Calls. A codec report while no audio link is up (the
    /// Phone's refused attempts, or its 0 while idle) is not a change.
    #[must_use]
    pub fn codec(&self) -> Option<Codec> {
        self.codec
    }

    /// How far Contacts and Call History are synced.
    #[must_use]
    pub fn contacts_state(&self) -> ContactsState {
        self.contacts_sync
    }

    /// The Contacts the Desktop holds.
    #[must_use]
    pub fn contacts(&self) -> &Contacts {
        &self.contacts
    }

    /// The Call History the Desktop holds, newest first.
    #[must_use]
    pub fn call_history(&self) -> &CallHistory {
        &self.call_history
    }

    /// The Contact `caller` is, if Contacts say.
    #[must_use]
    pub fn contact(&self, caller: &Caller) -> Option<ContactMatch> {
        match caller {
            Caller::Number(number) => self.contacts.directory().lookup(number),
            Caller::Withheld => None,
        }
    }

    /// What is wrong that the user can put right.
    #[must_use]
    pub fn problems(&self) -> Vec<Problem> {
        match self.contacts_sync {
            ContactsState::Denied => vec![Problem {
                kind: ProblemKind::ContactsDenied,
            }],
            ContactsState::Unavailable(Unavailable::PromptTimedOut) => vec![Problem {
                kind: ProblemKind::ContactsPromptUnanswered,
            }],
            _ => Vec::new(),
        }
    }

    /// Whether the Desktop has muted its microphone for the Call.
    #[must_use]
    pub fn muted(&self) -> bool {
        matches!(self.muting, Muting::On { .. })
    }

    /// What the Call Window's Mute control offers.
    fn mute(&self) -> Mute {
        match self.muting {
            Muting::On { .. } => Mute::On,
            Muting::Off if self.audio_location() == Some(AudioLocation::Desktop) => Mute::Off,
            Muting::Off => Mute::Unavailable,
        }
    }

    /// What the Desktop surfaces show.
    #[must_use]
    pub fn surfaces(&self) -> Surfaces {
        let banner = self
            .calls
            .iter()
            .find(|call| call.rings_here())
            .map(|call| BannerView {
                call: call.id,
                caller: call.caller.clone(),
                focused: self.focus == Focus::Banner,
                contact: self.contact(&call.caller),
            });
        let call_window = self
            .calls
            .iter()
            .find(|call| call.has_call_window())
            .map(|call| CallWindowView {
                call: call.id,
                caller: call.caller.clone(),
                contact: self.contact(&call.caller),
                phase: match call.active_since {
                    Some(active_since) if call.state.is_connected() => CallWindowPhase::Live {
                        active_since,
                        audio_location: self.audio_location().unwrap_or(AudioLocation::Phone),
                        wideband: self.codec.is_some_and(Codec::is_wideband),
                        mute: self.mute(),
                    },
                    _ => CallWindowPhase::Connecting,
                },
                focused: self.focus == Focus::CallWindow,
            })
            .or_else(|| {
                self.ended.as_ref().map(|ended| CallWindowView {
                    call: ended.id,
                    caller: ended.caller.clone(),
                    contact: self.contact(&ended.caller),
                    phase: CallWindowPhase::Ended,
                    focused: self.focus == Focus::CallWindow,
                })
            });
        Surfaces {
            banner,
            call_window,
        }
    }

    /// The Contacts state once the Phone leaves: a sync under way can't go
    /// on, leaving ready if anything is held.
    fn held_contacts_state(&self) -> ContactsState {
        match self.contacts_sync {
            ContactsState::Syncing | ContactsState::PermissionNeeded { .. }
                if self.contacts.directory().is_empty()
                    && self.call_history.entries().is_empty() =>
            {
                ContactsState::Unavailable(Unavailable::NotSynced)
            }
            ContactsState::Syncing | ContactsState::PermissionNeeded { .. } => ContactsState::Ready,
            other => other,
        }
    }

    /// Whether the Phone is here and hasn't refused to share Contacts.
    fn may_sync_contacts(&self) -> bool {
        self.connection == ConnectionState::Connected && self.contacts_sync != ContactsState::Denied
    }

    /// Why the Desktop refuses `input` now, if it does. The control
    /// interface asks before passing an intent on, so a refused request
    /// fails at once with the reason; [`step`] refuses the same way.
    #[must_use]
    pub fn refusal(&self, input: &Input) -> Option<Failure> {
        match input {
            Input::SetMuted(true) if !self.muted() => match self.audio_location() {
                None => Some(Failure::NoLiveCall),
                Some(AudioLocation::Phone) => Some(Failure::MuteUnavailable),
                Some(AudioLocation::Desktop) => None,
            },
            Input::SendTones(tones) => {
                if !self
                    .calls
                    .iter()
                    .any(|call| call.state == CallState::Active)
                {
                    Some(Failure::NoLiveCall)
                } else if keypad_tones(tones).is_none() {
                    Some(Failure::InvalidTones)
                } else {
                    None
                }
            }
            _ => None,
        }
    }

    fn is_our_phone(&self, address: &str) -> bool {
        self.phone
            .as_ref()
            .is_some_and(|phone| phone.address.eq_ignore_ascii_case(address))
    }

    fn call_mut(&mut self, id: CallId) -> Option<&mut Call> {
        self.calls.iter_mut().find(|call| call.id == id)
    }

    fn has_desktop_audio(&self) -> bool {
        !self.calls.is_empty() && self.transport.is_up()
    }
}

/// Works out the new state and the effects that follow from `input`.
#[must_use]
pub fn step(state: State, input: Input) -> (State, Vec<Effect>) {
    let was_ringing = state.ringing();
    let (mut state, mut effects) = decide(state, input);
    // Focus goes with the surface that had it.
    let surfaces = state.surfaces();
    match state.focus {
        Focus::Banner if surfaces.banner.is_none() => state.focus = Focus::None,
        Focus::CallWindow if surfaces.call_window.is_none() => state.focus = Focus::None,
        _ => {}
    }
    match (was_ringing, state.ringing()) {
        (false, true) => effects.insert(0, Effect::StartRinging),
        (true, false) => effects.insert(0, Effect::StopRinging),
        _ => {}
    }
    // Mute is the Desktop's microphone. Once the audio leaves the Desktop,
    // or the Call ends, the Phone's microphone must not stay silenced.
    if state.audio_location() != Some(AudioLocation::Desktop) {
        unmute(&mut state, &mut effects);
    }
    (state, effects)
}

/// What the Contacts sync reports.
fn contacts_sync(state: &mut State, input: Input, effects: &mut Vec<Effect>) {
    match input {
        Input::ContactsLoaded {
            contacts,
            call_history,
        } => {
            if state.contacts_sync == ContactsState::Unavailable(Unavailable::NotSynced) {
                state.contacts_sync = ContactsState::Ready;
            }
            state.contacts = contacts;
            state.call_history = call_history;
        }
        Input::ContactsSynced {
            contacts,
            call_history,
        } => {
            state.contacts_sync = ContactsState::Ready;
            state.contacts = contacts;
            state.call_history = call_history;
        }
        Input::CallHistoryRefreshed(call_history) => state.call_history = call_history,
        Input::ContactsPermissionNeeded { deadline } => {
            state.contacts_sync = ContactsState::PermissionNeeded { deadline };
        }
        Input::ContactsSyncFailed(SyncFailure::Denied) => {
            // The person said no: keep nothing of theirs.
            state.contacts_sync = ContactsState::Denied;
            state.contacts = Contacts::default();
            state.call_history = CallHistory::default();
            effects.push(Effect::ForgetContacts);
        }
        // A sync cut short by the Phone leaving isn't a failure to report;
        // what was synced before still names callers.
        Input::ContactsSyncFailed(SyncFailure::Unavailable(reason))
            if state.connection == ConnectionState::Connected =>
        {
            state.contacts_sync = ContactsState::Unavailable(reason);
        }
        _ => {}
    }
}

fn phone_connected(state: &mut State, effects: &mut Vec<Effect>) {
    state.connection = ConnectionState::Connected;
    // Every connect syncs, so the Desktop catches up with changes made on
    // the Phone while apart.
    state.contacts_sync = ContactsState::Syncing;
    effects.push(Effect::SyncContacts);
    // A crash or a dropped link mid-Call must never leave the next Call's
    // audio jumping to the Desktop.
    if !state.has_desktop_audio() {
        effects.push(Effect::Phone(PhoneCommand::KeepAudioOnPhone));
    }
}

fn decide(mut state: State, input: Input) -> (State, Vec<Effect>) {
    let mut effects = Vec::new();
    match input {
        // Desktop Calls works with one Phone: the first one paired.
        Input::PhonePaired(phone) if state.phone.is_none() => {
            effects.push(Effect::AttachPhone {
                address: phone.address.clone(),
            });
            state.phone = Some(phone);
            state.connection = ConnectionState::Connecting;
        }
        Input::PhoneUnpaired { address } if state.is_our_phone(&address) => {
            return (State::default(), vec![Effect::DetachPhone]);
        }
        Input::PhoneConnected { address } if state.is_our_phone(&address) => {
            phone_connected(&mut state, &mut effects);
        }
        Input::PhoneDisconnected { address }
            if state.is_our_phone(&address) && state.connection == ConnectionState::Connected =>
        {
            state.connection = ConnectionState::Disconnected;
            state.transport = Transport::Idle;
            state.contacts_sync = state.held_contacts_state();
        }
        Input::CallAdded {
            call,
            state: call_state,
            caller,
            at,
        } => {
            if state.calls.iter().all(|known| known.id != call) {
                if state.ended.as_ref().is_some_and(|ended| ended.id == call) {
                    state.ended = None;
                }
                state.calls.push(Call {
                    id: call,
                    direction: Direction::from_first_state(call_state),
                    state: call_state,
                    caller,
                    active_since: call_state.is_connected().then_some(at),
                    request: Request::None,
                });
            }
        }
        Input::CallChanged {
            call,
            state: call_state,
            at,
        } => {
            if let Some(known) = state.call_mut(call) {
                known.state = call_state;
                if call_state.is_connected() && known.active_since.is_none() {
                    known.active_since = Some(at);
                }
            }
        }
        Input::CallEnded { call } => {
            let known = state.calls.iter().any(|known| known.id == call);
            end_call(&mut state, call, &mut effects);
            // The Call is now in the Phone's Call History.
            if known && state.may_sync_contacts() {
                effects.push(Effect::RefreshCallHistory);
            }
        }
        input @ (Input::ContactsLoaded { .. }
        | Input::ContactsSynced { .. }
        | Input::CallHistoryRefreshed(_)
        | Input::ContactsPermissionNeeded { .. }
        | Input::ContactsSyncFailed(_)) => contacts_sync(&mut state, input, &mut effects),
        Input::TransportChanged(transport) => {
            state.transport = transport;
            if transport.is_up() && state.reported_codec.is_some() {
                state.codec = state.reported_codec;
            }
        }
        Input::CodecChanged(codec) => {
            state.reported_codec = codec;
            if codec.is_some() && state.transport.is_up() {
                state.codec = codec;
            }
        }
        Input::MicrophoneVolumeChanged(volume) => state.microphone_volume = Some(volume),
        Input::SetMuted(true) => mute(&mut state, &mut effects),
        Input::SetMuted(false) => unmute(&mut state, &mut effects),
        Input::SendTones(typed) => send_tones(&state, typed, &mut effects),
        Input::Answer(call) => answer(&mut state, call, &mut effects),
        Input::Decline(call) => decline(&mut state, call, &mut effects),
        Input::HangUp(call) => hang_up(&mut state, call, &mut effects),
        Input::Activate => activate(&mut state, &mut effects),
        Input::ReleaseFocus => state.focus = Focus::None,
        Input::PhoneCommandFailed(command) => command_failed(&mut state, &command, &mut effects),
        Input::TimerFired(Timer::CloseCallWindow(call)) => {
            if state.ended.as_ref().is_some_and(|ended| ended.id == call) {
                state.ended = None;
            }
        }
        Input::PhonePaired(_)
        | Input::PhoneUnpaired { .. }
        | Input::PhoneConnected { .. }
        | Input::PhoneDisconnected { .. } => {}
    }
    (state, effects)
}

/// The keypad tones in `typed`: digits, `*` and `#`, leaving out the
/// spaces, dashes, dots and brackets a pasted number may carry. `None` if
/// it has anything else, or no tone at all.
#[must_use]
pub fn keypad_tones(typed: &str) -> Option<String> {
    let mut tones = String::new();
    for character in typed.chars() {
        match character {
            '0'..='9' | '*' | '#' => tones.push(character),
            ' ' | '-' | '.' | '(' | ')' => {}
            _ => return None,
        }
    }
    (!tones.is_empty()).then_some(tones)
}

fn send_tones(state: &State, typed: String, effects: &mut Vec<Effect>) {
    let input = Input::SendTones(typed);
    if let Some(failure) = state.refusal(&input) {
        effects.push(Effect::ReportError(failure));
    } else if let Input::SendTones(typed) = input
        && let Some(tones) = keypad_tones(&typed)
    {
        effects.push(Effect::Phone(PhoneCommand::SendTones(tones)));
    }
}

fn mute(state: &mut State, effects: &mut Vec<Effect>) {
    if state.muted() {
        return;
    }
    if let Some(failure) = state.refusal(&Input::SetMuted(true)) {
        effects.push(Effect::ReportError(failure));
        return;
    }
    // A gain of 0, or PipeWire's 255 for "the Phone hasn't said", would
    // leave the microphone silent after unmuting.
    let restore = match state.microphone_volume {
        Some(volume @ 1..=MAX_MICROPHONE_VOLUME) => volume,
        _ => DEFAULT_MICROPHONE_VOLUME,
    };
    state.muting = Muting::On { restore };
    effects.push(Effect::Phone(PhoneCommand::SetMicrophoneVolume(0)));
}

fn unmute(state: &mut State, effects: &mut Vec<Effect>) {
    if let Muting::On { restore } = state.muting {
        state.muting = Muting::Off;
        effects.push(Effect::Phone(PhoneCommand::SetMicrophoneVolume(restore)));
    }
}

/// The keybinding: focus the Banner while a Call rings here, otherwise the
/// Call Window while a Call is live. Neither touches the Call itself.
fn activate(state: &mut State, effects: &mut Vec<Effect>) {
    let surfaces = state.surfaces();
    if surfaces.banner.is_some() {
        state.focus = Focus::Banner;
    } else if surfaces
        .call_window
        .is_some_and(|window| window.phase != CallWindowPhase::Ended)
    {
        state.focus = Focus::CallWindow;
    } else {
        // The Dialer opens here once it exists (ticket #11).
        effects.push(Effect::ReportError(Failure::DialerUnavailable));
    }
}

fn answer(state: &mut State, call: CallId, effects: &mut Vec<Effect>) {
    match state.call_mut(call) {
        Some(known) if known.rings_here() => {
            known.request = Request::Answer;
            effects.push(Effect::Phone(PhoneCommand::Answer(call)));
            effects.push(Effect::Phone(PhoneCommand::MoveAudioToDesktop));
        }
        Some(known) if known.request == Request::Answer => {}
        Some(_) => effects.push(Effect::ReportError(Failure::CallNotRinging)),
        None => effects.push(Effect::ReportError(Failure::NoSuchCall)),
    }
}

fn decline(state: &mut State, call: CallId, effects: &mut Vec<Effect>) {
    match state.call_mut(call) {
        Some(known) if known.rings_here() => {
            known.request = Request::Decline;
            effects.push(Effect::Phone(PhoneCommand::HangUp(call)));
        }
        Some(known) if known.request == Request::Decline => {}
        Some(_) => effects.push(Effect::ReportError(Failure::CallNotRinging)),
        None => effects.push(Effect::ReportError(Failure::NoSuchCall)),
    }
}

fn hang_up(state: &mut State, call: CallId, effects: &mut Vec<Effect>) {
    match state.call_mut(call) {
        Some(known) if matches!(known.request, Request::HangUp | Request::Decline) => {}
        Some(known) => {
            known.request = if known.rings_here() {
                Request::Decline
            } else {
                Request::HangUp
            };
            effects.push(Effect::Phone(PhoneCommand::HangUp(call)));
        }
        None => effects.push(Effect::ReportError(Failure::NoSuchCall)),
    }
}

fn end_call(state: &mut State, call: CallId, effects: &mut Vec<Effect>) {
    let Some(index) = state.calls.iter().position(|known| known.id == call) else {
        return;
    };
    let ended = state.calls.remove(index);
    if ended.has_call_window() {
        state.ended = Some(EndedCall {
            id: ended.id,
            caller: ended.caller,
        });
        effects.push(Effect::StartTimer {
            timer: Timer::CloseCallWindow(call),
            after_ms: CALL_ENDED_MS,
        });
    }
    if state.calls.is_empty() {
        state.codec = None;
        effects.push(Effect::Phone(PhoneCommand::KeepAudioOnPhone));
    }
}

fn command_failed(state: &mut State, command: &PhoneCommand, effects: &mut Vec<Effect>) {
    match *command {
        PhoneCommand::Answer(call) => {
            if let Some(known) = state.call_mut(call)
                && known.request == Request::Answer
            {
                known.request = Request::None;
            }
            effects.push(Effect::ReportError(Failure::AnswerFailed));
            if !state.has_desktop_audio() {
                effects.push(Effect::Phone(PhoneCommand::KeepAudioOnPhone));
            }
        }
        PhoneCommand::HangUp(call) => {
            if let Some(known) = state.call_mut(call)
                && matches!(known.request, Request::HangUp | Request::Decline)
            {
                known.request = Request::None;
            }
            effects.push(Effect::ReportError(Failure::HangUpFailed));
        }
        PhoneCommand::MoveAudioToDesktop => {
            effects.push(Effect::ReportError(Failure::AudioNotMoved));
        }
        // The shell has logged it; the next Call end or connect tries again.
        PhoneCommand::KeepAudioOnPhone => {}
        PhoneCommand::SendTones(_) => effects.push(Effect::ReportError(Failure::TonesNotSent)),
        PhoneCommand::SetMicrophoneVolume(0) => {
            if state.muted() {
                state.muting = Muting::Off;
                effects.push(Effect::ReportError(Failure::MuteFailed));
            }
        }
        PhoneCommand::SetMicrophoneVolume(restore) => {
            // Unmuting the user asked for failed: the microphone is still
            // silent, so say so. An unmute because the audio left the
            // Desktop or the Call ended has no Mute left to show.
            if !state.muted() && state.audio_location() == Some(AudioLocation::Desktop) {
                state.muting = Muting::On { restore };
                effects.push(Effect::ReportError(Failure::UnmuteFailed));
            }
        }
    }
}
