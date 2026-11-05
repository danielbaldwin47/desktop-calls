//! The Setup Window: pairs a Phone with the Desktop, step by step.
//!
//! Unlike the other surfaces it is an ordinary window, not a layer-shell
//! surface: it is a task the user works through, which tiles, takes focus
//! when it opens and can be moved aside. Each step's default button has
//! keyboard focus, so Enter takes the step; while choosing, the first Phone
//! listed has focus, the arrow keys move between Phones and Enter chooses.
//! Escape or closing the window cancels.
//!
//! It reports `setup-window` with `step`, `heading`, `instructions`,
//! `buttons` and `progress` (one per line), `phones` (names, one per line),
//! `summary`, `code`, `countdown`, `line`, `tip`, `notice`, `desktop` and
//! `focused`, and logs when each step and the first Phone listed reached the
//! screen, timed from the window opening.
//!
//! While the Phone asks whether to share Contacts, the countdown changes by
//! the second, and so does the time left reading Contacts: the window shows
//! them anew as they change, and draws nothing in between.

use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;
use std::rc::Rc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use desktop_calls_core::messages::{self, SetupWindowText};
use desktop_calls_core::{
    Input, KeybindingCheck, KeybindingStage, Phone, SetupStep, SetupWindowView, Timestamp,
};
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
.desktop-calls-setup .setup-line { font-family: monospace; padding: 8px; }
.desktop-calls-setup .setup-tip { opacity: 0.75; }
";

/// How often the window checks whether the countdown, or the time left
/// reading Contacts, changed: often enough that it changes on time.
const TICK: Duration = Duration::from_millis(200);

/// The Setup Window.
pub(super) struct SetupWindow {
    window: gtk::Window,
    heading: gtk::Label,
    summary: gtk::Label,
    instructions: gtk::Box,
    code: gtk::Label,
    progress: gtk::Label,
    line: gtk::Label,
    tip: gtk::Label,
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
    /// What is shown, so it is drawn again only when it changes.
    shown: RefCell<Option<(SetupWindowView, SetupWindowText)>>,
    /// The buttons shown, by label.
    shown_buttons: RefCell<Vec<&'static str>>,
    /// Shows the countdown anew while it runs.
    ticking: RefCell<Option<glib::SourceId>>,
}

