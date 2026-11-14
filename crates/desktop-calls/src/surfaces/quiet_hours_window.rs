//! The Quiet Hours Window: a switch and two times, saved as they change.
//!
//! Like the Setup Window it is an ordinary window, not a layer-shell
//! surface: it tiles, takes focus when it opens and can be moved aside. The
//! switch has keyboard focus when it opens. Tab and Shift+Tab go round the
//! switch and the two times; Space toggles the switch; in a time, Up and
//! Down step it, digits type one, and leaving it or Enter reads what was
//! typed. Escape or closing the window closes it, reading a time being
//! typed first.
//!
//! It reports `quiet-hours-window` with `switch` (`on` or `off`), `start`,
//! `end`, `notice`, `sentence`, `bar` (the spans the 24-hour bar fills, as
//! `HH:MM-HH:MM`, space-separated), `keys` (one per line), `focus`
//! (`switch`, `start` or `end`) and `focused`.

use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;
use std::rc::Rc;

use desktop_calls_core::messages::{self, QuietHoursWindowText};
use desktop_calls_core::{Input, QuietHoursField, QuietHoursWindowView};
use futures_channel::mpsc::UnboundedSender;
use gtk::glib;
use gtk::prelude::*;

use super::{
    SharedReport, after_next_frame, class, is_escape, reported, set_class, surface_mapped,
    surface_shown,
};
use crate::theme::space;

const NAME: &str = "quiet-hours-window";
const WIDTH: i32 = 340;
/// The 24-hour bar's height, and its hour marks.
const BAR_HEIGHT: i32 = 10;
const HOUR_MARKS: [&str; 4] = ["00", "06", "12", "18"];
const DAY_MINUTES: f64 = 24.0 * 60.0;
/// How strongly the bar's spans are filled with the accent.
const SPAN_ALPHA: f32 = 0.75;

const CSS: &str = "
window.desktop-calls-quiet-hours > box { padding: 20px; }
.desktop-calls-quiet-hours .quiet-time {
    font-family: monospace; font-feature-settings: \"tnum\";
    font-size: 26px; font-weight: 500; padding: 6px 12px; min-height: 0;
    background: var(--dc-raised); color: var(--dc-text); box-shadow: none;
}
.desktop-calls-quiet-hours .quiet-time:focus {
    box-shadow: inset 0 0 0 2px var(--dc-accent); outline: none;
}
.desktop-calls-quiet-hours .quiet-times.dim { opacity: 0.45; }
.desktop-calls-quiet-hours switch:focus { outline: 2px solid var(--dc-accent); outline-offset: 2px; }
.desktop-calls-quiet-hours .quiet-bar { background: var(--dc-raised); color: var(--dc-accent); }
.desktop-calls-quiet-hours .quiet-hour {
    font-family: monospace; font-size: 9.5px; color: var(--dc-secondary);
}
.desktop-calls-quiet-hours .quiet-sentence { font-size: 12.5px; color: var(--dc-secondary); }
.desktop-calls-quiet-hours .quiet-saved { color: var(--dc-answer); }
";

/// The spans of the day the 24-hour bar fills, in minutes after midnight.
type Spans = Rc<RefCell<Vec<(u16, u16)>>>;

/// One of the two times: its field, and what typing in it means.
struct TimeField {
    field: QuietHoursField,
    button: gtk::Button,
    label: gtk::Label,
}

/// The Quiet Hours Window.
pub(super) struct QuietHoursWindow {
    window: gtk::Window,
    heading: gtk::Label,
    switch: gtk::Switch,
    times: gtk::Box,
    from: gtk::Label,
    until: gtk::Label,
    start: TimeField,
    end: TimeField,
    bar: gtk::Box,
    spans: Spans,
    bar_area: gtk::DrawingArea,
    saved_icon: gtk::Image,
    sentence: gtk::Label,
    notice: gtk::Box,
    notice_text: gtk::Label,
    footer: gtk::Box,
    intents: UnboundedSender<Input>,
    report: SharedReport,
    content: Rc<RefCell<BTreeMap<String, String>>>,
    mapped: Rc<Cell<bool>>,
    /// While the window sets the switch itself, so it doesn't send that
    /// back.
    rendering: Rc<Cell<bool>>,
    /// The opening shown.
    opened: Cell<Option<u32>>,
    /// What is shown, so it is drawn again only when it changes.
    shown: RefCell<Option<QuietHoursWindowView>>,
    /// The keys shown in the footer.
    shown_keys: RefCell<Vec<(&'static [&'static str], &'static str)>>,
}

