//! The Setup Window's step, as the window shows it.

use super::{ContactsOutcome, KeybindingStage, ReadingContacts};
use crate::call::Timestamp;
use crate::connection::Phone;

/// The Setup Window's step.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SetupStep {
    /// Choose from the nearby Phones, in the order found.
    Choose { phones: Vec<Phone> },
    /// Waiting for the chosen Phone's pairing code.
    Pairing { phone: Phone },
    /// The code both screens should show, and Confirm.
    Confirm { phone: Phone, code: u32 },
    /// Confirmed; waiting for the pairing to finish.
    Confirmed { phone: Phone },
    /// Paired; connecting for Calls.
    Connecting { phone: Phone },
    /// The Phone refused hands-free: "Phone calls" is off for the Desktop.
    PhoneCallsOff { phone: Phone },
    /// The Phone didn't answer: out of range, or its Bluetooth is off.
    PhoneUnreachable { phone: Phone },
    /// The Phone no longer has this computer paired: Pair again.
    PairingLost { phone: Phone },
    /// Pair again: removing this computer's old pairing, and looking for
    /// the Phone to pair it afresh.
    PairingAgain { phone: Phone },
    /// Connected for Calls; the Phone is about to ask whether to share
    /// Contacts.
    ContactsWarning { phone: Phone },
    /// The Phone is asking whether to share Contacts, until `deadline`.
    ContactsPrompt { phone: Phone, deadline: Timestamp },
    /// Nobody answered the Phone before its prompt timed out.
    ContactsPromptTimedOut { phone: Phone },
    /// Shared: the sync is reading. `call_history` once the newest Calls
    /// are read, `contacts` while Contacts are read, and `contacts_read`
    /// once they are, while older Calls still are.
    ContactsSyncing {
        phone: Phone,
        call_history: bool,
        contacts: Option<ReadingContacts>,
        contacts_read: bool,
    },
    /// The Phone left before Contacts were read; `back` once it is
    /// connected again, to carry on.
    ContactsPhoneAway { phone: Phone, back: bool },
    /// Finished: what became of Contacts, and the keybinding. `connected`
    /// while the Phone is. `pair_another` when the window opened here for
    /// the Phone already set up, and offers to pair another.
    Done {
        phone: Phone,
        connected: bool,
        contacts: ContactsOutcome,
        keybinding: KeybindingStage,
        pair_another: bool,
    },
}

impl SetupStep {
    /// The lower-case name the control interface's `Surfaces` and the
    /// journal use.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Choose { .. } => "choose",
            Self::Pairing { .. } => "pairing",
            Self::Confirm { .. } => "confirm",
            Self::Confirmed { .. } => "confirmed",
            Self::Connecting { .. } => "connecting",
            Self::PhoneCallsOff { .. } => "phone-calls-off",
            Self::PhoneUnreachable { .. } => "phone-unreachable",
            Self::PairingLost { .. } => "pairing-lost",
            Self::PairingAgain { .. } => "pairing-again",
            Self::ContactsWarning { .. } => "contacts-warning",
            Self::ContactsPrompt { .. } => "contacts-prompt",
            Self::ContactsPromptTimedOut { .. } => "contacts-prompt-timed-out",
            Self::ContactsSyncing { .. } => "contacts-syncing",
            Self::ContactsPhoneAway { .. } => "contacts-phone-away",
            Self::Done { .. } => "done",
        }
    }
}
