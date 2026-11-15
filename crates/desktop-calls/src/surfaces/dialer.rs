//! The Dialer: search Contacts and the Call History, and Call the selected row.

use super::icons::{Icon, IconView};
use super::{
    Anchor, DIALER_LIST_HEIGHT, DIALER_PAGE, DIALER_WIDTH, SharedReport, Surface, after_next_frame,
    class, has_command_modifier, now, utc_offset_seconds,
};
use crate::logging::EventKind;
use crate::theme::space;
use desktop_calls_core::matching::{DialerRow, DialerRowKind};
use desktop_calls_core::messages::{self, DialerGlyph, Label};
use desktop_calls_core::{Caller, DialerKey, DialerView, Input, PhoneStatus, Timestamp};
use futures_channel::mpsc::UnboundedSender;
use gtk::glib;
use gtk::prelude::*;
use gtk4_layer_shell::{KeyboardMode, Layer};
use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;
use std::rc::Rc;

/// The Dialer.
pub(super) struct Dialer {
    pub(super) surface: Surface,
    entry: gtk::Entry,
    /// The search icon, redrawn unchanged once the Dialer is shown.
    search: gtk::DrawingArea,
    list: gtk::ListBox,
    scrolled: gtk::ScrolledWindow,
    /// The window's child, as tall as a full Dialer, and the card at its
    /// top that shows, as tall as its rows.
    root: gtk::Box,
    card: gtk::Box,
    /// Hidden; coloured with the accent.
    accent: gtk::Label,
    /// The label over each row's section, by row: see
    /// [`messages::dialer_section`].
    sections: Rc<RefCell<Vec<Option<&'static str>>>>,
    /// The notice strip under the search row, its icon, and its words: why
    /// Enter can't Call, or that calls are off and Enter calls anyway.
    notice: gtk::Box,
    notice_icon: IconView,
    notice_text: gtk::Label,
    footer: Footer,
    /// Set while the Dialer writes the query into `entry`, so the write
    /// isn't taken for typing.
    writing: Cell<bool>,
    /// Which opening of the Dialer is shown, while it is.
    opened: Cell<Option<u32>>,
    /// How many times the Dialer has opened, so a redraw left waiting by an
    /// earlier opening knows it is stale.
    openings: Rc<Cell<u32>>,
    /// The query the rows are for, so the frame that first shows the rows
    /// for a new one can be logged.
    query: RefCell<String>,
    /// The rows the list shows, and how many from the top it marks as
    /// Missed Calls the user hasn't seen.
    rows: RefCell<(Vec<DialerRow>, usize)>,
}

