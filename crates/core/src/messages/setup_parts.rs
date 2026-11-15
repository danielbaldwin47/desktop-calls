//! The Setup Window's parts beyond its heading and buttons: the four named
//! steps across the top, its instructions grouped by where each is done,
//! the sync's checklist and bar, and the finish page's two cards.

use crate::call::Timestamp;
use crate::keybinding::KeybindingCheck;
use crate::machine::{
    CONTACT_READ_MS, ContactsOutcome, KeybindingStage, ReadingContacts, SetupStep,
};

/// The four named steps across the top of the Setup Window, in order.
pub const SETUP_STAGES: [&str; 4] = ["Pair", "Share contacts", "Sync", "Shortcut"];

/// How one of the named steps across the top is marked.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StageMark {
    /// Done: ticked.
    Ticked,
    /// Gone past without being done, as sharing and syncing are when the
    /// Phone didn't share Contacts.
    Passed,
    /// The step the window is at.
    Now,
    /// Still to come.
    Ahead,
}

/// Which of [`SETUP_STAGES`] `step` belongs to. A Phone that leaves while
/// Contacts are shared or read is asked again, so that is sharing.
fn stage(step: &SetupStep) -> usize {
    match step {
        SetupStep::Choose { .. }
        | SetupStep::Pairing { .. }
        | SetupStep::Confirm { .. }
        | SetupStep::Confirmed { .. }
        | SetupStep::Connecting { .. }
        | SetupStep::PhoneCallsOff { .. }
        | SetupStep::PhoneUnreachable { .. } => 0,
        SetupStep::ContactsWarning { .. }
        | SetupStep::ContactsPrompt { .. }
        | SetupStep::ContactsPromptTimedOut { .. }
        | SetupStep::ContactsPhoneAway { .. } => 1,
        SetupStep::ContactsSyncing { .. } => 2,
        SetupStep::Done { .. } => 3,
    }
}

/// How each of [`SETUP_STAGES`] is marked at `step`.
#[must_use]
pub fn stage_marks(step: &SetupStep) -> [StageMark; 4] {
    let now = stage(step);
    let unshared = matches!(
        step,
        SetupStep::Done { contacts, .. } if !matches!(contacts, ContactsOutcome::Synced { .. })
    );
    // Once the keybinding is added, the last step is done too.
    let added = matches!(
        step,
        SetupStep::Done {
            keybinding: KeybindingStage::Checked(KeybindingCheck::Added { .. }),
            ..
        }
    );
    std::array::from_fn(|index| match index.cmp(&now) {
        std::cmp::Ordering::Less if unshared && index > 0 => StageMark::Passed,
        std::cmp::Ordering::Less => StageMark::Ticked,
        std::cmp::Ordering::Equal if added => StageMark::Ticked,
        std::cmp::Ordering::Equal => StageMark::Now,
        std::cmp::Ordering::Greater => StageMark::Ahead,
    })
}

/// A named step as shown and reported: ticked once done.
#[must_use]
pub fn stage_line(name: &str, mark: StageMark) -> String {
    if mark == StageMark::Ticked {
        format!("✓ {name}")
    } else {
        name.to_owned()
    }
}

/// Where an instruction is carried out.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Place {
    OnYourPhone,
    Here,
}

impl Place {
    /// The group's label.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::OnYourPhone => "On your phone",
            Self::Here => "Here",
        }
    }
}

/// One thing to do, one sentence, and where it is done.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Instruction {
    pub place: Place,
    pub text: String,
}

impl Instruction {
    pub(super) fn phone(text: impl Into<String>) -> Self {
        Self {
            place: Place::OnYourPhone,
            text: text.into(),
        }
    }

    pub(super) fn here(text: impl Into<String>) -> Self {
        Self {
            place: Place::Here,
            text: text.into(),
        }
    }

    /// The instruction on its own line, saying where when it is done on
    /// the Phone.
    #[must_use]
    pub fn line(&self) -> String {
        match self.place {
            Place::OnYourPhone => format!("{}: {}", Place::OnYourPhone.label(), self.text),
            Place::Here => self.text.clone(),
        }
    }
}

/// Instructions done in one place, under its label when it needs one.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InstructionGroup {
    pub place: Place,
    /// "Here" goes unlabelled when nothing is done on the Phone.
    pub label: Option<&'static str>,
    pub lines: Vec<String>,
}

