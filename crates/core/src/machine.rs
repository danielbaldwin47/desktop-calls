//! The Call state machine: one function from (state, input) to (new state,
//! effects). Every policy decision lives here.

use crate::call::{
    AudioLocation, Call, CallId, CallState, Caller, Codec, Direction, Request, Timestamp, Transport,
};
use crate::connection::{ConnectionState, Phone};
use crate::contacts::{CallHistory, Contacts, ContactsState, SyncFailure, Unavailable};
use crate::earlier_run::{EarlierCall, EarlierRun};
use crate::matching::{self, ContactMatch, DialerRow};
use crate::messages::{Failure, ProblemKind};

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
/// How long the Phone may take to bring the audio here after `Activate()`
/// before the Desktop asks once more, and then before it gives up. The
/// Pixel took under 0.25 s 5 times in 7, 2.9 s once, and ignored it once
/// (ADR-0005).
const HANDOFF_TIMEOUT_MS: u32 = 3_000;

/// The hands-free microphone gain to restore when the Phone's own value is
/// unusable: 15, the top of the hands-free range (0 to 15), which is also
/// PipeWire's default (`bluez5.hfp-hf.default-mic-volume`).
const DEFAULT_MICROPHONE_VOLUME: u8 = 15;
/// The top of the hands-free gain range.
const MAX_MICROPHONE_VOLUME: u8 = 15;

/// Something that happened, as the shell adapters report it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Input {
    /// What the run before this one left, at startup.
    EarlierRun(EarlierRun),
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
    /// The user asked to Call this number or `tel:` link, as typed.
    Dial(String),
    /// The user hung up the Call they placed before the Phone reported it.
    CancelDial,
    /// The user asked to move the Calls' audio here or to the Phone.
    MoveAudio(AudioLocation),
    /// The Phone refused to dial.
    DialFailed(PhoneRefusal),
    /// The user asked to mute (`true`) or unmute the Desktop's microphone.
    SetMuted(bool),
    /// The user typed or pasted these keypad tones.
    SendTones(String),
    /// The user pressed the Desktop Calls keybinding: do the most useful
    /// thing for what is happening now.
    Activate,
    /// The user let go of the surface that has keyboard focus (Escape).
    ReleaseFocus,
    /// Open the Dialer with `query` typed, selecting the Call History row of
    /// Call `select_call` if it has one.
    OpenDialer {
        query: String,
        select_call: Option<CallId>,
    },
    /// What is typed in the Dialer changed.
    DialerQuery(String),
    /// Move the Dialer's selection by this many rows: down if positive, up
    /// if negative, stopping at either end.
    DialerMove(i32),
    /// Enter in the Dialer: Call the selected row.
    DialerEnter,
    /// The user clicked the Dialer's row at this index: Call it.
    DialerChoose(usize),
    /// Escape in the Dialer: close it.
    CloseDialer,
    /// The Phone did this command.
    PhoneCommandDone(PhoneCommand),
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
    /// Let the Phone bring the next Call's audio to the Desktop.
    AllowAudioOnDesktop,
    /// Place a Call to this number, already checked as dialable.
    Dial(String),
    /// Keep the next Call's audio on the Phone unless the Desktop asks for it.
    KeepAudioOnPhone,
    /// Set the hands-free microphone gain: 0 mutes.
    SetMicrophoneVolume(u8),
    /// Play these keypad tones (digits, `*` and `#`) to the other party, in
    /// order.
    SendTones(String),
}

/// Why the Phone refused to dial, as its Telephony service reports it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PhoneRefusal {
    /// The Phone can't dial the number.
    InvalidFormat,
    /// The Phone's Calls are in a state that doesn't allow dialling.
    InvalidState,
    /// The Phone is busy with another command.
    InProgress,
    /// The Phone doesn't support dialling from the Desktop.
    NotSupported,
    /// No mobile network: no service, or emergency calls only.
    NoNetwork,
    /// The mobile network timed out.
    NetworkTimeout,
    /// Any other failure, or none given.
    Failed,
}

/// A timer the state machine can ask for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Timer {
    /// Close the Call Window that shows this ended Call.
    CloseCallWindow(CallId),
    /// Take down what the Call Window shows of the dial attempt with this
    /// number: its refusal or failure, or "Call ended" once the user hung
    /// up.
    DismissDial(u32),
    /// Give up on the Phone reporting the Call of this dial attempt.
    DialTimedOut(u32),
    /// The audio hasn't come here since this attempt to bring it here.
    HandoffTimedOut(u32),
    /// Calls from the earlier run that haven't shown again by now aren't
    /// live any more.
    ForgetEarlierRun,
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
    pub dialer: Option<DialerView>,
}

