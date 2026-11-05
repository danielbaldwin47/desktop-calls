//! What the Desktop's own Problems say, found by the setup checks, and the
//! words for putting them right.

use super::{Message, NextAction};
use crate::machine::{DesktopFix, FixFailure};
use crate::messages::ProblemKind;

/// What to tell the user about one of the Desktop's own Problems, or
/// `None` for a Problem of the Phone's.
pub(super) fn problem(problem: ProblemKind) -> Option<Message> {
    let (text, next_action) = match problem {
        ProblemKind::BluetoothOff => (
            "This computer's Bluetooth is off, so your phone can't connect; choose Turn on to turn it on.",
            None,
        ),
        ProblemKind::BluetoothBlocked => (
            "This computer's Bluetooth is switched off by airplane mode or a wireless switch, so it can't be turned on from here; turn airplane mode off, or copy this command and run it in a terminal.",
            Some(NextAction::Command("rfkill unblock bluetooth")),
        ),
        ProblemKind::ContactsSupportMissing => (
            "This computer is missing the Bluetooth file transfer service that brings Contacts and Call History over from your phone; copy this command and run it in a terminal to install it.",
            Some(NextAction::Command("sudo pacman -S --needed bluez-obex")),
        ),
        ProblemKind::AudioNotSetUp => (
            "This computer's audio isn't set up to connect to your phone for Calls by itself and keep their audio on the phone until you move it here; choose Set up audio to save that setting and restart the audio.",
            None,
        ),
        ProblemKind::BluezHandsFree => (
            "An experimental calling feature of this computer's Bluetooth is on and takes your phone's Calls before Desktop Calls can; to turn it off, set Experimental = false in /etc/bluetooth/main.conf (and remove -E from bluetoothd's command line if it is there), then copy this command and run it in a terminal.",
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
        ProblemKind::AudioNotSetUp => "Audio not set up for Calls",
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
            "Bluetooth didn't turn on; try again, or turn it on in your Bluetooth settings."
        }
        FixFailure::AudioNotSaved => {
            return format!(
                "The audio setting couldn't be saved; check that you can write to {settings_folder}, then try again."
            );
        }
        FixFailure::AudioNotRestarted => {
            "The audio setting is saved, but the audio didn't restart; log out and back in to use it."
        }
        FixFailure::CallInProgress => {
            "Setting up audio restarts it, which would cut off your Call; try again once the Call ends."
        }
    };
    text.to_owned()
}

/// The button that copies a Problem's command.
#[must_use]
pub fn copy_command_label() -> &'static str {
    "Copy command"
}

/// What the Setup Window says once a command is copied.
#[must_use]
pub fn command_copied() -> &'static str {
    "Copied; paste it in a terminal."
}
