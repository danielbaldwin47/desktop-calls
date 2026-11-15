//! The Setup Window: pairs a Phone with the Desktop, step by step.
//!
//! Unlike the other surfaces it is an ordinary window, not a layer-shell
//! surface: it is a task the user works through, which tiles, takes focus
//! when it opens and can be moved aside. Each step's default button has
//! keyboard focus, so Enter takes the step; while choosing, the first Phone
//! listed has focus, the arrow keys move between Phones and Enter chooses.
//! Escape or closing the window cancels. A footer line shows the keys that
//! work now; the window tells the core which control has the keyboard as
//! focus moves, so after Tab or Shift+Tab the footer names the button Enter
//! presses. While a Problem blocks the step, its card sits under the
//! heading (`setup_problems`) and the step's words are dimmed. While calls
//! are off, the Calls Off card (`setup_calls_off`) sits first under the
//! heading, with Turn calls on: while choosing a Phone it has the keyboard,
//! the step's words are dimmed and no Phone is listed.
//!
//! Four named steps run across the top (`setup_parts`), the current one
//! marked and the done ones ticked; the instructions are grouped under "On
//! your phone" and "Here"; the countdown says what it counts; the sync is a
//! checklist with a bar; and the finish page offers the keybinding and the
//! pin tip as two equal cards.
//!
//! It reports `setup-window` with `step`, `stage` (the named step),
//! `stages`, `heading`, `instructions`, `buttons` and `progress` (one per
//! line), `phones` (names, one per line), `summary`, `code`, `countdown`,
//! `countdown-label`, `sync-bar` (in thousandths), `sync-note`, `line`,
//! `keybinding` and `tip` (each card's title, then its text), `notice`,
//! `keys`,
//! `dimmed` (while a Problem or calls being off blocks the step),
//! `calls-off` (its heading), `calls-off-text`, `calls-off-button`,
//! `desktop` and `focused`, and
//! logs when each step and the first Phone listed reached the screen, timed
//! from the window opening.
//!
//! While the Phone asks whether to share Contacts, the countdown changes by
//! the second, and so does the time left reading Contacts: the window shows
//! them anew as they change, and draws nothing in between.

use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;
use std::rc::Rc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use desktop_calls_core::messages::{
    self, ProblemKind, SETUP_STAGES, SetupWindowText, StageMark, stage_line,
};
use desktop_calls_core::{
    Input, KeybindingCheck, KeybindingStage, Phone, SetupFocus, SetupStep, SetupWindowView,
    Timestamp,
};
use futures_channel::mpsc::UnboundedSender;
use gtk::glib;
use gtk::prelude::*;

use super::setup_calls_off::CallsOffCard;
use super::setup_parts::{
    self, Card, Instructions, Stages, SyncView, keep_runs_whole, wrapping_label,
};
use super::setup_problems::notice_card;
use super::{
    SharedReport, after_next_frame, class, footer, reported, set_class, surface_mapped,
    surface_shown,
};
use crate::logging::EventKind;
use crate::theme::space;

const NAME: &str = "setup-window";
const WIDTH: i32 = 480;

const CSS: &str = "
.desktop-calls-setup { padding: 20px; }
.desktop-calls-setup .setup-code {
    font-size: 2.6em; font-weight: bold; letter-spacing: 0.15em;
    font-feature-settings: \"tnum\";
}
.desktop-calls-setup .setup-phone { padding: 10px 12px; font-weight: bold; }
.desktop-calls-setup .setup-line { font-family: monospace; padding: 8px; }
/* A Problem and a notice are cards on the warning tint, with the warning
   colour down the left edge. */
.desktop-calls-setup .notice { border-left: 3px solid var(--dc-warning); }
/* A Problem's button: small, and the Fix filled with the accent. Every
   primary button, the Fix and the step's default alike, takes one label
   and the accent in the shade that label reads on at 4.5:1 (stock
   Adwaita's suggested blue doesn't under white) (#320). */
.desktop-calls-setup button.small {
    min-height: 30px; min-width: 0; padding: 0 10px; font-size: 12.5px;
}
.desktop-calls-setup button.accent,
.desktop-calls-setup button.suggested-action {
    background: var(--dc-accent-fill); color: var(--dc-accent-label);
}
/* Calls are off: a card in the Problem card's shape, on a neutral tint
   with a neutral edge, since a choice isn't a fault. */
.desktop-calls-setup .notice.calls-off {
    background: var(--dc-selection); border-left-color: var(--dc-secondary);
}
.desktop-calls-setup .notice.calls-off .notice-icon { color: var(--dc-secondary); }
/* The steps a Problem blocks: in secondary text, the weakest that still
   reads at 4.5:1, not faded. */
