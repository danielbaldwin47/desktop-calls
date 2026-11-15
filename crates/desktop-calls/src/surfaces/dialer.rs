//! The Dialer: search Contacts and the Call History, and Call the selected row.

use super::icons::{Icon, IconView};
use super::{
    Anchor, DIALER_LIST_HEIGHT, DIALER_PAGE, DIALER_WIDTH, SharedReport, Surface, after_next_frame,
    class, has_command_modifier, now, utc_offset_seconds,
};
use crate::logging::EventKind;
use crate::theme::space;
use desktop_calls_core::matching::DialerRow;
use desktop_calls_core::messages::{self, Label};
use desktop_calls_core::{DialerKey, DialerView, Input, PhoneStatus};
use futures_channel::mpsc::UnboundedSender;
use gtk::glib;
use gtk::prelude::*;
use gtk4_layer_shell::{KeyboardMode, Layer};
use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;
use std::rc::Rc;

mod rows;

use rows::RowView;

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
    /// Every row the list has, those past the rows shown hidden: kept
    /// between keystrokes and filled afresh (#350).
    row_views: RefCell<Vec<RowView>>,
    /// What the card was last fitted to the window with.
    fitted: RefCell<Option<Fitted>>,
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
            row_views: RefCell::default(),
            fitted: RefCell::default(),
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
        let (now, offset) = (now(), utc_offset_seconds());
        let texts: Vec<_> = view
            .rows
            .iter()
            .map(|row| messages::dialer_row(row, now, offset))
            .collect();
        // Afresh on opening too, so the time column follows the clock.
        if changed || opening {
            self.show_rows(&view.rows, &texts, view.new_missed);
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
        let strip_shows = notice.is_some() || calls_off.is_some();
        self.notice.set_visible(strip_shows);
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
        self.fit_if_changed(opening, strip_shows, &view.keys);
        report_rows(&view.rows, &texts, &mut content);
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

    /// Fills the list with `rows`, from the top, reading as `texts`, the
    /// first `new_missed` with a dot at their right: Missed Calls the user
    /// hasn't seen. Nothing is selected until the view says.
    fn show_rows(&self, rows: &[DialerRow], texts: &[messages::DialerRowText], new_missed: usize) {
        self.list.unselect_all();
        *self.sections.borrow_mut() = rows
            .iter()
            .map(|row| messages::dialer_section(row.kind))
            .collect();
        let underline = self.accent.color();
        let mut views = self.row_views.borrow_mut();
        while views.len() < rows.len() {
            let view = RowView::new();
            self.list.append(&view.list_row);
            views.push(view);
        }
        for (index, view) in views.iter().enumerate() {
            match rows.get(index).zip(texts.get(index)) {
                Some((row, text)) => view.show(row, text, &underline, index < new_missed),
                None => view.hide(),
            }
        }
        drop(views);
        self.list.invalidate_headers();
        self.scrolled.vadjustment().set_value(0.0);
        *self.rows.borrow_mut() = (rows.to_vec(), new_missed);
    }

    /// Fits the card to the window, as [`Self::fit`], on `opening` or when
    /// the notice strip or the footer's keys have changed since it last
    /// did: the card's height around its list changes only with them.
    /// Measuring the card lays out every row, which cost 4.5 ms a keystroke
    /// with 5,000 Contacts (#350). After the footer and the strip show what
    /// they will, so they are measured as drawn.
    fn fit_if_changed(&self, opening: bool, strip_shows: bool, keys: &[DialerKey]) {
        let fitted = Fitted {
            strip_shows,
            notice: self.notice_text.label().to_string(),
            keys: keys.to_vec(),
        };
        if opening || self.fitted.borrow().as_ref() != Some(&fitted) {
            self.fit(strip_shows);
            *self.fitted.borrow_mut() = Some(fitted);
        }
    }

    /// Keeps the window as tall as a full Dialer, the list as tall as its
    /// rows up to the list's height, less the notice strip's while it
    /// shows, so the card never outgrows the window.
    /// `strip_shows` while the notice strip does.
    fn fit(&self, strip_shows: bool) {
        let natural =
            |widget: &gtk::Widget| widget.measure(gtk::Orientation::Vertical, DIALER_WIDTH).1;
        let notice = if strip_shows {
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

/// Adds what each of `rows` shows, reading as `texts`, to `content`: its
/// line, its glyph and its matched letters, and where each section starts.
fn report_rows(
    rows: &[DialerRow],
    texts: &[messages::DialerRowText],
    content: &mut BTreeMap<String, String>,
) {
    for (index, (row, text)) in rows.iter().zip(texts).enumerate() {
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
    for (index, section) in section_starts(rows) {
        content.insert(format!("section-{index}"), section.to_owned());
    }
}

/// What the card's height around its list follows: whether the notice
/// strip shows, what it says, and the footer's keys.
#[derive(PartialEq, Eq)]
struct Fitted {
    strip_shows: bool,
    notice: String,
    keys: Vec<DialerKey>,
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
            if row.header().is_some() {
                row.set_header(None::<&gtk::Widget>);
            }
            return;
        };
        // The label this row had, if any, says the new section.
        if let Some(header) = row.header().and_downcast::<gtk::Label>() {
            header.set_text(label);
            return;
        }
        let header = gtk::Label::new(Some(label));
        header.add_css_class(class::EYEBROW);
        header.add_css_class("section");
        header.set_xalign(0.0);
        row.set_header(Some(&header));
    }
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