impl Dialer {
    pub(super) fn new(intents: &UnboundedSender<Input>, report: SharedReport) -> Rc<Self> {
        let surface = Surface::new(
            "dialer",
            Anchor::Centre,
            Layer::Overlay,
            KeyboardMode::Exclusive,
            Some(Input::CloseDialer),
            intents,
            report,
        );
        surface.window.set_default_size(DIALER_WIDTH, -1);
        surface.window.add_css_class("dialer");
        let entry = gtk::Entry::new();
        entry.set_placeholder_text(Some(messages::label(Label::DialerPlaceholder)));
        entry.set_hexpand(true);
        let search = IconView::new(Icon::Search);
        search.area.add_css_class("search-icon");
        let search_row = gtk::Box::new(gtk::Orientation::Horizontal, space::M);
        search_row.add_css_class(class::SEARCH_ROW);
        search_row.append(&search.area);
        search_row.append(&entry);
        let (notice, notice_icon, notice_text) = notice_strip();
        let list = gtk::ListBox::new();
        list.set_selection_mode(gtk::SelectionMode::Single);
        list.set_focusable(false);
        let no_matches = gtk::Label::new(Some(messages::label(Label::NoMatches)));
        no_matches.add_css_class("dim-label");
        list.set_placeholder(Some(&no_matches));
        let sections = Rc::new(RefCell::new(Vec::new()));
        list.set_header_func(section_headers(sections.clone()));
        let scrolled = gtk::ScrolledWindow::new();
        scrolled.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
        // As tall as its rows, up to the list's height.
        scrolled.set_propagate_natural_height(true);
        scrolled.set_max_content_height(DIALER_LIST_HEIGHT);
        scrolled.set_focusable(false);
        scrolled.set_child(Some(&list));
        let footer = Footer::new();
        // The accent colour, which Pango's underline under matched letters
        // can't take from CSS.
        let accent = gtk::Label::new(None);
        accent.add_css_class("accent-probe");
        accent.set_visible(false);
        let card = gtk::Box::new(gtk::Orientation::Vertical, 0);
        card.add_css_class(CARD);
        card.set_valign(gtk::Align::Start);
        card.append(&search_row);
        card.append(&notice);
        card.append(&scrolled);
        card.append(&footer.line);
        card.append(&accent);
        // The window keeps the height of a full Dialer, so the compositor
        // centres it where it always has and its top edge stays put; the
        // card in it, which is all that shows, follows its rows.
        let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
        root.append(&card);
        surface.window.set_child(Some(&root));
        let dialer = Rc::new(Self {
            surface,
            entry,
            search: search.area,
            list,
            scrolled,
            root,
            card,
            accent,
            sections,
            notice,
            notice_icon,
            notice_text,
            footer,
            writing: Cell::new(false),
            opened: Cell::default(),
            openings: Rc::default(),
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
            if is_alt_q(key, modifiers) {
                let _ = pressed.unbounded_send(Input::OpenQuietHoursWindow);
                return glib::Propagation::Stop;
            }
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
        let shown = self.rows.borrow();
        let changed = shown.0 != view.rows || shown.1 != view.new_missed;
        drop(shown);
        if changed {
            self.show_rows(&view.rows, view.new_missed);
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
        // Why Enter can't Call comes first; otherwise, while calls are off,
        // that Enter calls anyway.
        let calls_off = view
            .calls_off
            .map(|off| messages::dialer_calls_off(off.until));
        if let Some(notice) = notice {
            self.notice_icon.set(Icon::Warning);
            self.notice_text.set_text(notice);
        } else if let Some((off, enter)) = &calls_off {
            self.notice_icon.set(Icon::Moon);
            self.notice_text.set_markup(&format!(
                "<b>{}</b> {}",
                glib::markup_escape_text(off),
                glib::markup_escape_text(enter)
            ));
        }
        self.notice
            .set_visible(notice.is_some() || calls_off.is_some());
        self.fit();

        let mut content = BTreeMap::from([
            ("query".to_owned(), view.query.clone()),
            ("rows".to_owned(), view.rows.len().to_string()),
        ]);
        if view.new_missed > 0 {
            content.insert("new-missed".to_owned(), view.new_missed.to_string());
        }
        if let Some(selected) = view.selected {
            content.insert("selected".to_owned(), selected.to_string());
        }
        if let Some(notice) = notice {
            content.insert("notice".to_owned(), notice.to_owned());
        }
        if let Some((off, enter)) = calls_off {
            content.insert("calls-off".to_owned(), format!("{off} {enter}"));
        }
        let strip_says_off = notice.is_none() && content.contains_key("calls-off");
        self.footer
            .show(&view.keys, &view.phone, strip_says_off, &mut content);
        let (now, offset) = (now(), utc_offset_seconds());
        for (index, row) in view.rows.iter().enumerate() {
            let text = row_text(row, now, offset);
            content.insert(format!("row-{index}"), text.line());
            content.insert(format!("row-{index}-glyph"), text.glyph.as_str().to_owned());
            if !row.matched.is_empty() {
                let matched: Vec<&str> = row
                    .matched
                    .iter()
                    .filter_map(|range| row.name.get(range.clone()))
                    .collect();
                content.insert(format!("row-{index}-matched"), matched.join(","));
            }
        }
        for (index, section) in section_starts(&view.rows) {
            content.insert(format!("section-{index}"), section.to_owned());
        }
        self.surface.show(content);
        self.surface.focus(true);
        self.take_clicks_on_card_only();
        if opening {
            // The cursor at the end, nothing selected: typing adds to the
            // query rather than replacing it.
            self.entry.grab_focus_without_selecting();
            self.entry.set_position(-1);
            self.redraw_after_shown();
        }
    }

    /// Redraws the Dialer, unchanged, on the refresh after the one that
    /// showed its first frame, so that the costly frame comes before the
    /// first keystroke rather than with it. GTK's first frame drawn on a
    /// later refresh than a surface's first took 13 to 15 ms more paint
    /// than any after it, which made the first keystroke pay for it: 22 ms
    /// against 4 ms for later ones, and 48.5 ms under CI's load (ticket
    /// #249). A redraw on the frame GTK runs as soon as the first is shown,
    /// within the same refresh, did not take the cost. Measured on GTK
    /// 4.24.1; why that frame costs more is not known. Redrawing the search
    /// icon is the smallest change GTK draws a frame for. Each opening
    /// cancels a redraw an earlier one left waiting.
    fn redraw_after_shown(&self) {
        let opening = self.openings.get().wrapping_add(1);
        self.openings.set(opening);
        let (openings, window, search) = (
            self.openings.clone(),
            self.surface.window.clone(),
            self.search.clone(),
        );
        let current = move || openings.get() == opening;
        after_next_frame(&self.surface.window, move || {
            if !current() {
                return;
            }
            let shown = Cell::new(false);
            search.add_tick_callback(move |search, _| {
                // The first tick comes as the first frame is shown.
                if !shown.replace(true) {
                    return glib::ControlFlow::Continue;
                }
                if current() && window.is_visible() {
                    search.queue_draw();
                    after_next_frame(&window, || {
                        tracing::debug!(
                            event_kind = %EventKind::DialerRedrawnAfterShown,
                            "Redrew the Dialer after its first frame, ready for the first keystroke"
                        );
                    });
                }
                glib::ControlFlow::Break
            });
        });
    }

    /// Fills the list with `rows`, from the top, the first `new_missed`
    /// with a dot at their right: Missed Calls the user hasn't seen.
    fn show_rows(&self, rows: &[DialerRow], new_missed: usize) {
        self.list.remove_all();
        *self.sections.borrow_mut() = rows
            .iter()
            .map(|row| messages::dialer_section(row.kind))
            .collect();
        let underline = self.accent.color();
        let (now, offset) = (now(), utc_offset_seconds());
        for (index, row) in rows.iter().enumerate() {
            let text = row_text(row, now, offset);
            let shown = row_widget(row, &text, &underline, index < new_missed);
            let list_row = gtk::ListBoxRow::new();
            // The entry keeps the keyboard; the list only shows.
            list_row.set_focusable(false);
            list_row.set_child(Some(&shown));
            self.list.append(&list_row);
        }
        self.scrolled.vadjustment().set_value(0.0);
        *self.rows.borrow_mut() = (rows.to_vec(), new_missed);
    }

    /// Keeps the window as tall as a full Dialer, the list as tall as its
    /// rows up to the list's height, less the notice strip's while it
    /// shows, so the card never outgrows the window.
    fn fit(&self) {
        let natural =
            |widget: &gtk::Widget| widget.measure(gtk::Orientation::Vertical, DIALER_WIDTH).1;
        let notice = if self.notice.is_visible() {
            natural(self.notice.upcast_ref())
        } else {
            0
        };
        self.scrolled
            .set_max_content_height(DIALER_LIST_HEIGHT - notice);
        let around_list =
            natural(self.card.upcast_ref()) - natural(self.scrolled.upcast_ref()) - notice;
        self.root
            .set_size_request(-1, around_list + DIALER_LIST_HEIGHT);
    }

    /// Lets clicks below the card, where nothing shows, through to what is
    /// under the Dialer, once the card has its height.
    fn take_clicks_on_card_only(&self) {
        let (window, card) = (self.surface.window.clone(), self.card.clone());
        after_next_frame(&self.surface.window, move || {
            if let Some(surface) = window.surface() {
                let card = gtk::cairo::RectangleInt::new(0, 0, window.width(), card.height());
                surface.set_input_region(Some(&gtk::cairo::Region::create_rectangle(&card)));
            }
        });
    }

    /// Scrolls the list just enough to show `row`, and the label over its
    /// section if it starts one.
    fn scroll_to(&self, row: &gtk::ListBoxRow) {
        let Some(bounds) = row.compute_bounds(&self.list) else {
            return;
        };
        let header = row
            .header()
            .and_then(|header| header.compute_bounds(&self.list));
        let adjustment = self.scrolled.vadjustment();
        let top = f64::from(header.map_or(bounds.y(), |header| header.y()));
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
    messages::dialer_row(row, now, offset)
}

/// The card that shows: the Dialer's background, border and corners.
const CARD: &str = "dialer-card";

/// Where each section starts among `rows`, and its label.
fn section_starts(rows: &[DialerRow]) -> Vec<(usize, &'static str)> {
    let mut starts = Vec::new();
    let mut last = None;
    for (index, row) in rows.iter().enumerate() {
        let section = messages::dialer_section(row.kind);
        if let Some(label) = section.filter(|_| section != last) {
            starts.push((index, label));
        }
        last = section;
    }
    starts
}

/// Puts the label over each section where it starts, from `sections`, the
/// label for each row.
fn section_headers(
    sections: Rc<RefCell<Vec<Option<&'static str>>>>,
) -> impl Fn(&gtk::ListBoxRow, Option<&gtk::ListBoxRow>) + 'static {
    move |row, before| {
        let section = |row: &gtk::ListBoxRow| {
            usize::try_from(row.index())
                .ok()
                .and_then(|index| sections.borrow().get(index).copied().flatten())
        };
        let starts = section(row).filter(|label| before.and_then(section) != Some(*label));
        let Some(label) = starts else {
            row.set_header(None::<&gtk::Widget>);
            return;
        };
        let header = gtk::Label::new(Some(label));
        header.add_css_class(class::EYEBROW);
        header.add_css_class("section");
        header.set_xalign(0.0);
        row.set_header(Some(&header));
    }
}

/// A row of the list: its glyph, its name over its detail line, when at
/// the right, and the dot of a Missed Call the user hasn't seen if
/// `marked`. The letters of the name that matched what was typed are
/// underlined in `underline`.
fn row_widget(
    row: &DialerRow,
    text: &messages::DialerRowText,
    underline: &gtk::gdk::RGBA,
    marked: bool,
) -> gtk::Box {
    let title = gtk::Label::new(Some(&text.title));
    title.add_css_class("title");
    // A row with no name shows the bare number.
    if row.kind != DialerRowKind::CallNumber
        && row.name.trim().is_empty()
        && matches!(row.caller, Caller::Number(_))
    {
        title.add_css_class(class::NUMBER);
    }
    if text.title == row.name && !row.matched.is_empty() {
        title.set_attributes(Some(&underlined(&row.matched, underline)));
    }
    title.set_xalign(0.0);
    title.set_ellipsize(gtk::pango::EllipsizeMode::End);
    let lines = gtk::Box::new(gtk::Orientation::Vertical, 2);
    lines.set_hexpand(true);
    lines.set_valign(gtk::Align::Center);
    lines.append(&title);
    if text.missed || !text.detail.is_empty() {
        lines.append(&detail_line(text));
    }
    let shown = gtk::Box::new(gtk::Orientation::Horizontal, space::M);
    shown.add_css_class("dialer-row");
    shown.append(&glyph(&text.glyph));
    shown.append(&lines);
    if !text.time.is_empty() {
        let time = gtk::Label::new(Some(&text.time));
        time.add_css_class(class::META);
        time.set_valign(gtk::Align::Center);
        shown.append(&time);
    }
    if marked {
        let dot = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        dot.add_css_class(class::NEW_DOT);
        dot.set_valign(gtk::Align::Center);
        shown.append(&dot);
    }
    shown
}

/// The detail line: "Missed" in the theme's red for a Missed Call, then
/// the rest.
fn detail_line(text: &messages::DialerRowText) -> gtk::Box {
    let line = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    let mut rest = text.detail.clone();
    if text.missed {
        let missed = gtk::Label::new(Some(messages::label(Label::Missed)));
        missed.add_css_class(class::META);
        missed.add_css_class(class::MISSED);
        line.append(&missed);
        if !rest.is_empty() {
            rest = format!(" · {rest}");
        }
    }
    if !rest.is_empty() {
        let detail = gtk::Label::new(Some(&rest));
        detail.add_css_class(class::META);
        detail.set_xalign(0.0);
        detail.set_ellipsize(gtk::pango::EllipsizeMode::End);
        line.append(&detail);
    }
    line
}

/// The glyph at a row's left, 28 across: the direction of a Call History
/// row, red for a Missed Call, or an initial in a tile.
fn glyph(glyph: &DialerGlyph) -> gtk::Widget {
    let arrow = |icon, missed| {
        let view = IconView::new(icon);
        view.area.add_css_class("dialer-glyph");
        if missed {
            view.area.add_css_class(class::MISSED);
        }
        view.area.upcast::<gtk::Widget>()
    };
    match glyph {
        DialerGlyph::Missed => arrow(Icon::Incoming, true),
        DialerGlyph::Incoming => arrow(Icon::Incoming, false),
        DialerGlyph::Outgoing => arrow(Icon::Outgoing, false),
        DialerGlyph::Initial(initial) => {
            let tile = gtk::Label::new(Some(initial));
            tile.add_css_class(class::INITIAL_TILE);
            tile.add_css_class("dialer-glyph");
            tile.set_valign(gtk::Align::Center);
            tile.upcast()
        }
    }
}

/// Pango attributes that underline the bytes `ranges` in `colour`.
fn underlined(ranges: &[std::ops::Range<usize>], colour: &gtk::gdk::RGBA) -> gtk::pango::AttrList {
    let attributes = gtk::pango::AttrList::new();
    // Pango's colours are 16 bits a channel.
    let channel = |value: f32| {
        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "a channel between 0 and 1, clamped, scaled to 16 bits"
        )]
        let scaled = (value.clamp(0.0, 1.0) * 65_535.0).round() as u16;
        scaled
    };
    for range in ranges {
        let (Ok(start), Ok(end)) = (u32::try_from(range.start), u32::try_from(range.end)) else {
            continue;
        };
        let mut line = gtk::pango::AttrInt::new_underline(gtk::pango::Underline::Single);
        line.set_start_index(start);
        line.set_end_index(end);
        attributes.insert(line);
        let mut tint = gtk::pango::AttrColor::new_underline_color(
            channel(colour.red()),
            channel(colour.green()),
            channel(colour.blue()),
        );
        tint.set_start_index(start);
        tint.set_end_index(end);
        attributes.insert(tint);
    }
    attributes
}

/// Alt+Q, which opens the Quiet Hours Window, with no other command
/// modifier and whatever the Caps Lock.
fn is_alt_q(key: gtk::gdk::Key, modifiers: gtk::gdk::ModifierType) -> bool {
    use gtk::gdk::ModifierType;
    let held = modifiers
        & (ModifierType::CONTROL_MASK | ModifierType::ALT_MASK | ModifierType::SUPER_MASK);
    held == ModifierType::ALT_MASK && key.to_lower() == gtk::gdk::Key::q
}

/// The notice strip, hidden: an icon beside the notice's words.
fn notice_strip() -> (gtk::Box, IconView, gtk::Label) {
    let icon = IconView::new(Icon::Warning);
    icon.area.add_css_class("notice-icon");
    icon.area.set_valign(gtk::Align::Start);
    let text = gtk::Label::new(None);
    text.set_xalign(0.0);
    text.set_wrap(true);
    text.set_hexpand(true);
    let strip = gtk::Box::new(gtk::Orientation::Horizontal, space::M);
    strip.add_css_class(class::NOTICE);
    strip.add_css_class(class::STRIP);
    strip.append(&icon.area);
    strip.append(&text);
    strip.set_visible(false);
    (strip, icon, text)
}

/// The footer line: each key that works now with what it does, and at the
/// right where the Phone is.
struct Footer {
    line: gtk::Box,
    /// The keys, rebuilt only when they change.
    keys: gtk::Box,
    shown_keys: RefCell<Vec<DialerKey>>,
    live_dot: gtk::Box,
    phone: gtk::Label,
}

impl Footer {
    fn new() -> Self {
        let keys = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        keys.add_css_class("keys");
        keys.set_hexpand(true);
        let live_dot = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        live_dot.add_css_class(class::LIVE_DOT);
        live_dot.set_valign(gtk::Align::Center);
        let phone = gtk::Label::new(None);
        phone.set_ellipsize(gtk::pango::EllipsizeMode::End);
        phone.set_max_width_chars(36);
        let where_phone_is = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        where_phone_is.append(&live_dot);
        where_phone_is.append(&phone);
        let line = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        line.add_css_class(class::FOOTER_LINE);
        line.append(&keys);
        line.append(&where_phone_is);
        Self {
            line,
            keys,
            shown_keys: RefCell::default(),
            live_dot,
            phone,
        }
    }

