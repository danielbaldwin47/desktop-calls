//! Messages: every sentence and label the user reads comes from here.
//!
//! A failure kind goes in; one plain sentence and an optional next action come
//! out. The surfaces' labels and the Call timer's text come from here too. No
//! other code writes text for the user.

use crate::call::{AudioLocation, Caller, Timestamp};
use crate::contacts::CallHistoryKind;
use crate::contacts::NumberLabel;
use crate::indicator::{IndicatorEntry, IndicatorStatus, IndicatorView};
use crate::machine::RingingCaller;
use crate::matching::{ContactMatch, DialerRow, DialerRowKind};
use crate::missed::MissedCallNotice;
use crate::{ConnectionState, Phone};

mod call_window;
mod calls_off;
mod checks;
mod problems;
mod setup;
pub use call_window::{contact_lost, elapsed, state_line};
pub use calls_off::{
    calls_off_status, calls_off_until_unknown, calls_turned, phone_left_blocked_by_removal,
    phone_unblocked_for_removal,
};
pub use checks::{command_copied, copy_command_label, fix_failed, fix_label, fixing};
pub use problems::{ProblemKind, problem, problem_heading};
pub use setup::{SetupWindowText, setup_window};

/// A failure the user can be told about.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Failure {
    /// The CLI could not reach the long-running process.
    ServiceNotRunning,
    /// The long-running process answered with something the CLI can't read.
    UnexpectedReply,
    /// The long-running process could not start.
    ServiceFailedToStart,
    /// The Phone did not answer the Call.
    AnswerFailed,
    /// The Phone did not hang up the Call.
    HangUpFailed,
    /// The Desktop asked to hang up the Call, and can't tell whether the
    /// Phone did: the Hang up may have been lost on its way, or the Phone may
    /// have taken it with no word back before hands-free dropped.
    CallMayStillBeLive,
    /// The Desktop asked for the Call's audio, and the Phone kept it.
    AudioNotMoved,
    /// The Desktop asked to move the Call's audio to the Phone, and it
    /// stayed on the Desktop.
    AudioNotMovedToPhone,
    /// The Call asked about is over, or never existed.
    NoSuchCall,
    /// The Call asked about is no longer ringing.
    CallNotRinging,
    /// What the user asked to Call isn't a number the Phone can dial.
    Undialable,
    /// A Call can't be placed: no Phone is set up.
    PhoneNotSetUp,
    /// A Call can't be placed: the Phone is paired but not connected yet.
    PhoneConnecting,
    /// A Call can't be placed: the Phone is out of range or has Bluetooth off.
    PhoneOutOfRange,
    /// A Call can't be placed: the Phone no longer has the Desktop paired.
    PhonePairingLost,
    /// The Bluetooth connection to the Phone dropped mid-Call, which carries
    /// on on the Phone; the Desktop can't act on it until the Phone is back.
    ConnectionLost,
    /// A Call can't be placed while another Call is live.
    CallInProgress,
    /// The Phone refused the number it was asked to dial.
    NumberRefused,
    /// The Phone refused to dial because it is busy with a Call or a command.
    PhoneBusy,
    /// The Phone doesn't let the Desktop place Calls.
    DialNotSupported,
    /// The Phone has no mobile network to place the Call on.
    NoSignal,
    /// The mobile network didn't respond when the Phone dialled.
    NetworkError,
    /// The Phone didn't place the Call, for a reason it didn't give.
    DialFailed,
    /// There is no connected Call to act on.
    NoLiveCall,
    /// Mute works only while the Audio Location is the Desktop.
    MuteUnavailable,
    /// The Phone did not mute the microphone.
    MuteFailed,
    /// The Phone did not turn the microphone back on.
    UnmuteFailed,
    /// Keypad tones can only be digits, `*` and `#`.
    InvalidTones,
    /// The Phone did not play the keypad tones.
    TonesNotSent,
    /// The Call History row the user chose has a withheld number.
    WithheldNumber,
    /// No Call is waiting to be answered with Hold & Answer or End & Answer.
    NoWaitingCall,
    /// No Call is on hold for Swap to swap to.
    NoHeldCall,
    /// The Phone didn't hold the live Call and answer the waiting one; both
    /// are as they were.
    HoldAndAnswerFailed,
    /// The Phone didn't end the live Call and answer the waiting one; both
    /// are as they were.
    EndAndAnswerFailed,
    /// The Phone didn't swap the live Call and the held one; both are as
    /// they were.
    SwapFailed,
    /// Hold & Answer while the user's own Call is still being placed: a
    /// Call nobody has answered can't be put on hold.
    NotConnectedYet,
    /// Swap, or Hang up on either Call, while one is on hold and a Waiting
    /// Call rings: the Phone would answer the Waiting Call (`AT+CHLD=2`,
    /// `AT+CHLD=1`) or decline it (`AT+CHLD=0`) instead.
    WaitingCallFirst,
    /// Hold & Answer, or Answer, while another Call is on hold already: the
    /// Phone holds one Call at a time.
    AlreadyHolding,
    /// Hold & Answer or End & Answer while the only other Call is on hold:
    /// there is no live Call to hold or end.
    NothingToHoldOrEnd,
    /// A Call placed while calls are off: the Phone isn't connected.
    CallsOff,
    /// Calls turned off while a Call rings, is live, held, waiting or being
    /// placed, until a turn-off can wait for it (ticket #204).
    CallsOffDuringCall,
    /// BlueZ didn't block the Phone, so calls are still on.
    CallsOffFailed,
    /// BlueZ didn't unblock the Phone, so calls are still off.
    CallsOnFailed,
}