.desktop-calls-setup .setup-dimmed, .desktop-calls-setup .setup-dimmed label {
    color: var(--dc-secondary);
}
";

/// How often the window checks whether the countdown, or the time left
/// reading Contacts, changed: often enough that it changes on time.
const TICK: Duration = Duration::from_millis(200);

/// The Setup Window.
pub(super) struct SetupWindow {
    window: gtk::Window,
    /// The four named steps across the top.
    stages: Stages,
    heading: gtk::Label,
    summary: gtk::Label,
    instructions: Instructions,
    code: gtk::Label,
    /// What the countdown counts, under it.
    countdown_label: gtk::Label,
    sync: SyncView,
    line: gtk::Label,
    keybinding: Card,
    tip: Card,
    notice: gtk::Box,
    notice_text: gtk::Label,
    /// Says calls are off, with Turn calls on.
    calls_off: CallsOffCard,
    /// Turn calls on has the keyboard, as shown last.
    calls_off_focus: Cell<bool>,
    /// Said while no Phone is listed.
    empty: gtk::Label,
    list: gtk::ListBox,
    buttons: gtk::Box,
    footer: gtk::Box,
    /// The home directory, which a file in it is named relative to.
    home: Option<String>,
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

        let stages = Stages::new();
        let heading = gtk::Label::new(None);
        heading.add_css_class(class::HEADING);
        heading.set_xalign(0.0);
        heading.set_wrap(true);
        let summary = wrapping_label();
        let instructions = Instructions::new();
        let code = gtk::Label::new(None);
        code.add_css_class("setup-code");
        code.set_selectable(false);
        let countdown_label = gtk::Label::new(None);
        countdown_label.add_css_class("setup-countdown-label");
        let sync = SyncView::new();
        let line = setup_parts::line_label();
        let keybinding = Card::new();
        let tip = Card::new();
        let (notice, notice_text) = notice_card();
        let calls_off = CallsOffCard::new(intents);
        // Plain dimmed words, not a field that looks like an input.
        let empty = gtk::Label::new(None);
        empty.add_css_class("dim-label");
        empty.set_xalign(0.0);
        empty.set_visible(false);
        let list = gtk::ListBox::new();
        list.add_css_class("boxed-list");
        list.set_selection_mode(gtk::SelectionMode::Single);
        // What is listed here goes in the "Here" group.
        instructions.add_here(&empty);
        instructions.add_here(&list);
        let buttons = gtk::Box::new(gtk::Orientation::Horizontal, space::S);
        buttons.set_halign(gtk::Align::End);
        let footer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        footer.add_css_class(class::FOOTER_LINE);

        // The code, or the countdown, and what it counts: one block.
        let digits = gtk::Box::new(gtk::Orientation::Vertical, 2);
        digits.append(&code);
        digits.append(&countdown_label);
        let layout = gtk::Box::new(gtk::Orientation::Vertical, space::L);
        for widget in [
            stages.widget.upcast_ref::<gtk::Widget>(),
            heading.upcast_ref(),
            calls_off.widget.upcast_ref(),
            digits.upcast_ref(),
            summary.upcast_ref(),
            notice.upcast_ref(),
            instructions.widget.upcast_ref(),
            sync.widget.upcast_ref(),
            line.upcast_ref(),
            keybinding.widget.upcast_ref(),
            tip.widget.upcast_ref(),
            buttons.upcast_ref(),
            footer.upcast_ref(),
        ] {
            layout.append(widget);
        }
        window.set_child(Some(&layout));

