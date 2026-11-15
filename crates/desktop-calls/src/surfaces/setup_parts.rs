//! The Setup Window's parts: the four named steps across the top, the
//! instructions grouped under "On your phone" and "Here", the sync's
//! checklist and bar, and the finish page's cards.

use desktop_calls_core::messages::{
    self, InstructionGroup, Place, SETUP_STAGES, SetupCard, StageMark, SyncText, stage_line,
};
use gtk::prelude::*;

use super::icons::{Icon, IconView};
use super::{class, set_class};
use crate::theme::space;

pub(super) const CSS: &str = "
/* The four named steps: a 3 px bar over each name, green once done, the
   accent where setup is. */
.desktop-calls-setup .setup-stage {
    font-family: monospace; font-size: 10.5px; font-weight: 500;
    color: var(--dc-secondary);
}
.desktop-calls-setup .setup-stage .stage-bar {
    min-height: 3px; background: var(--dc-border);
}
.desktop-calls-setup .setup-stage.ticked .stage-bar { background: var(--dc-answer); }
.desktop-calls-setup .setup-stage.now .stage-bar { background: var(--dc-accent); }
.desktop-calls-setup .setup-stage.now { color: var(--dc-text); font-weight: 600; }
.desktop-calls-setup .setup-group-label { color: var(--dc-secondary); }
.desktop-calls-setup .setup-check { color: var(--dc-answer); font-weight: 600; }
.desktop-calls-setup progressbar.setup-bar trough,
.desktop-calls-setup progressbar.setup-bar progress {
    min-height: 4px; border-radius: 0;
}
.desktop-calls-setup progressbar.setup-bar trough { background: var(--dc-raised); }
.desktop-calls-setup progressbar.setup-bar progress { background: var(--dc-accent); }
/* The finish page's two cards: equal, raised, title over text. */
.desktop-calls-setup .setup-card { padding: 12px; background: var(--dc-raised); }
.desktop-calls-setup .setup-card .setup-card-title { font-weight: 600; }
.desktop-calls-setup .setup-card .setup-card-text { color: var(--dc-secondary); }
.desktop-calls-setup .setup-countdown-label { color: var(--dc-secondary); }
";

/// The four named steps across the top.
pub(super) struct Stages {
    pub(super) widget: gtk::Box,
    stages: Vec<(gtk::Box, gtk::Label)>,
}

impl Stages {
    pub(super) fn new() -> Self {
        let widget = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        widget.set_homogeneous(true);
        let stages = SETUP_STAGES
            .iter()
            .map(|name| {
                let bar = gtk::Box::new(gtk::Orientation::Horizontal, 0);
                bar.add_css_class("stage-bar");
                let label = gtk::Label::new(Some(name));
                label.set_xalign(0.0);
                label.set_ellipsize(gtk::pango::EllipsizeMode::End);
                let stage = gtk::Box::new(gtk::Orientation::Vertical, 5);
                stage.add_css_class("setup-stage");
                stage.append(&bar);
                stage.append(&label);
                widget.append(&stage);
                (stage, label)
            })
            .collect();
        Self { widget, stages }
    }

    pub(super) fn show(&self, marks: [StageMark; 4]) {
        for (((stage, label), name), mark) in self.stages.iter().zip(SETUP_STAGES).zip(marks) {
            set_class(stage, "ticked", mark == StageMark::Ticked);
            set_class(stage, "now", mark == StageMark::Now);
            label.set_text(&stage_line(name, mark));
        }
    }
}

/// Instructions done in one place, under its label.
struct Group {
    widget: gtk::Box,
    label: gtk::Box,
    lines: gtk::Box,
}

impl Group {
    fn new(place: Place) -> Self {
        let icon = IconView::new(match place {
            Place::OnYourPhone => Icon::Phone,
            Place::Here => Icon::Computer,
        });
        let name = gtk::Label::new(Some(place.label()));
        name.add_css_class(class::EYEBROW);
        let label = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        label.add_css_class("setup-group-label");
        label.append(&icon.area);
        label.append(&name);
        let lines = gtk::Box::new(gtk::Orientation::Vertical, 3);
        let widget = gtk::Box::new(gtk::Orientation::Vertical, space::S);
        widget.append(&label);
        widget.append(&lines);
        Self {
            widget,
            label,
            lines,
        }
    }

