//! What the Call state machine takes in and gives out: inputs from the shell
//! adapters, and the commands, timers and effects it asks them for.

use super::calls_off::CallsOffRecord;
use super::desktop::{DesktopCheck, DesktopFix, FixFailure};
use crate::call::{AudioLocation, CallId, CallState, Caller, Codec, Timestamp, Transport};
use crate::clock::{Deadline, QuietHours, WallClock};
use crate::connection::Phone;
use crate::contacts::{CallHistory, Contacts, SyncFailure, SyncProgress};
use crate::earlier_run::EarlierRun;
use crate::messages::Failure;
use crate::messages::ProblemKind;
use crate::missed::MissedCallNotice;

use crate::keybinding::HyprlandConfig;

/// Something that happened, as the shell adapters report it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Input {
    /// What the run before this one left, at startup.
    EarlierRun(EarlierRun),
    /// The Phone the user chose last in the Setup Window, as an earlier run
    /// kept it, at startup and before any Phone is reported: Desktop Calls
    /// follows it, and no other Phone, whatever BlueZ lists first.
    ChosenPhoneKept { address: String },
    /// A paired Phone that can carry Calls exists on the Desktop.
    PhonePaired(Phone),
    /// The paired Phone with this address is gone.
    PhoneUnpaired { address: String },
    /// The Phone with this address is connected for Calls.
    PhoneConnected { address: String },
    /// The Phone with this address is no longer connected for Calls.
    PhoneDisconnected { address: String },
    /// The Phone with this address has a Bluetooth connection to the
    /// Desktop, whether or not hands-free is up on it yet.
    PhonePresent { address: String },
    /// The Phone with this address has no Bluetooth connection to the
    /// Desktop: it is out of range, or its Bluetooth is off.
    PhoneAbsent { address: String },
    /// The Phone connected, as [`Effect::ConnectPhone`] asked with this
    /// attempt. An earlier attempt's result is ignored.
    ConnectPhoneDone { attempt: u32 },
    /// The Phone didn't connect, as [`Effect::ConnectPhone`] asked with this
    /// attempt. An earlier attempt's result is ignored.
    ConnectPhoneFailed {
        attempt: u32,
        failure: ConnectFailure,
    },
    /// The Desktop is about to suspend.
    Suspending,
    /// The Desktop resumed from suspend.
    Resumed,
    /// The wall clock, as the shell read it: at startup, on the minute, on
    /// resume, and at each instant [`Effect::ReadClockAt`] asked for.
    /// Monotonic timers stop while the Desktop is suspended; this doesn't.
    ClockTicked(WallClock),
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
    /// The Phone reported who is on the other end of a Call after reporting
    /// the Call: some Phones send the number a moment after a Call starts
    /// ringing.
    CallerChanged { call: CallId, caller: Caller },
    /// A Call is over, however it ended, `at` the time the Phone link heard.
    CallEnded { call: CallId, at: Timestamp },
    /// The Phone link gave up on this Call with no word from the Phone that
    /// the Hang up the user asked for worked: the Call went away without
    /// ending, and either didn't come back while the Phone was away, or a
    /// Call came back with no number to tell whether it is this one. It may
    /// carry on on the Phone.
    HangUpUndelivered(CallId),
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
    /// The user asked to put the live Call on hold and answer the Waiting
    /// Call.
    HoldAndAnswer,
    /// The user asked to end the live Call and answer the Waiting Call.
    EndAndAnswer,
    /// The user asked to swap the live Call and the Call on hold: the live
    /// one goes on hold and the held one is live again.
    Swap,
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
    /// Escape on the Call Window: collapse it to the Call Pill and give the
    /// keyboard back.
    CollapseCallWindow,
    /// The user clicked the Call Pill: expand the Call Window, with the
    /// keyboard.
    ExpandCallWindow,
    /// The user clicked the Indicator: expand the Call Window, collapsed
    /// during a Call, or otherwise open the Dialer.
    IndicatorActivated,
    /// The pointer moved over the Call Window (`true`) or left it.
    PointerOverCallWindow(bool),
    /// The Call Window got the keyboard (`true`) or lost it, however it
    /// did.
    CallWindowKeyboard(bool),
    /// The user clicked in the Call Window, or pressed a key in it.
    CallWindowUsed,
    /// The Call Window's keypad opened (`true`) or closed.
    KeypadShown(bool),
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
    /// How far the sync under way has got.
    ContactsSyncProgress(SyncProgress),
    /// The user asked the Phone to ask again whether to share Contacts,
    /// after nobody answered it in time.
    RetryContacts,
    /// The user chose to go on without Contacts, after nobody answered the
    /// Phone in time.
    SkipContacts,
    /// The Desktop's Do Not Disturb is on (`true`) or off, as read at
    /// startup and on every change.
    DoNotDisturbChanged(bool),
    /// BlueZ has listed the devices it knows, at startup: every paired
    /// Phone has been reported by now.
    PairedPhonesListed,
    /// The user asked for the Setup Window: from the Indicator, the
    /// control interface, the command line or a Problem.
    OpenSetupWindow,
    /// The user closed the Setup Window.
    CloseSetupWindow,
    /// What an earlier run kept of Calls Off, at startup and before any
    /// Phone is reported.
    CallsOffKept(CallsOffRecord),
    /// The user turned calls off, from the Indicator, the control interface
    /// or the command line: until `until`, or with `None` until they turn
    /// them on.
    TurnCallsOff { until: Option<Deadline> },
    /// The user turned calls on; a turn-off waiting for the last Call to
    /// end is cancelled.
    TurnCallsOn,
    /// The user chose Keep calls on: a turn-off waiting for the last Call
    /// to end is cancelled.
    KeepCallsOn,
    /// The Quiet Hours the user set, as kept from an earlier run, at
    /// startup and before the wall clock is first reported.
    QuietHoursKept(QuietHours),
    /// The user set Quiet Hours, from the command line or the control
    /// interface.
    SetQuietHours(QuietHours),
    /// The user turned Quiet Hours off.
    ClearQuietHours,
    /// BlueZ blocked (`blocked`) or unblocked the Phone with this address,
    /// as [`Effect::BlockPhone`] asked.
    PhoneBlocked { address: String, blocked: bool },
    /// BlueZ couldn't block (`blocked`) or unblock the Phone with this
    /// address.
    PhoneBlockFailed { address: String, blocked: bool },
    /// The Desktop is looking for Phones and is discoverable, as
    /// [`Effect::StartSetup`] asked, and shows on Phones as `desktop_name`.
    SetupReady { desktop_name: String },
    /// The Desktop can't look for Phones: Bluetooth is off or missing.
    SetupUnavailable,
    /// A Phone nearby that isn't paired with the Desktop, or its new name.
    PhoneNearby(Phone),
    /// The nearby Phone with this address is gone from BlueZ's list.
    PhoneNotNearby { address: String },
    /// The user chose this nearby Phone in the Setup Window, to pair it.
    ChoosePhone { address: String },
    /// Pairing with this Phone asks whether it shows the same code: six
    /// digits, 0 to 999999. Each ends in [`Effect::AnswerPairingCode`].
    PairingCode { phone: Phone, code: u32 },
    /// The user confirmed the pairing code shows on the Phone too.
    ConfirmPairing,
    /// The user turned the pairing down.
    RejectPairing,
    /// Pairing didn't happen, for this reason.
    PairingFailed(PairingFailure),
    /// The Phone refused hands-free when asked: its "Phone calls" setting
    /// for the Desktop is off.
    HandsFreeRefused,
    /// The Phone didn't answer when asked for hands-free: out of range, or
    /// its Bluetooth is off.
    HandsFreeUnreachable,
    /// The user asked to try again, after switching "Phone calls" on or
    /// bringing the Phone close.
    RetryHandsFree,
    /// A setup check found this about the Desktop: at startup, on every
    /// change it can follow, and after each [`Effect::CheckDesktop`].
    DesktopChecked(DesktopCheck),
    /// The user asked, in the Setup Window, to put this Problem right.
    Fix(ProblemKind),
    /// The Desktop did what [`Effect::Fix`] asked.
    FixDone(DesktopFix),
    /// What [`Effect::Fix`] asked didn't work.
    FixFailed(FixFailure),
    /// What the Desktop's Hyprland config holds, as
    /// [`Effect::CheckKeybinding`] asked.
    KeybindingChecked(HyprlandConfig),
    /// The Phone is back after leaving during the Setup Window's Contacts
    /// step: the user asked to carry on.
    ContinueContacts,
    /// The user asked to add the Desktop Calls keybinding: show the line
    /// first.
    AddKeybinding,
    /// The user confirmed the line shown: write it.
    ConfirmKeybinding,
    /// The user declined the line shown.
    CancelKeybinding,
    /// The line was written, as [`Effect::AddKeybinding`] asked.
    KeybindingAdded,
    /// The line couldn't be written.
    KeybindingNotAdded,
    /// Nothing was written: the file changed since the line was offered,
    /// and holds this now.
    KeybindingConfigChanged(HyprlandConfig),
    /// What the Desktop's Hyprland config holds: at startup and on every
    /// change, so the Banner names the keybinding actually set.
    KeybindingRead(HyprlandConfig),
}

