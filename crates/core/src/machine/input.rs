//! What the Call state machine takes in and gives out: inputs from the shell
//! adapters, and the commands, timers and effects it asks them for.

use crate::call::{AudioLocation, CallId, CallState, Caller, Codec, Timestamp, Transport};
use crate::connection::Phone;
use crate::contacts::{CallHistory, Contacts, SyncFailure};
use crate::earlier_run::EarlierRun;
use crate::messages::Failure;
use crate::missed::MissedCallNotice;

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
    /// The user clicked the Missed Call notification with this id: open
    /// the Dialer with its latest Call selected.
    OpenMissedCall(CallId),
    /// The user pressed the Missed Call notification's Call back.
    CallBack(CallId),
    /// The user dismissed the Missed Call notification with this id.
    MissedCallDismissed(CallId),
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
    /// A refresh of the Call History ended without one. Each
    /// [`Effect::RefreshCallHistory`] ends in this or
    /// [`Input::CallHistoryRefreshed`], in the order asked.
    CallHistoryRefreshFailed,
    /// The Phone is asking whether to share Contacts, until `deadline`.
    ContactsPermissionNeeded { deadline: Timestamp },
    /// A sync, or a refresh, ended without Contacts.
    ContactsSyncFailed(SyncFailure),
    /// The Desktop's Do Not Disturb is on (`true`) or off, as read at
    /// startup and on every change.
    DoNotDisturbChanged(bool),
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
    /// Send the live Call's audio back to the Phone, by dropping hands-free
    /// and connecting it again at once (ADR-0005). The Call goes with
    /// hands-free and comes back with it, still live.
    MoveAudioToPhone,
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

/// What became of a dial attempt, by its number. Each attempt is either
/// [`DialProgress::NotPlaced`] at once, or [`DialProgress::Asked`] and then,
/// in a later step, [`DialProgress::Placed`] or [`DialProgress::NotPlaced`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DialProgress {
    /// The Phone was asked to place the Call.
    Asked(u32),
    /// The Phone reported the Call, dialing or further on.
    Placed(u32),
    /// No Call was placed, for this reason: the one the Call Window shows,
    /// unless the user hung up first.
    NotPlaced(u32, Failure),
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
    /// The audio hasn't come here within the limit for this Handoff.
    HandoffTimedOut(u32),
    /// A second has passed since the Phone carried out `Activate()` for
    /// this Handoff: if the audio link hasn't started, ask once more.
    HandoffNotStarted(u32),
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
    /// What became of a dial attempt, for whoever asked for it.
    DialProgress(DialProgress),
    /// Sync Contacts and Call History from the attached Phone.
    SyncContacts,
    /// Pull the Phone's Call History again.
    RefreshCallHistory,
    /// Show the Missed Call notification, replacing the one with the same
    /// `notification` id if it is still shown.
    NotifyMissedCall(MissedCallNotice),
    /// Delete the Contacts and Call History held for the attached Phone,
    /// on disk too.
    ForgetContacts,
}
