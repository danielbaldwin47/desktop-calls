//! Contacts and Call History, as the Desktop holds them once synced from the
//! Phone, and how far the sync has got.

use std::fmt;
use std::sync::Arc;

use crate::call::{Caller, Timestamp};
use crate::matching::{Directory, HistoryIndex};

/// One of the Phone's Contacts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Contact {
    /// The name the Phone shows, which may be empty.
    pub name: String,
    pub numbers: Vec<ContactNumber>,
}

/// One of a Contact's numbers, as the Phone stores it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ContactNumber {
    pub number: String,
    pub label: NumberLabel,
}

/// What kind of number a Contact's number is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NumberLabel {
    Mobile,
    Home,
    Work,
    Main,
    Other,
    /// A label the person typed on the Phone.
    Custom(String),
}

impl NumberLabel {
    /// The lower-case name the control interface and the cache use; a
    /// custom label as typed.
    #[must_use]
    pub fn as_str(&self) -> &str {
        match self {
            Self::Mobile => "mobile",
            Self::Home => "home",
            Self::Work => "work",
            Self::Main => "main",
            Self::Other => "other",
            Self::Custom(label) => label,
        }
    }

    /// The label named `name` by [`NumberLabel::as_str`].
    #[must_use]
    pub fn from_name(name: &str) -> Self {
        match name {
            "mobile" => Self::Mobile,
            "home" => Self::Home,
            "work" => Self::Work,
            "main" => Self::Main,
            "other" => Self::Other,
            custom => Self::Custom(custom.to_owned()),
        }
    }
}

/// Which way a Call in the Call History went.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum CallHistoryKind {
    Incoming,
    Outgoing,
    /// A Missed Call.
    Missed,
}

impl CallHistoryKind {
    /// The lower-case name the control interface and the cache use.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Incoming => "incoming",
            Self::Outgoing => "outgoing",
            Self::Missed => "missed",
        }
    }

    /// The kind named `name` by [`CallHistoryKind::as_str`].
    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "incoming" => Some(Self::Incoming),
            "outgoing" => Some(Self::Outgoing),
            "missed" => Some(Self::Missed),
            _ => None,
        }
    }
}

/// One Call in the Phone's Call History.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CallHistoryEntry {
    pub kind: CallHistoryKind,
    /// The other party; a withheld number is kept as an entry too.
    pub caller: Caller,
    /// When, if the Phone said.
    pub at: Option<Timestamp>,
    /// The name the Phone gave the other party, which may be empty.
    pub name: String,
}

/// Contacts, indexed for matching, cheap to clone.
#[derive(Clone, Default)]
pub struct Contacts(Arc<Directory>);

impl Contacts {
    #[must_use]
    pub fn new(directory: Directory) -> Self {
        Self(Arc::new(directory))
    }

    #[must_use]
    pub fn directory(&self) -> &Directory {
        &self.0
    }
}

impl PartialEq for Contacts {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

impl Eq for Contacts {}

impl fmt::Debug for Contacts {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Contacts({})", self.0.len())
    }
}

/// The Call History, newest first, indexed for search, cheap to clone.
#[derive(Clone, Default)]
pub struct CallHistory(Arc<(Vec<CallHistoryEntry>, HistoryIndex)>);

impl CallHistory {
    /// Sorts `entries` newest first, entries with no time last, and indexes
    /// them for search with the Contacts in `directory`, which they were
    /// synced with. Slow for thousands of entries: build it off the main
    /// thread.
    #[must_use]
    pub fn new(mut entries: Vec<CallHistoryEntry>, directory: &Directory) -> Self {
        entries.sort_by_key(|entry| std::cmp::Reverse(entry.at));
        let index = HistoryIndex::new(&entries, directory);
        Self(Arc::new((entries, index)))
    }

    #[must_use]
    pub fn entries(&self) -> &[CallHistoryEntry] {
        &self.0.0
    }

    pub(crate) fn index(&self) -> &HistoryIndex {
        &self.0.1
    }
}

impl PartialEq for CallHistory {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

impl Eq for CallHistory {}

impl fmt::Debug for CallHistory {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "CallHistory({})", self.0.0.len())
    }
}

/// How far Contacts and Call History are synced from the Phone.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ContactsState {
    /// A sync is under way.
    Syncing,
    /// Contacts and Call History are on the Desktop.
    Ready,
    /// The Phone is asking the person holding it whether to share Contacts,
    /// until `deadline`.
    PermissionNeeded { deadline: Timestamp },
    /// The Phone refuses to share Contacts.
    Denied,
    /// There are no Contacts to use, for this reason.
    Unavailable(Unavailable),
}

impl Default for ContactsState {
    fn default() -> Self {
        Self::Unavailable(Unavailable::NotSynced)
    }
}

impl ContactsState {
    /// The lower-case name the control interface uses.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Syncing => "syncing",
            Self::Ready => "ready",
            Self::PermissionNeeded { .. } => "permission-needed",
            Self::Denied => "denied",
            Self::Unavailable(_) => "unavailable",
        }
    }
}

/// Why Contacts are unavailable.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Unavailable {
    /// No sync has finished, and nothing is cached.
    NotSynced,
    /// The Desktop has no PBAP client (obexd isn't running).
    NoContactsService,
    /// Nobody answered the Phone's prompt before it timed out.
    PromptTimedOut,
    /// The sync failed for another reason.
    Failed,
}

impl Unavailable {
    /// The lower-case reason the control interface uses.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::NotSynced => "not-synced",
            Self::NoContactsService => "no-contacts-service",
            Self::PromptTimedOut => "prompt-timed-out",
            Self::Failed => "failed",
        }
    }
}

/// Why a sync ended without Contacts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SyncFailure {
    /// The Phone refused.
    Denied,
    /// See [`Unavailable`].
    Unavailable(Unavailable),
}

/// How far a sync has got, as it goes: what the Setup Window shows while
/// Contacts and Call History are read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SyncProgress {
    /// The Phone agreed to share Contacts and Call History, after asking
    /// the person holding it or not: the sync is reading them.
    Shared,
    /// The newest Calls of the Call History are read, before Contacts.
    CallHistory(CallHistory),
    /// The sync is reading Contacts, this many if the Phone said, from `at`.
    ReadingContacts { count: Option<u32>, at: Timestamp },
    /// The sync has read everything it could, and reported it.
    Finished,
}