impl QuietHoursWindow {
    pub(super) fn new(intents: &UnboundedSender<Input>, report: SharedReport) -> Rc<Self> {
        add_css();
        let window = gtk::Window::new();
        window.set_default_size(WIDTH, -1);
        window.set_resizable(false);
        window.add_css_class("desktop-calls-quiet-hours");

        let heading = gtk::Label::new(None);
        heading.add_css_class(class::HEADING);
        heading.set_xalign(0.0);
        heading.set_hexpand(true);
        let switch = gtk::Switch::new();
        switch.set_valign(gtk::Align::Center);
        let top = gtk::Box::new(gtk::Orientation::Horizontal, space::M);
        top.append(&heading);
        top.append(&switch);

        let (from, start) = time_field(QuietHoursField::Start);
        let (until, end) = time_field(QuietHoursField::End);
        let times = gtk::Box::new(gtk::Orientation::Horizontal, space::M);
        times.add_css_class("quiet-times");
        for (eyebrow, field) in [(&from, &start), (&until, &end)] {
            let column = gtk::Box::new(gtk::Orientation::Vertical, 2);
            column.append(eyebrow);
            column.append(&field.button);
            times.append(&column);
        }

        let (bar, bar_area, spans) = day_bar();

        let saved_icon = gtk::Image::from_icon_name("object-select-symbolic");
        saved_icon.add_css_class("quiet-saved");
        saved_icon.set_valign(gtk::Align::Start);
        let sentence = gtk::Label::new(None);
        sentence.add_css_class("quiet-sentence");
        sentence.set_xalign(0.0);
        sentence.set_wrap(true);
        sentence.set_max_width_chars(40);
        let said = gtk::Box::new(gtk::Orientation::Horizontal, space::S);
        said.append(&saved_icon);
        said.append(&sentence);

        let notice_icon = gtk::Image::from_icon_name("dialog-warning-symbolic");
        notice_icon.add_css_class("notice-icon");
        let notice_text = gtk::Label::new(None);
        notice_text.set_xalign(0.0);
        notice_text.set_wrap(true);
        let notice = gtk::Box::new(gtk::Orientation::Horizontal, space::S);
        notice.add_css_class(class::NOTICE);
        notice.append(&notice_icon);
        notice.append(&notice_text);
        notice.set_visible(false);

        let footer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        footer.add_css_class(class::FOOTER_LINE);

        let layout = gtk::Box::new(gtk::Orientation::Vertical, space::L);
        for widget in [
            top.upcast_ref::<gtk::Widget>(),
            times.upcast_ref(),
            bar.upcast_ref(),
            notice.upcast_ref(),
            said.upcast_ref(),
            footer.upcast_ref(),
        ] {
            layout.append(widget);
        }
        window.set_child(Some(&layout));

        let quiet = Rc::new(Self {
            window,
            heading,
            switch,
            times,
            from,
            until,
            start,
            end,
            bar,
            spans,
            bar_area,
            saved_icon,
            sentence,
            notice,
            notice_text,
            footer,
            intents: intents.clone(),
            report,
            content: Rc::default(),
            mapped: Rc::default(),
            rendering: Rc::default(),
            opened: Cell::default(),
            shown: RefCell::default(),
            shown_keys: RefCell::default(),
        });
        quiet.connect();
        // Pre-warm, as for the other surfaces.
        WidgetExt::realize(&quiet.window);
        quiet
    }

