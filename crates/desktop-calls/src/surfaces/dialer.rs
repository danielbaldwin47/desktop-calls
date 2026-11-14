//! The Dialer: search Contacts and the Call History, and Call the selected row.

use super::call_window::notice_label;
use super::{
    DIALER_LIST_HEIGHT, DIALER_PAGE, DIALER_WIDTH, SharedReport, Surface, after_next_frame,
    has_command_modifier, now, utc_offset_seconds,
};
use crate::logging::EventKind;
use desktop_calls_core::matching::{DialerRow, DialerRowKind};
use desktop_calls_core::messages::{self, Label};
use desktop_calls_core::{CallHistoryKind, DialerView, Input, Timestamp};
use futures_channel::mpsc::UnboundedSender;
use gtk::glib;
use gtk::prelude::*;
use gtk4_layer_shell::{Edge, KeyboardMode, Layer, LayerShell as _};
use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;
use std::rc::Rc;

/// The Dialer.
pub(super) struct Dialer {
    pub(super) surface: Surface,
    entry: gtk::Entry,
    list: gtk::ListBox,
    scrolled: gtk::ScrolledWindow,
    notice: gtk::Label,
    /// Set while the Dialer writes the query into `entry`, so the write
    /// isn't taken for typing.
    writing: Cell<bool>,
    /// Which opening of the Dialer is shown, while it is.
    opened: Cell<Option<u32>>,
    /// The query the rows are for, so the frame that first shows the rows
    /// for a new one can be logged.
    query: RefCell<String>,
    /// The rows the list shows.
    rows: RefCell<Vec<DialerRow>>,
}

impl Dialer {
    pub(super) fn new(intents: &UnboundedSender<Input>, report: SharedReport) -> Rc<Self> {
        let surface = Surface::new(
            "dialer",
            Layer::Overlay,
            KeyboardMode::Exclusive,
            Some(Input::CloseDialer),
            intents,
            report,
        );
        surface.window.set_default_size(DIALER_WIDTH, -1);
        if gtk4_layer_shell::is_supported() {
            // Anchored to no edge, the compositor centres it in the space
            // other surfaces' exclusive zones (the bar's) leave.
            for edge in [Edge::Top, Edge::Right] {
                surface.window.set_anchor(edge, false);
                surface.window.set_margin(edge, 0);
            }
        }
        let entry = gtk::Entry::new();
        entry.add_css_class("dialer-entry");
        entry.set_placeholder_text(Some(messages::label(Label::DialerPlaceholder)));
        entry.set_hexpand(true);
        let notice = notice_label();
        notice.set_max_width_chars(60);
        let list = gtk::ListBox::new();
        list.set_selection_mode(gtk::SelectionMode::Single);
        list.set_focusable(false);
        let no_matches = gtk::Label::new(Some(messages::label(Label::NoMatches)));
        no_matches.add_css_class("dim-label");
        list.set_placeholder(Some(&no_matches));
        let scrolled = gtk::ScrolledWindow::new();
        scrolled.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
        scrolled.set_size_request(-1, DIALER_LIST_HEIGHT);
        scrolled.set_focusable(false);
        scrolled.set_child(Some(&list));
        let content = gtk::Box::new(gtk::Orientation::Vertical, 8);
        content.append(&entry);
        content.append(&notice);
        content.append(&scrolled);
        surface.window.set_child(Some(&content));
        let dialer = Rc::new(Self {
            surface,
            entry,
            list,
            scrolled,
            notice,
            writing: Cell::new(false),
            opened: Cell::default(),
            query: RefCell::default(),
            rows: RefCell::default(),
        });
        dialer.connect(intents);
        dialer
    }

    /// Typing searches, Up and Down move the selection, and Enter or a
    /// click Calls the selected row.
    fn connect(self: &Rc<Self>, intents: &UnboundedSender<Input>) {
        let weak = Rc::downgrade(self);
        let typed = intents.clone();
        self.entry.connect_changed(move |entry| {
            if weak.upgrade().is_some_and(|dialer| !dialer.writing.get()) {
                let query = entry.text().to_string();
                tracing::debug!(
                    event_kind = %EventKind::DialerQueryTyped,
                    query_chars = query.chars().count(),
                    "Typed into the Dialer"
                );
                let _ = typed.unbounded_send(Input::DialerQuery(query));
            }
        });
        let chosen = intents.clone();
        self.list.connect_row_activated(move |_, row| {
            if let Ok(index) = usize::try_from(row.index()) {
                let _ = chosen.unbounded_send(Input::DialerChoose(index));
            }
        });
        let keys = gtk::EventControllerKey::new();
        // Before the entry sees them, so Up and Down move the selection
        // and Enter Calls whichever widget has the keyboard.
        keys.set_propagation_phase(gtk::PropagationPhase::Capture);
        let pressed = intents.clone();
        keys.connect_key_pressed(move |_, key, _, modifiers| {
            if has_command_modifier(modifiers) {
                return glib::Propagation::Proceed;
            }
            let intent = match key {
                gtk::gdk::Key::Up | gtk::gdk::Key::KP_Up => Input::DialerMove(-1),
                gtk::gdk::Key::Down | gtk::gdk::Key::KP_Down => Input::DialerMove(1),
                gtk::gdk::Key::Page_Up | gtk::gdk::Key::KP_Page_Up => {
                    Input::DialerMove(-DIALER_PAGE)
                }
                gtk::gdk::Key::Page_Down | gtk::gdk::Key::KP_Page_Down => {
                    Input::DialerMove(DIALER_PAGE)
                }
                gtk::gdk::Key::Return | gtk::gdk::Key::KP_Enter | gtk::gdk::Key::ISO_Enter => {
                    Input::DialerEnter
                }
                _ => return glib::Propagation::Proceed,
            };
            let _ = pressed.unbounded_send(intent);
            glib::Propagation::Stop
        });
        self.surface.window.add_controller(keys);
    }

