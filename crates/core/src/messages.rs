//! Messages: every sentence and label the user reads comes from here.
//!
//! A failure kind goes in; one plain sentence and an optional next action come
//! out. The surfaces' labels and the Call timer's text come from here too. No
//! other code writes text for the user.

use crate::call::Caller;
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
    };
    Message { text, next_action }
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
}

/// The text of `label`.
#[must_use]
pub fn label(label: Label) -> &'static str {
    match label {
        Label::IncomingCall => "Incoming call",
        Label::Answer => "Answer",
        Label::Decline => "Decline",
        Label::HangUp => "Hang up",
        Label::Connecting => "Connecting…",
        Label::CallEnded => "Call ended",
        Label::AudioOnDesktop => "Audio on this computer",
        Label::AudioOnPhone => "Audio on phone",
        Label::Hd => "HD",
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
