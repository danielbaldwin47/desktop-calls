//! The Call Window's call-waiting row: the Held Call, with Swap and a Hang
//! up of its own; or, for a lone Call on hold, Resume. While a Waiting Call
//! rings, the Phone would apply Swap or that Hang up to it instead, so the
//! row says so in their place.

use super::super::{HANG_UP_HELD_SHORTCUT, SWAP_SHORTCUT, Who};
use super::show_shortcut;
use desktop_calls_core::messages::{self, Label};
use desktop_calls_core::{CallId, CallWindowPhase, CallWindowView, Input};
use futures_channel::mpsc::UnboundedSender;
use gtk::prelude::*;
use std::cell::Cell;
use std::collections::BTreeMap;
use std::rc::Rc;

pub(super) struct HeldRow {
    pub(super) row: gtk::Box,
    /// Who the Call on hold is with, after "On hold".
    caller_line: gtk::Box,
    caller: gtk::Label,
    pub(super) swap: gtk::Button,
    swap_label: gtk::Label,
    swap_shortcut: String,
    /// Hangs up the Call on hold, leaving the live one.
    pub(super) hang_up: gtk::Button,
    hang_up_shortcut: String,
    buttons: gtk::Box,
    /// Why Swap and Hang up aren't offered while a Waiting Call rings.
    waiting_first: gtk::Label,
    held: Rc<Cell<Option<CallId>>>,
}

impl HeldRow {
    pub(super) fn new(intents: &UnboundedSender<Input>) -> Self {
        let on_hold = gtk::Label::new(Some(messages::label(Label::OnHold)));
        on_hold.add_css_class("dim-label");
        let caller = gtk::Label::new(None);
        caller.add_css_class("held-caller");
        caller.set_xalign(0.0);
        caller.set_ellipsize(gtk::pango::EllipsizeMode::End);
        let caller_line = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        caller_line.append(&on_hold);
        caller_line.append(&caller);

        let swap = gtk::Button::new();
        swap.add_css_class("swap");
        let swap_label = gtk::Label::new(Some(messages::label(Label::Swap)));
        // As wide as its wider label, so it keeps its size as it changes.
        swap_label.set_width_chars(super::widest(&[Label::Swap, Label::Resume]));
        let swap_shortcut = show_shortcut(&swap, &swap_label, SWAP_SHORTCUT);
        let intents_for_swap = intents.clone();
        swap.connect_clicked(move |_| {
            let _ = intents_for_swap.unbounded_send(Input::Swap);
        });

        let hang_up = gtk::Button::new();
        hang_up.add_css_class("hang-up-held");
        let hang_up_shortcut = show_shortcut(
            &hang_up,
            &gtk::Label::new(Some(messages::label(Label::HangUp))),
            HANG_UP_HELD_SHORTCUT,
        );
        let held: Rc<Cell<Option<CallId>>> = Rc::default();
        let (call, intents) = (held.clone(), intents.clone());
        hang_up.connect_clicked(move |_| {
            if let Some(call) = call.get() {
                let _ = intents.unbounded_send(Input::HangUp(call));
            }
        });

        let buttons = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        buttons.set_homogeneous(true);
        buttons.append(&swap);
        buttons.append(&hang_up);
        let waiting_first = gtk::Label::new(Some(messages::label(Label::WaitingCallFirst)));
        waiting_first.add_css_class("dim-label");
        waiting_first.set_xalign(0.0);
        waiting_first.set_wrap(true);
        waiting_first.set_max_width_chars(36);
        let row = gtk::Box::new(gtk::Orientation::Vertical, 8);
        row.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
        row.append(&caller_line);
        row.append(&buttons);
        row.append(&waiting_first);
        row.set_visible(false);
        for button in [&swap, &hang_up] {
            button.set_focus_on_click(false);
        }
        Self {
            row,
            caller_line,
            caller,
            swap,
            swap_label,
            swap_shortcut,
            hang_up,
            hang_up_shortcut,
            buttons,
            waiting_first,
            held,
        }
    }

    /// Brings the row up to date with `view`, and adds what it shows to
    /// `content`: shown while a Call is on hold behind the live one, with
    /// Swap, or while the only Call is on hold, with Resume.
    pub(super) fn update(&self, view: &CallWindowView, content: &mut BTreeMap<String, String>) {
        let lone = view.held.is_none() && matches!(view.phase, CallWindowPhase::OnHold { .. });
        let can_act = !view.waiting_first && !view.connection_lost;
        self.held.set(view.held.as_ref().map(|held| held.call));
        self.row.set_visible(view.held.is_some() || lone);
        // Hidden controls ignore their shortcuts.
        self.swap
            .set_sensitive((view.held.is_some() || lone) && can_act);
        self.hang_up.set_sensitive(view.held.is_some() && can_act);
        self.hang_up.set_visible(view.held.is_some());
        self.buttons.set_visible(can_act);
        // With contact lost, the Call Window's notice says why.
        self.waiting_first.set_visible(view.waiting_first);
        self.caller_line.set_visible(view.held.is_some());
        if let Some(held) = &view.held {
            let who = Who::of(&held.caller, held.contact.as_ref());
            self.caller.set_text(&who.caller);
            content.insert("held-call".to_owned(), held.call.to_string());
            content.insert("held-caller".to_owned(), who.caller);
        }
        if view.connection_lost {
            return;
        }
        if !can_act {
            content.insert(
                "held-waiting-first".to_owned(),
                messages::label(Label::WaitingCallFirst).to_owned(),
            );
            return;
        }
        if view.held.is_some() {
            content.insert(
                "hang-up-held-shortcut".to_owned(),
                self.hang_up_shortcut.clone(),
            );
        }
        if view.held.is_some() || lone {
            let swap = messages::label(if lone { Label::Resume } else { Label::Swap });
            self.swap_label.set_text(swap);
            content.insert("swap".to_owned(), swap.to_owned());
            content.insert("swap-shortcut".to_owned(), self.swap_shortcut.clone());
        }
    }
}
