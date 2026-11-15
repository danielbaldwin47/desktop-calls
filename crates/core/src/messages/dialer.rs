//! What the Dialer says: each row, the labels over its sections, and its
//! footer, each key with what it does and where the Phone is.

use super::{Label, caller, caller_detail, label, number_label};
use crate::call::{Caller, Timestamp};
use crate::clock::TimeOfDay;
use crate::contacts::CallHistoryKind;
use crate::indicator::PhoneStatus;
use crate::machine::DialerKey;
use crate::matching::{DialerRow, DialerRowKind};

/// What stands at the left of a Dialer row: a Call History row's direction,
/// or a Contact's initial (`#` for a number).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DialerGlyph {
    Missed,
    Incoming,
    Outgoing,
    Initial(String),
}

impl DialerGlyph {
    /// The glyph as the control interface reports it: `missed`,
    /// `incoming`, `outgoing`, or the initial.
    #[must_use]
    pub fn as_str(&self) -> &str {
        match self {
            Self::Missed => "missed",
            Self::Incoming => "incoming",
            Self::Outgoing => "outgoing",
            Self::Initial(initial) => initial,
        }
    }
}

/// A Dialer row as shown: the glyph, the name over a detail line, and at
/// the right when the Call was.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DialerRowText {
    pub glyph: DialerGlyph,
    pub title: String,
    /// Whether the detail line starts with "Missed", in the theme's red.
    pub missed: bool,
    /// The rest of the detail line; may be empty.
    pub detail: String,
    /// When the Call was, for a Call History row whose time the Phone
    /// gave: see [`dialer_time`].
    pub time: String,
}

impl DialerRowText {
    /// The row on one line, as the Dialer reports it: "Alice Smith ·
    /// Missed · mobile · +44 7700 900123 · Tue 14:05".
    #[must_use]
    pub fn line(&self) -> String {
        let mut parts = vec![self.title.as_str()];
        if self.missed {
            parts.push(label(Label::Missed));
        }
        parts.extend(
            [self.detail.as_str(), self.time.as_str()]
                .into_iter()
                .filter(|part| !part.is_empty()),
        );
        parts.join(" · ")
    }
}

/// What the Dialer shows for `row`, at `now` in a time zone
/// `utc_offset_seconds` from UTC.
///
/// - A Contact: their initial and name over the number's label and the
///   number.
/// - A Call History row: its direction and who it was, over "Missed" for a
///   Missed Call, then the number's label and the number, "Not in
///   Contacts" for a number no Contact has, or "can't be called back" for
///   a withheld one; at the right, when.
/// - The number typed: `#` and "Call 07700 900123".
#[must_use]
pub fn dialer_row(row: &DialerRow, now: Timestamp, utc_offset_seconds: i64) -> DialerRowText {
    let named = !row.name.trim().is_empty();
    let shown_label = row.label.as_ref().map(number_label).unwrap_or_default();
    let initial = || {
        DialerGlyph::Initial(
            row.name
                .chars()
                .find(|character| character.is_alphabetic())
                .map_or_else(|| "#".to_owned(), |first| first.to_uppercase().collect()),
        )
    };
    let text = |glyph, title, detail| DialerRowText {
        glyph,
        title,
        missed: false,
        detail,
        time: String::new(),
    };
    match row.kind {
        DialerRowKind::CallNumber => text(
            initial(),
            format!("Call {}", row.shown_number),
            String::new(),
        ),
        DialerRowKind::Contact if named => text(
            initial(),
            row.name.clone(),
            caller_detail(&shown_label, &row.shown_number),
        ),
        DialerRowKind::Contact => text(initial(), caller(&row.caller), shown_label),
        DialerRowKind::CallHistory(kind) => {
            let glyph = match kind {
                CallHistoryKind::Missed => DialerGlyph::Missed,
                CallHistoryKind::Incoming => DialerGlyph::Incoming,
                CallHistoryKind::Outgoing => DialerGlyph::Outgoing,
            };
            let (title, detail) = if row.caller == Caller::Withheld {
                let title = if named {
                    row.name.clone()
                } else {
                    caller(&row.caller)
                };
                (title, label(Label::CantCallBack).to_owned())
            } else if named {
                (
                    row.name.clone(),
                    caller_detail(&shown_label, &row.shown_number),
                )
            } else if shown_label.is_empty() {
                (caller(&row.caller), "Not in Contacts".to_owned())
            } else {
                (caller(&row.caller), shown_label)
            };
            DialerRowText {
                glyph,
                title,
                missed: kind == CallHistoryKind::Missed,
                detail,
                time: row
                    .at
                    .map(|at| super::when::dialer_time(at, now, utc_offset_seconds))
                    .unwrap_or_default(),
            }
        }
    }
}

/// The label over the Dialer's rows of `kind` where they start: "Recent"
/// over the Call History and callers found in it, "Contacts" over
/// Contacts, none over the number typed.
#[must_use]
pub fn dialer_section(kind: DialerRowKind) -> Option<&'static str> {
    match kind {
        DialerRowKind::CallHistory(_) => Some("Recent"),
        DialerRowKind::Contact => Some("Contacts"),
        DialerRowKind::CallNumber => None,
    }
}

/// The keycaps for `key`, as shown, and what pressing it does: Enter Call,
/// Up Down Move, Alt Q Quiet Hours, Esc Close.
#[must_use]
pub fn dialer_key(key: DialerKey) -> (&'static [&'static str], &'static str) {
    match key {
        DialerKey::Call => (&["Enter"], "Call"),
        DialerKey::CallAnyway => (&["Enter"], "Call anyway"),
        DialerKey::Move => (&["↑", "↓"], "Move"),
        DialerKey::QuietHours => (&["Alt", "Q"], "Quiet Hours"),
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
/// state, and while calls are off, the Indicator's words for it. While
/// `strip_says_off`, the strip above the rows saying calls are off and
/// until when, the footer leaves out when they come back on: beside the
/// footer's keys the words wouldn't fit (ticket #279).
#[must_use]
pub fn dialer_phone(status: &PhoneStatus, strip_says_off: bool) -> String {
    match status {
        PhoneStatus::Connected(phone) => phone.name.clone(),
        PhoneStatus::NoPhone => "Not set up".to_owned(),
        PhoneStatus::CallsOff { .. } if strip_says_off => super::calls_off::calls_off(None, false),
        PhoneStatus::CallsOff { until, quiet_hours } => {
            super::calls_off::calls_off(*until, *quiet_hours)
        }
        PhoneStatus::NotConnected(_) => "Not connected".to_owned(),
        PhoneStatus::OutOfRange(_) => "Out of range".to_owned(),
        PhoneStatus::ConnectionLost(_) => "Reconnecting…".to_owned(),
    }
}