    fn show(&self, group: Option<&InstructionGroup>) {
        while let Some(child) = self.lines.first_child() {
            self.lines.remove(&child);
        }
        let lines = group.map_or(&[][..], |group| group.lines.as_slice());
        // Numbered when there is more than one thing to do.
        let numbered = lines.len() > 1;
        for (number, line) in (1..).zip(lines) {
            let label = wrapping_label();
            keep_runs_whole(&label, line);
            if numbered {
                // The number in a column of its own, so a step's wrapped
                // lines start under its text, not under its number (#326).
                let mark = gtk::Label::new(Some(&format!("{number}.")));
                mark.set_xalign(0.0);
                mark.set_valign(gtk::Align::Start);
                let row = gtk::Box::new(gtk::Orientation::Horizontal, space::XS);
                label.set_hexpand(true);
                row.append(&mark);
                row.append(&label);
                self.lines.append(&row);
            } else {
                self.lines.append(&label);
            }
        }
        self.lines.set_visible(!lines.is_empty());
        self.label
            .set_visible(group.is_some_and(|group| group.label.is_some()));
    }
}

/// The instructions, grouped by where each is done. The "Here" group also
/// holds what the window lists here, such as the Phones nearby.
pub(super) struct Instructions {
    pub(super) widget: gtk::Box,
    phone: Group,
    here: Group,
}

impl Instructions {
    pub(super) fn new() -> Self {
        let widget = gtk::Box::new(gtk::Orientation::Vertical, space::L);
        let phone = Group::new(Place::OnYourPhone);
        let here = Group::new(Place::Here);
        widget.append(&phone.widget);
        widget.append(&here.widget);
        Self {
            widget,
            phone,
            here,
        }
    }

    /// Puts `widget` in the "Here" group, under its instructions.
    pub(super) fn add_here(&self, widget: &impl IsA<gtk::Widget>) {
        self.here.widget.append(widget);
    }

    /// Shows `groups`, in their order; `here_shows` is whether something
    /// listed here shows without an instruction.
    pub(super) fn show(&self, groups: &[InstructionGroup], here_shows: bool) {
        let find = |place| groups.iter().find(|group| group.place == place);
        let (phone, here) = (find(Place::OnYourPhone), find(Place::Here));
        self.phone.show(phone);
        self.here.show(here);
        let here_visible = here.is_some() || here_shows;
        self.phone.widget.set_visible(phone.is_some());
        self.here.widget.set_visible(here_visible);
        let first = if groups
            .first()
            .is_some_and(|group| group.place == Place::Here)
        {
            &self.here.widget
        } else {
            &self.phone.widget
        };
        self.widget.reorder_child_after(first, None::<&gtk::Widget>);
        self.widget.set_visible(phone.is_some() || here_visible);
    }
}

/// The sync's checklist, its bar and what already works.
pub(super) struct SyncView {
    pub(super) widget: gtk::Box,
    rows: Vec<SyncRowView>,
    bar: gtk::ProgressBar,
    note: gtk::Label,
}

struct SyncRowView {
    check: gtk::Label,
    spinner: gtk::Spinner,
    name: gtk::Label,
    meta: gtk::Label,
}

