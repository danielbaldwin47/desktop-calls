//! The Call state machine: one function from (state, input) to (new state,
//! effects). Every policy decision lives here.

use crate::call::{
    AudioLocation, Call, CallId, CallState, Caller, Codec, Direction, Request, Transport,
};
use crate::connection::{ConnectionState, Phone};
use crate::contacts::{CallHistory, Contacts, ContactsState, Unavailable};
use crate::earlier_run::{EarlierCall, EarlierRun};
use crate::matching::{self, ContactMatch, DialerRow};
use crate::messages::{Failure, ProblemKind};
use crate::missed::MissedCalls;

mod audio;
mod calls;
mod dialling;
mod input;
mod missed_calls;
mod phone;
mod view;

use audio::{
    activate, hands_free_down, hands_free_up, microphone_volume_changed, move_audio, mute,
    release_mute, restore_audio, still_silenced, transport_changed, unmute,
};
pub use calls::keypad_tones;
use calls::{
    answer, call_added, call_changed, decline, hang_up, phone_ended_call, send_tones, timer_fired,
};
use dialling::{cancel_dial, dial, dial_failed, dialer_input, phone_unpaired, refusal_failure};
pub use input::{DialProgress, Effect, Input, PhoneCommand, PhoneRefusal, Timer};
use missed_calls::missed_call_input;
use phone::{command_done, command_failed, contacts_sync, pick_up_earlier_run};
pub use view::{
    BannerView, CallAudio, CallWindowPhase, CallWindowView, DialerView, Mute, Surfaces,
};

/// How many recent Calls the Desktop remembers the other party of, so
/// `OpenDialer` can select one.
const RECENT_CALLS: usize = 16;

/// How long the Call Window shows "Call ended" before it closes.
const CALL_ENDED_MS: u32 = 2_000;
/// How long Calls from the earlier run may take to show again after
/// startup: the Phone link lists them as soon as the Phone is attached.
/// A Call seen later is a new one.
const FORGET_EARLIER_RUN_MS: u32 = 30_000;
/// How long a refusal or failure of a Call placed here stays on the Call
/// Window: long enough to read a sentence.
const NOTICE_MS: u32 = 6_000;
/// How long the Phone may take to report a Call it was asked to dial. The
/// Pixel's took under a second.
const DIAL_TIMEOUT_MS: u32 = 15_000;
/// How long the Phone may take to bring the audio here, from the Desktop
/// asking until the Call Window says it didn't come. The Pixel took under
/// 0.5 s 12 times in 15, 2.8 s and 2.9 s once each, and ignored `Activate()`
/// once (ADR-0005, ticket #13).
const HANDOFF_TIMEOUT_MS: u32 = 3_000;
/// How long after `Activate()` returns the Phone may take to start opening
/// the audio link before the Desktop asks once more. The Pixel started
/// within 62 to 313 ms in 8 of 8 timed Handoffs later in a Call, but
/// ignored 3 of 4 `Activate()`s sent within a second of the Call going
/// active, and the second one then worked (tickets #13 and #70).
const HANDOFF_NOT_STARTED_MS: u32 = 1_000;
/// How long a Handoff to the Phone may take, from the Desktop asking until
/// hands-free and the Call are back. On the Pixel, hands-free was back 0.50
/// to 0.96 s after it dropped and the Call 30 to 60 ms after that, 5 times
/// in 5 (ticket #14's prototype).
const HANDOFF_TO_PHONE_TIMEOUT_MS: u32 = 5_000;

/// The hands-free microphone gain to restore when the Phone's own value is
/// unusable: 15, the top of the hands-free range (0 to 15), which is also
/// PipeWire's default (`bluez5.hfp-hf.default-mic-volume`).
const DEFAULT_MICROPHONE_VOLUME: u8 = 15;
/// The top of the hands-free gain range.
const MAX_MICROPHONE_VOLUME: u8 = 15;