/// The Dialer's content. While open it has keyboard focus.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DialerView {
    /// What is typed.
    pub query: String,
    /// Who to Call, best first; with nothing typed, the Call History.
    pub rows: Vec<DialerRow>,
    /// The row Enter Calls, while there are rows.
    pub selected: Option<usize>,
    /// Why Enter can't Call the selected row, or why what is typed can't be
    /// dialled.
    pub notice: Option<Failure>,
    /// Different each time the Dialer opens, so the shell knows to put
    /// `query` in the Dialer's text entry afresh, and leave it alone while
    /// the user types.
    pub opened: u32,
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
    /// The Call shown, or `None` for a Call placed here that the Phone
    /// hasn't reported yet, or couldn't place.
    pub call: Option<CallId>,
    pub caller: Caller,
    /// The Contact the other party is, if Contacts say.
    pub contact: Option<ContactMatch>,
    pub phase: CallWindowPhase,
    /// Where the Call's audio is, while there is a Call to move it for.
    pub audio: Option<CallAudio>,
    /// A failure to explain beside the Call.
    pub notice: Option<Failure>,
    /// The Call Window has keyboard focus.
    pub focused: bool,
}

/// What the Call Window says about the Call's audio, and so what its
/// Handoff button offers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CallAudio {
    /// Where the audio is now.
    pub location: AudioLocation,
    /// The Desktop has asked for the audio, and it hasn't come yet.
    pub moving_here: bool,
}