impl Failure {
    /// The lower-case kind the control interface's `Error` signal carries.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ServiceNotRunning => "service-not-running",
            Self::UnexpectedReply => "unexpected-reply",
            Self::ServiceFailedToStart => "service-failed-to-start",
            Self::AnswerFailed => "answer-failed",
            Self::HangUpFailed => "hang-up-failed",
            Self::CallMayStillBeLive => "call-may-still-be-live",
            Self::AudioNotMoved => "audio-not-moved",
            Self::AudioNotMovedToPhone => "audio-not-moved-to-phone",
            Self::NoSuchCall => "no-such-call",
            Self::CallNotRinging => "call-not-ringing",
            Self::Undialable => "undialable",
            Self::PhoneNotSetUp => "phone-not-set-up",
            Self::PhoneConnecting => "phone-connecting",
            Self::PhoneOutOfRange => "phone-out-of-range",
            Self::PhonePairingLost => "phone-pairing-lost",
            Self::ConnectionLost => "connection-lost",
            Self::CallInProgress => "call-in-progress",
            Self::NumberRefused => "number-refused",
            Self::PhoneBusy => "phone-busy",
            Self::DialNotSupported => "dial-not-supported",
            Self::NoSignal => "no-signal",
            Self::NetworkError => "network-error",
            Self::DialFailed => "dial-failed",
            Self::NoLiveCall => "no-live-call",
            Self::MuteUnavailable => "mute-unavailable",
            Self::MuteFailed => "mute-failed",
            Self::UnmuteFailed => "unmute-failed",
            Self::InvalidTones => "invalid-tones",
            Self::TonesNotSent => "tones-not-sent",
            Self::WithheldNumber => "withheld-number",
            Self::NoWaitingCall => "no-waiting-call",
            Self::NoHeldCall => "no-held-call",
            Self::HoldAndAnswerFailed => "hold-and-answer-failed",
            Self::EndAndAnswerFailed => "end-and-answer-failed",
            Self::SwapFailed => "swap-failed",
            Self::NotConnectedYet => "not-connected-yet",
            Self::WaitingCallFirst => "waiting-call-first",
            Self::AlreadyHolding => "already-holding",
            Self::NothingToHoldOrEnd => "nothing-to-hold-or-end",
            Self::CallsOff => "calls-off",
            Self::CallsOffDuringCall => "calls-off-during-call",
            Self::CallsOffFailed => "calls-off-failed",
            Self::CallsOnFailed => "calls-on-failed",
        }
    }
}

/// What the user can do about a failure.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NextAction {
    /// A command to copy and run.
    Command(&'static str),
}

/// A sentence for the user, with what they can do next.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Message {
    pub text: &'static str,
    pub next_action: Option<NextAction>,
}

