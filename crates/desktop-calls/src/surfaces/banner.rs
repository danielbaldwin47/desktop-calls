//! The Incoming Call Banner: an eyebrow with the ringing dot, who is
//! calling beside an initial tile, the choices, and a footer line naming
//! the keys that work now.
//!
//! While it lacks the keyboard, the footer names the keybinding that gives
//! it the keyboard and the letters that then act. Once it has it, each
//! choice shows its letter as a keycap and the footer says Escape gives the
//! keyboard back. A Waiting Call's choices say, under their labels, what
//! happens to the user's other Call.

use super::{
    Anchor, SharedReport, Surface, Who, class, detail_label, has_command_modifier, set_class,
};
use crate::theme::space;
use desktop_calls_core::messages::{self, FooterPart, Label};
use desktop_calls_core::{BannerChoice, BannerView, CallId, Input, RingingCaller, WaitingOptions};
use futures_channel::mpsc::UnboundedSender;
use gtk::glib;
use gtk::prelude::*;
use gtk4_layer_shell::{KeyboardMode, Layer};
use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;
use std::rc::Rc;

/// The Incoming Call Banner.
pub(super) struct Banner {
    pub(super) surface: Surface,
    eyebrow: gtk::Label,
    initial: gtk::Label,
    caller: gtk::Label,
    meta: gtk::Label,
    decline: Choice,
    answer: Choice,
    /// Hold & Answer, on a row of its own as the choice that loses nothing.
    hold_and_answer: Choice,
    /// End & Answer, beside Decline.
    end_and_answer: Choice,
    footer: gtk::Box,
    /// What the footer shows, so it is rebuilt only when that changes.
    footer_parts: RefCell<Vec<FooterPart>>,
    /// The Waiting Call's options, so H and E work only while offered.
    waiting: Rc<Cell<Option<WaitingOptions>>>,
}

/// One of the Banner's choices: a button with its label, its key as a
/// keycap while the Banner has the keyboard, and what it does to the user's
/// other Call under the label.
struct Choice {
    button: gtk::Button,
    keycap: gtk::Label,
    consequence: gtk::Label,
    /// What the control interface reports the consequence as.
    name: &'static str,
}

impl Choice {
    fn new(
        surface: &Surface,
        choice: BannerChoice,
        class: &str,
        intents: &UnboundedSender<Input>,
        make: fn(CallId) -> Input,
    ) -> Self {
        let label = label_of(choice);
        let button = surface.button(label, class, intents, make);
        let text = gtk::Label::new(Some(messages::label(label)));
        let keycap = gtk::Label::new(Some(letter(choice)));
        keycap.add_css_class(class::KEYCAP);
        keycap.set_visible(false);
        let line = gtk::Box::new(gtk::Orientation::Horizontal, space::S);
        line.set_halign(gtk::Align::Center);
        line.append(&text);
        line.append(&keycap);
        let consequence = gtk::Label::new(None);
        consequence.add_css_class(class::CONSEQUENCE);
        // A bare number doesn't fit a half-width button on one line; it
        // wraps rather than losing the digits that say whose Call ends.
        consequence.set_wrap(true);
        consequence.set_wrap_mode(gtk::pango::WrapMode::WordChar);
        consequence.set_justify(gtk::Justification::Center);
        consequence.set_max_width_chars(1);
        consequence.set_visible(false);
        let content = gtk::Box::new(gtk::Orientation::Vertical, 1);
        content.set_valign(gtk::Align::Center);
        content.append(&line);
        content.append(&consequence);
        button.set_child(Some(&content));
        Self {
            button,
            keycap,
            consequence,
            name: name_of(choice),
        }
    }

