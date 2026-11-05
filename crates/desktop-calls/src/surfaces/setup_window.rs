//! The Setup Window: pairs a Phone with the Desktop, step by step.
//!
//! Unlike the other surfaces it is an ordinary window, not a layer-shell
//! surface: it is a task the user works through, which tiles, takes focus
//! when it opens and can be moved aside. Each step's default button has
//! keyboard focus, so Enter takes the step; while choosing, the first Phone
//! listed has focus, the arrow keys move between Phones and Enter chooses.
//! Escape or closing the window cancels.
//!
//! It reports `setup-window` with `step`, `heading`, `instructions` and
//! `buttons` (one per line), `phones` (names, one per line), `code`,
//! `notice`, `desktop` and `focused`, and logs when each step and the first
//! Phone listed reached the screen, timed from the window opening.

use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;
use std::rc::Rc;
use std::time::Instant;

use desktop_calls_core::messages::{self, SetupWindowText};
use desktop_calls_core::{Input, Phone, SetupStep, SetupWindowView};
use futures_channel::mpsc::UnboundedSender;
use gtk::glib;
use gtk::prelude::*;

use super::{SharedReport, after_next_frame, reported, surface_mapped, surface_shown};
use crate::logging::EventKind;

const NAME: &str = "setup-window";
const WIDTH: i32 = 480;

const CSS: &str = "
.desktop-calls-setup { padding: 20px; }
.desktop-calls-setup .setup-code {
    font-size: 2.6em; font-weight: bold; letter-spacing: 0.15em;
    font-feature-settings: \"tnum\";
}
.desktop-calls-setup .setup-phone { padding: 10px 12px; font-weight: bold; }
.desktop-calls-setup .setup-keys { font-size: 0.85em; opacity: 0.75; }
";

/// The Setup Window.
pub(super) struct SetupWindow {
    window: gtk::Window,
    heading: gtk::Label,
    summary: gtk::Label,
    instructions: gtk::Box,
    code: gtk::Label,
    notice: gtk::Label,
    list: gtk::ListBox,
    buttons: gtk::Box,
    keys: gtk::Label,
    intents: UnboundedSender<Input>,
    report: SharedReport,
    content: Rc<RefCell<BTreeMap<String, String>>>,
    mapped: Rc<Cell<bool>>,
    /// The opening shown, and when it opened.
    opened: Cell<Option<(u32, Instant)>>,
    /// The step shown, by name.
    step: Cell<Option<&'static str>>,
    /// The Phones listed, in order.
    phones: RefCell<Vec<Phone>>,
    /// Whether this opening has listed a Phone yet.
    listed: Cell<bool>,
    /// What [`Self::put_on_top`] put there, whose focus a Phone appearing
    /// doesn't take.
    top: RefCell<Option<gtk::Widget>>,
    /// Whether what is on top keeps keyboard focus when a Phone appears.
    top_keeps_focus: Cell<bool>,
}

impl SetupWindow {
    pub(super) fn new(intents: &UnboundedSender<Input>, report: SharedReport) -> Rc<Self> {
        if let Some(display) = gtk::gdk::Display::default() {
            let provider = gtk::CssProvider::new();
            provider.load_from_string(CSS);
            gtk::style_context_add_provider_for_display(
                &display,
                &provider,
                gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
            );
        }
        let window = gtk::Window::new();
        window.set_default_size(WIDTH, -1);
        window.set_resizable(false);
        window.add_css_class("desktop-calls-setup");

        let heading = gtk::Label::new(None);
        heading.add_css_class("title-2");
        heading.set_xalign(0.0);
        heading.set_wrap(true);
        let summary = gtk::Label::new(None);
        summary.set_xalign(0.0);
        summary.set_wrap(true);
        summary.set_max_width_chars(50);
        let instructions = gtk::Box::new(gtk::Orientation::Vertical, 6);
        let code = gtk::Label::new(None);
        code.add_css_class("setup-code");
        code.set_selectable(false);
        let notice = gtk::Label::new(None);
        notice.add_css_class("error");
        notice.set_xalign(0.0);
        notice.set_wrap(true);
        notice.set_max_width_chars(50);
        let list = gtk::ListBox::new();
        list.add_css_class("boxed-list");
        list.set_selection_mode(gtk::SelectionMode::Single);
        let buttons = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        buttons.set_halign(gtk::Align::End);
        let keys = gtk::Label::new(None);
        keys.add_css_class("setup-keys");
        keys.set_xalign(0.0);

        let layout = gtk::Box::new(gtk::Orientation::Vertical, 14);
        for widget in [
            heading.upcast_ref::<gtk::Widget>(),
            summary.upcast_ref(),
            instructions.upcast_ref(),
            code.upcast_ref(),
            notice.upcast_ref(),
            list.upcast_ref(),
            buttons.upcast_ref(),
            keys.upcast_ref(),
        ] {
            layout.append(widget);
        }
        window.set_child(Some(&layout));

        let setup = Rc::new(Self {
            window,
            heading,
            summary,
            instructions,
            code,
            notice,
            list,
            buttons,
            keys,
            intents: intents.clone(),
            report,
            content: Rc::default(),
            mapped: Rc::default(),
            opened: Cell::default(),
            step: Cell::default(),
            phones: RefCell::default(),
            listed: Cell::default(),
            top: RefCell::default(),
            top_keeps_focus: Cell::default(),
        });
        setup.connect();
        // Pre-warm, as for the other surfaces.
        WidgetExt::realize(&setup.window);
        setup
    }