    fn connect(self: &Rc<Self>) {
        let (intents, rendering) = (self.intents.clone(), self.rendering.clone());
        self.switch.connect_active_notify(move |switch| {
            if !rendering.get() {
                let _ = intents.unbounded_send(Input::SwitchQuietHours(switch.is_active()));
            }
        });
        for time in [&self.start, &self.end] {
            self.connect_time(time);
        }
        let weak = Rc::downgrade(self);
        self.window.connect_close_request(move |_| {
            if let Some(quiet) = weak.upgrade() {
                quiet.close();
            }
            // The Call state machine closes it.
            glib::Propagation::Stop
        });
        let keys = gtk::EventControllerKey::new();
        keys.set_propagation_phase(gtk::PropagationPhase::Capture);
        let weak = Rc::downgrade(self);
        keys.connect_key_pressed(move |_, key, _, modifiers| {
            let Some(quiet) = weak.upgrade() else {
                return glib::Propagation::Proceed;
            };
            if is_escape(key, modifiers) {
                quiet.close();
                return glib::Propagation::Stop;
            }
            match key {
                gtk::gdk::Key::Tab if modifiers.is_empty() => quiet.go_round(true),
                gtk::gdk::Key::ISO_Left_Tab => quiet.go_round(false),
                gtk::gdk::Key::Tab if modifiers == gtk::gdk::ModifierType::SHIFT_MASK => {
                    quiet.go_round(false)
                }
                _ => glib::Propagation::Proceed,
            }
        });
        self.window.add_controller(keys);
        let weak = Rc::downgrade(self);
        self.window.connect_focus_widget_notify(move |_| {
            if let Some(quiet) = weak.upgrade() {
                quiet.publish();
            }
        });

        let weak = Rc::downgrade(self);
        self.window.connect_map(move |window| {
            let Some(quiet) = weak.upgrade() else {
                return;
            };
            quiet.mapped.set(true);
            quiet.publish();
            surface_mapped(NAME);
            after_next_frame(window, || surface_shown(NAME));
        });
        let weak = Rc::downgrade(self);
        self.window.connect_is_active_notify(move |_| {
            if let Some(quiet) = weak.upgrade() {
                quiet.publish();
            }
        });
        let (mapped, report) = (self.mapped.clone(), self.report.clone());
        self.window.connect_unmap(move |_| {
            mapped.set(false);
            report.borrow_mut().set(NAME, None);
        });
    }

    /// In a time: digits, Backspace, Up and Down, and Enter; leaving it
    /// reads what was typed.
    fn connect_time(&self, time: &TimeField) {
        let field = time.field;
        let keys = gtk::EventControllerKey::new();
        let intents = self.intents.clone();
        keys.connect_key_pressed(move |_, key, _, modifiers| {
            use gtk::gdk::Key;
            let input = match key {
                Key::Up | Key::KP_Up => Input::QuietHoursStep { field, later: true },
                Key::Down | Key::KP_Down => Input::QuietHoursStep {
                    field,
                    later: false,
                },
                Key::BackSpace => Input::QuietHoursErase(field),
                Key::Return | Key::KP_Enter => Input::QuietHoursFieldLeft(field),
                _ => match key.to_unicode().and_then(|typed| typed.to_digit(10)) {
                    Some(digit) if !super::has_command_modifier(modifiers) => {
                        Input::QuietHoursDigit {
                            field,
                            digit: u8::try_from(digit).unwrap_or(u8::MAX),
                        }
                    }
                    _ => return glib::Propagation::Proceed,
                },
            };
            let _ = intents.unbounded_send(input);
            glib::Propagation::Stop
        });
        time.button.add_controller(keys);
        let focus = gtk::EventControllerFocus::new();
        let intents = self.intents.clone();
        focus.connect_leave(move |_| {
            let _ = intents.unbounded_send(Input::QuietHoursFieldLeft(field));
        });
        time.button.add_controller(focus);
    }

    /// Reads a time being typed, then closes the window.
    fn close(&self) {
        if let Some(field) = self.focused_time() {
            self.send(Input::QuietHoursFieldLeft(field));
        }
        self.send(Input::CloseQuietHoursWindow);
    }