/// Where the Call in the Call Window is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CallWindowPhase {
    /// Answered here, and not yet connected.
    Connecting,
    /// Placed here, and the other end isn't ringing yet.
    Calling,
    /// Placed here, and ringing at the other end.
    Ringing,
    /// Placing it failed; the notice says why, and the window closes shortly.
    NotPlaced,
    /// Connected: the timer runs from `active_since`.
    Live {
        active_since: Timestamp,
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
    /// The attempt whose [`Timer::HandoffTimedOut`] counts.
    attempt: u32,
    /// The Phone has already been asked a second time.
    retried: bool,
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
    restore_audio: Option<AudioToRestore>,
    /// How many times the Phone has been asked to bring the audio here.
    handoff_attempts: u32,
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

    /// What the Call Window's Mute control offers.
    fn mute(&self) -> Mute {
        if self.muted() {
            Mute::On
        } else if self.audio_location() == Some(AudioLocation::Desktop) {
            Mute::Off
        } else {
            Mute::Unavailable
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
        // A refusal made beside a Call annotates it; a dial that failed on
        // its own has a Call Window of its own.
        let notice = self
            .notice
            .as_ref()
            .filter(|notice| notice.not_placed.is_none())
            .map(|notice| notice.failure)
            .or(self.audio_notice);
        let call_window = self
            .calls
            .iter()
            .find(|call| call.has_call_window() && self.unwanted != Some(call.id))
            .map(|call| CallWindowView {
                call: Some(call.id),
                caller: call.caller.clone(),
                contact: self.contact(&call.caller),
                phase: match (call.active_since, call.state) {
                    (Some(active_since), state) if state.is_connected() => CallWindowPhase::Live {
                        active_since,
                        wideband: self.codec.is_some_and(Codec::is_wideband),
                        mute: self.mute(),
                    },
                    (_, CallState::Dialing) => CallWindowPhase::Calling,
                    (_, CallState::Alerting) => CallWindowPhase::Ringing,
                    _ => CallWindowPhase::Connecting,
                },
                audio: call.state.is_live().then(|| CallAudio {
                    location: self.audio_location().unwrap_or(AudioLocation::Phone),
                    moving_here: self.moving_audio.is_some(),
                }),
                notice,
                focused: self.focus == Focus::CallWindow,
            })
            .or_else(|| {
                let dialling = self
                    .dialling
                    .as_ref()
                    .filter(|dialling| !dialling.cancelled)?;
                let caller = Caller::Number(dialling.number.clone());
                Some(CallWindowView {
                    call: None,
                    contact: self.contact(&caller),
                    caller,
                    audio: None,
                    phase: CallWindowPhase::Calling,
                    notice,
                    focused: self.focus == Focus::CallWindow,
                })
            })
            .or_else(|| {
                self.hung_up_dial.as_ref().map(|hung_up| CallWindowView {
                    call: None,
                    caller: hung_up.caller.clone(),
                    contact: self.contact(&hung_up.caller),
                    audio: None,
                    phase: CallWindowPhase::Ended,
                    notice: None,
                    focused: self.focus == Focus::CallWindow,
                })
            })
            .or_else(|| {
                let notice = self.notice.as_ref()?;
                let caller = notice.not_placed.clone()?;
                Some(CallWindowView {
                    call: None,
                    contact: self.contact(&caller),
                    caller,
                    audio: None,
                    phase: CallWindowPhase::NotPlaced,
                    notice: Some(notice.failure),
                    focused: self.focus == Focus::CallWindow,
                })
            })
            .or_else(|| {
                self.ended.as_ref().map(|ended| CallWindowView {
                    call: Some(ended.id),
                    caller: ended.caller.clone(),
                    contact: self.contact(&ended.caller),
                    audio: None,
                    phase: CallWindowPhase::Ended,
                    notice: None,
                    focused: self.focus == Focus::CallWindow,
                })
            });
        Surfaces {
            banner,
            call_window,
            dialer: self.dialer_view(),
        }
    }

    fn dialer_view(&self) -> Option<DialerView> {
        self.dialer.as_ref().map(|dialer| DialerView {
            query: dialer.query.clone(),
            rows: dialer.rows.clone(),
            selected: (!dialer.rows.is_empty()).then_some(dialer.selected),
            notice: dialer.notice,
            opened: dialer.opened,
        })
    }

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
            Input::MoveAudio(location) => {
                if !self.calls.iter().any(|call| call.state.is_live()) {
                    Some(Failure::NoLiveCall)
                } else if *location == AudioLocation::Phone
                    && self.audio_location() == Some(AudioLocation::Desktop)
                {
                    // Until the Desktop can close the audio link itself
                    // (ADR-0005), the user does it on the Phone.
                    Some(Failure::HandoffToPhoneUnavailable)
                } else {
                    None
                }
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

/// What the Contacts sync reports. The open Dialer lists what it brings.
fn contacts_sync(state: &mut State, input: Input, effects: &mut Vec<Effect>) {
    held_contacts(state, input, effects);
    refresh_dialer(state);
}

fn held_contacts(state: &mut State, input: Input, effects: &mut Vec<Effect>) {
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
        Input::ContactsSyncFailed(SyncFailure::Unavailable(reason)) if state.phone_is_here() => {
            state.contacts_sync = ContactsState::Unavailable(reason);
        }
        _ => {}
    }
}

fn phone_connected(state: &mut State, effects: &mut Vec<Effect>) {
    // Every connect syncs, so the Desktop catches up with changes made on
    // the Phone while apart.
    state.contacts_sync = ContactsState::Syncing;
    effects.push(Effect::SyncContacts);
    if state.has_desktop_audio() {
        state.connection = ConnectionState::Connected;
    } else {
        // A crash or a dropped link mid-Call must never leave the next
        // Call's audio jumping to the Desktop. Connected means ready for
        // Calls, so it waits until the Phone has done this.
        state.keeping_audio_on_phone = true;
        effects.push(Effect::Phone(PhoneCommand::KeepAudioOnPhone));
    }
}

/// Hands-free dropped. A Phone that never showed as connected stays
/// Connecting, as before it came into range.
fn phone_disconnected(state: &mut State) {
    if state.phone_is_here() {
        state.contacts_sync = state.held_contacts_state();
    }
    state.keeping_audio_on_phone = false;
    if state.audio_location() == Some(AudioLocation::Desktop) {
        state.restore_audio = Some(AudioToRestore { call_back: false });
    }
    if state.connection == ConnectionState::Connected {
        state.connection = ConnectionState::Disconnected;
        state.transport = Transport::Idle;
    }
}

/// The Phone reported `call`, unless it already had.
fn call_added(state: &mut State, mut call: Call, effects: &mut Vec<Effect>) {
    if state.calls.iter().any(|known| known.id == call.id) {
        return;
    }
    if state
        .ended
        .as_ref()
        .is_some_and(|ended| ended.id == call.id)
    {
        state.ended = None;
    }
    if call.direction == Direction::Outgoing {
        adopt_dialled(state, &mut call, effects);
    }
    if call.active_since.is_some()
        && let Some(index) = state
            .earlier_calls
            .iter()
            .position(|earlier| earlier.caller == call.caller)
    {
        // Connected before this run started: the Call it had then.
        let earlier = state.earlier_calls.remove(index);
        call.direction = earlier.direction;
        call.active_since = Some(earlier.active_since);
    }
    if state.recent_calls.len() == RECENT_CALLS {
        state.recent_calls.remove(0);
    }
    state.recent_calls.push((call.id, call.caller.clone()));
    state.calls.push(call);
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
        Input::PhoneUnpaired { address } if state.is_our_phone(&address) => {
            return (State::default(), vec![Effect::DetachPhone]);
        }
        Input::PhoneConnected { address } if state.is_our_phone(&address) => {
            phone_connected(&mut state, &mut effects);
        }
        Input::PhoneDisconnected { address } if state.is_our_phone(&address) => {
            phone_disconnected(&mut state);
            dial_failed(&mut state, Failure::PhoneOutOfRange, &mut effects);
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
                request: Request::None,
            };
            call_added(&mut state, call, &mut effects);
        }
        Input::CallChanged {
            call,
            state: call_state,
            at,
        } => call_changed(&mut state, call, call_state, at, &mut effects),
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
            command_done(&mut state, &command);
            restore_audio(&mut state, &mut effects);
        }
        Input::PhoneCommandFailed(command) => command_failed(&mut state, &command, &mut effects),
        Input::PhonePaired(_)
        | Input::PhoneUnpaired { .. }
        | Input::PhoneConnected { .. }
        | Input::PhoneDisconnected { .. } => {}
    }
    (state, effects)
}