    /// Enter on a Phone chooses it; Escape or closing the window cancels;
    /// what is shown is reported once mapped, and as focus comes and goes.
    fn connect(self: &Rc<Self>) {
        let weak = Rc::downgrade(self);
        self.list.connect_row_activated(move |_, row| {
            let Some(setup) = weak.upgrade() else {
                return;
            };
            let phone = usize::try_from(row.index())
                .ok()
                .and_then(|index| setup.phones.borrow().get(index).cloned());
            if let Some(phone) = phone {
                setup.send(Input::ChoosePhone {
                    address: phone.address,
                });
            }
        });
        let intents = self.intents.clone();
        self.window.connect_close_request(move |_| {
            let _ = intents.unbounded_send(Input::CloseSetupWindow);
            // The Call state machine closes it.
            glib::Propagation::Stop
        });
        let keys = gtk::EventControllerKey::new();
        keys.set_propagation_phase(gtk::PropagationPhase::Capture);
        let intents = self.intents.clone();
        keys.connect_key_pressed(move |_, key, _, _| {
            if key != gtk::gdk::Key::Escape {
                return glib::Propagation::Proceed;
            }
            let _ = intents.unbounded_send(Input::CloseSetupWindow);
            glib::Propagation::Stop
        });
        self.window.add_controller(keys);

        let (content, mapped, report) = (
            self.content.clone(),
            self.mapped.clone(),
            self.report.clone(),
        );
        self.window.connect_map(move |window| {
            mapped.set(true);
            report
                .borrow_mut()
                .set(NAME, Some(reported(&content.borrow(), window)));
            surface_mapped(NAME);
            after_next_frame(window, || surface_shown(NAME));
        });
        let (content, mapped, report) = (
            self.content.clone(),
            self.mapped.clone(),
            self.report.clone(),
        );
        self.window.connect_is_active_notify(move |window| {
            if mapped.get() {
                report
                    .borrow_mut()
                    .set(NAME, Some(reported(&content.borrow(), window)));
            }
        });
        let (mapped, report) = (self.mapped.clone(), self.report.clone());
        self.window.connect_unmap(move |_| {
            mapped.set(false);
            report.borrow_mut().set(NAME, None);
        });
    }

    /// Puts `widget` above everything else in the window.
    pub(super) fn put_on_top(&self, widget: &impl IsA<gtk::Widget>) {
        if let Some(layout) = self.window.child().and_downcast::<gtk::Box>() {
            layout.prepend(widget);
            self.top.replace(Some(widget.clone().upcast()));
        }
    }

    /// Says whether what is on top keeps keyboard focus when a Phone
    /// appears; otherwise the Phone takes it, as without it.
    pub(super) fn top_keeps_focus(&self, keeps: bool) {
        self.top_keeps_focus.set(keeps);
    }

    /// Gives keyboard focus back to the step, if nothing in the window has
    /// it.
    pub(super) fn focus_if_lost(&self) {
        if self.window.is_visible() && GtkWindowExt::focus(&self.window).is_none() {
            self.focus_default();
        }
    }

    /// Whether what is on top has keyboard focus.
    fn top_has_focus(&self) -> bool {
        let top = self.top.borrow();
        GtkWindowExt::focus(&self.window)
            .zip(top.as_ref())
            .is_some_and(|(focus, top)| focus.is_ancestor(top))
    }

