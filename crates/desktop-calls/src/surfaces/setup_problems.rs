//! The Desktop's Problems, under the Setup Window's heading: each one card
//! on the warning tint, with its icon, heading, one plain sentence, and
//! either the button that puts it right once pressed (Turn on, Set up
//! audio) or the command to run and a button that copies it. Nothing else
//! in the window says the Problem again. While Bluetooth is off or blocked
//! on the choose step, the first button has keyboard focus and shows its
//! keycap, so Enter presses it; otherwise the step keeps focus, so Enter
//! pairs or confirms, and Shift+Tab reaches the Problems.
//!
//! It reports `setup-problems` while shown, with `problems` (kinds),
//! `headings`, `messages`, `commands` and `buttons` (one per line, in
//! order), `fixing` (what is under way), `notice` (why a fix didn't work),
//! `copied` (the command last copied) and `focused` (keyboard focus is on
//! one of its buttons).

use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;
use std::rc::Rc;

use desktop_calls_core::messages::{self, NextAction, ProblemKind};
use desktop_calls_core::{FixFailure, Input, SetupProblemsView, SetupWindowView};
use futures_channel::mpsc::UnboundedSender;
use gtk::prelude::*;

use super::icons::{Icon, IconView};
use super::{SharedReport, class};
use crate::theme::space;
use crate::wireplumber;

const NAME: &str = "setup-problems";