/// What to tell the user about `failure`.
#[must_use]
#[expect(
    clippy::too_many_lines,
    reason = "one arm per Failure: the sentences read best side by side, as one table"
)]
pub fn failure(failure: Failure) -> Message {
    let (text, next_action) = match failure {
        Failure::ServiceNotRunning => (
            "Desktop Calls isn't running.",
            Some(NextAction::Command("desktop-calls service")),
        ),
        Failure::UnexpectedReply => (
            "Desktop Calls answered in a way this command doesn't understand; is an older version still running?",
            None,
        ),
        Failure::ServiceFailedToStart => (
            "Desktop Calls couldn't start; the journal has the details.",
            Some(NextAction::Command(
                "journalctl --user -t desktop-calls -n 20",
            )),
        ),
        Failure::AnswerFailed => (
            "Your phone didn't answer the Call; answer it on your phone instead.",
            None,
        ),
        Failure::HangUpFailed => (
            "Your phone didn't hang up the Call; hang up on your phone instead.",
            None,
        ),
        Failure::CallMayStillBeLive => (
            "The Call may still be live on your phone; check your phone and hang up there if it is.",
            None,
        ),
        Failure::AudioNotMoved => (
            "Your phone kept the audio; to hear the Call here, tap the audio button on your phone's call screen.",
            None,
        ),
        Failure::AudioNotMovedToPhone => (
            "The audio stayed here; to move it to your phone, tap the audio button on your phone's call screen.",
            None,
        ),
        Failure::NoSuchCall => ("That Call has already ended.", None),
        Failure::CallNotRinging => ("That Call isn't ringing any more.", None),
        Failure::Undialable => (
            "That isn't a number your phone can dial; use digits, spaces and + * # only.",
            None,
        ),
        Failure::PhoneNotSetUp => ("Phone not connected: no phone is set up yet.", None),
        Failure::PhoneConnecting => ("Phone not connected yet; try again in a moment.", None),
        Failure::PhoneOutOfRange => (
            "Phone not connected: it's out of range or has Bluetooth off.",
            None,
        ),
        Failure::PhonePairingLost => (
            "Phone not connected: it has forgotten this computer, so pair them again.",
            Some(NextAction::Command("desktop-calls connect")),
        ),
        Failure::ConnectionLost => (
            "Connection to phone lost: the Call continues on your phone.",
            None,
        ),
        Failure::CallInProgress => ("Hang up the current Call first.", None),
        Failure::NumberRefused => (
            "Your phone can't dial that number; check it and try again.",
            None,
        ),
        Failure::PhoneBusy => (
            "Your phone is busy with another Call; try again in a moment.",
            None,
        ),
        Failure::DialNotSupported => (
            "Your phone doesn't let this computer place Calls; call from your phone instead.",
            None,
        ),
        Failure::NoSignal => (
            "Your phone has no mobile signal, or is in airplane mode.",
            None,
        ),
        Failure::NetworkError => (
            "The mobile network didn't answer your phone; try again in a moment.",
            None,
        ),
        Failure::DialFailed => (
            "Your phone couldn't place the Call; check it has signal and isn't in airplane mode.",
            None,
        ),
        Failure::NoLiveCall => ("There's no connected Call right now.", None),
        Failure::MuteUnavailable => (
            "Mute works only while the Call's audio is on this computer; mute on your phone instead.",
            None,
        ),
        Failure::MuteFailed => (
            "Your phone didn't mute the microphone, so the other person can still hear you.",
            None,
        ),
        Failure::UnmuteFailed => (
            "Your phone didn't turn the microphone back on, so you're still muted; try again.",
            None,
        ),
        Failure::InvalidTones => ("Only digits, * and # can be sent as tones.", None),
        Failure::TonesNotSent => ("Your phone didn't send the tones; try again.", None),
        Failure::WithheldNumber => (
            "That caller withheld their number, so it can't be called back.",
            None,
        ),
        Failure::NoWaitingCall => ("There's no Waiting Call to answer.", None),
        Failure::NoHeldCall => ("There's no Call on hold to swap to.", None),
        Failure::HoldAndAnswerFailed => (
            "Your phone didn't put your Call on hold, so both Calls are as they were; answer on your phone instead.",
            None,
        ),
        Failure::EndAndAnswerFailed => (
            "Your phone didn't end your Call, so both Calls are as they were; end it on your phone instead.",
            None,
        ),
        Failure::SwapFailed => (
            "Your phone didn't swap the Calls, so both are as they were; swap them on your phone instead.",
            None,
        ),
        Failure::NotConnectedYet => (
            "Your Call hasn't connected yet, so it can't be put on hold; choose End & Answer or Decline.",
            None,
        ),
        Failure::WaitingCallFirst => (label(Label::WaitingCallFirst), None),
        Failure::AlreadyHolding => (
            "You already have a Call on hold, so this one can't go on hold too; choose End & Answer or Decline.",
            None,
        ),
        Failure::NothingToHoldOrEnd => (
            "Your other Call is on hold already, so there's nothing to hold or end; choose Answer, which keeps it on hold, or Decline.",
            None,
        ),
        Failure::CallsOff => (
            "Calls are off, so your phone isn't connected; turn calls on to call.",
            Some(NextAction::Command("desktop-calls calls on")),
        ),
        Failure::CallsOffDuringCall => (
            "Calls can't be turned off during a Call; turn them off once it has ended.",
            None,
        ),
        Failure::CallsOffFailed => (
            "Calls couldn't be turned off: Bluetooth didn't disconnect your phone. Try again.",
            None,
        ),
        Failure::CallsOnFailed => (
            "Calls couldn't be turned on: Bluetooth didn't let your phone connect again. Try again.",
            None,
        ),
    };
    Message { text, next_action }
}

