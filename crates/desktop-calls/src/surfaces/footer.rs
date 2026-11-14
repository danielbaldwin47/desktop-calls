//! The footer line of an ordinary window: each key that works now as
//! keycaps with what it does, one row, from the view's list of keys.

use desktop_calls_core::messages::FooterKeys;
use gtk::prelude::*;

use super::class;

/// Fills `footer`, a box with the [`class::FOOTER_LINE`] class, with `keys`.
pub(super) fn show(footer: &gtk::Box, keys: &FooterKeys) {
    while let Some(child) = footer.first_child() {
        footer.remove(&child);
    }
    for (caps, does) in keys {
        let keycaps = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        keycaps.add_css_class("keycaps");
        for cap in *caps {
            let keycap = gtk::Label::new(Some(cap));
            keycap.add_css_class(class::KEYCAP);
            keycap.set_valign(gtk::Align::Center);
            keycaps.append(&keycap);
        }
        let group = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        group.append(&keycaps);
        group.append(&gtk::Label::new(Some(does)));
        footer.append(&group);
    }
}

/// The keys as the control interface reports them: `Enter Pair · Esc Close`.
pub(super) fn text(keys: &FooterKeys) -> String {
    keys.iter()
        .map(|(caps, does)| format!("{} {does}", caps.join(" ")))
        .collect::<Vec<_>>()
        .join(" · ")
}