impl SetupWindow {
    pub(super) fn new(intents: &UnboundedSender<Input>, report: SharedReport) -> Rc<Self> {
        add_css();
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
        let progress = gtk::Label::new(None);
        progress.set_xalign(0.0);
        let line = gtk::Label::new(None);
        line.add_css_class("setup-line");
        line.add_css_class("card");
        line.set_xalign(0.0);
        line.set_wrap(true);
        line.set_wrap_mode(gtk::pango::WrapMode::Char);
        line.set_max_width_chars(50);
        // Selectable with the pointer, to copy; Tab still moves only
        // between the buttons.
        line.set_selectable(true);
        line.set_focusable(false);
        let tip = gtk::Label::new(None);
        tip.add_css_class("setup-tip");
        tip.set_xalign(0.0);
        tip.set_wrap(true);
        tip.set_max_width_chars(50);
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
            progress.upcast_ref(),
            line.upcast_ref(),
            notice.upcast_ref(),
            list.upcast_ref(),
            tip.upcast_ref(),
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
            progress,
            line,
            tip,
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
            shown: RefCell::default(),
            shown_buttons: RefCell::default(),
            ticking: RefCell::default(),
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
        let (window, buttons) = (self.window.clone(), self.buttons.clone());
        keys.connect_key_pressed(move |_, key, _, _| match key {
            gtk::gdk::Key::Escape => {
                let _ = intents.unbounded_send(Input::CloseSetupWindow);
                glib::Propagation::Stop
            }
            gtk::gdk::Key::Tab => between_buttons(&window, &buttons, true),
            gtk::gdk::Key::ISO_Left_Tab => between_buttons(&window, &buttons, false),
            _ => glib::Propagation::Proceed,
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

    pub(super) fn render(self: &Rc<Self>, view: Option<&SetupWindowView>) {
        let Some(view) = view else {
            self.opened.set(None);
            self.step.set(None);
            self.shown.replace(None);
            self.shown_buttons.borrow_mut().clear();
            self.tick(false);
            self.window.set_visible(false);
            return;
        };
        self.tick(matches!(
            view.step,
            SetupStep::ContactsPrompt { .. } | SetupStep::ContactsSyncing { .. }
        ));
        let text = messages::setup_window(view, now());
        let shown = (view.clone(), text.clone());
        if self.shown.borrow().as_ref() == Some(&shown) {
            return;
        }
        self.shown.replace(Some(shown));
        self.show(view, &text);
    }

    /// While `on`, shows the window anew as the time left changes.
    fn tick(self: &Rc<Self>, on: bool) {
        let mut ticking = self.ticking.borrow_mut();
        match (on, ticking.is_some()) {
            (true, false) => {
                let weak = Rc::downgrade(self);
                *ticking = Some(glib::timeout_add_local(TICK, move || {
                    let Some(setup) = weak.upgrade() else {
                        return glib::ControlFlow::Break;
                    };
                    let view = setup.shown.borrow().as_ref().map(|(view, _)| view.clone());
                    setup.render(view.as_ref());
                    glib::ControlFlow::Continue
                }));
            }
            (false, true) => {
                if let Some(source) = ticking.take() {
                    source.remove();
                }
            }
            _ => {}
        }
    }

    fn show(&self, view: &SetupWindowView, text: &SetupWindowText) {
        let opening = self
            .opened
            .get()
            .is_none_or(|(opened, _)| opened != view.opened);
        if opening {
            self.opened.set(Some((view.opened, Instant::now())));
            self.step.set(None);
            self.listed.set(false);
        }
        let step = view.step.as_str();
        let new_step = self.step.replace(Some(step)) != Some(step);
        let new_buttons = *self.shown_buttons.borrow() != text.buttons;
        self.window.set_title(Some(text.title));
        self.heading.set_text(&text.heading);
        self.summary
            .set_text(text.summary.as_deref().unwrap_or_default());
        self.summary.set_visible(text.summary.is_some());
        self.show_instructions(&text.instructions);
        // The pairing code, or the countdown: both big digits.
        let digits = text.code.as_deref().or(text.countdown.as_deref());
        self.code.set_text(digits.unwrap_or_default());
        self.code.set_visible(digits.is_some());
        self.progress.set_text(&text.progress.join("\n"));
        self.progress.set_visible(!text.progress.is_empty());
        self.line.set_text(text.line.as_deref().unwrap_or_default());
        self.line.set_visible(text.line.is_some());
        self.tip.set_text(text.tip.unwrap_or_default());
        self.tip.set_visible(text.tip.is_some());
        self.notice.set_text(text.notice.unwrap_or_default());
        self.notice.set_visible(text.notice.is_some());
        self.keys.set_text(&text.keys);
        let phones = match &view.step {
            SetupStep::Choose { phones } => phones.as_slice(),
            _ => &[],
        };
        let relisted = self.show_phones(phones, text, new_step);
        if new_step || new_buttons {
            self.show_buttons(&view.step, text);
            self.shown_buttons.replace(text.buttons.clone());
        }

        let content = report(view, text);
        let unchanged = *self.content.borrow() == content;
        *self.content.borrow_mut() = content;
        if !self.window.is_visible() {
            self.window.present();
        } else if self.mapped.get() && !unchanged {
            self.report
                .borrow_mut()
                .set(NAME, Some(reported(&self.content.borrow(), &self.window)));
        }
        if new_step || new_buttons {
            self.focus_default();
        }
        if new_step {
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

/// Tab, or Shift+Tab when not `forward`, on a button moves to the next
/// button, or the previous, round from the last to the first: GTK would
/// move focus out of the window past the last.
fn between_buttons(window: &gtk::Window, buttons: &gtk::Box, forward: bool) -> glib::Propagation {
    let mut all = Vec::new();
    let mut child = buttons.first_child();
    while let Some(button) = child {
        child = button.next_sibling();
        all.push(button);
    }
    let focused = gtk::prelude::GtkWindowExt::focus(window);
    let Some(at) = all
        .iter()
        .position(|button| Some(button) == focused.as_ref())
    else {
        return glib::Propagation::Proceed;
    };
    let next = if forward {
        (at + 1) % all.len()
    } else {
        (at + all.len() - 1) % all.len()
    };
    if let Some(button) = all.get(next) {
        button.grab_focus();
    }
    glib::Propagation::Stop
}

/// What each of a step's buttons asks for, in the order of its labels.
/// Adds the Setup Window's styles to the display.
fn add_css() {
    if let Some(display) = gtk::gdk::Display::default() {
        let provider = gtk::CssProvider::new();
        provider.load_from_string(CSS);
        gtk::style_context_add_provider_for_display(
            &display,
            &provider,
            gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );
    }
}

fn button_intents(step: &SetupStep) -> Vec<Input> {
    match step {
        SetupStep::Confirm { .. } => vec![Input::ConfirmPairing, Input::RejectPairing],
        SetupStep::PhoneCallsOff { .. } | SetupStep::PhoneUnreachable { .. } => {
            vec![Input::RetryHandsFree, Input::CloseSetupWindow]
        }
        SetupStep::ContactsPromptTimedOut { .. } => {
            vec![Input::RetryContacts, Input::SkipContacts]
        }
        SetupStep::ContactsPhoneAway { back: true, .. } => {
            vec![Input::ContinueContacts, Input::CloseSetupWindow]
        }
        SetupStep::Done {
            keybinding: KeybindingStage::Checked(KeybindingCheck::Available { .. }),
            ..
        } => vec![Input::AddKeybinding, Input::CloseSetupWindow],
        SetupStep::Done {
            keybinding: KeybindingStage::Confirming(_),
            ..
        } => vec![Input::ConfirmKeybinding, Input::CancelKeybinding],
        SetupStep::Choose { .. }
        | SetupStep::Pairing { .. }
        | SetupStep::Confirmed { .. }
        | SetupStep::Connecting { .. }
        | SetupStep::ContactsWarning { .. }
        | SetupStep::ContactsPrompt { .. }
        | SetupStep::ContactsSyncing { .. }
        | SetupStep::ContactsPhoneAway { back: false, .. }
        | SetupStep::Done { .. } => vec![Input::CloseSetupWindow],
    }
}

/// Now, for the countdown.
fn now() -> Timestamp {
    Timestamp(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |elapsed| {
                u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX)
            }),
    )
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
    if let Some(countdown) = &text.countdown {
        content.insert("countdown".to_owned(), countdown.clone());
    }
    // What the countdown counts to, in ms since the epoch.
    if let SetupStep::ContactsPrompt { deadline, .. } = &view.step {
        content.insert("deadline".to_owned(), deadline.0.to_string());
    }
    if !text.progress.is_empty() {
        content.insert("progress".to_owned(), text.progress.join("\n"));
    }
    if let Some(line) = &text.line {
        content.insert("line".to_owned(), line.clone());
    }
    if let Some(tip) = text.tip {
        content.insert("tip".to_owned(), tip.to_owned());
    }
    if let Some(notice) = text.notice {
        content.insert("notice".to_owned(), notice.to_owned());
    }
    if let Some(desktop) = &view.desktop_name {
        content.insert("desktop".to_owned(), desktop.clone());
    }
    content
}