/// Whether the Desktop muted the microphone, and the gain to put back.
///
/// Mute shows only what the Phone has done: a change it hasn't confirmed
/// yet shows as the state before it. Showing Muted while the other person
/// can still hear would be the worst thing Mute could do.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Muting {
    #[default]
    Off,
    /// The Phone was asked for a gain of 0 and hasn't confirmed: unmuted.
    Silencing {
        restore: u8,
    },
    On {
        restore: u8,
    },
    /// The Phone was asked for `restore` and hasn't confirmed: muted.
    Restoring {
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

/// A Call the user placed here that the Phone hasn't reported yet.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Dialling {
    number: String,
    attempt: u32,
    /// The user hung up already; the Call is hung up as soon as it appears.
    cancelled: bool,
}

/// A dial attempt the user hung up before the Phone reported its Call: the
/// Call Window shows "Call ended" for it for a while.
#[derive(Clone, Debug, PartialEq, Eq)]
struct HungUpDial {
    attempt: u32,
    caller: Caller,
}

/// A failure of a dial attempt, shown on the Call Window for a while.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Notice {
    failure: Failure,
    attempt: u32,
    /// Who the user tried to Call, when no other Call is in the window: the
    /// window then shows this attempt as not placed.
    not_placed: Option<Caller>,
}

/// The open Dialer.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Dialer {
    query: String,
    rows: Vec<DialerRow>,
    selected: usize,
    notice: Option<Failure>,
    opened: u32,
}

/// The Desktop asked the Phone to bring the audio here, and it hasn't yet.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct MovingAudio {
    /// Which Handoff this is, so only its own timers count.
    handoff: u32,
    /// The Phone has already been asked a second time.
    retried: bool,
}

/// The Desktop asked to move the audio to the Phone, by dropping hands-free
/// and connecting it again (ADR-0005), and hands-free or the Call isn't
/// back yet. Meanwhile the Phone still shows as connected: it is in range,
/// and hands-free is coming straight back.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct MovingToPhone {
    /// Which Handoff this is, so only its own timer counts.
    handoff: u32,
    /// Hands-free has dropped, taking the audio to the Phone.
    dropped: bool,
    /// Hands-free is back since it dropped.
    phone_back: bool,
    /// The Phone has reported the Call again since hands-free dropped.
    call_back: bool,
}

/// Hands-free dropped mid-Call with the audio on the Desktop, as it does
/// when `WirePlumber` restarts: the audio comes back here once hands-free and
/// the Call are both back.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct AudioToRestore {
    /// The Phone has reported the Call again.
    call_back: bool,
}

/// Everything the Call state machine knows.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct State {
    phone: Option<Phone>,
    connection: ConnectionState,
    /// The Phone connected, and was asked to keep the next Call's audio on
    /// the Phone; it shows as connected once it has.
    keeping_audio_on_phone: bool,
    calls: Vec<Call>,
    transport: Transport,
    /// The Phone's latest codec report, whatever the transport is doing.
    reported_codec: Option<Codec>,
    /// The codec the Desktop's audio negotiated during the current Calls.
    codec: Option<Codec>,
    ended: Option<EndedCall>,
    dialling: Option<Dialling>,
    notice: Option<Notice>,
    hung_up_dial: Option<HungUpDial>,
    /// The Call of a dial the user hung up first: never shown, and hung up
    /// as soon as it appeared.
    unwanted: Option<CallId>,
    /// How many times the user has asked to dial.
    dial_attempts: u32,
    /// The Phone's latest report of its hands-free microphone gain.
    microphone_volume: Option<u8>,
    muting: Muting,
    /// The gain the Desktop last silenced the microphone from, until the
    /// Phone reports it given back. While the Phone reports 0 with the audio
    /// on the Desktop, the microphone shows as muted, from this gain: hands-free
    /// may have gone before the Desktop could give it back.
    silenced: Option<u8>,
    /// The earlier run's Calls that haven't shown again yet.
    earlier_calls: Vec<EarlierCall>,
    moving_audio: Option<MovingAudio>,
    moving_to_phone: Option<MovingToPhone>,
    restore_audio: Option<AudioToRestore>,
    /// How many Handoffs the Desktop has started, either way.
    handoffs: u32,
    /// Why the audio isn't where the user asked: shown beside the Call until
    /// the audio moves, the user asks again, or the Call ends.
    audio_notice: Option<Failure>,
    focus: Focus,
    contacts_sync: ContactsState,
    contacts: Contacts,
    call_history: CallHistory,
    dialer: Option<Dialer>,
    /// How many times the Dialer has opened.
    dialer_openings: u32,
    /// The other party of each recent Call, oldest first.
    recent_calls: Vec<(CallId, Caller)>,
    missed: MissedCalls,
    do_not_disturb: bool,
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

    /// What the next run needs if this one stops now: each connected Call,
    /// and each Call of the earlier run not seen again yet, with the
    /// microphone gain to give back if the Desktop silenced it. The shell
    /// keeps it for the next run whenever it changes, so that a run started
    /// after a crash picks up the Calls as they were.
    #[must_use]
    pub fn for_next_run(&self) -> EarlierRun {
        let mut calls: Vec<EarlierCall> = self
            .calls
            .iter()
            .filter_map(|call| {
                Some(EarlierCall {
                    caller: call.caller.clone(),
                    direction: call.direction,
                    active_since: call.active_since?,
                })
            })
            .collect();
        calls.extend(self.earlier_calls.iter().cloned());
        let restore_microphone_volume = match self.muting {
            Muting::Silencing { restore }
            | Muting::On { restore }
            | Muting::Restoring { restore } => Some(restore),
            Muting::Off => self.silenced,
        };
        EarlierRun {
            calls,
            restore_microphone_volume,
        }
    }

    /// Whether the Desktop has muted its microphone for the Call.
    #[must_use]
    pub fn muted(&self) -> bool {
        matches!(self.muting, Muting::On { .. } | Muting::Restoring { .. })
    }

    /// Whether the Desktop's Do Not Disturb is on. Off until said otherwise,
    /// as on a Desktop without one.
    #[must_use]
    pub fn do_not_disturb(&self) -> bool {
        self.do_not_disturb
    }
}