/// Why pairing with a Phone didn't happen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PairingFailure {
    /// The person holding the Phone, or the user, turned it down, or it was
    /// cancelled.
    Rejected,
    /// The Phone didn't answer.
    Unreachable,
    /// Anything else.
    Failed,
}

/// What the Phone link can be asked to do with Calls.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PhoneCommand {
    /// Answer this ringing Call.
    Answer(CallId),
    /// Hang up this Call; for a ringing Call, that declines it.
    HangUp(CallId),
    /// Put the live Call on hold and answer this Waiting Call.
    HoldAndAnswer(CallId),
    /// End the live Call and answer this Waiting Call.
    EndAndAnswer(CallId),
    /// Answer this Waiting Call while the only other Call stays on hold:
    /// PipeWire's `SwapCalls`, `AT+CHLD=2`.
    AnswerBesideHeld(CallId),
    /// Put the live Call on hold and make the Held Call live again.
    Swap,
    /// Bring the live Call's audio to the Desktop.
    MoveAudioToDesktop,
    /// Send the live Call's audio back to the Phone, by dropping hands-free
    /// and connecting it again at once (ADR-0005). The Call goes with
    /// hands-free and comes back with it, still live.
    MoveAudioToPhone,
    /// Connect hands-free to the Phone again, after it dropped for
    /// [`PhoneCommand::MoveAudioToPhone`] and didn't come back. Hands-free
    /// coming up is reported as [`Input::PhoneConnected`], whenever it does.
    ConnectHandsFree,
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