    fn send(&self, input: Input) {
        let _ = self.intents.unbounded_send(input);
    }

    pub(super) fn render(&self, view: Option<&SetupWindowView>) {
        let Some(view) = view else {
            self.opened.set(None);
            self.step.set(None);
            self.window.set_visible(false);
            return;
        };
        let opening = self
            .opened
            .get()
            .is_none_or(|(opened, _)| opened != view.opened);
        if opening {
            self.opened.set(Some((view.opened, Instant::now())));
            self.step.set(None);
            self.listed.set(false);
        }
        let text = messages::setup_window(view);
        let step = view.step.as_str();
        let new_step = self.step.replace(Some(step)) != Some(step);
        self.window.set_title(Some(text.title));
        self.heading.set_text(&text.heading);
        self.summary
            .set_text(text.summary.as_deref().unwrap_or_default());
        self.summary.set_visible(text.summary.is_some());
        self.show_instructions(&text.instructions);
        self.code.set_text(text.code.as_deref().unwrap_or_default());
        self.code.set_visible(text.code.is_some());
        self.notice.set_text(text.notice.unwrap_or_default());
        self.notice.set_visible(text.notice.is_some());
        self.keys.set_text(&text.keys);
        let phones = match &view.step {
            SetupStep::Choose { phones } => phones.as_slice(),
            _ => &[],
        };
        let relisted = self.show_phones(phones, &text, new_step);
        if new_step {
            self.show_buttons(&view.step, &text);
        }

        let content = report(view, &text);
        let unchanged = *self.content.borrow() == content;
        *self.content.borrow_mut() = content;
        if !self.window.is_visible() {
            self.window.present();
        } else if self.mapped.get() && !unchanged {
            self.report
                .borrow_mut()
                .set(NAME, Some(reported(&self.content.borrow(), &self.window)));
        }
        if new_step {
            self.focus_default();
            self.step_reached(step);
        } else if relisted
            && self.list.focus_child().is_none()
            // Before the first Phone shows, focus on top is GTK's default
            // and goes to the Phone; after, the user put it there.
            && !(self.top_has_focus() && (self.top_keeps_focus.get() || self.listed.get()))
        {
            self.focus_default();
        }
        if !phones.is_empty() && !self.listed.replace(true) {
            self.phone_listed();
        }
    }

    fn show_instructions(&self, instructions: &[String]) {
        while let Some(child) = self.instructions.first_child() {
            self.instructions.remove(&child);
        }
        // Numbered when there is more than one thing to do.
        let numbered = instructions.len() > 1;
        for (number, instruction) in (1..).zip(instructions) {
            let line = if numbered {
                format!("{number}. {instruction}")
            } else {
                instruction.clone()
            };
            let label = gtk::Label::new(Some(&line));
            label.set_xalign(0.0);
            label.set_wrap(true);
            label.set_max_width_chars(50);
            self.instructions.append(&label);
        }
        self.instructions.set_visible(!instructions.is_empty());
    }

    /// Lists `phones`, keeping the one selected; whether the list changed.
    fn show_phones(&self, phones: &[Phone], text: &SetupWindowText, new_step: bool) -> bool {
        let empty = text.empty_list.map(|empty| {
            let label = gtk::Label::new(Some(empty));
            label.add_css_class("dim-label");
            label.set_margin_top(12);
            label.set_margin_bottom(12);
            label
        });
        self.list.set_placeholder(empty.as_ref());
        self.list.set_visible(text.empty_list.is_some());
        if *self.phones.borrow() == phones && !new_step {
            return false;
        }
        let selected = self
            .list
            .selected_row()
            .and_then(|row| usize::try_from(row.index()).ok())
            .and_then(|index| {
                self.phones
                    .borrow()
                    .get(index)
                    .map(|phone| phone.address.clone())
            });
        self.list.remove_all();
        for phone in phones {
            let name = gtk::Label::new(Some(&phone.name));
            name.add_css_class("setup-phone");
            name.set_xalign(0.0);
            let row = gtk::ListBoxRow::new();
            row.set_child(Some(&name));
            self.list.append(&row);
        }
        let keep = selected
            .and_then(|address| phones.iter().position(|phone| phone.address == address))
            .unwrap_or(0);
        if let Some(row) = i32::try_from(keep)
            .ok()
            .and_then(|index| self.list.row_at_index(index))
        {
            self.list.select_row(Some(&row));
        }
        *self.phones.borrow_mut() = phones.to_vec();
        true
    }