impl State {
    /// Why a Call can't be placed now, if it can't.
    fn dial_refusal(&self) -> Option<Failure> {
        match self.connection {
            ConnectionState::NotSetUp => Some(Failure::PhoneNotSetUp),
            ConnectionState::Connecting => Some(Failure::PhoneConnecting),
            ConnectionState::Disconnected => Some(Failure::PhoneOutOfRange),
            ConnectionState::Connected if !self.calls.is_empty() || self.dialling.is_some() => {
                Some(Failure::CallInProgress)
            }
            ConnectionState::Connected => None,
        }
    }

    /// Whether `a` and `b` are the same number, however each is written.
    fn same_number(&self, a: &Caller, b: &Caller) -> bool {
        let (Caller::Number(a_number), Caller::Number(b_number)) = (a, b) else {
            return false;
        };
        let dialable = matching::dialable(a_number);
        if dialable.is_some() && dialable == matching::dialable(b_number) {
            return true;
        }
        let contact = self.contact(a);
        contact.is_some() && contact == self.contact(b)
    }

    /// Whether `a` and `b` are the same caller: the same number, or both
    /// withheld.
    fn same_caller(&self, a: &Caller, b: &Caller) -> bool {
        (*a == Caller::Withheld && *b == Caller::Withheld) || self.same_number(a, b)
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
        self.phone_is_here() && self.contacts_sync != ContactsState::Denied
    }