/// The Indicator's status: its menu's first line, and its tooltip without
/// the Problems.
#[must_use]
pub fn indicator_status(status: &IndicatorStatus) -> String {
    match status {
        IndicatorStatus::NoPhone => "No Phone set up".to_owned(),
        IndicatorStatus::CallsOff => calls_off::CALLS_OFF.to_owned(),
        IndicatorStatus::NotConnected(phone) => format!("{} not connected", phone.name),
        IndicatorStatus::Connected(phone) => format!("{} connected", phone.name),
        IndicatorStatus::OutOfRange(phone) => format!("{} out of range", phone.name),
        // The Call Window's words for it.
        IndicatorStatus::ConnectionLost(phone) => format!("Connection to {} lost", phone.name),
        // No one to say the Call is from yet: the Banner's own words.
        IndicatorStatus::Ringing(RingingCaller::Identifying) => {
            label(Label::IdentifyingCaller).to_owned()
        }
        IndicatorStatus::Ringing(RingingCaller::Known { caller, contact }) => {
            format!("Call from {}", caller_name(caller, contact.as_ref()))
        }
        IndicatorStatus::InCall { caller, contact } => {
            format!("In a Call with {}", caller_name(caller, contact.as_ref()))
        }
        // The Call Window's notice, naming the Call.
        IndicatorStatus::CallOnPhone { caller, contact } => format!(
            "Connection to phone lost: the Call with {} continues on your phone",
            caller_name(caller, contact.as_ref())
        ),
    }
}

/// What the Indicator says on hover: its status, then each Problem.
#[must_use]
pub fn indicator_tooltip(view: &IndicatorView) -> String {
    let mut tooltip = indicator_status(&view.status);
    for problem in &view.problems {
        tooltip.push_str(" · ");
        tooltip.push_str(problem_heading(*problem));
    }
    tooltip
}

/// What an entry in the Indicator's menu says. The status line says
/// [`indicator_status`]; a separator says nothing.
#[must_use]
pub fn indicator_entry(entry: &IndicatorEntry, view: &IndicatorView) -> String {
    match entry {
        IndicatorEntry::Status => indicator_status(&view.status),
        IndicatorEntry::OpenDialer => "Open Dialer".to_owned(),
        IndicatorEntry::ConnectPhone => "Connect your phone…".to_owned(),
        IndicatorEntry::Separator => String::new(),
        IndicatorEntry::TurnCallsOff { .. } => calls_off::TURN_CALLS_OFF.to_owned(),
        IndicatorEntry::CallsOffUntilTurnedOn => calls_off::UNTIL_TURNED_ON.to_owned(),
        IndicatorEntry::TurnCallsOn => calls_off::TURN_CALLS_ON.to_owned(),
        IndicatorEntry::HangUp(_) => label(Label::HangUp).to_owned(),
        IndicatorEntry::HangUpOnPhone => label(Label::HangUpOnPhone).to_owned(),
        IndicatorEntry::Mute(_) => label(Label::Mute).to_owned(),
        IndicatorEntry::MoveAudio {
            to: AudioLocation::Desktop,
            moving: true,
        } => label(Label::MovingAudioHere).to_owned(),
        IndicatorEntry::MoveAudio {
            to: AudioLocation::Phone,
            moving: true,
        } => label(Label::MovingAudioToPhone).to_owned(),
        IndicatorEntry::MoveAudio {
            to: AudioLocation::Desktop,
            ..
        } => "Move audio to this computer".to_owned(),
        IndicatorEntry::MoveAudio {
            to: AudioLocation::Phone,
            ..
        } => label(Label::MoveAudioToPhone).to_owned(),
        // Its click opens the Setup Window.
        IndicatorEntry::Problem(ProblemKind::PairingLost) => "Pairing lost: pair again…".to_owned(),
        IndicatorEntry::Problem(
            problem @ (ProblemKind::ContactsDenied | ProblemKind::ContactsPromptUnanswered),
        ) => problem_heading(*problem).to_owned(),
        // Its click opens the Setup Window, which puts it right.
        IndicatorEntry::Problem(problem) => format!("{}: put it right…", problem_heading(*problem)),
    }
}

/// What `desktop-calls audio here` or `audio phone` did: the Phone was
/// asked to move the audio to `location`, or it was there already.
#[must_use]
pub fn audio_requested(location: AudioLocation, already_there: bool) -> &'static str {
    match (location, already_there) {
        (AudioLocation::Desktop, false) => "Moving the audio to this computer.",
        (AudioLocation::Desktop, true) => "The audio is already on this computer.",
        (AudioLocation::Phone, false) => "Moving the audio to your phone.",
        (AudioLocation::Phone, true) => "The audio is already on your phone.",
    }
}

/// Why the control interface's `MoveAudio` refused its argument.
#[must_use]
pub fn unknown_audio_location() -> &'static str {
    "The audio can move to \"desktop\" or \"phone\"."
}

/// What `desktop-calls mute` did.
#[must_use]
pub fn mute_toggled(muted: bool) -> &'static str {
    if muted { "Muted." } else { "Unmuted." }
}

