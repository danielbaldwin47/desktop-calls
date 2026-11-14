//! The Call Window's Held Call row, at the top: an "On hold" eyebrow over
//! the Held Call's caller, with Swap and a small red-outlined Hang up for
//! the Held Call beside them. While a Waiting Call rings, the Phone would
//! apply Swap or that Hang up to it instead, so the row says so in their
//! place.

use super::super::{HANG_UP_HELD_SHORTCUT, SWAP_SHORTCUT, Who, class, keycap};
use super::icons::{Icon, IconView};
use super::show_shortcut;
use crate::theme::space;
use desktop_calls_core::messages::{self, Label};
use desktop_calls_core::{CallId, CallWindowPhase, CallWindowView, Input};
use futures_channel::mpsc::UnboundedSender;
use gtk::prelude::*;
use std::cell::Cell;
use std::collections::BTreeMap;
use std::rc::Rc;

pub(super) struct HeldRow {
    pub(super) row: gtk::Box,
    /// The eyebrow and the Held Call's caller, with Swap and Hang up.
    line: gtk::Box,
    caller: gtk::Label,
    pub(super) swap: gtk::Button,
    swap_shortcut: String,
    /// Hangs up the Held Call, leaving the live one.
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
        on_hold.add_css_class(class::EYEBROW);
        on_hold.set_xalign(0.0);
        let caller = gtk::Label::new(None);
        caller.add_css_class("held-caller");
        caller.set_xalign(0.0);
        caller.set_ellipsize(gtk::pango::EllipsizeMode::End);
        let who = gtk::Box::new(gtk::Orientation::Vertical, 2);
        who.set_hexpand(true);
        who.append(&on_hold);
        who.append(&caller);

        let swap = gtk::Button::new();
        swap.add_css_class("swap");
        swap.add_css_class(class::SMALL);
        let swap_shortcut = show_shortcut(
            &swap,
            None,
            &gtk::Label::new(Some(messages::label(Label::Swap))),
            SWAP_SHORTCUT,
        );
        swap.set_hexpand(false);
        let intents_for_swap = intents.clone();
        swap.connect_clicked(move |_| {
            let _ = intents_for_swap.unbounded_send(Input::Swap);
        });

        // Small and outlined, so it can't be taken for the live Call's
        // Hang up; its tooltip names it and its shortcut.
        let hang_up = gtk::Button::new();
        hang_up.add_css_class("hang-up-held");
        hang_up.add_css_class(class::SMALL);
        hang_up.set_child(Some(&IconView::new(Icon::HangUp).area));
        let (_, hang_up_shortcut) = keycap(HANG_UP_HELD_SHORTCUT);
        hang_up.set_tooltip_text(Some(&format!(
            "{} ({hang_up_shortcut})",
            messages::label(Label::HangUpHeldCall)
        )));
        hang_up.update_property(&[gtk::accessible::Property::Label(messages::label(
            Label::HangUpHeldCall,
        ))]);
        let held: Rc<Cell<Option<CallId>>> = Rc::default();
        let (call, intents) = (held.clone(), intents.clone());
        hang_up.connect_clicked(move |_| {
            if let Some(call) = call.get() {
                let _ = intents.unbounded_send(Input::HangUp(call));
            }
        });

        let buttons = gtk::Box::new(gtk::Orientation::Horizontal, space::S);
        buttons.set_valign(gtk::Align::Center);
        buttons.append(&swap);
        buttons.append(&hang_up);
        let line = gtk::Box::new(gtk::Orientation::Horizontal, space::S);
        line.append(&who);
        line.append(&buttons);
        let waiting_first = gtk::Label::new(Some(messages::label(Label::WaitingCallFirst)));
        waiting_first.add_css_class("dim-label");
        waiting_first.set_xalign(0.0);
        waiting_first.set_wrap(true);
        waiting_first.set_max_width_chars(36);
        // At the top of the Call Window, above the live Call, so the live
        // Call's controls stay put as it comes and goes.
        let row = gtk::Box::new(gtk::Orientation::Vertical, space::S);
        row.add_css_class(class::HELD_ROW);
        row.append(&line);
        row.append(&waiting_first);
        row.set_visible(false);
        for button in [&swap, &hang_up] {
            button.set_focus_on_click(false);
        }
        Self {
            row,
            line,
            caller,
            swap,
            swap_shortcut,
            hang_up,
            hang_up_shortcut,
            buttons,
            waiting_first,
            held,
        }
    }

    /// Brings the row up to date with `view`, and adds what it shows to
    /// `content`: shown while a Call is on hold behind the live one. While
    /// the only Call is on hold and a Waiting Call rings, it holds just the
    /// sentence that says why Resume isn't offered.
    pub(super) fn update(&self, view: &CallWindowView, content: &mut BTreeMap<String, String>) {
        let lone = view.held.is_none() && matches!(view.phase, CallWindowPhase::OnHold { .. });
        let can_act = view.held.is_some() && !view.waiting_first && !view.connection_lost;
        // With contact lost, the Call Window's notice says why.
        let waiting_first =
            (view.held.is_some() || lone) && view.waiting_first && !view.connection_lost;
        self.held.set(view.held.as_ref().map(|held| held.call));
        self.row.set_visible(view.held.is_some() || waiting_first);
        self.line.set_visible(view.held.is_some());
        // Hidden controls ignore their shortcuts.
        self.swap.set_sensitive(can_act);
        self.hang_up.set_sensitive(can_act);
        self.buttons.set_visible(can_act);
        self.waiting_first.set_visible(waiting_first);
        if waiting_first {
            content.insert(
                "held-waiting-first".to_owned(),
                messages::label(Label::WaitingCallFirst).to_owned(),
            );
        }
        let Some(held) = &view.held else {
            return;
        };
        let who = Who::of(&held.caller, held.contact.as_ref());
        who.show(&self.caller);
        content.insert("held-call".to_owned(), held.call.to_string());
        content.insert("held-caller".to_owned(), who.caller);
        if !can_act {
            return;
        }
        content.insert(
            "hang-up-held-shortcut".to_owned(),
            self.hang_up_shortcut.clone(),
        );
        content.insert("swap".to_owned(), messages::label(Label::Swap).to_owned());
        content.insert("swap-shortcut".to_owned(), self.swap_shortcut.clone());
    }
}
