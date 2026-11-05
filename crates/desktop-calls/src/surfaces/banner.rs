//! The Incoming Call Banner.

use super::{SharedReport, Surface, Who, detail_label, has_command_modifier};
use desktop_calls_core::messages::{self, Label};
use desktop_calls_core::{BannerView, CallId, Input, WaitingOptions};
use futures_channel::mpsc::UnboundedSender;
use gtk::glib;
use gtk::prelude::*;
use gtk4_layer_shell::{KeyboardMode, Layer};
use std::cell::Cell;
use std::collections::BTreeMap;
use std::rc::Rc;

/// The Incoming Call Banner.
pub(super) struct Banner {
    pub(super) surface: Surface,
    heading: gtk::Label,
    caller: gtk::Label,
    detail: gtk::Label,
    answer: gtk::Button,
    /// The Waiting Call's options, shown instead of Answer.
    waiting: WaitingButtons,
}

/// Hold & Answer, on a row of its own as the choice that loses nothing, and
/// End & Answer beside Decline.
struct WaitingButtons {
    hold_and_answer: gtk::Button,
    end_and_answer: gtk::Button,
    /// Which are shown, so H and E work only then.
    shown: Rc<Cell<Option<WaitingOptions>>>,
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
        let waiting = WaitingButtons {
            hold_and_answer: surface.button(Label::HoldAndAnswer, "answer", intents, |_| {
                Input::HoldAndAnswer
            }),
            end_and_answer: surface.button(Label::EndAndAnswer, "end-and-answer", intents, |_| {
                Input::EndAndAnswer
            }),
            shown: Rc::default(),
        };
        let buttons = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        buttons.append(&decline);
        buttons.append(&answer);
        buttons.append(&waiting.end_and_answer);
        let content = gtk::Box::new(gtk::Orientation::Vertical, 8);
        content.append(&heading);
        content.append(&caller);
        content.append(&detail);
        content.append(&waiting.hold_and_answer);
        content.append(&buttons);
        surface.window.set_child(Some(&content));
        let banner = Self {
            surface,
            heading,
            caller,
            detail,
            answer,
            waiting,
        };
        banner.on_letters(intents);
        banner
    }

    /// While the Banner has keyboard focus, A answers and D declines; for a
    /// Waiting Call, H is Hold & Answer and E End & Answer too.
    fn on_letters(&self, intents: &UnboundedSender<Input>) {
        let keys = gtk::EventControllerKey::new();
        keys.set_propagation_phase(gtk::PropagationPhase::Capture);
        let (call, intents, waiting) = (
            self.surface.call.clone(),
            intents.clone(),
            self.waiting.shown.clone(),
        );
        keys.connect_key_pressed(move |_, key, _, modifiers| {
            let make: fn(CallId) -> Input = match key.to_lower() {
                gtk::gdk::Key::a => Input::Answer,
                gtk::gdk::Key::d => Input::Decline,
                gtk::gdk::Key::h if waiting.get() == Some(WaitingOptions::HoldOrEnd) => {
                    |_| Input::HoldAndAnswer
                }
                gtk::gdk::Key::e if waiting.get().is_some() => |_| Input::EndAndAnswer,
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
        let who = if view.awaiting_number {
            Who {
                caller: messages::label(Label::IdentifyingCaller).to_owned(),
                detail: None,
            }
        } else {
            Who::of(&view.caller, view.contact.as_ref())
        };
        self.caller.set_text(&who.caller);
        who.show_detail(&self.detail);
        // While focused, the heading says which keys do what.
        let heading = messages::label(match (view.waiting, view.focused) {
            (None, true) => Label::BannerKeys,
            (None, false) => Label::IncomingCall,
            (Some(WaitingOptions::HoldOrEnd), true) => Label::WaitingCallKeys,
            (Some(WaitingOptions::End), true) => Label::WaitingCallEndKeys,
            (Some(_), false) => Label::WaitingCall,
        });
        let can_hold = view.waiting == Some(WaitingOptions::HoldOrEnd);
        self.waiting.shown.set(view.waiting);
        self.answer.set_visible(view.waiting.is_none());
        self.waiting.hold_and_answer.set_visible(can_hold);
        self.waiting
            .end_and_answer
            .set_visible(view.waiting.is_some());
        self.heading.set_text(heading);
        self.surface.call.set(Some(view.call));
        let mut content = BTreeMap::from([
            ("call".to_owned(), view.call.to_string()),
            ("heading".to_owned(), heading.to_owned()),
        ]);
        if let Some(options) = view.waiting {
            let shown: &[Label] = match options {
                WaitingOptions::HoldOrEnd => {
                    &[Label::HoldAndAnswer, Label::EndAndAnswer, Label::Decline]
                }
                WaitingOptions::End => &[Label::EndAndAnswer, Label::Decline],
            };
            content.insert(
                "options".to_owned(),
                shown
                    .iter()
                    .map(|label| messages::label(*label))
                    .collect::<Vec<_>>()
                    .join(", "),
            );
        }
        who.report(&mut content);
        self.surface.show(content);
        self.surface.focus(view.focused);
        if view.focused {
            // Enter and Space then answer, never decline, and never end a
            // Call: with only End & Answer and Decline, they do nothing.
            match view.waiting {
                None => {
                    self.answer.grab_focus();
                }
                Some(WaitingOptions::HoldOrEnd) => {
                    self.waiting.hold_and_answer.grab_focus();
                }
                Some(WaitingOptions::End) => {
                    GtkWindowExt::set_focus(&self.surface.window, None::<&gtk::Widget>);
                }
            }
        }
    }
}