    /// Moves keyboard focus to the next control that takes it, or the one
    /// before, round from the last to the first.
    fn go_round(&self, forwards: bool) -> glib::Propagation {
        let mut controls: Vec<gtk::Widget> = [
            self.switch.upcast_ref::<gtk::Widget>(),
            self.start.button.upcast_ref(),
            self.end.button.upcast_ref(),
        ]
        .into_iter()
        .filter(|control| control.is_sensitive())
        .cloned()
        .collect();
        if !forwards {
            controls.reverse();
        }
        let focus = GtkWindowExt::focus(&self.window);
        let next = controls
            .iter()
            .position(|control| focus.as_ref() == Some(control))
            .map_or(0, |at| (at + 1) % controls.len());
        if let Some(control) = controls.get(next) {
            control.grab_focus();
        }
        glib::Propagation::Stop
    }

    fn focused_time(&self) -> Option<QuietHoursField> {
        let focus = GtkWindowExt::focus(&self.window)?;
        [&self.start, &self.end]
            .into_iter()
            .find(|time| focus == *time.button.upcast_ref::<gtk::Widget>())
            .map(|time| time.field)
    }

    fn send(&self, input: Input) {
        let _ = self.intents.unbounded_send(input);
    }

    pub(super) fn render(&self, view: Option<&QuietHoursWindowView>) {
        let Some(view) = view else {
            self.opened.set(None);
            self.shown.replace(None);
            self.window.set_visible(false);
            return;
        };
        let opening = self.opened.replace(Some(view.opened)) != Some(view.opened);
        // The switch the user flipped shows what was saved, even when that
        // changed nothing.
        self.show_switch(view.on);
        if !opening && self.shown.borrow().as_ref() == Some(view) {
            return;
        }
        self.shown.replace(Some(view.clone()));
        let text = messages::quiet_hours_window(view);
        self.show(view, &text);
        *self.content.borrow_mut() = report(view, &text);
        if opening || !self.window.is_visible() {
            GtkWindowExt::set_focus(&self.window, Some(&self.switch));
            self.window.present();
        }
        self.publish();
    }

    fn show(&self, view: &QuietHoursWindowView, text: &QuietHoursWindowText) {
        self.window.set_title(Some(text.title));
        self.heading.set_text(text.heading);
        self.switch
            .update_property(&[gtk::accessible::Property::Label(text.switch)]);
        self.from.set_text(text.from);
        self.until.set_text(text.until);
        for (time, shown) in [(&self.start, &text.start), (&self.end, &text.end)] {
            time.label.set_text(shown);
            time.button.set_sensitive(view.on);
        }
        set_class(&self.times, "dim", !view.on);
        self.bar.set_visible(view.on);
        self.spans.replace(view.off_spans.clone());
        self.bar_area.queue_draw();
        self.notice_text.set_text(text.notice.unwrap_or_default());
        self.notice.set_visible(text.notice.is_some());
        self.saved_icon.set_visible(view.on);
        self.sentence.set_text(&text.sentence);
        if *self.shown_keys.borrow() != text.keys {
            self.show_keys(&text.keys);
            self.shown_keys.replace(text.keys.clone());
        }
    }

    fn show_switch(&self, on: bool) {
        if self.switch.is_active() != on || self.switch.state() != on {
            self.rendering.set(true);
            self.switch.set_active(on);
            self.switch.set_state(on);
            self.rendering.set(false);
        }
    }