/// `instructions` grouped by where each is done, the groups in the order
/// their places first come.
#[must_use]
pub fn grouped(instructions: &[Instruction]) -> Vec<InstructionGroup> {
    let on_phone = instructions
        .iter()
        .any(|instruction| instruction.place == Place::OnYourPhone);
    let mut groups: Vec<InstructionGroup> = Vec::new();
    for instruction in instructions {
        if let Some(group) = groups
            .iter_mut()
            .find(|group| group.place == instruction.place)
        {
            group.lines.push(instruction.text.clone());
            continue;
        }
        groups.push(InstructionGroup {
            place: instruction.place,
            label: on_phone.then_some(instruction.place.label()),
            lines: vec![instruction.text.clone()],
        });
    }
    groups
}

/// One line of the sync's checklist.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SyncRow {
    pub done: bool,
    pub name: &'static str,
    /// The count, and the time left, while they are known.
    pub meta: Option<String>,
}

impl SyncRow {
    /// The row on one line: ticked once done.
    #[must_use]
    pub fn line(&self) -> String {
        let mark = if self.done { "✓ " } else { "" };
        match &self.meta {
            Some(meta) => format!("{mark}{} · {meta}", self.name),
            None => format!("{mark}{}", self.name),
        }
    }
}

/// The sync under way: a checklist, a bar and what already works.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SyncText {
    pub rows: Vec<SyncRow>,
    /// How far the bar has got, in thousandths.
    pub bar: u16,
    pub note: &'static str,
}

/// Recent Calls come first and take about a tenth of the sync; Contacts
/// fill the rest of the bar as their time runs.
const CALLS_SHARE: u64 = 100;
/// The bar stops short of full until Contacts are read.
const BAR_BEFORE_READ: u64 = 980;

/// The sync at `now`: recent Calls, then Contacts.
pub(super) fn sync(
    call_history: bool,
    contacts: Option<ReadingContacts>,
    contacts_read: bool,
    now: Timestamp,
) -> SyncText {
    let rows = vec![
        SyncRow {
            done: call_history,
            name: "Recent Calls",
            meta: None,
        },
        SyncRow {
            done: contacts_read,
            name: "Contacts",
            meta: (!contacts_read)
                .then(|| contacts_meta(contacts, now))
                .flatten(),
        },
    ];
    let calls = if call_history { CALLS_SHARE } else { 0 };
    let bar = if contacts_read {
        1_000
    } else {
        (calls + contacts_share(contacts, now)).min(BAR_BEFORE_READ)
    };
    SyncText {
        rows,
        bar: u16::try_from(bar).unwrap_or(1_000),
        note: "Calls already work. Names appear as Contacts arrive.",
    }
}

/// "312 · about 9 s left", while the count is known.
fn contacts_meta(reading: Option<ReadingContacts>, now: Timestamp) -> Option<String> {
    let ReadingContacts {
        count: Some(count),
        done_by,
    } = reading?
    else {
        return None;
    };
    Some(match done_by.map(|done_by| seconds_until(done_by, now)) {
        Some(0) | None => count.to_string(),
        Some(seconds) => format!("{count} · about {seconds} s left"),
    })
}

/// The thousandths of the bar Contacts have filled at `now`, by the time
/// they take at the Phone's pace.
fn contacts_share(reading: Option<ReadingContacts>, now: Timestamp) -> u64 {
    let Some(ReadingContacts {
        count: Some(count),
        done_by: Some(done_by),
    }) = reading
    else {
        return 0;
    };
    let total = u64::from(count) * CONTACT_READ_MS;
    if total == 0 {
        return 0;
    }
    let left = done_by.0.saturating_sub(now.0).min(total);
    (total - left) * (1_000 - CALLS_SHARE) / total
}

/// Whole seconds from `now` until `then`, rounded up; 0 once past.
pub(super) fn seconds_until(then: Timestamp, now: Timestamp) -> u64 {
    then.0.saturating_sub(now.0).div_ceil(1_000)
}

/// One of the finish page's two cards: what it offers, and how.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SetupCard {
    pub title: &'static str,
    pub text: String,
}

impl SetupCard {
    /// The card as reported: its title, then its text.
    #[must_use]
    pub fn reported(&self) -> String {
        format!("{}\n{}", self.title, self.text)
    }
}

/// The finish page's card for keeping the Indicator in sight.
pub(super) fn pin_card() -> SetupCard {
    SetupCard {
        title: "Keep Desktop Calls in the bar",
        text: "In Omarchy, right-click the arrow at the bar's tray and pin Desktop Calls. Until then it hides in the drawer."
            .to_owned(),
    }
}
