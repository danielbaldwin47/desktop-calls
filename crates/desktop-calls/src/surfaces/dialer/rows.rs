//! The Dialer's rows, built once and filled afresh on each keystroke.
//!
//! Each keystroke changes up to 50 rows. Building them anew cost about
//! 3.5 ms a keystroke with 5,000 Contacts under CI's limits, most of it
//! GTK making each row's eight widgets and their styles; filling rows kept
//! from the last keystroke costs a fraction of that (#350). A row that has
//! nothing to show is hidden, at the end of the list.

use super::super::icons::{Icon, IconView};
use super::super::{class, set_class};
use crate::theme::space;
use desktop_calls_core::Caller;
use desktop_calls_core::matching::{DialerRow, DialerRowKind};
use desktop_calls_core::messages::{self, DialerGlyph, DialerRowText, Label};
use gtk::prelude::*;

/// A row of the list: its glyph, its name over its detail line, when at
/// the right, and the dot of a Missed Call the user hasn't seen.
pub(super) struct RowView {
    pub(super) list_row: gtk::ListBoxRow,
    /// The direction of a Call History row, red for a Missed Call.
    arrow: IconView,
    /// A Contact's initial in a tile.
    initial: gtk::Label,
    title: gtk::Label,
    /// "Missed" in the theme's red, then the rest of the detail line.
    detail_line: gtk::Box,
    missed: gtk::Label,
    detail: gtk::Label,
    time: gtk::Label,
    dot: gtk::Box,
}

impl RowView {
    pub(super) fn new() -> Self {
        let arrow = IconView::new(Icon::Incoming);
        arrow.area.add_css_class("dialer-glyph");
        let initial = gtk::Label::new(None);
        initial.add_css_class(class::INITIAL_TILE);
        initial.add_css_class("dialer-glyph");
        initial.set_valign(gtk::Align::Center);
        let title = gtk::Label::new(None);
        title.add_css_class("title");
        title.set_xalign(0.0);
        title.set_ellipsize(gtk::pango::EllipsizeMode::End);
        let missed = gtk::Label::new(Some(messages::label(Label::Missed)));
        missed.add_css_class(class::META);
        missed.add_css_class(class::MISSED);
        let detail = gtk::Label::new(None);
        detail.add_css_class(class::META);
        detail.set_xalign(0.0);
        detail.set_ellipsize(gtk::pango::EllipsizeMode::End);
        let detail_line = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        detail_line.append(&missed);
        detail_line.append(&detail);
        let lines = gtk::Box::new(gtk::Orientation::Vertical, 2);
        lines.set_hexpand(true);
        lines.set_valign(gtk::Align::Center);
        lines.append(&title);
        lines.append(&detail_line);
        let time = gtk::Label::new(None);
        time.add_css_class(class::META);
        time.set_valign(gtk::Align::Center);
        let dot = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        dot.add_css_class(class::NEW_DOT);
        dot.set_valign(gtk::Align::Center);
        let shown = gtk::Box::new(gtk::Orientation::Horizontal, space::M);
        shown.add_css_class("dialer-row");
        shown.append(&arrow.area);
        shown.append(&initial);
        shown.append(&lines);
        shown.append(&time);
        shown.append(&dot);
        let list_row = gtk::ListBoxRow::new();
        // The entry keeps the keyboard; the list only shows.
        list_row.set_focusable(false);
        list_row.set_child(Some(&shown));
        Self {
            list_row,
            arrow,
            initial,
            title,
            detail_line,
            missed,
            detail,
            time,
            dot,
        }
    }

    /// Shows `row`, as `text` reads, with the dot if `marked`. The letters
    /// of the name that matched what was typed are underlined in
    /// `underline`.
    pub(super) fn show(
        &self,
        row: &DialerRow,
        text: &DialerRowText,
        underline: &gtk::gdk::RGBA,
        marked: bool,
    ) {
        self.show_glyph(&text.glyph);
        self.title.set_text(&text.title);
        // A row with no name shows the bare number.
        set_class(
            &self.title,
            class::NUMBER,
            row.kind != DialerRowKind::CallNumber
                && row.name.trim().is_empty()
                && matches!(row.caller, Caller::Number(_)),
        );
        let matched = text.title == row.name && !row.matched.is_empty();
        self.title.set_attributes(
            matched
                .then(|| underlined(&row.matched, underline))
                .as_ref(),
        );
        self.missed.set_visible(text.missed);
        let rest = if text.missed && !text.detail.is_empty() {
            format!(" · {}", text.detail)
        } else {
            text.detail.clone()
        };
        self.detail.set_text(&rest);
        self.detail.set_visible(!rest.is_empty());
        self.detail_line
            .set_visible(text.missed || !text.detail.is_empty());
        self.time.set_text(&text.time);
        self.time.set_visible(!text.time.is_empty());
        self.dot.set_visible(marked);
        self.list_row.set_visible(true);
    }

    /// The glyph at the row's left, 28 across: the direction of a Call
    /// History row, red for a Missed Call, or an initial in a tile.
    fn show_glyph(&self, glyph: &DialerGlyph) {
        let arrow = match glyph {
            DialerGlyph::Missed | DialerGlyph::Incoming => Some(Icon::Incoming),
            DialerGlyph::Outgoing => Some(Icon::Outgoing),
            DialerGlyph::Initial(initial) => {
                self.initial.set_text(initial);
                None
            }
        };
        if let Some(icon) = arrow {
            self.arrow.set(icon);
        }
        set_class(
            &self.arrow.area,
            class::MISSED,
            matches!(glyph, DialerGlyph::Missed),
        );
        self.arrow.area.set_visible(arrow.is_some());
        self.initial.set_visible(arrow.is_none());
    }

    /// Hides the row: the list has fewer rows to show.
    pub(super) fn hide(&self) {
        self.list_row.set_visible(false);
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
        // Low: below the descenders, nearer the mockup's 3 px offset.
        let mut line = gtk::pango::AttrInt::new_underline(gtk::pango::Underline::Low);
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
