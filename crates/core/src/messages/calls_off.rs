//! What the user reads about Calls Off: the Indicator's status and menu
//! entries, and what the command line says.

use crate::clock::TimeOfDay;

/// The menu entry that turns calls off, over its choices.
pub(super) const TURN_CALLS_OFF: &str = "Turn calls off";
/// The choice under Turn calls off with no deadline.
pub(super) const UNTIL_TURNED_ON: &str = "Until I turn them on";
/// The menu entry that turns calls on.
pub(super) const TURN_CALLS_ON: &str = "Turn calls on";
/// The menu entry that cancels a turn-off waiting for the last Call.
pub(super) const KEEP_CALLS_ON: &str = "Keep calls on";
/// The Indicator's status while a turn-off waits for the last Call.
pub(super) const PENDING: &str = "Calls turn off when this Call ends";
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

/// The line `desktop-calls status` adds while a turn-off waits for the
/// last Call to end: until `until` once off, or until turned on.
#[must_use]
pub fn calls_off_pending_status(until: Option<TimeOfDay>) -> String {
    match until {
        Some(until) => format!(
            "Calls turn off when this Call ends, until {until}; if the Call lasts past {until}, they stay on."
        ),
        None => "Calls turn off when this Call ends, until you turn them on.".to_owned(),
    }
}

/// What `desktop-calls calls off` said during a Call: the turn-off waits
/// for the last Call to end, and the Call carries on.
#[must_use]
pub fn calls_off_waits(until: Option<TimeOfDay>) -> String {
    format!(
        "{} Your Call carries on; run desktop-calls calls on to keep calls on.",
        calls_off_pending_status(until)
    )
}

/// What `desktop-calls calls on` said while a turn-off waited for the
/// last Call to end.
#[must_use]
pub fn calls_kept_on() -> &'static str {
    "Calls stay on: they won't turn off when this Call ends."
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

/// What removing Desktop Calls says once it has unblocked the Phone it had
/// blocked for Calls Off.
#[must_use]
pub fn phone_unblocked_for_removal() -> &'static str {
    "Calls are on again: your phone is unblocked."
}

/// What removing Desktop Calls says when BlueZ didn't unblock the Phone it
/// had blocked for Calls Off: the command that does, for later.
#[must_use]
pub fn phone_left_blocked_by_removal(address: &str) -> String {
    format!(
        "Bluetooth didn't unblock your phone: once it is running, run `bluetoothctl unblock {address}` to let it connect to this computer again."
    )
}