    fn show_keys(&self, keys: &[(&'static [&'static str], &'static str)]) {
        while let Some(child) = self.footer.first_child() {
            self.footer.remove(&child);
        }
        for (caps, does) in keys {
            let item = gtk::Box::new(gtk::Orientation::Horizontal, 0);
            for cap in *caps {
                let label = gtk::Label::new(Some(cap));
                label.add_css_class(class::KEYCAP);
                item.append(&label);
            }
            item.append(&gtk::Label::new(Some(does)));
            self.footer.append(&item);
        }
    }

    /// Reports what is shown, with which control has keyboard focus.
    fn publish(&self) {
        if !self.mapped.get() {
            return;
        }
        let mut content = self.content.borrow().clone();
        let focus = GtkWindowExt::focus(&self.window);
        let focus = if focus.as_ref() == Some(self.switch.upcast_ref()) {
            Some("switch")
        } else {
            self.focused_time().map(|field| match field {
                QuietHoursField::Start => "start",
                QuietHoursField::End => "end",
            })
        };
        if let Some(focus) = focus {
            content.insert("focus".to_owned(), focus.to_owned());
        }
        self.report
            .borrow_mut()
            .set(NAME, Some(reported(&content, &self.window)));
    }
}

/// The 24-hour bar, with its hours marked: the whole of it, where it draws
/// the spans, and the spans it draws.
fn day_bar() -> (gtk::Box, gtk::DrawingArea, Spans) {
    let spans: Spans = Rc::default();
    let bar_area = gtk::DrawingArea::new();
    bar_area.add_css_class("quiet-bar");
    bar_area.set_content_height(BAR_HEIGHT);
    bar_area.set_hexpand(true);
    let drawn = spans.clone();
    bar_area.set_draw_func(move |area, cairo, width, height| {
        let colour = area.color();
        cairo.set_source_rgba(
            f64::from(colour.red()),
            f64::from(colour.green()),
            f64::from(colour.blue()),
            f64::from(colour.alpha() * SPAN_ALPHA),
        );
        for (from, to) in drawn.borrow().iter() {
            let left = f64::from(*from) / DAY_MINUTES * f64::from(width);
            let right = f64::from(*to) / DAY_MINUTES * f64::from(width);
            cairo.rectangle(left, 0.0, right - left, f64::from(height));
        }
        let _ = cairo.fill();
    });
    let marks = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    marks.set_homogeneous(true);
    for mark in HOUR_MARKS {
        let label = gtk::Label::new(Some(mark));
        label.add_css_class("quiet-hour");
        label.set_xalign(0.0);
        marks.append(&label);
    }
    let bar = gtk::Box::new(gtk::Orientation::Vertical, 3);
    bar.append(&bar_area);
    bar.append(&marks);
    (bar, bar_area, spans)
}

/// A time's eyebrow and field.
fn time_field(field: QuietHoursField) -> (gtk::Label, TimeField) {
    let eyebrow = gtk::Label::new(None);
    eyebrow.add_css_class(class::EYEBROW);
    eyebrow.set_xalign(0.0);
    let label = gtk::Label::new(None);
    let button = gtk::Button::new();
    button.set_child(Some(&label));
    button.add_css_class("quiet-time");
    button.set_halign(gtk::Align::Start);
    (
        eyebrow,
        TimeField {
            field,
            button,
            label,
        },
    )
}

fn report(view: &QuietHoursWindowView, text: &QuietHoursWindowText) -> BTreeMap<String, String> {
    let bar: Vec<String> = view
        .off_spans
        .iter()
        .map(|(from, to)| format!("{}-{}", clock(*from), clock(*to)))
        .collect();
    let keys: Vec<String> = text
        .keys
        .iter()
        .map(|(caps, does)| format!("{} {does}", caps.join(" ")))
        .collect();
    let mut content = BTreeMap::from([
        (
            "switch".to_owned(),
            if view.on { "on" } else { "off" }.to_owned(),
        ),
        ("start".to_owned(), text.start.clone()),
        ("end".to_owned(), text.end.clone()),
        ("sentence".to_owned(), text.sentence.clone()),
        ("keys".to_owned(), keys.join("\n")),
    ]);
    if !bar.is_empty() {
        content.insert("bar".to_owned(), bar.join(" "));
    }
    if let Some(notice) = text.notice {
        content.insert("notice".to_owned(), notice.to_owned());
    }
    content
}

/// Minutes after midnight as `HH:MM`, midnight at the end of the day as
/// 24:00.
fn clock(minutes: u16) -> String {
    format!("{:02}:{:02}", minutes / 60, minutes % 60)
}

/// Adds the Quiet Hours Window's styles to the display.
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
