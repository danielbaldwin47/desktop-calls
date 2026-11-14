//! What the Dialer's footer says: each key with what it does, and where the
//! Phone is.

use crate::clock::TimeOfDay;
use crate::indicator::PhoneStatus;
use crate::machine::DialerKey;

/// The keycaps for `key`, as shown, and what pressing it does: Enter Call,
/// Up Down Move, Esc Close.
#[must_use]
pub fn dialer_key(key: DialerKey) -> (&'static [&'static str], &'static str) {
    match key {
        DialerKey::Call => (&["Enter"], "Call"),
        DialerKey::CallAnyway => (&["Enter"], "Call anyway"),
        DialerKey::Move => (&["↑", "↓"], "Move"),
        DialerKey::Close => (&["Esc"], "Close"),
    }
}

/// What the Dialer's strip says while calls are off, until `until` if they
/// end at a time: what holds, shown bold, then what Enter does.
#[must_use]
pub fn dialer_calls_off(until: Option<TimeOfDay>) -> (String, &'static str) {
    let off = match until {
        Some(until) => format!("Calls are off until {until}."),
        None => "Calls are off.".to_owned(),
    };
    (off, "Enter calls anyway; calls go off again after.")
}

/// Where the Phone is, at the right of the Dialer's footer: its name while
/// it is connected, which the footer marks with a live dot, otherwise its
/// state, and while calls are off, the Indicator's words for it.
#[must_use]
pub fn dialer_phone(status: &PhoneStatus) -> String {
    match status {
        PhoneStatus::Connected(phone) => phone.name.clone(),
        PhoneStatus::NoPhone => "Not set up".to_owned(),
        // The Indicator's words, with when calls come back on.
        PhoneStatus::CallsOff { until, quiet_hours } => {
            super::calls_off::calls_off(*until, *quiet_hours)
        }
        PhoneStatus::NotConnected(_) => "Not connected".to_owned(),
        PhoneStatus::OutOfRange(_) => "Out of range".to_owned(),
        PhoneStatus::ConnectionLost(_) => "Reconnecting…".to_owned(),
    }
}