/// Places a Call to `text`, a number or a `tel:` link, unless it can't be
/// dialled or placed now. A Call placed here has its audio on the Desktop,
/// so the Phone may bring the audio here before it dials.
fn dial(state: &mut State, text: &str, effects: &mut Vec<Effect>) {
    state.dial_attempts += 1;
    let attempt = state.dial_attempts;
    let Some(number) = matching::dialable(text) else {
        let caller = Caller::Number(text.trim().to_owned());
        refuse(state, Failure::Undialable, attempt, caller, effects);
        return;
    };
    if let Some(failure) = state.dial_refusal() {
        refuse(state, failure, attempt, Caller::Number(number), effects);
        return;
    }
    // The Call is starting, so the Dialer's work is done.
    state.dialer = None;
    state.ended = None;
    state.notice = None;
    state.hung_up_dial = None;
    state.dialling = Some(Dialling {
        number: number.clone(),
        attempt,
        cancelled: false,
    });
    effects.push(Effect::Phone(PhoneCommand::AllowAudioOnDesktop));
    effects.push(Effect::Phone(PhoneCommand::Dial(number)));
    effects.push(Effect::StartTimer {
        timer: Timer::DialTimedOut(attempt),
        after_ms: DIAL_TIMEOUT_MS,
    });
}

/// The first outgoing Call after a dial is the one dialled: it takes the
/// dialled number if the Phone withholds it, and is hung up at once if the
/// user hung up first.
fn adopt_dialled(state: &mut State, call: &mut Call, effects: &mut Vec<Effect>) {
    let Some(dialling) = state.dialling.take() else {
        return;
    };
    if call.caller == Caller::Withheld {
        call.caller = Caller::Number(dialling.number);
    }
    if dialling.cancelled {
        call.request = Request::HangUp;
        state.unwanted = Some(call.id);
        effects.push(Effect::Phone(PhoneCommand::HangUp(call.id)));
    }
}

/// Startup with what the run before left: its Calls take their direction
/// and timer from it once the Phone lists them again, and a microphone it
/// silenced for Mute still counts as muted.
fn pick_up_earlier_run(state: &mut State, earlier: EarlierRun, effects: &mut Vec<Effect>) {
    if !earlier.calls.is_empty() {
        effects.push(Effect::StartTimer {
            timer: Timer::ForgetEarlierRun,
            after_ms: FORGET_EARLIER_RUN_MS,
        });
    }
    state.earlier_calls = earlier.calls;
    state.silenced = earlier.restore_microphone_volume;
}