    /// Shows `keys` and where the Phone is, and adds what it shows to
    /// `content`: `keys`, `phone`, and `phone-live` while the live dot
    /// shows. `strip_says_off` while the strip says calls are off.
    fn show(
        &self,
        keys: &[DialerKey],
        phone: &PhoneStatus,
        strip_says_off: bool,
        content: &mut BTreeMap<String, String>,
    ) {
        let words: Vec<String> = keys
            .iter()
            .map(|key| {
                let (caps, does) = messages::dialer_key(*key);
                format!("{} {does}", caps.join(" "))
            })
            .collect();
        content.insert("keys".to_owned(), words.join(" · "));
        if *self.shown_keys.borrow() != keys {
            while let Some(child) = self.keys.first_child() {
                self.keys.remove(&child);
            }
            for key in keys {
                let (caps, does) = messages::dialer_key(*key);
                let group = gtk::Box::new(gtk::Orientation::Horizontal, 0);
                let keycaps = gtk::Box::new(gtk::Orientation::Horizontal, 0);
                keycaps.add_css_class("keycaps");
                for cap in caps {
                    let keycap = gtk::Label::new(Some(cap));
                    keycap.add_css_class(class::KEYCAP);
                    keycap.set_valign(gtk::Align::Center);
                    keycaps.append(&keycap);
                }
                group.append(&keycaps);
                group.append(&gtk::Label::new(Some(does)));
                self.keys.append(&group);
            }
            *self.shown_keys.borrow_mut() = keys.to_vec();
        }
        let live = matches!(phone, PhoneStatus::Connected(_));
        let words = messages::dialer_phone(phone, strip_says_off);
        self.phone.set_text(&words);
        self.live_dot.set_visible(live);
        content.insert("phone".to_owned(), words);
        if live {
            content.insert("phone-live".to_owned(), "true".to_owned());
        }
    }
}
