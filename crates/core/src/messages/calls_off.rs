//! What the user reads about Calls Off: the Indicator's status and menu
//! entries, and what the command line says.

/// The Indicator's status while calls are off.
pub(super) const CALLS_OFF: &str = "Calls off";
/// The menu entry that turns calls off, over its choices.
pub(super) const TURN_CALLS_OFF: &str = "Turn calls off";
/// The choice under Turn calls off.
pub(super) const UNTIL_TURNED_ON: &str = "Until I turn them on";
/// The menu entry that turns calls on.
pub(super) const TURN_CALLS_ON: &str = "Turn calls on";

/// The line `desktop-calls status` adds while calls are off; none while
/// they are on.
#[must_use]
pub fn calls_off_status(off: bool) -> Option<&'static str> {
    off.then_some("Calls are off until you turn them on: your phone stays disconnected.")
}

/// What `desktop-calls calls off` or `calls on` did.
#[must_use]
pub fn calls_turned(off: bool) -> &'static str {
    if off {
        "Calls are off: your phone stays disconnected until you turn calls on."
    } else {
        "Calls are on: connecting your phone."
    }
}

/// Why the control interface's `TurnCallsOff` refused its argument: only
/// "until turned on" exists yet.
#[must_use]
pub fn calls_off_until_unknown() -> &'static str {
    "Calls can only be turned off until you turn them on (0) for now."
}