fn timer_fired(state: &mut State, timer: Timer, effects: &mut Vec<Effect>) {
    match timer {
        Timer::ForgetEarlierRun => state.earlier_calls.clear(),
        Timer::DismissDial(attempt) => {
            if state
                .notice
                .as_ref()
                .is_some_and(|notice| notice.attempt == attempt)
            {
                state.notice = None;
            }
            if state
                .hung_up_dial
                .as_ref()
                .is_some_and(|hung_up| hung_up.attempt == attempt)
            {
                state.hung_up_dial = None;
            }
        }
        Timer::DialTimedOut(attempt) => {
            if state
                .dialling
                .as_ref()
                .is_some_and(|dialling| dialling.attempt == attempt)
            {
                dial_failed(state, Failure::DialFailed, effects);
            }
        }
        Timer::HandoffTimedOut(attempt) => {
            if state
                .moving_audio
                .is_some_and(|moving| moving.attempt == attempt)
            {
                audio_not_here(state, effects);
            }
        }
        Timer::CloseCallWindow(call) => {
            if state.ended.as_ref().is_some_and(|ended| ended.id == call) {
                state.ended = None;
            }
        }
    }
}

/// The user hung up the Call they placed before the Phone reported it: the
/// Call Window says "Call ended", and the Call is hung up when it appears.
fn cancel_dial(state: &mut State, effects: &mut Vec<Effect>) {
    let Some(dialling) = state
        .dialling
        .as_mut()
        .filter(|dialling| !dialling.cancelled)
    else {
        return;
    };
    dialling.cancelled = true;
    let attempt = dialling.attempt;
    state.hung_up_dial = Some(HungUpDial {
        attempt,
        caller: Caller::Number(dialling.number.clone()),
    });
    effects.push(Effect::StartTimer {
        timer: Timer::DismissDial(attempt),
        after_ms: CALL_ENDED_MS,
    });
}

/// Refuses dial attempt `attempt` to `caller`. With no other Call in the
/// Call Window, the window shows the attempt as not placed; otherwise the
/// refusal is a note beside that Call.
fn refuse(
    state: &mut State,
    failure: Failure,
    attempt: u32,
    caller: Caller,
    effects: &mut Vec<Effect>,
) {
    let alone = state.calls.is_empty() && state.dialling.is_none();
    if alone {
        state.ended = None;
        state.hung_up_dial = None;
    }
    notify(state, failure, attempt, alone.then_some(caller), effects);
}

/// The pending dial is over without a Call: say why, unless the user hung
/// up already, and keep the next Call's audio on the Phone.
fn dial_failed(state: &mut State, failure: Failure, effects: &mut Vec<Effect>) {
    let Some(dialling) = state.dialling.take() else {
        return;
    };
    if !dialling.cancelled {
        let caller = Caller::Number(dialling.number);
        notify(state, failure, dialling.attempt, Some(caller), effects);
    }
    if !state.has_desktop_audio() {
        effects.push(Effect::Phone(PhoneCommand::KeepAudioOnPhone));
    }
}

fn notify(
    state: &mut State,
    failure: Failure,
    attempt: u32,
    not_placed: Option<Caller>,
    effects: &mut Vec<Effect>,
) {
    state.notice = Some(Notice {
        failure,
        attempt,
        not_placed,
    });
    effects.push(Effect::ReportError(failure));
    effects.push(Effect::StartTimer {
        timer: Timer::DismissDial(attempt),
        after_ms: NOTICE_MS,
    });
}

fn refusal_failure(refusal: PhoneRefusal) -> Failure {
    match refusal {
        PhoneRefusal::InvalidFormat => Failure::NumberRefused,
        PhoneRefusal::InvalidState | PhoneRefusal::InProgress => Failure::PhoneBusy,
        PhoneRefusal::NotSupported => Failure::DialNotSupported,
        PhoneRefusal::NoNetwork => Failure::NoSignal,
        PhoneRefusal::NetworkTimeout => Failure::NetworkError,
        PhoneRefusal::Failed => Failure::DialFailed,
    }
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
    if matches!(state.muting, Muting::Silencing { .. } | Muting::On { .. }) {
        return;
    }
    if let Some(failure) = state.refusal(&Input::SetMuted(true)) {
        effects.push(Effect::ReportError(failure));
        return;
    }
    let restore = match state.muting {
        Muting::Restoring { restore } => restore,
        // A gain of 0, or PipeWire's 255 for "the Phone hasn't said", would
        // leave the microphone silent after unmuting.
        _ => match state.microphone_volume {
            Some(volume @ 1..=MAX_MICROPHONE_VOLUME) => volume,
            _ => DEFAULT_MICROPHONE_VOLUME,
        },
    };
    state.muting = Muting::Silencing { restore };
    effects.push(Effect::Phone(PhoneCommand::SetMicrophoneVolume(0)));
}