    pub(super) fn render(&self, view: Option<&DialerView>) {
        let Some(view) = view else {
            self.opened.set(None);
            self.surface.hide();
            return;
        };
        let opening = self.opened.replace(Some(view.opened)) != Some(view.opened);
        let followed = self.query.replace(view.query.clone()) != view.query;
        if followed && !opening {
            let query_chars = view.query.chars().count();
            after_next_frame(&self.surface.window, move || {
                tracing::debug!(
                    event_kind = %EventKind::DialerQueryShown,
                    query_chars,
                    "Drew the Dialer's rows for what was typed"
                );
            });
        }
        if opening {
            // Afresh: what the Dialer opened with. After that the entry is
            // the user's, and only follows them.
            self.writing.set(true);
            self.entry.set_text(&view.query);
            self.entry.set_position(-1);
            self.writing.set(false);
        }
        if *self.rows.borrow() != view.rows {
            self.show_rows(&view.rows);
        }
        if let Some(row) = view
            .selected
            .and_then(|selected| i32::try_from(selected).ok())
            .and_then(|selected| self.list.row_at_index(selected))
        {
            self.list.select_row(Some(&row));
            self.scroll_to(&row);
        }
        let notice = view.notice.map(|failure| messages::failure(failure).text);
        self.notice.set_text(notice.unwrap_or_default());
        self.notice.set_visible(notice.is_some());

        let mut content = BTreeMap::from([
            ("query".to_owned(), view.query.clone()),
            ("rows".to_owned(), view.rows.len().to_string()),
        ]);
        if let Some(selected) = view.selected {
            content.insert("selected".to_owned(), selected.to_string());
        }
        if let Some(notice) = notice {
            content.insert("notice".to_owned(), notice.to_owned());
        }
        let (now, offset) = (now(), utc_offset_seconds());
        for (index, row) in view.rows.iter().enumerate() {
            let text = row_text(row, now, offset);
            content.insert(format!("row-{index}"), text.line());
        }
        self.surface.show(content);
        self.surface.focus(true);
        if opening {
            // The cursor at the end, nothing selected: typing adds to the
            // query rather than replacing it.
            self.entry.grab_focus_without_selecting();
            self.entry.set_position(-1);
        }
    }

    /// Fills the list with `rows`, from the top.
    fn show_rows(&self, rows: &[DialerRow]) {
        self.list.remove_all();
        let (now, offset) = (now(), utc_offset_seconds());
        for row in rows {
            let text = row_text(row, now, offset);
            let title = gtk::Label::new(Some(&text.title));
            title.add_css_class("title");
            title.set_xalign(0.0);
            title.set_ellipsize(gtk::pango::EllipsizeMode::End);
            let lines = gtk::Box::new(gtk::Orientation::Vertical, 2);
            lines.add_css_class("dialer-row");
            lines.append(&title);
            if !text.detail.is_empty() {
                let detail = gtk::Label::new(Some(&text.detail));
                detail.set_xalign(0.0);
                detail.set_ellipsize(gtk::pango::EllipsizeMode::End);
                // Missed Calls stand out in the theme's red.
                detail.add_css_class(
                    if row.kind == DialerRowKind::CallHistory(CallHistoryKind::Missed) {
                        "error"
                    } else {
                        "dim-label"
                    },
                );
                lines.append(&detail);
            }
            let list_row = gtk::ListBoxRow::new();
            // The entry keeps the keyboard; the list only shows.
            list_row.set_focusable(false);
            list_row.set_child(Some(&lines));
            self.list.append(&list_row);
        }
        self.scrolled.vadjustment().set_value(0.0);
        *self.rows.borrow_mut() = rows.to_vec();
    }

    /// Scrolls the list just enough to show `row`.
    fn scroll_to(&self, row: &gtk::ListBoxRow) {
        let Some(bounds) = row.compute_bounds(&self.list) else {
            return;
        };
        let adjustment = self.scrolled.vadjustment();
        let top = f64::from(bounds.y());
        let bottom = f64::from(bounds.y() + bounds.height());
        if top < adjustment.value() {
            adjustment.set_value(top);
        } else if bottom > adjustment.value() + adjustment.page_size() {
            adjustment.set_value(bottom - adjustment.page_size());
        }
    }
}

/// How `row` reads, with when a Call History row's Call was.
fn row_text(row: &DialerRow, now: Timestamp, offset: i64) -> messages::DialerRowText {
    let when = row.at.map(|at| messages::when(at, now, offset));
    messages::dialer_row(row, when.as_deref())
}
