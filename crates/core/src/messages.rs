//! Messages: every sentence and label the user reads comes from here.
//!
//! A failure kind goes in; one plain sentence and an optional next action come
//! out. The surfaces' labels and the Call timer's text come from here too. No
//! other code writes text for the user.

use crate::call::Caller;
use crate::contacts::NumberLabel;
use crate::matching::ContactMatch;
use crate::{ConnectionState, Phone};

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
    /// The Call was answered, but its audio did not come to the Desktop.
    AudioNotMoved,
    /// The Call asked about is over, or never existed.
    NoSuchCall,
    /// The Call asked about is no longer ringing.
    CallNotRinging,
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
    /// The keybinding has nothing to focus, and the Dialer it would open
    /// isn't built yet.
    DialerUnavailable,
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
            Self::AudioNotMoved => "audio-not-moved",
            Self::NoSuchCall => "no-such-call",
            Self::CallNotRinging => "call-not-ringing",
            Self::NoLiveCall => "no-live-call",
            Self::MuteUnavailable => "mute-unavailable",
            Self::MuteFailed => "mute-failed",
            Self::UnmuteFailed => "unmute-failed",
            Self::InvalidTones => "invalid-tones",
            Self::TonesNotSent => "tones-not-sent",
            Self::DialerUnavailable => "dialer-unavailable",
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
        Failure::AudioNotMoved => (
            "The Call is answered, but its audio stayed on your phone; tap the audio button on your phone's call screen to switch.",
            None,
        ),
        Failure::NoSuchCall => ("That Call has already ended.", None),
        Failure::CallNotRinging => ("That Call isn't ringing any more.", None),
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
        Failure::DialerUnavailable => ("Desktop Calls can't open the Dialer yet.", None),
    };
    Message { text, next_action }
}

/// Something wrong that the user can put right, shown until it is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProblemKind {
    /// The Phone refuses to share Contacts and Call History.
    ContactsDenied,
    /// The Phone asked whether to share Contacts, and nobody answered.
    ContactsPromptUnanswered,
}

impl ProblemKind {
    /// The lower-case kind the control interface's `Problems` carries.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ContactsDenied => "contacts-denied",
            Self::ContactsPromptUnanswered => "contacts-prompt-unanswered",
        }
    }
}

/// What to tell the user about `problem`: one line, saying how to put it
/// right.
#[must_use]
pub fn problem(problem: ProblemKind) -> Message {
    let text = match problem {
        ProblemKind::ContactsDenied => {
            "Your phone isn't sharing Contacts, so Calls show numbers instead of names; to allow it, open Bluetooth settings on your phone, tap the settings icon next to this computer and turn on Contacts and call history."
        }
        ProblemKind::ContactsPromptUnanswered => {
            "Your phone asked whether to share Contacts and nobody answered, so Calls show numbers instead of names; turn Bluetooth off and on again on your phone to be asked again."
        }
    };
    Message {
        text,
        next_action: None,
    }
}

/// What `desktop-calls mute` did.
#[must_use]
pub fn mute_toggled(muted: bool) -> &'static str {
    if muted { "Muted." } else { "Unmuted." }
}

/// One sentence saying whether the Phone is connected.
#[must_use]
pub fn status(connection_state: &ConnectionState, phone: Option<&Phone>) -> String {
    match (connection_state, phone) {
        (ConnectionState::NotSetUp, _) | (_, None) => "No Phone is set up yet.".to_owned(),
        (ConnectionState::Connecting, Some(phone)) => {
            format!("Connecting to {}.", describe(phone))
        }
        (ConnectionState::Connected, Some(phone)) => format!("Connected to {}.", describe(phone)),
        (ConnectionState::Disconnected, Some(phone)) => {
            format!("{} is out of range or has Bluetooth off.", describe(phone))
        }
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
    Answer,
    Decline,
    HangUp,
    /// The Call Window's status while the Call connects.
    Connecting,
    /// The Call Window's status once the Call is over.
    CallEnded,
    /// Audio Location: the Desktop.
    AudioOnDesktop,
    /// Audio Location: the Phone.
    AudioOnPhone,
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
}

/// The text of `label`.
#[must_use]
pub fn label(label: Label) -> &'static str {
    match label {
        Label::IncomingCall => "Incoming call",
        Label::BannerKeys => "Press A to answer or D to decline",
        Label::Answer => "Answer",
        Label::Decline => "Decline",
        Label::HangUp => "Hang up",
        Label::Connecting => "Connecting…",
        Label::CallEnded => "Call ended",
        Label::AudioOnDesktop => "Audio on this computer",
        Label::AudioOnPhone => "Audio on phone",
        Label::Hd => "HD",
        Label::Mute => "Mute",
        Label::Unmute => "Unmute",
        Label::Keypad => "Keypad",
        Label::MicrophoneMuted => "Microphone muted",
        Label::KeypadPlaceholder => "Type or paste digits to send tones",
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

/// The Call timer's text for `seconds`: `m:ss`, or `h:mm:ss` from an hour.
#[must_use]
pub fn elapsed(seconds: u64) -> String {
    let (hours, minutes, seconds) = (seconds / 3_600, seconds / 60 % 60, seconds % 60);
    if hours > 0 {
        format!("{hours}:{minutes:02}:{seconds:02}")
    } else {
        format!("{minutes}:{seconds:02}")
    }
}