/// The user asked to unmute.
fn unmute(state: &mut State, effects: &mut Vec<Effect>) {
    if let Muting::Silencing { restore } | Muting::On { restore } = state.muting {
        state.muting = Muting::Restoring { restore };
        effects.push(Effect::Phone(PhoneCommand::SetMicrophoneVolume(restore)));
    }
}

/// The audio left the Desktop, or the Call ended: the gain goes back at
/// once, as there is no Mute left to show.
fn release_mute(state: &mut State, effects: &mut Vec<Effect>) {
    if let Muting::Silencing { restore } | Muting::On { restore } = state.muting {
        effects.push(Effect::Phone(PhoneCommand::SetMicrophoneVolume(restore)));
        state.silenced = Some(restore);
    }
    state.muting = Muting::Off;
}

/// With the audio on the Desktop, a microphone the Desktop silenced and the
/// Phone still reports at 0 is muted: showing it unmuted would have the
/// user talk to no one.
fn still_silenced(state: &mut State) {
    if state.muting == Muting::Off
        && state.microphone_volume == Some(0)
        && let Some(restore) = state.silenced.take()
    {
        state.muting = Muting::On { restore };
    }
}

fn command_done(state: &mut State, command: &PhoneCommand) {
    if let PhoneCommand::SetMicrophoneVolume(volume) = command
        && state.silenced == Some(*volume)
    {
        state.silenced = None;
    }
    match (command, state.muting) {
        (PhoneCommand::KeepAudioOnPhone, _) if state.keeping_audio_on_phone => {
            state.keeping_audio_on_phone = false;
            state.connection = ConnectionState::Connected;
        }
        (PhoneCommand::SetMicrophoneVolume(0), Muting::Silencing { restore }) => {
            state.muting = Muting::On { restore };
        }
        (PhoneCommand::SetMicrophoneVolume(volume), Muting::Restoring { restore })
            if *volume == restore =>
        {
            state.muting = Muting::Off;
        }
        _ => {}
    }
}

/// The keybinding: focus the Banner while a Call rings here, otherwise the
/// Call Window while a Call is live, otherwise open the Dialer. Neither
/// touches the Call itself. Only one surface can have the keyboard, so the
/// Dialer closes when another takes it.
fn activate(state: &mut State) {
    let surfaces = state.surfaces();
    if surfaces.banner.is_some() {
        state.focus = Focus::Banner;
        state.dialer = None;
    } else if surfaces.call_window.is_some_and(|window| {
        !matches!(
            window.phase,
            CallWindowPhase::Ended | CallWindowPhase::NotPlaced
        )
    }) {
        state.focus = Focus::CallWindow;
        state.dialer = None;
    } else if state.dialer.is_none() {
        open_dialer(state, String::new(), None);
    }
}

/// What the user does in the Dialer.
fn dialer_input(state: &mut State, input: Input, effects: &mut Vec<Effect>) {
    match input {
        Input::OpenDialer { query, select_call } => open_dialer(state, query, select_call),
        Input::DialerQuery(query) => dialer_query(state, query),
        Input::DialerMove(by) => dialer_move(state, by),
        Input::DialerEnter => dialer_enter(state, None, effects),
        Input::DialerChoose(index) => dialer_enter(state, Some(index), effects),
        Input::CloseDialer => state.dialer = None,
        _ => {}
    }
}

/// Opens the Dialer afresh with `query` typed. It takes the keyboard.
fn open_dialer(state: &mut State, query: String, select_call: Option<CallId>) {
    state.focus = Focus::None;
    state.dialer_openings = state.dialer_openings.wrapping_add(1);
    let rows = matching::search(&query, &state.contacts, &state.call_history);
    let caller = select_call.and_then(|call| {
        state
            .recent_calls
            .iter()
            .rev()
            .find(|(id, _)| *id == call)
            .map(|(_, caller)| caller)
    });
    let selected = caller
        .and_then(|caller| {
            rows.iter()
                .position(|row| state.same_number(&row.caller, caller))
        })
        .unwrap_or(0);
    state.dialer = Some(Dialer {
        notice: undialable(&query, &rows),
        query,
        rows,
        selected,
        opened: state.dialer_openings,
    });
}

