//! Messages: every sentence the user reads comes from here.
//!
//! A failure kind goes in; one plain sentence and an optional next action come
//! out. No other code writes text for the user.

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
    match failure {
        Failure::ServiceNotRunning => Message {
            text: "Desktop Calls isn't running.",
            next_action: Some(NextAction::Command("desktop-calls service")),
        },
        Failure::UnexpectedReply => Message {
            text: "Desktop Calls answered in a way this command doesn't understand; is an older version still running?",
            next_action: None,
        },
        Failure::ServiceFailedToStart => Message {
            text: "Desktop Calls couldn't start; the journal has the details.",
            next_action: Some(NextAction::Command(
                "journalctl --user -t desktop-calls -n 20",
            )),
        },
    }
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