    /// Whether hands-free is up, including while the Phone is still being
    /// asked to keep the next Call's audio before it shows as connected.
    fn phone_is_here(&self) -> bool {
        self.connection == ConnectionState::Connected || self.keeping_audio_on_phone
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
            Input::MoveAudio(_) if !self.calls.iter().any(|call| call.state.is_live()) => {
                Some(Failure::NoLiveCall)
            }
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
    if state.audio_location() == Some(AudioLocation::Desktop) {
        still_silenced(&mut state);
    } else {
        release_mute(&mut state, &mut effects);
    }
    (state, effects)
}

fn decide(mut state: State, input: Input) -> (State, Vec<Effect>) {
    let mut effects = Vec::new();
    match input {
        // Desktop Calls works with one Phone: the first one paired.
        Input::EarlierRun(earlier) => pick_up_earlier_run(&mut state, earlier, &mut effects),
        Input::PhonePaired(phone) if state.phone.is_none() => {
            effects.push(Effect::AttachPhone {
                address: phone.address.clone(),
            });
            state.phone = Some(phone);
            state.connection = ConnectionState::Connecting;
        }
        Input::PhoneUnpaired { address } if state.is_our_phone(&address) => return unpair(&state),
        Input::PhoneConnected { address } if state.is_our_phone(&address) => {
            hands_free_up(&mut state, &mut effects);
        }
        Input::PhoneDisconnected { address } if state.is_our_phone(&address) => {
            hands_free_down(&mut state, &mut effects);
        }
        Input::CallAdded {
            call,
            state: call_state,
            caller,
            at,
        } => {
            let call = Call {
                id: call,
                direction: Direction::from_first_state(call_state),
                state: call_state,
                caller,
                active_since: call_state.is_connected().then_some(at),
                added_at: at,
                request: Request::None,
            };
            call_added(&mut state, call, &mut effects);
        }
        Input::CallChanged {
            call,
            state: call_state,
            at,
        } => call_changed(&mut state, call, call_state, at, &mut effects),
        Input::CallEnded { call } => phone_ended_call(&mut state, call, &mut effects),
        input @ (Input::ContactsLoaded { .. }
        | Input::ContactsSynced { .. }
        | Input::ContactsPermissionNeeded { .. }
        | Input::ContactsSyncFailed(_)) => contacts_sync(&mut state, input, &mut effects),
        input @ (Input::CallHistoryRefreshed(_)
        | Input::CallHistoryRefreshFailed
        | Input::OpenMissedCall(_)
        | Input::CallBack(_)
        | Input::MissedCallDismissed(_)) => missed_call_input(&mut state, input, &mut effects),
        Input::TransportChanged(transport) => {
            transport_changed(&mut state, transport, &mut effects);
        }
        Input::CodecChanged(codec) => {
            state.reported_codec = codec;
            if codec.is_some() && state.transport.is_up() {
                state.codec = codec;
            }
        }
        Input::MicrophoneVolumeChanged(volume) => microphone_volume_changed(&mut state, volume),
        Input::SetMuted(true) => mute(&mut state, &mut effects),
        Input::SetMuted(false) => unmute(&mut state, &mut effects),
        Input::SendTones(typed) => send_tones(&state, typed, &mut effects),
        Input::Answer(call) => answer(&mut state, call, &mut effects),
        Input::Decline(call) => decline(&mut state, call, &mut effects),
        Input::HangUp(call) => hang_up(&mut state, call, &mut effects),
        Input::Dial(text) => dial(&mut state, &text, &mut effects),
        Input::CancelDial => cancel_dial(&mut state, &mut effects),
        Input::MoveAudio(location) => move_audio(&mut state, location, &mut effects),
        Input::DialFailed(refusal) => {
            dial_failed(&mut state, refusal_failure(refusal), &mut effects);
        }
        Input::TimerFired(timer) => timer_fired(&mut state, timer, &mut effects),
        Input::Activate => activate(&mut state),
        Input::ReleaseFocus => state.focus = Focus::None,
        input @ (Input::OpenDialer { .. }
        | Input::DialerQuery(_)
        | Input::DialerMove(_)
        | Input::DialerEnter
        | Input::DialerChoose(_)
        | Input::CloseDialer) => dialer_input(&mut state, input, &mut effects),
        Input::PhoneCommandDone(command) => {
            command_done(&mut state, &command, &mut effects);
            restore_audio(&mut state, &mut effects);
        }
        Input::PhoneCommandFailed(command) => command_failed(&mut state, &command, &mut effects),
        Input::DoNotDisturbChanged(on) => state.do_not_disturb = on,
        Input::PhonePaired(_)
        | Input::PhoneUnpaired { .. }
        | Input::PhoneConnected { .. }
        | Input::PhoneDisconnected { .. } => {}
    }
    (state, effects)
}

/// The Phone was unpaired: the Desktop forgets it and its Calls, and keeps
/// its own settings.
fn unpair(state: &State) -> (State, Vec<Effect>) {
    let mut effects = vec![Effect::DetachPhone];
    phone_unpaired(state, &mut effects);
    let unpaired = State {
        do_not_disturb: state.do_not_disturb,
        ..State::default()
    };
    (unpaired, effects)
}
