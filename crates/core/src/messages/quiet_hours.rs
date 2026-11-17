//! What the Quiet Hours Window says: its heading, its two times, the
//! sentence saying what was saved, a notice, and its keys.

use crate::clock::{QuietHours, TimeOfDay};
use crate::machine::{QuietHoursDay, QuietHoursField, QuietHoursSentence, QuietHoursWindowView};

/// The Indicator's menu entry that opens the window, with the Quiet Hours
/// set, if any, as a hint after it: "Quiet Hours…  22:00–07:00". Omarchy's
/// tray draws one label per entry and nothing beside it, so the hint is in
/// the label.
pub(super) fn menu_entry(hours: Option<QuietHours>) -> String {
    match hours {
        Some(hours) => format!("Quiet hours…  {}–{}", hours.start(), hours.end()),
        None => "Quiet hours…".to_owned(),
    }
}

/// The Quiet Hours Window's text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QuietHoursWindowText {
    /// The window's title.
    pub title: &'static str,
    pub heading: &'static str,
    /// What the switch does, for screen readers.
    pub switch: &'static str,
    /// The keycap beside the switch while it has keyboard focus, when Space
    /// would flip it (#301).
    pub switch_key: &'static str,
    /// The small headings over the two times.
    pub from: &'static str,
    pub until: &'static str,
    /// The times, `HH:MM`; while digits are typed, those, the rest `_`.
    pub start: String,
    pub end: String,
    pub sentence: String,
    /// Why nothing was saved.
    pub notice: Option<&'static str>,
    /// The keys that work, each with what it does, for the footer: Up and
    /// Down are two keys doing one thing.
    pub keys: Vec<(&'static [&'static str], &'static str)>,
}

/// What the Quiet Hours Window says for `view`.
#[must_use]
pub fn quiet_hours_window(view: &QuietHoursWindowView) -> QuietHoursWindowText {
    let field = |field: QuietHoursField, time: TimeOfDay| match &view.typing {
        Some((typed, digits)) if *typed == field => typing(digits),
        _ => time.to_string(),
    };
    QuietHoursWindowText {
        title: "Quiet hours",
        heading: "Quiet hours",
        switch: "Quiet hours on",
        switch_key: "Space",
        from: "Calls off from",
        until: "until",
        start: field(QuietHoursField::Start, view.start),
        end: field(QuietHoursField::End, view.end),
        sentence: if view.refused {
            kept(view.sentence)
        } else {
            sentence(view.sentence)
        },
        notice: view.refused.then_some("Start and end can't be the same"),
        keys: if view.on {
            vec![
                (&["Tab"], "Next field"),
                (&["↑", "↓"], "15 min"),
                (&["Esc"], "Close"),
            ]
        } else {
            vec![(&["Space"], "Switch on"), (&["Esc"], "Close")]
        },
    }
}

/// Digits typed so far, filling `HH:MM` from the left.
fn typing(digits: &str) -> String {
    let mut shown: Vec<char> = digits
        .chars()
        .chain(std::iter::repeat('_'))
        .take(4)
        .collect();
    shown.insert(2, ':');
    shown.into_iter().collect()
}

/// While times are refused, what is still saved (#314).
fn kept(sentence: QuietHoursSentence) -> String {
    match sentence {
        QuietHoursSentence::Saved { start, end, .. }
        | QuietHoursSentence::OffNow { start, end }
        | QuietHoursSentence::Pending { start, end } => {
            format!("Not saved. Still {start}–{end}.")
        }
        QuietHoursSentence::Off => "Not saved.".to_owned(),
    }
}

fn sentence(sentence: QuietHoursSentence) -> String {
    match sentence {
        QuietHoursSentence::Off => "Quiet hours are off.".to_owned(),
        QuietHoursSentence::Saved { day, .. } => {
            let day = match day {
                Some(QuietHoursDay::Today) => ", starting today",
                Some(QuietHoursDay::Tonight) => ", starting tonight",
                Some(QuietHoursDay::Tomorrow) => ", starting tomorrow",
                None => "",
            };
            format!("Saved. Every day{day}.")
        }
        QuietHoursSentence::OffNow { end, .. } => {
            format!("Saved. Calls are off now, until {end}.")
        }
        QuietHoursSentence::Pending { .. } => "Saved. Calls turn off after this call.".to_owned(),
    }
}
