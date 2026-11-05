//! The Incoming Call Banner.

use super::{SharedReport, Surface, Who, detail_label, has_command_modifier};
use desktop_calls_core::messages::{self, Label};
use desktop_calls_core::{BannerView, CallId, Input, RingingCaller};
use futures_channel::mpsc::UnboundedSender;
use gtk::glib;
use gtk::prelude::*;
use gtk4_layer_shell::{KeyboardMode, Layer};
use std::collections::BTreeMap;

/// The Incoming Call Banner.
pub(super) struct Banner {
    pub(super) surface: Surface,
    heading: gtk::Label,
    caller: gtk::Label,
    detail: gtk::Label,
    answer: gtk::Button,
}

impl Banner {
    pub(super) fn new(intents: &UnboundedSender<Input>, report: SharedReport) -> Self {
        let surface = Surface::new(
            "banner",
            Layer::Overlay,
            KeyboardMode::None,
            Input::ReleaseFocus,
            intents,
            report,
        );
        let heading = gtk::Label::new(Some(messages::label(Label::IncomingCall)));
        heading.add_css_class("dim-label");
        heading.set_xalign(0.0);
        let caller = gtk::Label::new(None);
        caller.add_css_class("caller");
        caller.set_xalign(0.0);
        let detail = detail_label();
        let decline = surface.button(Label::Decline, "decline", intents, Input::Decline);
        let answer = surface.button(Label::Answer, "answer", intents, Input::Answer);
        let buttons = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        buttons.append(&decline);
        buttons.append(&answer);
        let content = gtk::Box::new(gtk::Orientation::Vertical, 8);
        content.append(&heading);
        content.append(&caller);
        content.append(&detail);
        content.append(&buttons);
        surface.window.set_child(Some(&content));
        let banner = Self {
            surface,
            heading,
            caller,
            detail,
            answer,
        };
        banner.on_letters(intents);
        banner
    }

    /// While the Banner has keyboard focus, A answers and D declines.
    fn on_letters(&self, intents: &UnboundedSender<Input>) {
        let keys = gtk::EventControllerKey::new();
        keys.set_propagation_phase(gtk::PropagationPhase::Capture);
        let (call, intents) = (self.surface.call.clone(), intents.clone());
        keys.connect_key_pressed(move |_, key, _, modifiers| {
            let make: fn(CallId) -> Input = match key.to_lower() {
                gtk::gdk::Key::a => Input::Answer,
                gtk::gdk::Key::d => Input::Decline,
                _ => return glib::Propagation::Proceed,
            };
            if has_command_modifier(modifiers) {
                return glib::Propagation::Proceed;
            }
            if let Some(call) = call.get() {
                let _ = intents.unbounded_send(make(call));
            }
            glib::Propagation::Stop
        });
        self.surface.window.add_controller(keys);
    }

    pub(super) fn render(&self, view: Option<&BannerView>) {
        let Some(view) = view else {
            self.surface.hide();
            return;
        };
        let who = match &view.caller {
            RingingCaller::Identifying => Who {
                caller: messages::label(Label::IdentifyingCaller).to_owned(),
                detail: None,
            },
            RingingCaller::Known { caller, contact } => Who::of(caller, contact.as_ref()),
        };
        self.caller.set_text(&who.caller);
        who.show_detail(&self.detail);
        // While focused, the heading says which keys do what.
        let heading = messages::label(if view.focused {
            Label::BannerKeys
        } else {
            Label::IncomingCall
        });
        self.heading.set_text(heading);
        self.surface.call.set(Some(view.call));
        let mut content = BTreeMap::from([
            ("call".to_owned(), view.call.to_string()),
            ("heading".to_owned(), heading.to_owned()),
        ]);
        who.report(&mut content);
        self.surface.show(content);
        self.surface.focus(view.focused);
        if view.focused {
            // Enter and Space then answer, never decline.
            self.answer.grab_focus();
        }
    }
}