/// What is typed can't be dialled and matches nothing: say so before Enter.
fn undialable(query: &str, rows: &[DialerRow]) -> Option<Failure> {
    (rows.is_empty() && matching::looks_undialable(query)).then_some(Failure::Undialable)
}

fn dialer_query(state: &mut State, query: String) {
    if state
        .dialer
        .as_ref()
        .is_none_or(|dialer| dialer.query == query)
    {
        return;
    }
    let rows = matching::search(&query, &state.contacts, &state.call_history);
    if let Some(dialer) = state.dialer.as_mut() {
        dialer.notice = undialable(&query, &rows);
        dialer.query = query;
        dialer.rows = rows;
        dialer.selected = 0;
    }
}

/// Contacts or the Call History changed: the open Dialer lists them anew,
/// keeping the selection where it was as far as it can.
fn refresh_dialer(state: &mut State) {
    let Some(query) = state.dialer.as_ref().map(|dialer| dialer.query.clone()) else {
        return;
    };
    let rows = matching::search(&query, &state.contacts, &state.call_history);
    if let Some(dialer) = state.dialer.as_mut() {
        dialer.selected = dialer.selected.min(rows.len().saturating_sub(1));
        dialer.rows = rows;
    }
}

fn dialer_move(state: &mut State, by: i32) {
    let Some(dialer) = state.dialer.as_mut() else {
        return;
    };
    let last = dialer.rows.len().saturating_sub(1);
    let steps = usize::try_from(by.unsigned_abs()).unwrap_or(usize::MAX);
    dialer.selected = if by >= 0 {
        dialer.selected.saturating_add(steps).min(last)
    } else {
        dialer.selected.saturating_sub(steps)
    };
}

/// Calls the Dialer's row at `index`, or the selected row. If it can't be
/// called now, the Dialer stays open and says why.
fn dialer_enter(state: &mut State, index: Option<usize>, effects: &mut Vec<Effect>) {
    let Some(dialer) = state.dialer.as_mut() else {
        return;
    };
    if let Some(index) = index.filter(|index| *index < dialer.rows.len()) {
        dialer.selected = index;
    }
    let number = match dialer.rows.get(dialer.selected) {
        None => Err(undialable(&dialer.query, &dialer.rows)),
        Some(row) => row.dial().ok_or(Some(if row.caller == Caller::Withheld {
            Failure::WithheldNumber
        } else {
            Failure::Undialable
        })),
    };
    let failure = match number {
        Ok(number) => match state.dial_refusal() {
            None => {
                dial(state, &number, effects);
                return;
            }
            refusal => refusal,
        },
        Err(failure) => failure,
    };
    if let (Some(failure), Some(dialer)) = (failure, state.dialer.as_mut()) {
        dialer.notice = Some(failure);
        effects.push(Effect::ReportError(failure));
    }
}

/// The Call audio link changed. Once the audio the Desktop asked for is
/// here, `RejectSCO` is set again (ADR-0005); it doesn't close the open link.
/// The audio moving either way answers any notice about where it is.
fn transport_changed(state: &mut State, transport: Transport, effects: &mut Vec<Effect>) {
    let was_here = state.transport == Transport::Active;
    state.transport = transport;
    if transport.is_up() && state.reported_codec.is_some() {
        state.codec = state.reported_codec;
    }
    let is_here = transport == Transport::Active;
    if was_here != is_here {
        state.audio_notice = None;
    }
    if transport.is_up() {
        // It came back by itself.
        state.restore_audio = None;
    }
    if is_here && state.moving_audio.take().is_some() {
        effects.push(Effect::Phone(PhoneCommand::KeepAudioOnPhone));
    }
}

/// The Phone reported a Call's new state.
fn call_changed(
    state: &mut State,
    call: CallId,
    call_state: CallState,
    at: Timestamp,
    effects: &mut Vec<Effect>,
) {
    let Some(known) = state.call_mut(call) else {
        return;
    };
    known.state = call_state;
    if call_state.is_connected() && known.active_since.is_none() {
        known.active_since = Some(at);
    }
    if let Some(restore) = &mut state.restore_audio {
        restore.call_back = true;
    }
    restore_audio(state, effects);
}

fn microphone_volume_changed(state: &mut State, volume: u8) {
    state.microphone_volume = Some(volume);
    if volume != 0 {
        state.silenced = None;
    }
}