    /// Shows the choice, its keycap while `keys`, and `consequence` under
    /// its label; reports both.
    fn show(
        &self,
        keys: bool,
        consequence: Option<String>,
        content: &mut BTreeMap<String, String>,
    ) {
        self.button.set_visible(true);
        self.keycap.set_visible(keys);
        set_class(&self.button, class::TWO_LINE, consequence.is_some());
        match consequence {
            Some(consequence) => {
                self.consequence.set_text(&consequence);
                self.consequence.set_visible(true);
                content.insert(format!("{}-consequence", self.name), consequence);
            }
            None => self.consequence.set_visible(false),
        }
    }
}

/// What the control interface reports `choice`'s consequence as, before
/// `-consequence`.
fn name_of(choice: BannerChoice) -> &'static str {
    match choice {
        BannerChoice::Answer => "answer",
        BannerChoice::Decline => "decline",
        BannerChoice::HoldAndAnswer => "hold-and-answer",
        BannerChoice::EndAndAnswer => "end-and-answer",
    }
}

/// The key that makes `choice` while the Banner has the keyboard.
fn letter(choice: BannerChoice) -> &'static str {
    match choice {
        BannerChoice::Answer => "A",
        BannerChoice::Decline => "D",
        BannerChoice::HoldAndAnswer => "H",
        BannerChoice::EndAndAnswer => "E",
    }
}

impl Banner {
    pub(super) fn new(intents: &UnboundedSender<Input>, report: SharedReport) -> Self {
        let surface = Surface::new(
            "banner",
            Anchor::TopRight,
            Layer::Overlay,
            KeyboardMode::None,
            Some(Input::ReleaseFocus),
            intents,
            report,
        );
        let eyebrow = gtk::Label::new(Some(messages::label(Label::IncomingCall)));
        eyebrow.add_css_class(class::EYEBROW);
        eyebrow.set_xalign(0.0);
        let dot = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        dot.add_css_class(class::RINGING_DOT);
        dot.set_valign(gtk::Align::Center);
        let eyebrow_row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        eyebrow_row.append(&dot);
        eyebrow_row.append(&eyebrow);

        let initial = gtk::Label::new(None);
        initial.add_css_class(class::INITIAL_TILE);
        initial.set_valign(gtk::Align::Center);
        let caller = gtk::Label::new(None);
        caller.add_css_class(class::WHO);
        caller.set_xalign(0.0);
        caller.set_ellipsize(gtk::pango::EllipsizeMode::End);
        let meta = detail_label();
        meta.set_ellipsize(gtk::pango::EllipsizeMode::End);
        let who = gtk::Box::new(gtk::Orientation::Vertical, 2);
        who.set_valign(gtk::Align::Center);
        who.set_hexpand(true);
        who.append(&caller);
        who.append(&meta);
        let who_row = gtk::Box::new(gtk::Orientation::Horizontal, space::M);
        who_row.append(&initial);
        who_row.append(&who);

        let decline = Choice::new(
            &surface,
            BannerChoice::Decline,
            "decline",
            intents,
            Input::Decline,
        );
        let answer = Choice::new(
            &surface,
            BannerChoice::Answer,
            "answer",
            intents,
            Input::Answer,
        );
        let hold_and_answer = Choice::new(
            &surface,
            BannerChoice::HoldAndAnswer,
            "answer",
            intents,
            |_| Input::HoldAndAnswer,
        );
        let end_and_answer = Choice::new(
            &surface,
            BannerChoice::EndAndAnswer,
            "end-and-answer",
            intents,
            |_| Input::EndAndAnswer,
        );
        // Decline left, Answer (or End & Answer) right, of equal widths.
        let buttons = gtk::Box::new(gtk::Orientation::Horizontal, space::S);
        buttons.set_homogeneous(true);
        buttons.append(&decline.button);
        buttons.append(&answer.button);
        buttons.append(&end_and_answer.button);

        let footer = gtk::Box::new(gtk::Orientation::Horizontal, 5);
        footer.add_css_class(class::FOOTER_LINE);
        footer.add_css_class(class::RUN);

        let content = gtk::Box::new(gtk::Orientation::Vertical, space::M);
        content.append(&eyebrow_row);
        content.append(&who_row);
        content.append(&hold_and_answer.button);
        content.append(&buttons);
        content.append(&footer);
        surface.window.set_child(Some(&content));
        let banner = Self {
            surface,
            eyebrow,
            initial,
            caller,
            meta,
            decline,
            answer,
            hold_and_answer,
            end_and_answer,
            footer,
            footer_parts: RefCell::default(),
            waiting: Rc::default(),
        };
        banner.on_letters(intents);
        banner
    }

