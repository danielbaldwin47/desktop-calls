//! What the user reads about Calls Off: the Indicator's status and menu
//! entries, and what the command line says.

use crate::clock::TimeOfDay;

/// The menu entry that turns calls off, over its choices.
pub(super) const TURN_CALLS_OFF: &str = "Turn calls off";
/// The choice under Turn calls off with no deadline.
pub(super) const UNTIL_TURNED_ON: &str = "Until I turn them on";
/// The menu entry that turns calls on.
pub(super) const TURN_CALLS_ON: &str = "Turn calls on";
/// Calls turned off until a time that has already passed.
pub(super) const UNTIL_PASSED: &str =
    "That time has already passed, so calls stay on; choose a time ahead.";

/// The Indicator's status while calls are off: until `until`, or until the
/// user turns them on.
pub(super) fn calls_off(until: Option<TimeOfDay>) -> String {
    match until {
        Some(until) => format!("Calls off until {until}"),
        None => "Calls off".to_owned(),
    }
}

/// The choice under Turn calls off that turns them off for an hour, ending
/// at `until`.
pub(super) fn for_an_hour(until: TimeOfDay) -> String {
    format!("For 1 hour (until {until})")
}

/// The choice under Turn calls off that turns them off until the morning,
/// at `until`.
pub(super) fn until_morning(until: TimeOfDay) -> String {
    format!("Until tomorrow morning ({until})")
}

/// The line `desktop-calls status` adds while calls are off, until `until`
/// or until the user turns them on; none while they are on.
#[must_use]
pub fn calls_off_status(off: bool, until: Option<TimeOfDay>) -> Option<String> {
    off.then(|| match until {
        Some(until) => {
            format!("Calls are off until {until}: your phone stays disconnected until then.")
        }
        None => "Calls are off until you turn them on: your phone stays disconnected.".to_owned(),
    })
}

/// What `desktop-calls calls off` or `calls on` did. Calls turned off come
/// back on at `until`, or when the user turns them on.
#[must_use]
pub fn calls_turned(off: bool, until: Option<TimeOfDay>) -> String {
    match (off, until) {
        (true, Some(until)) => format!(
            "Calls are off until {until}: your phone stays disconnected, and connects again then."
        ),
        (true, None) => {
            "Calls are off: your phone stays disconnected until you turn calls on.".to_owned()
        }
        (false, _) => "Calls are on: connecting your phone.".to_owned(),
    }
}

/// Why `desktop-calls calls off --for` refused its length: none, or longer
/// than a day, which the time calls come back on couldn't say.
#[must_use]
pub fn calls_off_for_too_long() -> &'static str {
    "Calls can be turned off for 1 minute to 24 hours, such as --for 90m or --for 2h; for longer, run desktop-calls calls off and turn them on yourself."
}

/// Why `desktop-calls calls off --until` refused its time.
#[must_use]
pub fn calls_off_until_unreadable() -> &'static str {
    "Give the time calls come back on as HH:MM on the 24-hour clock, such as --until 21:30."
}
