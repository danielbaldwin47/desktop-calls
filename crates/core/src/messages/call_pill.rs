//! What the Call Pill says beyond the Call Window's words: its tag and its
//! tooltips.

use super::{Label, label, setup::shown_keys};
use crate::keybinding::KEYS;
use crate::machine::CallPillTag;

/// The Call Pill's tag beside the caller.
#[must_use]
pub fn call_pill_tag(tag: CallPillTag) -> &'static str {
    match tag {
        CallPillTag::Muted => "Muted",
        CallPillTag::OnPhone => "On phone",
        CallPillTag::OneOnHold => label(Label::OneOnHold),
    }
}

/// The tooltip of the Call Pill's button labelled `name`, whose shortcut in
/// the expanded Call Window is shown as `shortcut`: "Hang up · Ctrl+H".
#[must_use]
pub fn call_pill_tooltip(name: Label, shortcut: &str) -> String {
    format!("{} · {shortcut}", label(name))
}

/// The tooltip of the Call Pill's body, which a click expands: "Show Call ·
/// Super+Alt+C", with the Desktop Calls keybinding, which expands it too.
#[must_use]
pub fn show_call_tooltip() -> String {
    format!("Show Call · {}", shown_keys(KEYS).replace(" + ", "+"))
}
