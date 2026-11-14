//! What the Quiet Hours Window says: its heading, its two times, the
//! sentence saying what was saved, a notice, and its keys.

use crate::clock::TimeOfDay;
use crate::machine::{QuietHoursDay, QuietHoursField, QuietHoursSentence, QuietHoursWindowView};

/// The Indicator's menu entry that opens the window.
pub(super) const MENU_ENTRY: &str = "Quiet Hours…";

/// The Quiet Hours Window's text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QuietHoursWindowText {
    /// The window's title.
    pub title: &'static str,
    pub heading: &'static str,
    /// What the switch does, for screen readers.
    pub switch: &'static str,
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
        title: "Quiet Hours",
        heading: "Quiet Hours",
        switch: "Quiet Hours on",
        from: "Calls off from",
        until: "until",
        start: field(QuietHoursField::Start, view.start),
        end: field(QuietHoursField::End, view.end),
        sentence: sentence(view.sentence),
        notice: view.refused.then_some("Start and end must differ"),
        keys: if view.on {
            vec![
                (&["Tab"], "Next"),
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

fn sentence(sentence: QuietHoursSentence) -> String {
    match sentence {
        QuietHoursSentence::Off => {
            "Off. Calls stay on unless you turn them off from the Indicator.".to_owned()
        }
        QuietHoursSentence::Saved { start, end, day } => {
            let day = match day {
                Some(QuietHoursDay::Today) => " today",
                Some(QuietHoursDay::Tonight) => " tonight",
                Some(QuietHoursDay::Tomorrow) => " tomorrow",
                None => "",
            };
            format!("Saved. Calls go off{day} at {start} and come back at {end}, every day.")
        }
        QuietHoursSentence::OffNow { start, end } => {
            format!("Saved. Calls are off until {end}, and every day from {start} to {end}.")
        }
        QuietHoursSentence::Pending { start, end } => format!(
            "Saved. Calls go off when this Call ends, until {end}, and every day from {start} to {end}."
        ),
    }
}