/// Why the Phone didn't connect when the Desktop asked.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConnectFailure {
    /// The Phone didn't answer: out of range, Bluetooth off, or busy. Worth
    /// trying again.
    Unreachable,
    /// The Phone answered, and no longer has the Desktop paired. Trying
    /// again would only ask the person holding it to pair.
    PairingLost,
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
    /// Time to ask BlueZ to connect the Phone again, after the wait with
    /// this number.
    Reconnect(u32),
    /// The Call Window has said for long enough, by the loss with this
    /// number, that the connection to the Phone was lost.
    ConnectionLost(u32),
    /// The Call Window has said for long enough why the Phone refused Hold &
    /// answer, End & Answer or Swap, by the refusal with this number.
    DismissCallWaitingNotice(u32),
    /// The Setup Window's step with this number has waited long enough.
    SetupStep(u32),
    /// The Phone has had long enough to report the number of this Call,
    /// which rang with none: if it hasn't by now, the number is withheld.
    NumberLate(CallId),
    /// Time to check again the Desktop's Problems that can't be followed as
    /// they change.
    CheckDesktop,
    /// Collapse the Call Window to the Call Pill, if this collapse timer,
    /// by number, is still the one that counts.
    CollapseCallWindow(u32),
}

/// Something the shell adapters must do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Effect {
    /// Follow the Phone with this address: report when it connects and
    /// disconnects for Calls.
    AttachPhone { address: String },
    /// Stop following the Phone.
    DetachPhone,
    /// Ask BlueZ to connect the Phone with this address. Each ends in
    /// [`Input::ConnectPhoneDone`] or [`Input::ConnectPhoneFailed`], echoing
    /// the attempt so its result belongs only to this operation. With
    /// `away`, the Desktop last heard that the Phone has no Bluetooth
    /// connection: if BlueZ has it connected after all, that comes back as
    /// [`Input::PhonePresent`] before the result, and the Phone isn't paged
    /// (ticket #212).
    ConnectPhone {
        address: String,
        attempt: u32,
        away: bool,
    },
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
    /// This many Calls from the earlier run were forgotten: they never
    /// showed again in the time allowed, so they had ended. For the log.
    EarlierCallsForgotten(usize),
    /// Look for nearby Phones, make the Desktop discoverable, and take
    /// pairing requests, until [`Effect::StopSetup`]. Answered by
    /// [`Input::SetupReady`] or [`Input::SetupUnavailable`].
    StartSetup,
    /// Stop looking for Phones, stop being discoverable, and turn down
    /// pairing requests.
    StopSetup,
    /// Pair with the nearby Phone with this address. Its code comes as
    /// [`Input::PairingCode`]; a failure as [`Input::PairingFailed`].
    PairPhone { address: String },
    /// Answer the pairing code asked about last: `confirm` if the user
    /// confirmed it.
    AnswerPairingCode { confirm: bool },
    /// Keep the Phone with this address as the Phone the user chose last,
    /// for the next run to follow.
    KeepChosenPhone { address: String },
    /// Let the Phone with this address connect without asking each time.
    TrustPhone { address: String },
    /// Ask the Phone with this address for hands-free alone. A refusal
    /// comes as [`Input::HandsFreeRefused`], or [`Input::HandsFreeUnreachable`]
    /// if it didn't answer.
    ConnectHandsFree { address: String },
    /// Check again what can't be followed as it changes, and report each
    /// finding as [`Input::DesktopChecked`].
    CheckDesktop,
    /// Put a Desktop Problem right, as the user asked. Ends in
    /// [`Input::FixDone`] or [`Input::FixFailed`].
    Fix(DesktopFix),
    /// Find what the Desktop's Hyprland config can take for the Desktop
    /// Calls keybinding. Answered by [`Input::KeybindingChecked`].
    CheckKeybinding,
    /// Write `added` to the Hyprland config `file`, which held `seen` when
    /// the user saw and confirmed the line: `added` is `seen` with that
    /// line. Only if `file` still holds `seen`. Answered by
    /// [`Input::KeybindingAdded`], [`Input::KeybindingNotAdded`] or
    /// [`Input::KeybindingConfigChanged`].
    AddKeybinding {
        file: String,
        seen: String,
        added: String,
    },
    /// Keep `record` of Calls Off for the next run, replacing what was
    /// kept; with `None`, keep nothing.
    KeepCallsOff(Option<CallsOffRecord>),
    /// Have BlueZ block (`blocked`) or unblock the Phone with this address.
    /// Ends in [`Input::PhoneBlocked`] or
    /// [`Input::PhoneBlockFailed`].
    BlockPhone { address: String, blocked: bool },
    /// Report [`Input::ClockTicked`] again at this instant, by the wall
    /// clock, besides on the minute.
    ReadClockAt(Timestamp),
    /// Keep these Quiet Hours for the next run, or with `None` none, and
    /// report when they next begin and end with every
    /// [`Input::ClockTicked`], starting at once.
    KeepQuietHours(Option<QuietHours>),
    /// The deadline of calls off for the Phone with this address passed:
    /// calls come back on. For the log.
    CallsOffDeadlinePassed { address: String },
    /// Calls were turned off during a Call, for the Phone with this
    /// address: they go off once no Call is left. For the log.
    CallsOffPending { address: String },
    /// The turn-off waiting for the last Call to end, for the Phone with
    /// this address, was cancelled: calls stay on. For the log.
    CallsKeptOn { address: String },
}