/// The Problems shown, and the Setup Window's opening and step by name.
type Shown = (Vec<ProblemKind>, Option<(u32, &'static str)>);

/// The Problems part of the Setup Window.
pub(super) struct SetupProblems {
    /// Holds everything; shown only while there are Problems.
    pub(super) widget: gtk::Box,
    problems: gtk::Box,
    status: gtk::Label,
    notice: gtk::Box,
    notice_text: gtk::Label,
    intents: UnboundedSender<Input>,
    report: SharedReport,
    content: RefCell<BTreeMap<String, String>>,
    mapped: Cell<bool>,
    /// Keyboard focus is on one of its buttons.
    focused: Cell<bool>,
    /// The Problems shown, and the Setup Window opening and step they show
    /// in: when any changes, the first button takes focus again.
    shown: RefCell<Shown>,
    /// Each fix button.
    fix_buttons: RefCell<Vec<gtk::Button>>,
    /// The first button, which takes focus, and its keycap, shown while
    /// Enter presses it.
    first_button: RefCell<Option<(gtk::Button, gtk::Label)>>,
    /// The command last copied, while the same Problems show.
    copied: RefCell<Option<String>>,
}

impl SetupProblems {
    pub(super) fn new(intents: &UnboundedSender<Input>, report: SharedReport) -> Rc<Self> {
        let widget = gtk::Box::new(gtk::Orientation::Vertical, space::S);
        widget.set_visible(false);
        let problems = gtk::Box::new(gtk::Orientation::Vertical, space::S);
        let status = gtk::Label::new(None);
        status.set_xalign(0.0);
        status.add_css_class("dim-label");
        let (notice, notice_text) = notice_card();
        for child in [
            problems.upcast_ref::<gtk::Widget>(),
            status.upcast_ref(),
            notice.upcast_ref(),
        ] {
            widget.append(child);
        }
        let problems = Rc::new(Self {
            widget,
            problems,
            status,
            notice,
            notice_text,
            intents: intents.clone(),
            report,
            content: RefCell::default(),
            mapped: Cell::default(),
            focused: Cell::default(),
            shown: RefCell::default(),
            fix_buttons: RefCell::default(),
            first_button: RefCell::default(),
            copied: RefCell::default(),
        });
        problems.connect();
        problems
    }

    fn connect(self: &Rc<Self>) {
        let weak = Rc::downgrade(self);
        self.widget.connect_map(move |_| {
            if let Some(problems) = weak.upgrade() {
                problems.mapped.set(true);
                problems.publish();
            }
        });
        // Whether keyboard focus is on one of its buttons, as GTK says.
        let focus = gtk::EventControllerFocus::new();
        for (signal_enter, focused) in [(true, true), (false, false)] {
            let weak = Rc::downgrade(self);
            let changed = move |_: &gtk::EventControllerFocus| {
                if let Some(problems) = weak.upgrade() {
                    problems.focused.set(focused);
                    problems.set_focused_field();
                    problems.publish();
                }
            };
            if signal_enter {
                focus.connect_enter(changed);
            } else {
                focus.connect_leave(changed);
            }
        }
        self.widget.add_controller(focus);
        let weak = Rc::downgrade(self);
        self.widget.connect_unmap(move |_| {
            if let Some(problems) = weak.upgrade() {
                problems.mapped.set(false);
                problems.report.borrow_mut().set(NAME, None);
            }
        });
    }

    fn set_focused_field(&self) {
        let mut content = self.content.borrow_mut();
        if self.focused.get() {
            content.insert("focused".to_owned(), "true".to_owned());
        } else {
            content.remove("focused");
        }
    }

    fn publish(&self) {
        if self.mapped.get() {
            self.report
                .borrow_mut()
                .set(NAME, Some(self.content.borrow().clone()));
        }
    }

    /// Shows `view` in the Setup Window showing `window`, or hides.
    pub(super) fn render(
        self: &Rc<Self>,
        view: Option<&SetupProblemsView>,
        window: Option<&SetupWindowView>,
    ) {
        let Some(view) = view else {
            self.shown.replace((Vec::new(), None));
            self.copied.replace(None);
            self.widget.set_visible(false);
            return;
        };
        let shown = (
            view.problems.clone(),
            window.map(|window| (window.opened, window.step.as_str())),
        );
        let changed = *self.shown.borrow() != shown;
        if changed {
            let rebuild = self.shown.borrow().0 != shown.0;
            self.shown.replace(shown);
            if rebuild {
                self.copied.replace(None);
                self.show_problems(&view.problems);
            }
        }
        for button in self.fix_buttons.borrow().iter() {
            button.set_sensitive(view.fixing.is_none());
        }
        // The first button shows the key that presses it, while it has the
        // keyboard.
        if let Some((_, keycap)) = self.first_button.borrow().as_ref() {
            keycap.set_visible(view.blocks_step);
        }
        // What is under way, or else that a command was copied.
        let status = view.fixing.map(messages::fixing).or_else(|| {
            self.copied
                .borrow()
                .as_ref()
                .map(|_| messages::command_copied())
        });
        self.status.set_text(status.unwrap_or_default());
        self.status.set_visible(status.is_some());
        let notice = view.notice.map(notice_text);
        self.notice_text
            .set_text(notice.as_deref().unwrap_or_default());
        self.notice.set_visible(notice.is_some());
        self.widget.set_visible(true);
        *self.content.borrow_mut() = self.report_content(view);
        self.set_focused_field();
        self.publish();
        if changed
            && view.blocks_step
            && let Some((button, _)) = self.first_button.borrow().as_ref()
        {
            button.grab_focus();
        }
    }

    fn show_problems(self: &Rc<Self>, problems: &[ProblemKind]) {
        while let Some(child) = self.problems.first_child() {
            self.problems.remove(&child);
        }
        let mut fix_buttons = Vec::new();
        let mut first_button = None;
        for kind in problems {
            let message = messages::problem(*kind);
            let card = gtk::Box::new(gtk::Orientation::Horizontal, space::M);
            card.add_css_class(class::NOTICE);
            card.add_css_class("problem-card");
            let icon = IconView::new(Icon::Warning);
            icon.area.add_css_class("notice-icon");
            icon.area.set_valign(gtk::Align::Start);
            let body = gtk::Box::new(gtk::Orientation::Vertical, space::XS);
            body.set_hexpand(true);
            let heading = gtk::Label::new(Some(messages::problem_heading(*kind)));
            heading.add_css_class("notice-heading");
            heading.set_xalign(0.0);
            let text = gtk::Label::new(Some(message.text));
            text.set_xalign(0.0);
            text.set_wrap(true);
            text.set_max_width_chars(50);
            body.append(&heading);
            body.append(&text);
            // Under the sentence, one row: the command, if the user must run
            // it, and the button at the right. Beside the sentence, a long
            // one would be squeezed into a narrow column.
            let actions = gtk::Box::new(gtk::Orientation::Horizontal, space::M);
            actions.set_margin_top(space::XS);
            let button = if let Some(label) = messages::fix_label(*kind) {
                let (button, keycap) = fix_button(label);
                button.add_css_class(class::ACCENT);
                // Alone in its row, it takes the row to sit at its right.
                button.set_hexpand(true);
                let (intents, kind) = (self.intents.clone(), *kind);
                button.connect_clicked(move |_| {
                    let _ = intents.unbounded_send(Input::Fix(kind));
                });
                fix_buttons.push(button.clone());
                Some((button, keycap))
            } else if let Some(NextAction::Command(command)) = message.next_action {
                let shown = gtk::Label::new(Some(command));
                shown.add_css_class("monospace");
                shown.set_selectable(true);
                shown.set_xalign(0.0);
                shown.set_wrap(true);
                shown.set_wrap_mode(gtk::pango::WrapMode::Word);
                shown.set_hexpand(true);
                shown.set_valign(gtk::Align::Center);
                actions.append(&shown);
                let (button, keycap) = fix_button(messages::copy_command_label());
                let weak = Rc::downgrade(self);
                button.connect_clicked(move |button| {
                    button.clipboard().set_text(command);
                    if let Some(problems) = weak.upgrade() {
                        problems.copied.replace(Some(command.to_owned()));
                        problems.status.set_text(messages::command_copied());
                        problems.status.set_visible(true);
                        problems
                            .content
                            .borrow_mut()
                            .insert("copied".to_owned(), command.to_owned());
                        problems.publish();
                    }
                });
                Some((button, keycap))
            } else {
                None
            };
            if let Some((button, keycap)) = button {
                button.set_halign(gtk::Align::End);
                button.set_valign(gtk::Align::Center);
                actions.append(&button);
                body.append(&actions);
                first_button.get_or_insert((button, keycap));
            }
            card.append(&icon.area);
            card.append(&body);
            self.problems.append(&card);
        }
        self.fix_buttons.replace(fix_buttons);
        self.first_button.replace(first_button);
    }

    fn report_content(&self, view: &SetupProblemsView) -> BTreeMap<String, String> {
        let lines = |each: &dyn Fn(ProblemKind) -> Option<String>| {
            view.problems
                .iter()
                .filter_map(|kind| each(*kind))
                .collect::<Vec<_>>()
                .join("\n")
        };
        let mut content = BTreeMap::from([
            (
                "problems".to_owned(),
                lines(&|kind| Some(kind.as_str().to_owned())),
            ),
            (
                "headings".to_owned(),
                lines(&|kind| Some(messages::problem_heading(kind).to_owned())),
            ),
            (
                "messages".to_owned(),
                lines(&|kind| Some(messages::problem(kind).text.to_owned())),
            ),
            (
                "buttons".to_owned(),
                lines(&|kind| {
                    messages::fix_label(kind)
                        .or_else(|| {
                            messages::problem(kind)
                                .next_action
                                .map(|_| messages::copy_command_label())
                        })
                        .map(str::to_owned)
                }),
            ),
        ]);
        let commands = lines(&|kind| match messages::problem(kind).next_action {
            Some(NextAction::Command(command)) if messages::fix_label(kind).is_none() => {
                Some(command.to_owned())
            }
            _ => None,
        });
        if !commands.is_empty() {
            content.insert("commands".to_owned(), commands);
        }
        if let Some(fix) = view.fixing {
            content.insert("fixing".to_owned(), messages::fixing(fix).to_owned());
        }
        if let Some(failure) = view.notice {
            content.insert("notice".to_owned(), notice_text(failure));
        }
        if let Some(copied) = self.copied.borrow().as_ref() {
            content.insert("copied".to_owned(), copied.clone());
        }
        content
    }
}

/// A button with `label` and, after it, the keycap for the key that presses
/// it, hidden until it does.
fn fix_button(label: &str) -> (gtk::Button, gtk::Label) {
    let keycap = gtk::Label::new(Some(messages::problem_keycap()));
    keycap.add_css_class(class::KEYCAP);
    keycap.set_valign(gtk::Align::Center);
    keycap.set_visible(false);
    let content = gtk::Box::new(gtk::Orientation::Horizontal, space::S);
    content.set_halign(gtk::Align::Center);
    content.append(&gtk::Label::new(Some(label)));
    content.append(&keycap);
    let button = gtk::Button::new();
    button.add_css_class(class::SMALL);
    button.set_child(Some(&content));
    (button, keycap)
}

/// A notice for the Setup Window, hidden: the warning icon beside the
/// words of why the last attempt didn't work. It is a card on the warning
/// tint, like a Problem, but says what went wrong, not a standing Problem.
pub(super) fn notice_card() -> (gtk::Box, gtk::Label) {
    let icon = IconView::new(Icon::Warning);
    icon.area.add_css_class("notice-icon");
    icon.area.set_valign(gtk::Align::Start);
    let text = gtk::Label::new(None);
    text.set_xalign(0.0);
    text.set_wrap(true);
    text.set_max_width_chars(50);
    text.set_hexpand(true);
    let card = gtk::Box::new(gtk::Orientation::Horizontal, space::M);
    card.add_css_class(class::NOTICE);
    card.append(&icon.area);
    card.append(&text);
    card.set_visible(false);
    (card, text)
}

/// Why a fix didn't work, naming where the audio settings go.
fn notice_text(failure: FixFailure) -> String {
    let folder = wireplumber::drop_in_folder().map_or_else(
        || "~/.config/wireplumber/wireplumber.conf.d".to_owned(),
        |folder| folder.display().to_string(),
    );
    messages::fix_failed(failure, &folder)
}