        let setup = Rc::new(Self {
            window,
            stages,
            heading,
            summary,
            instructions,
            code,
            countdown_label,
            sync,
            line,
            keybinding,
            tip,
            notice,
            notice_text,
            calls_off,
            calls_off_focus: Cell::default(),
            empty,
            list,
            buttons,
            footer,
            home: std::env::var("HOME").ok(),
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
            gtk::gdk::Key::Tab => round_from_the_last_button(&window, &buttons),
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

    /// Puts `widget` under the Calls Off card, above the step: where a
    /// Problem that blocks the step is read.
    pub(super) fn put_on_top(&self, widget: &impl IsA<gtk::Widget>) {
        if let Some(layout) = self.window.child().and_downcast::<gtk::Box>() {
            layout.insert_child_after(widget, Some(&self.calls_off.widget));
            self.top.replace(Some(widget.clone().upcast()));
        }
    }

    /// Says whether what is on top keeps keyboard focus when a Phone
    /// appears; otherwise the Phone takes it, as without it.
    pub(super) fn top_keeps_focus(&self, keeps: bool) {
        self.top_keeps_focus.set(keeps);
    }

    /// Tells the core which control has the keyboard whenever focus moves,
    /// for the footer to name what Enter presses. `problem_of` names the
    /// Problem whose card's button a widget is.
    pub(super) fn follow_focus(
        self: &Rc<Self>,
        problem_of: impl Fn(&gtk::Widget) -> Option<ProblemKind> + 'static,
    ) {
        let weak = Rc::downgrade(self);
        self.window.connect_focus_widget_notify(move |_| {
            if let Some(setup) = weak.upgrade() {
                setup.send(Input::SetupWindowFocus(setup.focused_control(&problem_of)));
            }
        });
    }

    /// The control that has the keyboard, as the core names it.
    fn focused_control(
        &self,
        problem_of: &impl Fn(&gtk::Widget) -> Option<ProblemKind>,
    ) -> Option<SetupFocus> {
        let focus = GtkWindowExt::focus(&self.window)?;
        let within = |widget: &gtk::Widget| focus == *widget || focus.is_ancestor(widget);
        if within(self.list.upcast_ref()) {
            return Some(SetupFocus::Phone);
        }
        if within(self.calls_off.button.upcast_ref()) {
            return Some(SetupFocus::TurnCallsOn);
        }
        if let Some(kind) = problem_of(&focus) {
            return Some(SetupFocus::Problem(kind));
        }
        let buttons: Vec<gtk::Widget> =
            std::iter::successors(self.buttons.first_child(), WidgetExt::next_sibling).collect();
        // The default button, first in the core's order, is last in the box.
        let place = buttons.iter().position(within)?;
        Some(SetupFocus::Button(buttons.len() - 1 - place))
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
            if let Some(step) = self.step.get() {
                let since_opened_ms = self.since_opened_ms();
                tracing::info!(
                    event_kind = %EventKind::SetupClosed,
                    step,
                    since_opened_ms,
                    "The Setup Window closed at step {step}, {since_opened_ms} ms after it opened"
                );
            }
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
        let text = messages::setup_window(view, now(), self.home.as_deref());
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
        self.stages.show(text.stages);
        self.heading.set_text(&text.heading);
        self.summary
            .set_text(text.summary.as_deref().unwrap_or_default());
        self.summary.set_visible(text.summary.is_some());
        // Calls being off, or a Problem, stop the step: its words in
        // secondary text.
        let waits_for_calls = view.calls_off.is_some_and(|off| off.blocks_step);
        let dimmed = view.problem_has_focus.is_some() || waits_for_calls;
        for widget in [
            self.summary.upcast_ref::<gtk::Widget>(),
            self.instructions.widget.upcast_ref(),
        ] {
            set_class(widget, "setup-dimmed", dimmed);
        }
        // The pairing code, or the countdown in seconds with what it
        // counts: both big digits.
        let countdown = text
            .countdown
            .as_ref()
            .map(|seconds| format!("{seconds} s"));
        let digits = text.code.as_deref().or(countdown.as_deref());
        self.code.set_text(digits.unwrap_or_default());
        self.countdown_label
            .set_text(text.countdown_label.unwrap_or_default());
        self.countdown_label
            .set_visible(text.countdown_label.is_some());
        if let Some(block) = self.code.parent() {
            block.set_visible(digits.is_some());
        }
        self.sync.show(text.sync.as_ref());
        keep_runs_whole(&self.line, text.line.as_deref().unwrap_or_default());
        self.line.set_visible(text.line.is_some());
        self.keybinding.show(text.keybinding.as_ref());
        self.tip.show(text.tip.as_ref());
        self.notice_text
            .set_text(text.notice.as_deref().unwrap_or_default());
        self.notice.set_visible(text.notice.is_some());
        self.calls_off
            .show(text.calls_off.as_ref(), waits_for_calls);
        let took_keyboard = self.calls_off_focus.replace(waits_for_calls);
        footer::show(&self.footer, &text.keys);
        self.footer.set_visible(!text.keys.is_empty());
        let phones = match &view.step {
            SetupStep::Choose { phones } => phones.as_slice(),
            _ => &[],
        };
        let relisted = self.show_phones(phones, text, new_step);
        // What is listed here shows while choosing, with or without a Phone.
        self.instructions.show(
            &messages::grouped(&text.instructions),
            text.empty_list.is_some(),
        );
        if new_step || new_buttons {
            self.show_buttons(&view.step, text);
            self.shown_buttons.replace(text.buttons.clone());
        }

        let mut content = report(view, text);
        // The words in place of the list, as shown (#315).
        if self.empty.is_visible() {
            content.insert("looking".to_owned(), self.empty.text().to_string());
        }
        let unchanged = *self.content.borrow() == content;
        *self.content.borrow_mut() = content;
        if !self.window.is_visible() {
            self.window.present();
        } else if self.mapped.get() && !unchanged {
            self.report
                .borrow_mut()
                .set(NAME, Some(reported(&self.content.borrow(), &self.window)));
        }
        if new_step || new_buttons || waits_for_calls != took_keyboard {
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

    /// Lists `phones`, keeping the one selected; whether the list changed.
    fn show_phones(&self, phones: &[Phone], text: &SetupWindowText, new_step: bool) -> bool {
        // While no Phone is nearby, plain dimmed words; the list shows only
        // once there is a Phone in it.
        let looking = text.empty_list.filter(|_| phones.is_empty());
        self.empty.set_text(looking.unwrap_or_default());
        self.empty.set_visible(looking.is_some());
        self.list
            .set_visible(text.empty_list.is_some() && !phones.is_empty());
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
        // Turn calls on, while pairing waits for calls to be on.
        if self.calls_off_focus.get() {
            self.calls_off.button.grab_focus();
            return;
        }
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

/// Tab on the last button moves round to the first: GTK would move focus
/// out of the window. Shift+Tab is GTK's own, so from the first button it
/// reaches the Problems above.
fn round_from_the_last_button(window: &gtk::Window, buttons: &gtk::Box) -> glib::Propagation {
    let (Some(first), Some(last)) = (buttons.first_child(), buttons.last_child()) else {
        return glib::Propagation::Proceed;
    };
    if gtk::prelude::GtkWindowExt::focus(window).as_ref() != Some(&last) {
        return glib::Propagation::Proceed;
    }
    first.grab_focus();
    glib::Propagation::Stop
}

/// What each of a step's buttons asks for, in the order of its labels.
/// Adds the Setup Window's styles to the display.
fn add_css() {
    if let Some(display) = gtk::gdk::Display::default() {
        let provider = gtk::CssProvider::new();
        provider.load_from_string(&format!("{CSS}{}", setup_parts::CSS));
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
        SetupStep::PairingLost { .. } => vec![Input::PairAgain, Input::CloseSetupWindow],
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
        | SetupStep::PairingAgain { .. }
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
    let instructions: Vec<String> = text
        .instructions
        .iter()
        .map(messages::Instruction::line)
        .collect();
    let stages: Vec<String> = SETUP_STAGES
        .iter()
        .zip(text.stages)
        .map(|(name, mark)| stage_line(name, mark))
        .collect();
    let now = SETUP_STAGES
        .iter()
        .zip(text.stages)
        .find_map(|(name, mark)| (mark == StageMark::Now).then_some(*name));
    let mut content = BTreeMap::from([
        ("step".to_owned(), view.step.as_str().to_owned()),
        ("stage".to_owned(), now.unwrap_or_default().to_owned()),
        ("stages".to_owned(), stages.join("\n")),
        ("heading".to_owned(), text.heading.clone()),
        ("keys".to_owned(), footer::text(&text.keys)),
        ("instructions".to_owned(), instructions.join("\n")),
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
    if let Some(label) = text.countdown_label {
        content.insert("countdown-label".to_owned(), label.to_owned());
    }
    if let Some(sync) = &text.sync {
        let rows: Vec<String> = sync.rows.iter().map(messages::SyncRow::line).collect();
        content.insert("progress".to_owned(), rows.join("\n"));
        content.insert("sync-bar".to_owned(), sync.bar.to_string());
        content.insert("sync-note".to_owned(), sync.note.to_owned());
    }
    if let Some(line) = &text.line {
        content.insert("line".to_owned(), line.clone());
    }
    if let Some(card) = &text.keybinding {
        content.insert("keybinding".to_owned(), card.reported());
    }
    if let Some(tip) = &text.tip {
        content.insert("tip".to_owned(), tip.reported());
    }
    if let Some(notice) = &text.notice {
        content.insert("notice".to_owned(), notice.to_owned());
    }
    if let Some(card) = &text.calls_off {
        content.insert("calls-off".to_owned(), card.heading.to_owned());
        content.insert("calls-off-text".to_owned(), card.text.clone());
        content.insert("calls-off-button".to_owned(), card.button.to_owned());
    }
    if view.problem_has_focus.is_some() || view.calls_off.is_some_and(|off| off.blocks_step) {
        content.insert("dimmed".to_owned(), "true".to_owned());
    }
    if let Some(desktop) = &view.desktop_name {
        content.insert("desktop".to_owned(), desktop.clone());
    }
    content
}
