//! The card the Setup Window shows while calls are off: the Problem card's
//! shape (icon, heading, one sentence, a button under it at the right), in
//! a neutral colour, so a choice doesn't look like a fault. Its button,
//! Turn calls on, shows the key that presses it while it has the keyboard,
//! which it takes while the window is choosing a Phone.

use desktop_calls_core::Input;
use desktop_calls_core::messages::SetupCallsOffText;
use futures_channel::mpsc::UnboundedSender;
use gtk::prelude::*;

use super::class;
use super::icons::{Icon, IconView};
use super::setup_problems::fix_button;
use crate::theme::space;

/// The Calls Off card, hidden until calls are off.
pub(super) struct CallsOffCard {
    pub(super) widget: gtk::Box,
    heading: gtk::Label,
    text: gtk::Label,
    pub(super) button: gtk::Button,
    keycap: gtk::Label,
}

impl CallsOffCard {
    pub(super) fn new(intents: &UnboundedSender<Input>) -> Self {
        let icon = IconView::new(Icon::Moon);
        icon.area.add_css_class("notice-icon");
        icon.area.set_valign(gtk::Align::Start);
        let heading = gtk::Label::new(None);
        heading.add_css_class("notice-heading");
        heading.set_xalign(0.0);
        let text = gtk::Label::new(None);
        text.set_xalign(0.0);
        text.set_wrap(true);
        text.set_max_width_chars(50);
        let (button, keycap) = fix_button("");
        button.add_css_class(class::ACCENT);
        // Alone in its row, it takes the row to sit at its right.
        button.set_hexpand(true);
        button.set_halign(gtk::Align::End);
        button.set_valign(gtk::Align::Center);
        let intents = intents.clone();
        button.connect_clicked(move |_| {
            let _ = intents.unbounded_send(Input::TurnCallsOn);
        });
        let actions = gtk::Box::new(gtk::Orientation::Horizontal, space::M);
        actions.set_margin_top(space::XS);
        actions.append(&button);
        let body = gtk::Box::new(gtk::Orientation::Vertical, space::XS);
        body.set_hexpand(true);
        for child in [
            heading.upcast_ref::<gtk::Widget>(),
            text.upcast_ref(),
            actions.upcast_ref(),
        ] {
            body.append(child);
        }
        let widget = gtk::Box::new(gtk::Orientation::Horizontal, space::M);
        widget.add_css_class(class::NOTICE);
        widget.add_css_class("calls-off");
        widget.append(&icon.area);
        widget.append(&body);
        widget.set_visible(false);
        Self {
            widget,
            heading,
            text,
            button,
            keycap,
        }
    }

    /// Shows `card`, or hides while calls are on. `takes_keyboard` while
    /// Enter presses the button.
    pub(super) fn show(&self, card: Option<&SetupCallsOffText>, takes_keyboard: bool) {
        if let Some(card) = card {
            self.heading.set_text(card.heading);
            self.text.set_text(&card.text);
            if let Some(label) = self
                .button
                .child()
                .and_downcast::<gtk::Box>()
                .and_then(|content| content.first_child())
                .and_downcast::<gtk::Label>()
            {
                label.set_text(card.button);
            }
        }
        self.keycap.set_visible(card.is_some() && takes_keyboard);
        self.widget.set_visible(card.is_some());
    }
}