/// One sentence saying whether the Phone is connected.
/// `hands_free_lost`: the Phone is in range without hands-free, which the
/// Desktop is connecting again (ticket #216).
#[must_use]
pub fn status(
    connection_state: &ConnectionState,
    phone: Option<&Phone>,
    hands_free_lost: bool,
) -> String {
    match (connection_state, phone) {
        (ConnectionState::NotSetUp, _) | (_, None) => "No Phone is set up yet.".to_owned(),
        (ConnectionState::Connecting, Some(phone)) => {
            format!("Connecting to {}.", describe(phone))
        }
        (ConnectionState::Connected, Some(phone)) => format!("Connected to {}.", describe(phone)),
        (ConnectionState::Disconnected, Some(phone)) if hands_free_lost => format!(
            "{} is in range, but not connected for Calls: connecting it again.",
            describe(phone)
        ),
        (ConnectionState::Disconnected, Some(phone)) => {
            format!("{} is out of range or has Bluetooth off.", describe(phone))
        }
        (ConnectionState::PairingLost, Some(phone)) => format!(
            "{} has forgotten this computer; pair them again.",
            describe(phone)
        ),
    }
}

fn describe(phone: &Phone) -> String {
    format!("{} ({})", phone.name, phone.address)
}

/// A fixed piece of text on a surface.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Label {
    /// The Incoming Call Banner's heading.
    IncomingCall,
    /// The Incoming Call Banner's heading while it has keyboard focus: the
    /// keys that work.
    BannerKeys,
    /// The Incoming Call Banner's caller line while a Call that rang with
    /// no number may still get one: not yet "Withheld number", which may
    /// be untrue.
    IdentifyingCaller,
    Answer,
    Decline,
    HangUp,
    /// Hang up's label while the Desktop has lost contact with the Call,
    /// which carries on on the Phone: where to hang up instead.
    HangUpOnPhone,
    /// The Call Window's status while the Call connects.
    Connecting,
    /// The Call Window's status while a Call placed here starts.
    Calling,
    /// The Call Window's status while the other end rings.
    Ringing,
    /// The Call Window's status when a Call couldn't be placed.
    CallNotPlaced,
    /// The Call Window's status once the Call is over.
    CallEnded,
    /// Audio Location: the Phone.
    AudioOnPhone,
    /// The Handoff button while the audio is on the Desktop.
    MoveAudioToPhone,
    /// The Handoff button while the Phone brings the audio here.
    MovingAudioHere,
    /// The Handoff button while the audio goes to the Phone, until
    /// hands-free and the Call are back.
    MovingAudioToPhone,
    /// Wideband speech.
    Hd,
    /// The Call Window's button that mutes the microphone.
    Mute,
    /// The same button while the microphone is muted.
    Unmute,
    /// The Call Window's button that shows the keypad's buttons.
    Keypad,
    /// What the Call Window says, unmistakably, while muted.
    MicrophoneMuted,
    /// The keypad's tone entry, before anything is typed.
    KeypadPlaceholder,
    /// The Dialer's text entry, before anything is typed.
    DialerPlaceholder,
    /// What the Dialer lists when nothing matches what is typed.
    NoMatches,
    /// A Call History row's kind: a Missed Call.
    Missed,
    /// A Call History row's kind: an answered incoming Call.
    Incoming,
    /// A Call History row's kind: a Call placed from the Phone or here.
    Outgoing,
    /// A Call History row with a withheld number.
    CantCallBack,
    /// The Missed Call notification's button that places the Call.
    CallBack,
    /// The Missed Call notification's button that closes it.
    Dismiss,
    /// The Missed Call notification's click, for desktops that label it.
    ShowInDialer,
    /// The Incoming Call Banner's heading for a Waiting Call.
    WaitingCall,
    /// The same heading while the Banner has keyboard focus: the keys that
    /// work.
    WaitingCallKeys,
    /// The same, while the user's own Call is still being placed or another
    /// is on hold already, so only End & Answer and Decline are offered.
    WaitingCallEndKeys,
    /// The same, while the user's only other Call is on hold, so Answer and
    /// Decline are offered.
    WaitingCallAnswerKeys,
    /// The Banner's button that holds the live Call and answers the waiting
    /// one.
    HoldAndAnswer,
    /// The Banner's button that ends the live Call and answers the waiting
    /// one.
    EndAndAnswer,
    /// The Call Window's button that swaps the live Call and the held one.
    Swap,
    /// The same button with only a Call on hold: it makes that Call live
    /// again.
    Resume,
    /// What the Call Window says of a Call on hold.
    OnHold,
    /// What the Call Window says in place of Swap, Resume and Hang up while a
    /// Call is on hold and a Waiting Call rings.
    WaitingCallFirst,
    /// What the Call Pill says in place of the timer while the Desktop has
    /// lost contact with the Call, which carries on on the Phone.
    OnYourPhone,
    /// What the Call Pill adds while another Call is on hold.
    OneOnHold,
    /// The Call Window's audio tile while the audio is on the Desktop.
    ToPhone,
    /// The Call Window's audio tile while the audio is on the Phone.
    MoveHere,
    /// The Call Window's audio tile while a Handoff is under way.
    Moving,
    /// The line under a bare number: no Contact has it.
    NotInContacts,
    /// The heading of the Call Window's notice while the Desktop has lost
    /// contact with the Call.
    ConnectionToPhoneLost,
    /// The tooltip of the Call Window's Hang up for the Held Call.
    HangUpHeldCall,
}