/// Brings the audio back here once hands-free and the Call are back after
/// a drop that took it from the Desktop.
fn restore_audio(state: &mut State, effects: &mut Vec<Effect>) {
    let ready = state.restore_audio.is_some_and(|restore| restore.call_back)
        && state.connection == ConnectionState::Connected;
    if ready {
        state.restore_audio = None;
        move_audio(state, AudioLocation::Desktop, effects);
    }
}

/// The user asked to move the audio to `location`.
fn move_audio(state: &mut State, location: AudioLocation, effects: &mut Vec<Effect>) {
    let input = Input::MoveAudio(location);
    if let Some(failure) = state.refusal(&input) {
        effects.push(Effect::ReportError(failure));
        if state.calls.iter().any(|call| call.state.is_live()) {
            state.audio_notice = Some(failure);
        }
        return;
    }
    if location == AudioLocation::Desktop
        && state.audio_location() == Some(AudioLocation::Phone)
        && state.moving_audio.is_none()
    {
        state.audio_notice = None;
        bring_audio_here(state, false, effects);
    }
}

/// Asks the Phone to bring the audio here, and starts the wait for it.
fn bring_audio_here(state: &mut State, retried: bool, effects: &mut Vec<Effect>) {
    effects.push(Effect::Phone(PhoneCommand::MoveAudioToDesktop));
    start_waiting_for_audio(state, retried, effects);
}

fn start_waiting_for_audio(state: &mut State, retried: bool, effects: &mut Vec<Effect>) {
    state.handoff_attempts += 1;
    let attempt = state.handoff_attempts;
    state.moving_audio = Some(MovingAudio { attempt, retried });
    effects.push(Effect::StartTimer {
        timer: Timer::HandoffTimedOut(attempt),
        after_ms: HANDOFF_TIMEOUT_MS,
    });
}

/// An attempt to bring the audio here failed or ran out of time. The Phone
/// ignored `Activate()` once in 7 tries on the Pixel, so it is asked once
/// more; after that the user is told how to do it on the Phone. `RejectSCO`
/// stays false, so doing it on the Phone works.
fn audio_not_here(state: &mut State, effects: &mut Vec<Effect>) {
    match state.moving_audio {
        Some(MovingAudio { retried: false, .. }) => bring_audio_here(state, true, effects),
        Some(MovingAudio { retried: true, .. }) => {
            state.moving_audio = None;
            state.audio_notice = Some(Failure::AudioNotMoved);
            effects.push(Effect::ReportError(Failure::AudioNotMoved));
        }
        None => {}
    }
}

fn answer(state: &mut State, call: CallId, effects: &mut Vec<Effect>) {
    match state.call_mut(call) {
        Some(known) if known.rings_here() => {
            known.request = Request::Answer;
            effects.push(Effect::Phone(PhoneCommand::Answer(call)));
            bring_audio_here(state, false, effects);
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
    if state.unwanted == Some(call) {
        state.unwanted = None;
    } else if ended.has_call_window() {
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
        state.moving_audio = None;
        state.restore_audio = None;
        state.audio_notice = None;
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
            // The batch stopped here: the audio was never asked for.
            state.moving_audio = None;
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
        PhoneCommand::MoveAudioToDesktop => audio_not_here(state, effects),
        // Without these the Phone never dialled.
        PhoneCommand::AllowAudioOnDesktop | PhoneCommand::Dial(_) => {
            dial_failed(state, Failure::DialFailed, effects);
        }
        // The shell has logged it; the next Call end or connect tries again.
        // The Phone is there all the same, so it shows as connected.
        PhoneCommand::KeepAudioOnPhone => {
            if state.keeping_audio_on_phone {
                state.keeping_audio_on_phone = false;
                state.connection = ConnectionState::Connected;
            }
        }
        PhoneCommand::SendTones(_) => effects.push(Effect::ReportError(Failure::TonesNotSent)),
        PhoneCommand::SetMicrophoneVolume(0) => {
            if let Muting::Silencing { .. } = state.muting {
                state.muting = Muting::Off;
                effects.push(Effect::ReportError(Failure::MuteFailed));
            }
        }
        PhoneCommand::SetMicrophoneVolume(_) => {
            // Unmuting the user asked for failed: the microphone is still
            // silent, so say so. An unmute because the audio left the
            // Desktop or the Call ended has no Mute left to show.
            if let Muting::Restoring { restore } = state.muting
                && state.audio_location() == Some(AudioLocation::Desktop)
            {
                state.muting = Muting::On { restore };
                effects.push(Effect::ReportError(Failure::UnmuteFailed));
            }
        }
    }
}
