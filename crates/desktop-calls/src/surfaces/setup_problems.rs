//! The Desktop's Problems, at the top of the Setup Window: each with its
//! heading, one plain sentence, and either the button that puts it right
//! once pressed (Turn on, Set up audio) or the command to run and a button
//! that copies it. When the Problems first show, or change, the first
//! one's button has keyboard focus, so Enter presses it; Tab moves on.
//!
//! It reports `setup-problems` while shown, with `problems` (kinds),
//! `headings`, `messages`, `commands` and `buttons` (one per line, in
//! order), `fixing` (what is under way), `notice` (why a fix didn't work)
//! and `copied` (the command last copied).

use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;
use std::rc::Rc;

use desktop_calls_core::messages::{self, NextAction, ProblemKind};
use desktop_calls_core::{Input, SetupProblemsView, SetupWindowView};
use futures_channel::mpsc::UnboundedSender;
use gtk::prelude::*;

use super::SharedReport;

const NAME: &str = "setup-problems";

/// The Problems shown, and the Setup Window's opening and step by name.
type Shown = (Vec<ProblemKind>, Option<(u32, &'static str)>);

/// The Problems part of the Setup Window.
pub(super) struct SetupProblems {
    /// Holds everything; shown only while there are Problems.
    pub(super) widget: gtk::Box,
    problems: gtk::Box,
    status: gtk::Label,
    notice: gtk::Label,
    intents: UnboundedSender<Input>,
    report: SharedReport,
    content: RefCell<BTreeMap<String, String>>,
    mapped: Cell<bool>,
    /// The Problems shown, and the Setup Window opening and step they show
    /// in: when any changes, the first button takes focus again.
    shown: RefCell<Shown>,
    /// Each fix button.
    fix_buttons: RefCell<Vec<gtk::Button>>,
    /// The first button, which takes focus.
    first_button: RefCell<Option<gtk::Button>>,
    /// The command last copied, while the same Problems show.
    copied: RefCell<Option<String>>,
}

impl SetupProblems {
    pub(super) fn new(intents: &UnboundedSender<Input>, report: SharedReport) -> Rc<Self> {
        let widget = gtk::Box::new(gtk::Orientation::Vertical, 10);
        widget.set_visible(false);
        let problems = gtk::Box::new(gtk::Orientation::Vertical, 10);
        let status = gtk::Label::new(None);
        status.set_xalign(0.0);
        status.add_css_class("dim-label");
        let notice = gtk::Label::new(None);
        notice.set_xalign(0.0);
        notice.set_wrap(true);
        notice.set_max_width_chars(50);
        notice.add_css_class("error");
        for child in [
            problems.upcast_ref::<gtk::Widget>(),
            status.upcast_ref(),
            notice.upcast_ref(),
        ] {
            widget.append(child);
        }
        widget.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
        let problems = Rc::new(Self {
            widget,
            problems,
            status,
            notice,
            intents: intents.clone(),
            report,
            content: RefCell::default(),
            mapped: Cell::default(),
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
        let weak = Rc::downgrade(self);
        self.widget.connect_unmap(move |_| {
            if let Some(problems) = weak.upgrade() {
                problems.mapped.set(false);
                problems.report.borrow_mut().set(NAME, None);
            }
        });
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
        // What is under way, or else that a command was copied.
        let status = view.fixing.map(messages::fixing).or_else(|| {
            self.copied
                .borrow()
                .as_ref()
                .map(|_| messages::command_copied())
        });
        self.status.set_text(status.unwrap_or_default());
        self.status.set_visible(status.is_some());
        let notice = view.notice.map(messages::fix_failed);
        self.notice.set_text(notice.unwrap_or_default());
        self.notice.set_visible(notice.is_some());
        self.widget.set_visible(true);
        *self.content.borrow_mut() = self.report_content(view);
        self.publish();
        if changed && let Some(button) = self.first_button.borrow().as_ref() {
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
            let row = gtk::Box::new(gtk::Orientation::Vertical, 6);
            let heading = gtk::Label::new(Some(messages::problem_heading(*kind)));
            heading.add_css_class("heading");
            heading.set_xalign(0.0);
            let text = gtk::Label::new(Some(message.text));
            text.set_xalign(0.0);
            text.set_wrap(true);
            text.set_max_width_chars(50);
            row.append(&heading);
            row.append(&text);
            let button = if let Some(label) = messages::fix_label(*kind) {
                let button = gtk::Button::with_label(label);
                button.add_css_class("suggested-action");
                let (intents, kind) = (self.intents.clone(), *kind);
                button.connect_clicked(move |_| {
                    let _ = intents.unbounded_send(Input::Fix(kind));
                });
                fix_buttons.push(button.clone());
                Some(button)
            } else if let Some(NextAction::Command(command)) = message.next_action {
                let shown = gtk::Label::new(Some(command));
                shown.add_css_class("monospace");
                shown.set_selectable(true);
                shown.set_xalign(0.0);
                row.append(&shown);
                let button = gtk::Button::with_label(messages::copy_command_label());
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
                Some(button)
            } else {
                None
            };
            if let Some(button) = button {
                button.set_halign(gtk::Align::Start);
                row.append(&button);
                first_button.get_or_insert(button);
            }
            self.problems.append(&row);
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
            content.insert(
                "notice".to_owned(),
                messages::fix_failed(failure).to_owned(),
            );
        }
        if let Some(copied) = self.copied.borrow().as_ref() {
            content.insert("copied".to_owned(), copied.clone());
        }
        content
    }
}