/// The text of `label`.
#[must_use]
pub fn label(label: Label) -> &'static str {
    match label {
        Label::IncomingCall => "Incoming call",
        Label::BannerKeys => "Press A to answer or D to decline",
        Label::IdentifyingCaller => "Identifying caller…",
        Label::Answer => "Answer",
        Label::Decline => "Decline",
        Label::HangUp => "Hang up",
        Label::HangUpOnPhone => "Hang up on your phone",
        Label::Connecting => "Connecting…",
        Label::Calling => "Calling…",
        Label::Ringing => "Ringing…",
        Label::CallNotPlaced => "Call not placed",
        Label::CallEnded => "Call ended",
        Label::AudioOnPhone => "Audio on phone",
        Label::MoveAudioToPhone => "Move audio to phone",
        Label::MovingAudioHere => "Moving audio here…",
        Label::MovingAudioToPhone => "Moving audio to phone…",
        Label::Hd => "HD",
        Label::Mute => "Mute",
        Label::Unmute => "Unmute",
        Label::Keypad => "Keypad",
        Label::MicrophoneMuted => "Microphone muted",
        Label::KeypadPlaceholder => "Type or paste digits to send tones",
        Label::DialerPlaceholder => "Type a name or number",
        Label::NoMatches => "No matches",
        Label::Missed => "Missed",
        Label::Incoming => "Incoming",
        Label::Outgoing => "Outgoing",
        Label::CantCallBack => "can't be called back",
        Label::CallBack => "Call back",
        Label::Dismiss => "Dismiss",
        Label::ShowInDialer => "Show in Dialer",
        Label::WaitingCall => "Waiting Call",
        Label::WaitingCallKeys => "Press H to Hold & Answer, E to End & Answer, or D to decline",
        Label::WaitingCallEndKeys => "Press E to End & Answer, or D to decline",
        Label::WaitingCallAnswerKeys => {
            "Press A to answer, keeping your other Call on hold, or D to decline"
        }
        Label::HoldAndAnswer => "Hold & Answer",
        Label::EndAndAnswer => "End & Answer",
        Label::Swap => "Swap",
        Label::Resume => "Resume",
        Label::OnHold => "On hold",
        Label::WaitingCallFirst => {
            "Answer or decline the Waiting Call first: until then, your phone would act on it instead of these Calls."
        }
        Label::OnYourPhone => "On your phone",
        Label::OneOnHold => "1 on hold",
        Label::ToPhone => "To phone",
        Label::MoveHere => "Move here",
        Label::Moving => "Moving…",
        Label::NotInContacts => "Not in Contacts",
        Label::ConnectionToPhoneLost => "Connection to phone lost",
        Label::HangUpHeldCall => "Hang up the Held Call",
    }
}

/// How the other party is shown: the Contact's name, if Contacts name them
/// and the name isn't empty, otherwise as [`caller`] shows them.
#[must_use]
pub fn caller_name(caller: &Caller, contact: Option<&ContactMatch>) -> String {
    match contact {
        Some(contact) if !contact.name.trim().is_empty() => contact.name.clone(),
        _ => self::caller(caller),
    }
}

/// What kind of number a Contact's number is, as the surfaces show it.
#[must_use]
pub fn number_label(label: &NumberLabel) -> String {
    label.as_str().to_lowercase()
}

/// The line under a Contact's name: the number's label and the number,
/// such as `mobile · +44 7700 900123`.
#[must_use]
pub fn caller_detail(number_label: &str, number: &str) -> String {
    if number_label.is_empty() {
        number.to_owned()
    } else {
        format!("{number_label} · {number}")
    }
}

/// How the other party is shown until Contacts can name them: the number,
/// formatted for reading, or "Withheld number".
#[must_use]
pub fn caller(caller: &Caller) -> String {
    match caller {
        Caller::Number(number) => crate::number::format(number),
        Caller::Withheld => "Withheld number".to_owned(),
    }
}

/// Does in advance the work the first [`caller`] for a number would
/// otherwise do: loading the phone-number metadata. That work is slow enough
/// (over a second on a slow CPU in a debug build) to hold up the Incoming
/// Call Banner, so the shell calls this once, off the main thread, at
/// startup.
pub fn prepare_caller() {
    crate::number::prepare();
}