    /// The step's buttons. The default one goes last, at the window's
    /// corner, as GNOME's dialogs have it, and is marked when there is a
    /// choice.
    fn show_buttons(&self, step: &SetupStep, text: &SetupWindowText) {
        while let Some(child) = self.buttons.first_child() {
            self.buttons.remove(&child);
        }
        let buttons = text.buttons.iter().zip(button_intents(step)).enumerate();
        for (index, (label, intent)) in buttons.rev() {
            let button = gtk::Button::with_label(label);
            if index == 0 && text.buttons.len() > 1 {
                button.add_css_class("suggested-action");
            }
            let intents = self.intents.clone();
            button.connect_clicked(move |_| {
                let _ = intents.unbounded_send(intent.clone());
            });
            self.buttons.append(&button);
        }
    }

    /// Keyboard focus on the first Phone while choosing, otherwise on the
    /// step's default button.
    fn focus_default(&self) {
        if self.step.get() == Some("choose") {
            if let Some(row) = self.list.selected_row() {
                row.grab_focus();
            }
            return;
        }
        // The default button is last in the box.
        if let Some(button) = self.buttons.last_child() {
            button.grab_focus();
        }
    }

    fn since_opened_ms(&self) -> u64 {
        self.opened.get().map_or(0, |(_, at)| {
            u64::try_from(at.elapsed().as_millis()).unwrap_or(u64::MAX)
        })
    }

    /// Logs when `step` first reached the screen, for the journal's
    /// per-step timings.
    fn step_reached(&self, step: &'static str) {
        let Some((_, at)) = self.opened.get() else {
            return;
        };
        after_next_frame(&self.window, move || {
            let since_opened_ms = u64::try_from(at.elapsed().as_millis()).unwrap_or(u64::MAX);
            tracing::info!(
                event_kind = %EventKind::SetupStepReached,
                step,
                since_opened_ms,
                "The Setup Window is at step {step}, {since_opened_ms} ms after it opened"
            );
        });
    }

    fn phone_listed(&self) {
        let opened_ms = self.since_opened_ms();
        after_next_frame(&self.window, move || {
            tracing::info!(
                event_kind = %EventKind::SetupPhoneListed,
                since_opened_ms = opened_ms,
                "The Setup Window listed a Phone"
            );
        });
    }
}

/// What each of a step's buttons asks for, in the order of its labels.
fn button_intents(step: &SetupStep) -> Vec<Input> {
    match step {
        SetupStep::Confirm { .. } => vec![Input::ConfirmPairing, Input::RejectPairing],
        SetupStep::PhoneCallsOff { .. } | SetupStep::PhoneUnreachable { .. } => {
            vec![Input::RetryHandsFree, Input::CloseSetupWindow]
        }
        SetupStep::Choose { .. }
        | SetupStep::Pairing { .. }
        | SetupStep::Confirmed { .. }
        | SetupStep::Connecting { .. }
        | SetupStep::Done { .. } => vec![Input::CloseSetupWindow],
    }
}

/// What the window reports for `view`.
fn report(view: &SetupWindowView, text: &SetupWindowText) -> BTreeMap<String, String> {
    let mut content = BTreeMap::from([
        ("step".to_owned(), view.step.as_str().to_owned()),
        ("heading".to_owned(), text.heading.clone()),
        ("keys".to_owned(), text.keys.clone()),
        ("instructions".to_owned(), text.instructions.join("\n")),
        ("buttons".to_owned(), text.buttons.join("\n")),
    ]);
    if let SetupStep::Choose { phones } = &view.step {
        let names: Vec<&str> = phones.iter().map(|phone| phone.name.as_str()).collect();
        content.insert("phones".to_owned(), names.join("\n"));
    }
    if let Some(summary) = &text.summary {
        content.insert("summary".to_owned(), summary.clone());
    }
    if let Some(code) = &text.code {
        content.insert("code".to_owned(), code.clone());
    }
    if let Some(notice) = text.notice {
        content.insert("notice".to_owned(), notice.to_owned());
    }
    if let Some(desktop) = &view.desktop_name {
        content.insert("desktop".to_owned(), desktop.clone());
    }
    content
}
