//! The Problems the user can put right: their names, and what to say
//! about each.

use super::{Message, NextAction, checks};

/// Something wrong that the user can put right, shown until it is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProblemKind {
    /// The Phone refuses to share Contacts and Call History.
    ContactsDenied,
    /// The Phone asked whether to share Contacts, and nobody answered.
    ContactsPromptUnanswered,
    /// The Phone no longer has the Desktop paired.
    PairingLost,
    /// The Desktop's Bluetooth is off.
    BluetoothOff,
    /// The Desktop's Bluetooth is kept off by its radio switch.
    BluetoothBlocked,
    /// The Desktop can't sync Contacts and Call History: its Bluetooth
    /// file-transfer service isn't installed.
    ContactsSupportMissing,
    /// The Desktop's audio doesn't connect hands-free to the Phone by
    /// itself, or doesn't keep a Call's audio on the Phone until asked.
    AudioNotSetUp,
    /// BlueZ's own experimental hands-free competes with Desktop Calls for
    /// the Phone's hands-free link.
    BluezHandsFree,
}

impl ProblemKind {
    /// The lower-case kind the control interface's `Problems` carries.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ContactsDenied => "contacts-denied",
            Self::ContactsPromptUnanswered => "contacts-prompt-unanswered",
            Self::PairingLost => "pairing-lost",
            Self::BluetoothOff => "bluetooth-off",
            Self::BluetoothBlocked => "bluetooth-blocked",
            Self::ContactsSupportMissing => "contacts-support-missing",
            Self::AudioNotSetUp => "audio-not-set-up",
            Self::BluezHandsFree => "bluez-hands-free",
        }
    }
}

/// What to tell the user about `problem`: one line, saying how to put it
/// right.
#[must_use]
pub fn problem(problem: ProblemKind) -> Message {
    if let Some(message) = checks::problem(problem) {
        return message;
    }
    let (text, next_action) = match problem {
        ProblemKind::ContactsDenied => (
            "Your phone isn't sharing Contacts, so Calls show numbers instead of names; to allow it, open Bluetooth settings on your phone, tap the settings icon next to this computer and turn on Contacts and call history.",
            None,
        ),
        ProblemKind::ContactsPromptUnanswered => (
            "Your phone asked whether to share Contacts and nobody answered, so Calls show numbers instead of names; turn Bluetooth off and on again on your phone to be asked again.",
            None,
        ),
        ProblemKind::PairingLost => (
            "Your phone has forgotten this computer, so Calls can't reach it; choose Pair again to pair them again.",
            Some(NextAction::Command("desktop-calls connect")),
        ),
        ProblemKind::BluetoothOff
        | ProblemKind::BluetoothBlocked
        | ProblemKind::ContactsSupportMissing
        | ProblemKind::AudioNotSetUp
        | ProblemKind::BluezHandsFree => ("", None),
    };
    Message { text, next_action }
}

/// A Problem in a few words, for a menu entry or a tooltip.
#[must_use]
pub fn problem_heading(problem: ProblemKind) -> &'static str {
    match problem {
        ProblemKind::ContactsDenied => "Contacts not shared",
        ProblemKind::ContactsPromptUnanswered => "Contacts request unanswered",
        ProblemKind::PairingLost => "Pairing lost",
        desktop => checks::heading(desktop).unwrap_or_default(),
    }
}