/// A Dialer row as shown: a title, and a detail line that may be empty.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DialerRowText {
    pub title: String,
    pub detail: String,
}

impl DialerRowText {
    /// The title and detail on one line, as the Dialer reports a row.
    #[must_use]
    pub fn line(&self) -> String {
        if self.detail.is_empty() {
            self.title.clone()
        } else {
            format!("{} · {}", self.title, self.detail)
        }
    }
}

/// What the Dialer shows for `row`: a Contact's name over the number's label
/// and the number; for a Call History row who it was over its kind
/// (Missed, Incoming or Outgoing) and `when`; for the number typed,
/// "Call 07700 900123".
#[must_use]
pub fn dialer_row(row: &DialerRow, when: Option<&str>) -> DialerRowText {
    let number_detail = || {
        caller_detail(
            &row.label.as_ref().map(number_label).unwrap_or_default(),
            &row.shown_number,
        )
    };
    match row.kind {
        DialerRowKind::CallNumber => DialerRowText {
            title: format!("Call {}", row.shown_number),
            detail: String::new(),
        },
        DialerRowKind::Contact if !row.name.trim().is_empty() => DialerRowText {
            title: row.name.clone(),
            detail: number_detail(),
        },
        DialerRowKind::Contact => DialerRowText {
            title: caller(&row.caller),
            detail: row.label.as_ref().map(number_label).unwrap_or_default(),
        },
        DialerRowKind::CallHistory(kind) => {
            let named = !row.name.trim().is_empty();
            let title = if named {
                row.name.clone()
            } else {
                caller(&row.caller)
            };
            let mut detail = vec![
                label(match kind {
                    CallHistoryKind::Missed => Label::Missed,
                    CallHistoryKind::Incoming => Label::Incoming,
                    CallHistoryKind::Outgoing => Label::Outgoing,
                })
                .to_owned(),
            ];
            if named {
                detail.push(number_detail());
            }
            detail.extend(when.map(str::to_owned));
            if row.caller == Caller::Withheld {
                detail.push(label(Label::CantCallBack).to_owned());
            }
            DialerRowText {
                title,
                detail: detail.join(" · "),
            }
        }
    }
}

/// What a desktop notification says: its summary, and the body under it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NotificationText {
    pub summary: String,
    pub body: String,
}

/// What the Missed Call notification says: who, with the Contact's number
/// label, and when, as [`when`] shows it; for repeats, how many and when
/// the latest was. Omarchy lets a notification through Do Not Disturb if it
/// looks like a verification code (4 to 8 digits near a word such as code,
/// pin or confirm), so none of these words is used, and in a Contact's name
/// or number label each is hidden from Omarchy (see
/// [`hide_verification_code_keywords`]).
#[must_use]
pub fn missed_call(
    notice: &MissedCallNotice,
    now: Timestamp,
    utc_offset_seconds: i64,
) -> NotificationText {
    let who = match (&notice.caller, notice.count) {
        (Caller::Withheld, 1) => "a withheld number".to_owned(),
        (Caller::Withheld, _) => "withheld numbers".to_owned(),
        (caller, 1) => match &notice.contact {
            Some(contact) if !contact.name.trim().is_empty() => {
                format!("{} ({})", contact.name, number_label(&contact.label))
            }
            _ => self::caller(caller),
        },
        (caller, _) => caller_name(caller, notice.contact.as_ref()),
    };
    let summary = if notice.count == 1 {
        format!("Missed call from {who}")
    } else {
        format!("{} missed calls from {who}", notice.count)
    };
    let time = when(notice.at, now, utc_offset_seconds);
    let mut body = if notice.count == 1 {
        time
    } else {
        format!("Latest {time}")
    };
    if notice.caller == Caller::Withheld {
        body.push_str(" · ");
        body.push_str(label(Label::CantCallBack));
    }
    NotificationText {
        summary: hide_verification_code_keywords(&summary),
        body: hide_verification_code_keywords(&body),
    }
}

/// An invisible character that isn't a letter, so it ends a word for
/// Omarchy's pattern matching. A line never breaks at it, and Omarchy's
/// notification card draws a summary with it pixel for pixel as without
/// (the word joiner, U+2060, comes from a fallback font that makes the line
/// taller).
const ZERO_WIDTH_NON_JOINER: char = '\u{200c}';

