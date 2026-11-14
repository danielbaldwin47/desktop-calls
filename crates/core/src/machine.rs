//! The Call state machine: one function from (state, input) to (new state,
//! effects). Every policy decision lives here.

use crate::call::{
    AudioLocation, Call, CallId, CallState, Caller, Codec, Direction, Request, Transport,
};
use crate::clock::{Deadline, QuietHours, WallClock};
use crate::connection::{ConnectionState, Phone};
use crate::contacts::{CallHistory, Contacts, ContactsState, Unavailable};
use crate::earlier_run::{EarlierCall, EarlierRun};
use crate::matching::{self, ContactMatch, DialerRow};
use crate::messages::{Failure, ProblemKind};
use crate::missed::MissedCalls;

mod audio;
mod call_waiting;
mod calls;
mod calls_off;
mod collapse;
mod desktop;
mod dialling;
mod input;
mod missed_calls;
mod phone;
mod presence;
mod quiet_hours;
mod setup;
mod view;

use audio::{
    activate, hands_free_down, hands_free_up, microphone_volume_changed, move_audio, mute,
    release_mute, restore_audio, still_silenced, transport_changed, unmute,
};
use call_waiting::{end_and_answer, hold_and_answer, swap};
pub use calls::keypad_tones;
use calls::{
    answer, call_added, call_changed, caller_changed, decline, hang_up, phone_ended_call,
    send_tones, timer_fired,
};
use calls_off::CallsOff;
pub use calls_off::CallsOffRecord;
use collapse::{CallWindowForm, follow_call_window, form_input};
use desktop::DesktopChecks;
pub use desktop::{BluetoothPower, DesktopCheck, DesktopFix, FixFailure, SetupProblemsView};
use dialling::{cancel_dial, dial, dial_failed, dialer_input, phone_unpaired, refusal_failure};
pub use input::{
    ConnectFailure, DialProgress, Effect, Input, PairingFailure, PhoneCommand, PhoneRefusal, Timer,
};
use missed_calls::missed_call_input;
use phone::{command_done, command_failed, contacts_sync, phone_paired, pick_up_earlier_run};
use presence::{Presence, forget_phone, hang_up_undelivered, presence_input};
use setup::Setup;
pub use setup::{
    ContactsOutcome, KeybindingStage, ReadingContacts, SetupNotice, SetupStep, SetupWindowView,
};
pub use view::{
    BannerView, CallAudio, CallWindowPhase, CallWindowView, DialerView, HeldCall, Mute,
    RingingCaller, StateLine, Surfaces, WaitingOptions,
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
/// How long the Desktop waits between asking BlueZ to connect the Phone
/// while it is away: after the first failed connect, the second, and every
/// one after. A connect to a Phone out of range fails after BlueZ's page
/// timeout, 5.12 s, and one in range answers within a page scan, 1.28 s,
/// so a Phone back just after a failed connect is connected about 3 + 1.3
/// + 1 s later: inside the 10 s the Desktop promises (spec story 31).
const RECONNECT_WAITS_MS: [u32; 3] = [1_000, 2_000, 3_000];
/// How long the Call Window says the connection to the Phone was lost
/// before it closes, if the Phone isn't back by then.
const CONNECTION_LOST_MS: u32 = 10_000;
/// How long a Call that lost the connection, and that the Phone link gave
/// up on while hands-free was away, may take to show again once hands-free
/// is back. A Call the Phone link still holds then gets 3 s from it to be
/// listed again (ADR-0007), and this timer doesn't start for it.
const LOST_CALL_BACK_MS: u32 = 2_000;
/// How long a Call that rang with no number may wait for the Phone to
/// report one before the Banner says the number is withheld. PipeWire 1.6.9
/// registers the Call on `+CIEV: callsetup,1` and sets the number from the
/// `+CLIP` after the first `RING`; the Simulated Phone's
/// `late-line-identification` quirk sends it 1 s late (ticket #86).
const NUMBER_LATE_MS: u32 = 2_000;

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
    /// How long it was connected, in seconds; `None` if it never was.
    lasted: Option<u64>,
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
    /// The Call whose Answer asked for this Handoff, if one did. Its Answer
    /// comes first in the same batch of Phone commands.
    answering: Option<CallId>,
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

/// The Bluetooth connection to the Phone dropped with this Call live in the
/// Call Window: it carries on on the Phone, and the Call Window says so.
#[derive(Clone, Debug, PartialEq, Eq)]
struct ConnectionLost {
    /// The Call as it was when the connection dropped.
    call: Call,
    /// The Call on hold behind it, as it was.
    held: Option<Call>,
    /// What the Call Window says about it.
    notice: Failure,
    /// Which loss this is, so only its own timer counts.
    loss: u32,
}

/// Everything the Call state machine knows.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "each bool is an independent fact about a different part of the Desktop, not a mode"
)]
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
    /// How many times the Desktop has asked BlueZ to connect a Phone.
    connect_attempts: u32,
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
    /// Why the Phone refused Hold & Answer, End & Answer or Swap: shown
    /// beside the Calls for a while, until they change, or the user asks
    /// again.
    waiting_notice: Option<Failure>,
    /// How many times the Phone has refused those, so only the latest
    /// refusal's timer takes its notice down.
    waiting_notices: u32,
    focus: Focus,
    contacts_sync: ContactsState,
    /// Contacts have been read for this Phone: from the cache, or by a sync
    /// that finished.
    contacts_held: bool,
    contacts: Contacts,
    call_history: CallHistory,
    dialer: Option<Dialer>,
    /// How many times the Dialer has opened.
    dialer_openings: u32,
    /// The other party of each recent Call, oldest first.
    recent_calls: Vec<(CallId, Caller)>,
    missed: MissedCalls,
    do_not_disturb: bool,
    presence: Presence,
    connection_lost: Option<ConnectionLost>,
    /// How many times the connection was lost mid-Call.
    losses: u32,
    /// The open Setup Window.
    setup: Option<Setup>,
    /// The Setup Window has had its one chance to open by itself.
    setup_offered: bool,
    /// How many times the Setup Window has opened.
    setup_openings: u32,
    /// The address of the Phone the user chose last in the Setup Window.
    /// Once there is one, no other Phone is followed, and it outlives the
    /// Phone being unpaired, so a restart never picks another silently.
    chosen_phone: Option<String>,
    /// Every Phone BlueZ has reported paired, followed or not, so the Setup
    /// Window can offer the others when none is followed.
    paired_phones: Vec<Phone>,
    /// Phones nearby that aren't paired with the Desktop, in the order
    /// BlueZ found them.
    nearby_phones: Vec<Phone>,
    /// Incoming Calls that rang with no number, while the Phone may still
    /// report one.
    awaiting_number: Vec<CallId>,
    /// What the setup checks found of the Desktop.
    desktop: DesktopChecks,
    /// Whether the Call Window is collapsed to the Call Pill, and what keeps
    /// it expanded.
    form: CallWindowForm,
    /// Whether calls are on or off. It outlives the Phone being unpaired.
    calls_off: CallsOff,
    /// The wall clock as the shell last reported it, while the Desktop was
    /// awake.
    clock: Option<WallClock>,
    /// The Quiet Hours the user set, if any.
    quiet_hours: Option<QuietHours>,
    /// While calls are on through Quiet Hours the user turned them on in:
    /// when those Quiet Hours next begin.
    kept_on_until: Option<Deadline>,
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

    /// Whether the Phone is connected. While calls are off nothing connects
    /// it, so it is never "connecting".
    #[must_use]
    pub fn connection_state(&self) -> &ConnectionState {
        match self.connection {
            ConnectionState::Connecting if !self.calls_on() => &ConnectionState::Disconnected,
            _ => &self.connection,
        }
    }

    /// The Phone's Calls, oldest first.
    #[must_use]
    pub fn calls(&self) -> &[Call] {
        &self.calls
    }

    /// Whether the Desktop ringtone should be playing: a Call rings here, no
    /// other Call is live, so a Waiting Call never rings over a live one, and
    /// Do Not Disturb is off. The Banner shows either way.
    #[must_use]
    pub fn ringing(&self) -> bool {
        !self.do_not_disturb
            && !self.calls_out_of_reach()
            && self.calls.iter().any(Call::rings_here)
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
    /// The Desktop's own come first: without them, the Phone's can't be
    /// put right.
    pub fn problems(&self) -> Vec<Problem> {
        let mut kinds = self.desktop.problems();
        if self.presence.pairing_lost {
            kinds.push(ProblemKind::PairingLost);
        } else {
            match self.contacts_sync {
                ContactsState::Denied => kinds.push(ProblemKind::ContactsDenied),
                ContactsState::Unavailable(Unavailable::PromptTimedOut) => {
                    kinds.push(ProblemKind::ContactsPromptUnanswered);
                }
                _ => {}
            }
        }
        kinds.into_iter().map(|kind| Problem { kind }).collect()
    }

    /// Whether a Problem of the Desktop's own keeps the Phone from
    /// connecting for Calls.
    #[must_use]
    pub fn desktop_keeps_phone_away(&self) -> bool {
        self.desktop
            .problems()
            .iter()
            .any(|kind| *kind != ProblemKind::ContactsSupportMissing)
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
        if !self.calls_on() && self.phone.is_some() {
            return Some(Failure::CallsOff);
        }
        match self.connection {
            ConnectionState::NotSetUp => Some(Failure::PhoneNotSetUp),
            ConnectionState::Connecting => Some(Failure::PhoneConnecting),
            ConnectionState::Disconnected => Some(Failure::PhoneOutOfRange),
            ConnectionState::PairingLost => Some(Failure::PhonePairingLost),
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
    /// on, leaving ready only if Contacts were read before. Recent Calls a
    /// first sync read before it was cut short don't make it ready.
    fn held_contacts_state(&self) -> ContactsState {
        match self.contacts_sync {
            ContactsState::Syncing
            | ContactsState::PermissionNeeded { .. }
            | ContactsState::Unavailable(Unavailable::Failed)
                if self.contacts_held =>
            {
                ContactsState::Ready
            }
            // A failure as the link dropped is the sync cut short too.
            ContactsState::Syncing
            | ContactsState::PermissionNeeded { .. }
            | ContactsState::Unavailable(Unavailable::Failed) => {
                ContactsState::Unavailable(Unavailable::NotSynced)
            }
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
        if self.connection_lost.is_some()
            && matches!(
                input,
                Input::SetMuted(true)
                    | Input::MoveAudio(_)
                    | Input::SendTones(_)
                    | Input::HoldAndAnswer
                    | Input::EndAndAnswer
                    | Input::Swap
            )
        {
            return Some(Failure::ConnectionLost);
        }
        match input {
            Input::TurnCallsOff { until } => self.calls_off_refusal(*until),
            Input::SetMuted(true) if !self.muted() => match self.audio_location() {
                None => Some(Failure::NoLiveCall),
                Some(AudioLocation::Phone) => Some(Failure::MuteUnavailable),
                Some(AudioLocation::Desktop) => None,
            },
            Input::MoveAudio(_) if !self.calls.iter().any(|call| call.state.is_live()) => {
                Some(Failure::NoLiveCall)
            }
            Input::HoldAndAnswer | Input::EndAndAnswer if self.waiting_call().is_none() => {
                Some(Failure::NoWaitingCall)
            }
            Input::HoldAndAnswer | Input::EndAndAnswer
                if self.waiting_options() == WaitingOptions::Answer =>
            {
                Some(Failure::NothingToHoldOrEnd)
            }
            Input::HoldAndAnswer if self.waiting_options() == WaitingOptions::End => {
                Some(if self.has_held_call() {
                    Failure::AlreadyHolding
                } else {
                    Failure::NotConnectedYet
                })
            }
            Input::Swap if !self.has_held_call() => Some(Failure::NoHeldCall),
            // `AT+CHLD=2` would answer the Waiting Call instead.
            Input::Swap if self.has_waiting_call() => Some(Failure::WaitingCallFirst),
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

    /// Keeps `paired_phones` as BlueZ reports Phones paired and unpaired.
    fn note_paired_phones(&mut self, input: &Input) {
        match input {
            Input::PhonePaired(phone) => {
                match self
                    .paired_phones
                    .iter_mut()
                    .find(|known| known.address.eq_ignore_ascii_case(&phone.address))
                {
                    Some(known) => *known = phone.clone(),
                    None => self.paired_phones.push(phone.clone()),
                }
            }
            Input::PhoneUnpaired { address } => self
                .paired_phones
                .retain(|known| !known.address.eq_ignore_ascii_case(address)),
            _ => {}
        }
    }

    /// Whether the user's choice of Phone allows following `phone`.
    fn may_follow(&self, phone: &Phone) -> bool {
        self.chosen_phone
            .as_ref()
            .is_none_or(|chosen| chosen.eq_ignore_ascii_case(&phone.address))
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
    setup::follow_connection(&mut state, &mut effects);
    setup::follow_contacts(&mut state, &mut effects);
    follow_call_window(&mut state, surfaces.call_window.as_ref(), &mut effects);
    calls_off::apply_pending(&mut state, &mut effects);
    quiet_hours::apply(&mut state, &mut effects);
    calls_off::hold_back_connects(&state, &mut effects);
    (state, effects)
}

#[expect(
    clippy::too_many_lines,
    reason = "one arm per kind of Input, each handing off to its own function: the dispatch reads best as one table"
)]
fn decide(mut state: State, input: Input) -> (State, Vec<Effect>) {
    let mut effects = Vec::new();
    state.note_paired_phones(&input);
    match input {
        // Desktop Calls works with one Phone: the first one paired.
        Input::EarlierRun(earlier) => pick_up_earlier_run(&mut state, earlier, &mut effects),
        // Except a Phone the user pairs in the Setup Window, which it
        // switches to.
        Input::PhonePaired(phone) if state.setup_pairs(&phone.address) => {
            setup::paired(&mut state, phone, &mut effects);
        }
        Input::ChosenPhoneKept { address } => state.chosen_phone = Some(address),
        // Whichever Phone BlueZ lists first is followed, unless the user
        // chose one: then only that one, and with it unpaired the Setup
        // Window opens once BlueZ has listed everything.
        Input::PhonePaired(phone) if state.phone.is_none() && state.may_follow(&phone) => {
            phone_paired(&mut state, phone, &mut effects);
        }
        Input::PhoneUnpaired { address } if state.is_our_phone(&address) => return unpair(&state),
        Input::PhoneConnected { address } if state.is_our_phone(&address) => {
            hands_free_up(&mut state, &mut effects);
        }
        Input::PhoneDisconnected { address } if state.is_our_phone(&address) => {
            hands_free_down(&mut state, &mut effects);
        }
        input @ (Input::ConnectPhoneDone { .. } | Input::ConnectPhoneFailed { .. }) => {
            presence_input(&mut state, input, &mut effects);
        }
        input @ (Input::PhonePresent { .. }
        | Input::PhoneAbsent { .. }
        | Input::Suspending
        | Input::Resumed) => presence_input(&mut state, input, &mut effects),
        input @ Input::CallAdded { .. } => phone_added_call(&mut state, input, &mut effects),
        Input::CallChanged {
            call,
            state: call_state,
            at,
        } => call_changed(&mut state, call, call_state, at, &mut effects),
        Input::CallerChanged { call, caller } => caller_changed(&mut state, call, caller),
        Input::CallEnded { call, at } => phone_ended_call(&mut state, call, at, &mut effects),
        Input::HangUpUndelivered(call) => hang_up_undelivered(&mut state, call, &mut effects),
        input @ (Input::ContactsLoaded { .. }
        | Input::ContactsSynced { .. }
        | Input::ContactsPermissionNeeded { .. }
        | Input::ContactsSyncProgress(_)
        | Input::ContactsSyncFailed(_)) => contacts_sync(&mut state, input, &mut effects),
        input @ (Input::CallHistoryRefreshed(_)
        | Input::CallHistoryRefreshFailed
        | Input::OpenMissedCall(_)
        | Input::CallBack(_)
        | Input::MissedCallDismissed(_)) => missed_call_input(&mut state, input, &mut effects),
        Input::TransportChanged(transport) => {
            transport_changed(&mut state, transport, &mut effects);
        }
        Input::CodecChanged(codec) => codec_changed(&mut state, codec),
        Input::MicrophoneVolumeChanged(volume) => microphone_volume_changed(&mut state, volume),
        Input::SetMuted(true) => mute(&mut state, &mut effects),
        Input::SetMuted(false) => unmute(&mut state, &mut effects),
        Input::SendTones(typed) => send_tones(&state, typed, &mut effects),
        Input::Answer(call) => answer(&mut state, call, &mut effects),
        Input::Decline(call) => decline(&mut state, call, &mut effects),
        Input::HangUp(call) => hang_up(&mut state, call, &mut effects),
        Input::HoldAndAnswer => hold_and_answer(&mut state, &mut effects),
        Input::EndAndAnswer => end_and_answer(&mut state, &mut effects),
        Input::Swap => swap(&mut state, &mut effects),
        Input::Dial(text) => dial(&mut state, &text, &mut effects),
        Input::CancelDial => cancel_dial(&mut state, &mut effects),
        Input::MoveAudio(location) => move_audio(&mut state, location, &mut effects),
        Input::DialFailed(refusal) => {
            dial_failed(&mut state, refusal_failure(refusal), &mut effects);
        }
        Input::TimerFired(timer) => timer_fired(&mut state, timer, &mut effects),
        Input::Activate => activate(&mut state),
        Input::ReleaseFocus => state.focus = Focus::None,
        input @ (Input::CollapseCallWindow
        | Input::ExpandCallWindow
        | Input::IndicatorActivated
        | Input::PointerOverCallWindow(_)
        | Input::CallWindowKeyboard(_)
        | Input::CallWindowUsed
        | Input::KeypadShown(_)) => form_input(&mut state, &input),
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
        input @ (Input::PairedPhonesListed
        | Input::OpenSetupWindow
        | Input::CloseSetupWindow
        | Input::SetupReady { .. }
        | Input::SetupUnavailable
        | Input::PhoneNearby(_)
        | Input::PhoneNotNearby { .. }
        | Input::ChoosePhone { .. }
        | Input::PairingCode { .. }
        | Input::ConfirmPairing
        | Input::RejectPairing
        | Input::PairingFailed(_)
        | Input::HandsFreeRefused
        | Input::HandsFreeUnreachable
        | Input::RetryHandsFree
        | Input::RetryContacts
        | Input::SkipContacts
        | Input::ContinueContacts
        | Input::KeybindingChecked(_)
        | Input::AddKeybinding
        | Input::ConfirmKeybinding
        | Input::CancelKeybinding
        | Input::KeybindingAdded
        | Input::KeybindingNotAdded
        | Input::KeybindingConfigChanged(_)) => setup::setup_input(&mut state, input, &mut effects),
        Input::DesktopChecked(check) => desktop::checked(&mut state, check, &mut effects),
        Input::Fix(problem) => desktop::fix(&mut state, problem, &mut effects),
        Input::FixDone(fix) => desktop::fix_done(&mut state, fix, &mut effects),
        Input::FixFailed(failure) => desktop::fix_failed(&mut state, failure),
        input @ (Input::CallsOffKept(_)
        | Input::TurnCallsOff { .. }
        | Input::ClockTicked(_)
        | Input::TurnCallsOn
        | Input::KeepCallsOn
        | Input::PhoneBlocked { .. }
        | Input::PhoneBlockFailed { .. }) => {
            calls_off::calls_off_input(&mut state, input, &mut effects);
        }
        input @ (Input::QuietHoursKept(_) | Input::SetQuietHours(_) | Input::ClearQuietHours) => {
            quiet_hours::quiet_hours_input(&mut state, &input, &mut effects);
        }
        Input::PhonePaired(_)
        | Input::PhoneUnpaired { .. }
        | Input::PhoneConnected { .. }
        | Input::PhoneDisconnected { .. } => {}
    }
    (state, effects)
}

/// The Phone reported a Call: `input` is `Input::CallAdded`.
fn phone_added_call(state: &mut State, input: Input, effects: &mut Vec<Effect>) {
    let Input::CallAdded {
        call,
        state: call_state,
        caller,
        at,
    } = input
    else {
        return;
    };
    let call = Call {
        id: call,
        direction: Direction::from_first_state(call_state),
        state: call_state,
        caller,
        active_since: call_state.is_connected().then_some(at),
        added_at: at,
        request: Request::None,
    };
    await_number(state, &call, effects);
    call_added(state, call, effects);
}

fn codec_changed(state: &mut State, codec: Option<Codec>) {
    state.reported_codec = codec;
    if codec.is_some() && state.transport.is_up() {
        state.codec = codec;
    }
}

/// An incoming Call that rings with no number may get one a moment later:
/// until [`NUMBER_LATE_MS`] has passed, the Banner doesn't say the number
/// is withheld.
fn await_number(state: &mut State, call: &Call, effects: &mut Vec<Effect>) {
    if call.caller == Caller::Withheld && call.direction == Direction::Incoming {
        state.awaiting_number.push(call.id);
        effects.push(Effect::StartTimer {
            timer: Timer::NumberLate(call.id),
            after_ms: NUMBER_LATE_MS,
        });
    }
}

/// The Phone was unpaired: the Desktop forgets it and its Calls, and keeps
/// its own settings, whether it is asleep, and the Setup Window.
fn unpair(state: &State) -> (State, Vec<Effect>) {
    let mut effects = vec![Effect::DetachPhone];
    phone_unpaired(state, &mut effects);
    calls_off::phone_forgotten(state, &mut effects);
    (forget_phone_keeping_desktop(state), effects)
}

/// The state once the Desktop stops following its Phone: nothing of the
/// Phone is left, and the Desktop's own settings, whether it is asleep and
/// the Setup Window stay.
fn forget_phone_keeping_desktop(state: &State) -> State {
    State {
        do_not_disturb: state.do_not_disturb,
        setup: state.setup.clone(),
        setup_offered: state.setup_offered,
        setup_openings: state.setup_openings,
        chosen_phone: state.chosen_phone.clone(),
        paired_phones: state.paired_phones.clone(),
        nearby_phones: state.nearby_phones.clone(),
        desktop: state.desktop.clone(),
        clock: state.clock,
        quiet_hours: state.quiet_hours,
        kept_on_until: state.kept_on_until,
        ..forget_phone(state)
    }
}