    /// While the Banner has keyboard focus, A answers and D declines; for a
    /// Waiting Call, H is Hold & Answer and E End & Answer too, where the
    /// Banner offers them.
    fn on_letters(&self, intents: &UnboundedSender<Input>) {
        let keys = gtk::EventControllerKey::new();
        keys.set_propagation_phase(gtk::PropagationPhase::Capture);
        let (call, intents, waiting) = (
            self.surface.call.clone(),
            intents.clone(),
            self.waiting.clone(),
        );
        keys.connect_key_pressed(move |_, key, _, modifiers| {
            let make: fn(CallId) -> Input = match key.to_lower() {
                gtk::gdk::Key::a => Input::Answer,
                gtk::gdk::Key::d => Input::Decline,
                gtk::gdk::Key::h if waiting.get() == Some(WaitingOptions::HoldOrEnd) => {
                    |_| Input::HoldAndAnswer
                }
                gtk::gdk::Key::e
                    if matches!(
                        waiting.get(),
                        Some(WaitingOptions::HoldOrEnd | WaitingOptions::End)
                    ) =>
                {
                    |_| Input::EndAndAnswer
                }
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

    fn choice(&self, choice: BannerChoice) -> &Choice {
        match choice {
            BannerChoice::Answer => &self.answer,
            BannerChoice::Decline => &self.decline,
            BannerChoice::HoldAndAnswer => &self.hold_and_answer,
            BannerChoice::EndAndAnswer => &self.end_and_answer,
        }
    }

    pub(super) fn render(&self, view: Option<&BannerView>) {
        let Some(view) = view else {
            self.surface.hide();
            return;
        };
        let mut content = BTreeMap::from([("call".to_owned(), view.call.to_string())]);
        self.show_who(view, &mut content);
        let eyebrow = messages::label(if view.waiting.is_some() {
            Label::WaitingCall
        } else {
            Label::IncomingCall
        });
        self.eyebrow.set_text(eyebrow);
        content.insert("heading".to_owned(), eyebrow.to_owned());
        self.waiting.set(view.waiting);
        self.surface.call.set(Some(view.call));

        let choices = view.choices();
        for choice in [
            BannerChoice::Answer,
            BannerChoice::Decline,
            BannerChoice::HoldAndAnswer,
            BannerChoice::EndAndAnswer,
        ] {
            if !choices.contains(&choice) {
                self.choice(choice).button.set_visible(false);
            }
        }
        for &choice in &choices {
            self.choice(choice).show(
                view.focused,
                messages::banner_consequence(view, choice),
                &mut content,
            );
        }
        // The bottom row's two buttons stand as tall as each other.
        let tall = choices.iter().any(|&choice| {
            choice != BannerChoice::HoldAndAnswer
                && messages::banner_consequence(view, choice).is_some()
        });
        for choice in [&self.decline, &self.answer, &self.end_and_answer] {
            set_class(&choice.button, class::TWO_LINE, tall);
        }
        let letters: Vec<&str> = choices.iter().map(|&choice| letter(choice)).collect();
        if view.focused {
            content.insert("keycaps".to_owned(), letters.join(", "));
        }
        if view.waiting.is_some() {
            content.insert(
                "options".to_owned(),
                choices
                    .iter()
                    .map(|&choice| messages::label(label_of(choice)))
                    .collect::<Vec<_>>()
                    .join(", "),
            );
        }
        self.show_footer(view, &letters, &mut content);
        self.surface.show(content);
        self.surface.focus(view.focused);
        if view.focused {
            // Enter and Space then answer, never decline, and never end a
            // Call: with only End & Answer and Decline, they do nothing.
            match view.waiting {
                None | Some(WaitingOptions::Answer) => {
                    self.answer.button.grab_focus();
                }
                Some(WaitingOptions::HoldOrEnd) => {
                    self.hold_and_answer.button.grab_focus();
                }
                Some(WaitingOptions::End) => {
                    GtkWindowExt::set_focus(&self.surface.window, None::<&gtk::Widget>);
                }
            }
        }
    }

    /// Who is calling: the initial tile (the Contact's initial, or `#`),
    /// the name or the number, and under it the number's label and the
    /// number, or "Not in Contacts" for a bare number.
    fn show_who(&self, view: &BannerView, content: &mut BTreeMap<String, String>) {
        let who = match &view.caller {
            RingingCaller::Identifying => Who {
                caller: messages::label(Label::IdentifyingCaller).to_owned(),
                detail: None,
                bare_number: false,
            },
            RingingCaller::Known { caller, contact } => Who::of(caller, contact.as_ref()),
        };
        who.show(&self.caller);
        who.show_detail(&self.meta);
        if who.bare_number {
            self.meta.set_text(messages::label(Label::NotInContacts));
            self.meta.set_visible(true);
        }
        if self.meta.is_visible() {
            content.insert("meta".to_owned(), self.meta.text().to_string());
        }
        let initial = if who.detail.is_some() {
            who.caller
                .chars()
                .next()
                .map_or_else(|| "#".to_owned(), |first| first.to_uppercase().collect())
        } else {
            "#".to_owned()
        };
        self.initial.set_text(&initial);
        content.insert("initial".to_owned(), initial);
        who.report(content);
    }

    /// The footer line: keycaps with the words between them.
    fn show_footer(
        &self,
        view: &BannerView,
        letters: &[&str],
        content: &mut BTreeMap<String, String>,
    ) {
        let parts = view
            .footer
            .as_ref()
            .map(|footer| messages::banner_footer(footer, letters))
            .unwrap_or_default();
        self.footer.set_visible(!parts.is_empty());
        if !parts.is_empty() {
            content.insert("footer".to_owned(), messages::footer_text(&parts));
        }
        if *self.footer_parts.borrow() == parts {
            return;
        }
        while let Some(child) = self.footer.first_child() {
            self.footer.remove(&child);
        }
        // Keys pressed together sit close, and a comma against the key
        // before it: each run of keys, with any comma after it, is one group.
        let mut group: Option<gtk::Box> = None;
        for part in &parts {
            match part {
                FooterPart::Key(key) => {
                    let label = gtk::Label::new(Some(key));
                    label.add_css_class(class::KEYCAP);
                    label.set_valign(gtk::Align::Center);
                    group
                        .get_or_insert_with(|| {
                            let keys = gtk::Box::new(gtk::Orientation::Horizontal, 2);
                            self.footer.append(&keys);
                            keys
                        })
                        .append(&label);
                }
                FooterPart::Words(",") => {
                    let comma = gtk::Label::new(Some(","));
                    match group.take() {
                        Some(keys) => keys.append(&comma),
                        None => self.footer.append(&comma),
                    }
                }
                FooterPart::Words(words) => {
                    group = None;
                    let label = gtk::Label::new(Some(words));
                    label.set_wrap(true);
                    label.set_xalign(0.0);
                    label.set_valign(gtk::Align::Center);
                    self.footer.append(&label);
                }
            }
        }
        *self.footer_parts.borrow_mut() = parts;
    }
}

/// The label of `choice`'s button.
fn label_of(choice: BannerChoice) -> Label {
    match choice {
        BannerChoice::Answer => Label::Answer,
        BannerChoice::Decline => Label::Decline,
        BannerChoice::HoldAndAnswer => Label::HoldAndAnswer,
        BannerChoice::EndAndAnswer => Label::EndAndAnswer,
    }
}
