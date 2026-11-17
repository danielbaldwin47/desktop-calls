//! What the Desktop's own Problems say, found by the setup checks, and the
//! words for putting them right.

use super::{Message, NextAction};
use crate::machine::{DesktopFix, FixFailure};
use crate::messages::ProblemKind;

/// What to tell the user about one of the Desktop's own Problems, or
/// `None` for a Problem of the Phone's.
pub(super) fn problem(problem: ProblemKind) -> Option<Message> {
    let (text, next_action) = match problem {
        ProblemKind::BluetoothOff => ("Your phone can't connect.", None),
        ProblemKind::BluetoothBlocked => (
            "Turn airplane mode off, or run this command in a terminal.",
            Some(NextAction::Command("rfkill unblock bluetooth")),
        ),
        ProblemKind::ContactsSupportMissing => (
            "Needed for names and call history. To install it, run this command in a terminal.",
            Some(NextAction::Command("sudo pacman -S --needed bluez-obex")),
        ),
        ProblemKind::AudioNotSetUp => (
            "Your phone's calls won't show here until it is. Setting it up restarts the audio.",
            None,
        ),
        ProblemKind::BluezHandsFree => (
            "It takes your phone's calls before Desktop Calls can. Set Experimental = false in /etc/bluetooth/main.conf (and remove -E from bluetoothd's command line, if it's there), then run this command in a terminal.",
            Some(NextAction::Command("sudo systemctl restart bluetooth")),
        ),
        ProblemKind::ContactsDenied
        | ProblemKind::ContactsPromptUnanswered
        | ProblemKind::PairingLost => return None,
    };
    Some(Message { text, next_action })
}

/// One of the Desktop's own Problems in a few words, or `None` for a
/// Problem of the Phone's.
pub(super) fn heading(problem: ProblemKind) -> Option<&'static str> {
    Some(match problem {
        ProblemKind::BluetoothOff => "Bluetooth is off",
        ProblemKind::BluetoothBlocked => "Bluetooth is blocked",
        ProblemKind::ContactsSupportMissing => "Contacts support not installed",
        ProblemKind::AudioNotSetUp => "Audio not set up for calls",
        ProblemKind::BluezHandsFree => "Bluetooth's own calling is on",
        ProblemKind::ContactsDenied
        | ProblemKind::ContactsPromptUnanswered
        | ProblemKind::PairingLost => return None,
    })
}

/// The button that puts `problem` right in the Setup Window, if the
/// Desktop can put it right itself.
#[must_use]
pub fn fix_label(problem: ProblemKind) -> Option<&'static str> {
    match problem {
        ProblemKind::BluetoothOff => Some("Turn on"),
        ProblemKind::AudioNotSetUp => Some("Set up audio"),
        _ => None,
    }
}

/// What the Setup Window says while `fix` is under way.
#[must_use]
pub fn fixing(fix: DesktopFix) -> &'static str {
    match fix {
        DesktopFix::TurnOnBluetooth => "Turning Bluetooth on…",
        DesktopFix::SetUpAudio => "Setting up audio…",
    }
}

/// Why putting a Problem right didn't work. `settings_folder` is where the
/// audio settings are saved.
#[must_use]
pub fn fix_failed(failure: FixFailure, settings_folder: &str) -> String {
    let text = match failure {
        FixFailure::BluetoothNotTurnedOn => {
            "Bluetooth didn't turn on. Try your Bluetooth settings."
        }
        FixFailure::AudioNotSaved => {
            return format!("Couldn't save the audio setting in {settings_folder}.");
        }
        FixFailure::AudioNotRestarted => {
            "Saved, but the audio didn't restart. Log out and back in."
        }
        FixFailure::CallInProgress => "That would cut off your call. Try again once it ends.",
    };
    text.to_owned()
}

/// The button that copies a Problem's command.
#[must_use]
pub fn copy_command_label() -> &'static str {
    "Copy command"
}

/// The keycap on a Problem's button while Enter presses it.
#[must_use]
pub fn problem_keycap() -> &'static str {
    "Enter"
}

/// What the Setup Window says once a command is copied.
#[must_use]
pub fn command_copied() -> &'static str {
    "Copied"
}