impl SyncView {
    pub(super) fn new() -> Self {
        let widget = gtk::Box::new(gtk::Orientation::Vertical, space::M);
        let rows = (0..2)
            .map(|_| {
                let check = gtk::Label::new(Some("✓"));
                check.add_css_class("setup-check");
                // The tick and the spinner share one 16 px slot, so the
                // names line up.
                check.set_size_request(16, -1);
                let spinner = gtk::Spinner::new();
                spinner.set_size_request(16, 16);
                let name = gtk::Label::new(None);
                name.set_xalign(0.0);
                name.set_hexpand(true);
                let meta = gtk::Label::new(None);
                meta.add_css_class(class::META);
                let row = gtk::Box::new(gtk::Orientation::Horizontal, space::M);
                row.append(&check);
                row.append(&spinner);
                row.append(&name);
                row.append(&meta);
                widget.append(&row);
                SyncRowView {
                    check,
                    spinner,
                    name,
                    meta,
                }
            })
            .collect();
        let bar = gtk::ProgressBar::new();
        bar.add_css_class("setup-bar");
        let note = wrapping_label();
        note.add_css_class(class::META);
        widget.append(&bar);
        widget.append(&note);
        widget.set_visible(false);
        Self {
            widget,
            rows,
            bar,
            note,
        }
    }

    pub(super) fn show(&self, sync: Option<&SyncText>) {
        self.widget.set_visible(sync.is_some());
        let Some(sync) = sync else {
            for row in &self.rows {
                row.spinner.stop();
            }
            return;
        };
        for (view, row) in self.rows.iter().zip(&sync.rows) {
            view.check.set_visible(row.done);
            view.spinner.set_visible(!row.done);
            view.spinner.set_spinning(!row.done);
            view.name.set_text(row.name);
            view.meta.set_text(row.meta.as_deref().unwrap_or_default());
        }
        self.bar.set_fraction(f64::from(sync.bar) / 1_000.0);
        self.note.set_text(sync.note);
    }
}

/// One of the finish page's cards.
pub(super) struct Card {
    pub(super) widget: gtk::Box,
    title: gtk::Label,
    text: gtk::Label,
}

impl Card {
    pub(super) fn new() -> Self {
        let title = gtk::Label::new(None);
        title.add_css_class("setup-card-title");
        title.set_xalign(0.0);
        let text = wrapping_label();
        text.add_css_class("setup-card-text");
        // Selectable with the pointer, to copy a command; Tab still moves
        // only between the buttons.
        text.set_selectable(true);
        text.set_focusable(false);
        let widget = gtk::Box::new(gtk::Orientation::Vertical, 2);
        widget.add_css_class("setup-card");
        widget.append(&title);
        widget.append(&text);
        widget.set_visible(false);
        Self {
            widget,
            title,
            text,
        }
    }

    pub(super) fn show(&self, card: Option<&SetupCard>) {
        self.widget.set_visible(card.is_some());
        if let Some(card) = card {
            self.title.set_text(card.title);
            keep_runs_whole(&self.text, &card.text);
        }
    }
}

/// The keybinding's line, exactly as it is written: monospace, on a card.
pub(super) fn line_label() -> gtk::Label {
    let line = wrapping_label();
    line.add_css_class("setup-line");
    line.add_css_class("card");
    // Breaks only at a space or after a `/`: see `keep_runs_whole`.
    line.set_wrap_mode(gtk::pango::WrapMode::Word);
    // Selectable with the pointer, to copy; Tab still moves only between
    // the buttons.
    line.set_selectable(true);
    line.set_focusable(false);
    line
}

/// A label for a sentence: left-aligned, wrapping at the window's width.
pub(super) fn wrapping_label() -> gtk::Label {
    let label = gtk::Label::new(None);
    label.set_xalign(0.0);
    label.set_wrap(true);
    label.set_max_width_chars(50);
    label
}

/// Sets `label`'s text so it can break only at a space or after a `/`,
/// never inside a word or a path's name: Pango may not break inside each
/// run, so the text itself stays exactly as given, to copy.
pub(super) fn keep_runs_whole(label: &gtk::Label, text: &str) {
    label.set_text(text);
    let attributes = gtk::pango::AttrList::new();
    for run in messages::unbreakable_runs(text) {
        let mut no_break = gtk::pango::AttrInt::new_allow_breaks(false);
        no_break.set_start_index(u32::try_from(run.start).unwrap_or(u32::MAX));
        no_break.set_end_index(u32::try_from(run.end).unwrap_or(u32::MAX));
        attributes.insert(no_break);
    }
    label.set_attributes(Some(&attributes));
}