/// A part of a verification-code keyword.
enum KeywordPart {
    Letters(&'static str),
    /// White space or a hyphen.
    Separator,
    /// White space, a hyphen, or nothing.
    OptionalSeparator,
}

/// Omarchy 4.5's verification-code keywords: `CODE_WORDS` in its
/// notification plugin's `Detect.js`,
/// `code|otp|one[\s-]?time|verification|verify|passcode|pin|2fa|two[\s-]factor|security key|confirm(?:ation)?`.
const KEYWORDS: [&[KeywordPart]; 12] = {
    use KeywordPart::{Letters, OptionalSeparator, Separator};
    [
        &[Letters("code")],
        &[Letters("otp")],
        &[Letters("one"), OptionalSeparator, Letters("time")],
        &[Letters("verification")],
        &[Letters("verify")],
        &[Letters("passcode")],
        &[Letters("pin")],
        &[Letters("2fa")],
        &[Letters("two"), Separator, Letters("factor")],
        &[Letters("security key")],
        &[Letters("confirmation")],
        &[Letters("confirm")],
    ]
};

/// `text` with a [`ZERO_WIDTH_NON_JOINER`] after the first letter of each whole word
/// Omarchy takes for a verification-code keyword, so it finds none and a
/// Contact's name can't let a notification through Do Not Disturb. The text
/// looks the same.
///
/// Omarchy matches the keywords as JavaScript does without the `u` flag:
/// ignoring ASCII case only, with `\b` between an ASCII word character
/// (`[A-Za-z0-9_]`) and anything else, and `\s` as Unicode white space.
fn hide_verification_code_keywords(text: &str) -> String {
    let mut hidden = String::with_capacity(text.len());
    let mut after_word = false;
    for (at, character) in text.char_indices() {
        hidden.push(character);
        if !after_word && is_keyword_at(&text[at..]) {
            hidden.push(ZERO_WIDTH_NON_JOINER);
        }
        after_word = is_ascii_word(character);
    }
    hidden
}

/// Whether `text` starts with a whole verification-code keyword.
fn is_keyword_at(text: &str) -> bool {
    KEYWORDS.iter().any(|parts| {
        let mut rest = text;
        for part in *parts {
            rest = match part {
                KeywordPart::Letters(letters) => match rest.get(..letters.len()) {
                    Some(start) if start.eq_ignore_ascii_case(letters) => &rest[letters.len()..],
                    _ => return false,
                },
                KeywordPart::Separator | KeywordPart::OptionalSeparator => {
                    match rest.chars().next() {
                        Some(next) if next == '-' || is_javascript_space(next) => {
                            &rest[next.len_utf8()..]
                        }
                        _ if matches!(part, KeywordPart::OptionalSeparator) => rest,
                        _ => return false,
                    }
                }
            };
        }
        !rest.chars().next().is_some_and(is_ascii_word)
    })
}

/// JavaScript's `\w`.
fn is_ascii_word(character: char) -> bool {
    character.is_ascii_alphanumeric() || character == '_'
}

/// JavaScript's `\s`: white space and line terminators.
fn is_javascript_space(character: char) -> bool {
    matches!(
        character,
        '\t' | '\n' | '\u{b}' | '\u{c}' | '\r' | ' ' | '\u{a0}' | '\u{1680}' | '\u{2000}'
            ..='\u{200a}'
                | '\u{2028}'
                | '\u{2029}'
                | '\u{202f}'
                | '\u{205f}'
                | '\u{3000}'
                | '\u{feff}'
    )
}

const MONTHS: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

/// When a Call was, as the Dialer shows it: the time if today, "Yesterday"
/// and the time, otherwise the date, with the year if not this year.
/// `utc_offset_seconds` is the Desktop's time zone's offset now.
#[must_use]
pub fn when(at: Timestamp, now: Timestamp, utc_offset_seconds: i64) -> String {
    let local = |moment: Timestamp| {
        i64::try_from(moment.0 / 1_000).unwrap_or(i64::MAX) + utc_offset_seconds
    };
    let (at, now) = (local(at), local(now));
    let time = format!(
        "{:02}:{:02}",
        at.rem_euclid(86_400) / 3_600,
        at.rem_euclid(3_600) / 60
    );
    let (day, today) = (at.div_euclid(86_400), now.div_euclid(86_400));
    match today - day {
        0 => time,
        1 => format!("Yesterday {time}"),
        _ => {
            let (year, month, day) = civil_date(day);
            let month = MONTHS[month - 1];
            if year == civil_date(today).0 {
                format!("{day} {month}")
            } else {
                format!("{day} {month} {year}")
            }
        }
    }
}

/// The year, month (1 to 12) and day of the month of the day `days` after
/// 1970-01-01, in the proleptic Gregorian calendar (Howard Hinnant's
/// `civil_from_days`).
fn civil_date(days: i64) -> (i64, usize, i64) {
    let days = days + 719_468;
    let era = days.div_euclid(146_097);
    let day_of_era = days.rem_euclid(146_097);
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let shifted_month = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * shifted_month + 2) / 5 + 1;
    let month = if shifted_month < 10 {
        shifted_month + 3
    } else {
        shifted_month - 9
    };
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    (year, usize::try_from(month).unwrap_or(1), day)
}
