//! The Ringer Window (issue #432): which Ring Sound calls ring with, and at
//! what volume, saved as they change.
//!
//! An ordinary window, like the Quiet Hours Window: it tiles, takes focus
//! when it opens and can be moved aside. The sounds are chips in two
//! groups of three columns, "Built in" and "Android", then the user's own
//! file, once there is one, beside "Choose a file…". Choosing a chip, by a
//! click, Space or Enter, saves it and plays it once; Play plays the sound
//! chosen again, and reads Stop while one plays. Letting go of the volume
//! slider saves it and plays the sound at it; keys move it, and it saves
//! once they rest. The chip chosen has keyboard focus when it opens; Tab
//! and the arrows go round the controls. Escape stops the sound playing, or
//! with none closes the window, as closing it does.
//!
//! It reports `ringer-window` with `sound` (the key of the Ring Sound
//! chosen), `built-in` and `android` (the chips' names, one per line),
//! `file` (the user's own file's name, once chosen), `volume`, `play`
//! (`Play` or `Stop`), `notice`, `keys` (one per line), `focus` (a Ring
//! Sound's key, `play`, `choose-file` or `volume`) and `focused`.

use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;
use std::rc::Rc;
use std::time::Duration;

use desktop_calls_core::messages::{self, RingerWindowText};
use desktop_calls_core::{Input, LEAST_VOLUME, RingSound, RingerWindowView};
use futures_channel::mpsc::UnboundedSender;
use gtk::prelude::*;
use gtk::{gio, glib};

use super::{
    SharedReport, after_next_frame, class, is_escape, reported, surface_mapped, surface_shown,
};
use crate::theme::space;

const NAME: &str = "ringer-window";
const WIDTH: i32 = 380;
const COLUMNS: i32 = 3;
/// How long the volume rests, moved by keys, before it is saved and
/// played.
const VOLUME_SETTLES: Duration = Duration::from_millis(350);

const CSS: &str = "
window.desktop-calls-ringer > box { padding: 20px; }
.desktop-calls-ringer button.ring-sound { min-height: 34px; padding: 0 10px; }
.desktop-calls-ringer button.ring-sound:checked {
    background: var(--dc-accent-fill); color: var(--dc-accent-label);
}
.desktop-calls-ringer button.ring-sound:focus-visible,
.desktop-calls-ringer button.choose-file:focus-visible,
.desktop-calls-ringer button.play:focus-visible {
    outline: 2px solid var(--dc-accent); outline-offset: 2px;
}
.desktop-calls-ringer .volume-value {
    font-family: monospace; font-feature-settings: \"tnum\"; min-width: 5ch;
}
";

/// A Ring Sound's chip.
struct Chip {
    sound: RingSound,
    button: gtk::ToggleButton,
}

/// The Ringer Window.
pub(super) struct RingerWindow {
    window: gtk::Window,
    heading: gtk::Label,
    play: gtk::Button,
    built_in: gtk::Label,
    android: gtk::Label,
    own_file: gtk::Label,
    chips: Vec<Chip>,
    /// The chip for the user's own file, shown once there is one.
    file_chip: gtk::ToggleButton,
    choose_file: gtk::Button,
    notice: gtk::Box,
    notice_heading: gtk::Label,
    notice_detail: gtk::Label,
    volume_heading: gtk::Label,
    volume: gtk::Scale,
    volume_value: gtk::Label,
    footer: gtk::Box,
    intents: UnboundedSender<Input>,
    report: SharedReport,
    content: Rc<RefCell<BTreeMap<String, String>>>,
    mapped: Rc<Cell<bool>>,
    /// While the window sets the slider itself, so it doesn't send that
    /// back.
    rendering: Rc<Cell<bool>>,
    /// While a pointer holds the slider: it is saved when let go.
    held: Rc<Cell<bool>>,
    /// The save of a volume moved by keys, waiting for them to rest.
    settling: Rc<RefCell<Option<glib::SourceId>>>,
    /// The opening shown.
    opened: Cell<Option<u32>>,
    /// What is shown, so it is drawn again only when it changes.
    shown: RefCell<Option<RingerWindowView>>,
    /// The text shown, for the file chooser and Play.
    text: RefCell<Option<RingerWindowText>>,
}

impl RingerWindow {
    pub(super) fn new(intents: &UnboundedSender<Input>, report: SharedReport) -> Rc<Self> {
        add_css();
        let window = gtk::Window::new();
        window.set_default_size(WIDTH, -1);
        window.set_resizable(false);
        window.add_css_class("desktop-calls-ringer");

        let heading = gtk::Label::new(None);
        heading.add_css_class(class::HEADING);
        heading.set_xalign(0.0);
        heading.set_hexpand(true);
        let play = gtk::Button::new();
        play.add_css_class("play");
        play.set_valign(gtk::Align::Center);
        let top = gtk::Box::new(gtk::Orientation::Horizontal, space::M);
        top.append(&heading);
        top.append(&play);

        let built_in = eyebrow();
        let android = eyebrow();
        let own_file = eyebrow();
        let mut chips = Vec::new();
        let built_in_grid = chip_grid(&RingSound::BUILT_IN, &mut chips);
        let android_grid = chip_grid(&RingSound::ANDROID, &mut chips);
        let (files, file_chip, choose_file) = file_row(chips.first().map(|chip| &chip.button));

        let (notice, notice_heading, notice_detail) = notice();

        let volume_heading = eyebrow();
        let (volume_row, volume, volume_value) = volume_row();

        let footer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        footer.add_css_class(class::FOOTER_LINE);

        let section = |title: &gtk::Label, body: &gtk::Widget| {
            let section = gtk::Box::new(gtk::Orientation::Vertical, space::XS + 2);
            section.append(title);
            section.append(body);
            section
        };
        let layout = gtk::Box::new(gtk::Orientation::Vertical, space::L);
        layout.append(&top);
        layout.append(&section(&built_in, built_in_grid.upcast_ref()));
        layout.append(&section(&android, android_grid.upcast_ref()));
        let file_section = section(&own_file, files.upcast_ref());
        file_section.append(&notice);
        layout.append(&file_section);
        layout.append(&section(&volume_heading, volume_row.upcast_ref()));
        layout.append(&footer);
        window.set_child(Some(&layout));

        let ringer = Rc::new(Self {
            window,
            heading,
            play,
            built_in,
            android,
            own_file,
            chips,
            file_chip,
            choose_file,
            notice,
            notice_heading,
            notice_detail,
            volume_heading,
            volume,
            volume_value,
            footer,
            intents: intents.clone(),
            report,
            content: Rc::default(),
            mapped: Rc::default(),
            rendering: Rc::default(),
            held: Rc::default(),
            settling: Rc::default(),
            opened: Cell::default(),
            shown: RefCell::default(),
            text: RefCell::default(),
        });
        ringer.connect();
        // Pre-warm, as for the other surfaces.
        WidgetExt::realize(&ringer.window);
        ringer
    }

    fn connect(self: &Rc<Self>) {
        for chip in &self.chips {
            let (intents, sound) = (self.intents.clone(), chip.sound);
            chip.button.connect_clicked(move |_| {
                let _ = intents.unbounded_send(Input::ChooseRingSound(sound));
            });
        }
        let intents = self.intents.clone();
        self.file_chip.connect_clicked(move |_| {
            let _ = intents.unbounded_send(Input::ChooseRingSound(RingSound::File));
        });
        let weak = Rc::downgrade(self);
        self.play.connect_clicked(move |_| {
            if let Some(ringer) = weak.upgrade() {
                let previewing = ringer
                    .shown
                    .borrow()
                    .as_ref()
                    .is_some_and(|view| view.previewing);
                ringer.send(if previewing {
                    Input::StopPreview
                } else {
                    Input::PreviewRingSound
                });
            }
        });
        let weak = Rc::downgrade(self);
        self.choose_file.connect_clicked(move |_| {
            if let Some(ringer) = weak.upgrade() {
                ringer.choose_file();
            }
        });
        self.connect_volume();
        let intents = self.intents.clone();
        self.window.connect_close_request(move |_| {
            let _ = intents.unbounded_send(Input::CloseRingerWindow);
            // The Call state machine closes it.
            glib::Propagation::Stop
        });
        let keys = gtk::EventControllerKey::new();
        keys.set_propagation_phase(gtk::PropagationPhase::Capture);
        let intents = self.intents.clone();
        keys.connect_key_pressed(move |_, key, _, modifiers| {
            if is_escape(key, modifiers) {
                let _ = intents.unbounded_send(Input::RingerWindowEscape);
                return glib::Propagation::Stop;
            }
            glib::Propagation::Proceed
        });
        self.window.add_controller(keys);
        let weak = Rc::downgrade(self);
        self.window.connect_focus_widget_notify(move |_| {
            if let Some(ringer) = weak.upgrade() {
                ringer.publish();
            }
        });
        let weak = Rc::downgrade(self);
        self.window.connect_map(move |window| {
            let Some(ringer) = weak.upgrade() else {
                return;
            };
            ringer.mapped.set(true);
            ringer.publish();
            surface_mapped(NAME);
            after_next_frame(window, || surface_shown(NAME));
        });
        let weak = Rc::downgrade(self);
        self.window.connect_is_active_notify(move |_| {
            if let Some(ringer) = weak.upgrade() {
                ringer.publish();
            }
        });
        let (mapped, report) = (self.mapped.clone(), self.report.clone());
        self.window.connect_unmap(move |_| {
            mapped.set(false);
            report.borrow_mut().set(NAME, None);
        });
    }

    /// The slider: its value shows as it moves; it is saved when a pointer
    /// lets go of it, or once keys that moved it rest.
    fn connect_volume(self: &Rc<Self>) {
        let weak = Rc::downgrade(self);
        self.volume.connect_value_changed(move |scale| {
            let Some(ringer) = weak.upgrade() else {
                return;
            };
            ringer
                .volume_value
                .set_text(&messages::ringer_volume(percent(scale.value())));
            if ringer.rendering.get() || ringer.held.get() {
                return;
            }
            if let Some(waiting) = ringer.settling.take() {
                waiting.remove();
            }
            let settled = Rc::downgrade(&ringer);
            let source = glib::timeout_add_local_once(VOLUME_SETTLES, move || {
                if let Some(ringer) = settled.upgrade() {
                    ringer.settling.replace(None);
                    ringer.save_volume();
                }
            });
            ringer.settling.replace(Some(source));
        });
        // Sees the pointer's press and release before the slider's own
        // gesture takes them.
        let pointer = gtk::EventControllerLegacy::new();
        pointer.set_propagation_phase(gtk::PropagationPhase::Capture);
        let weak = Rc::downgrade(self);
        pointer.connect_event(move |_, event| {
            let Some(ringer) = weak.upgrade() else {
                return glib::Propagation::Proceed;
            };
            match event.event_type() {
                gtk::gdk::EventType::ButtonPress | gtk::gdk::EventType::TouchBegin => {
                    ringer.held.set(true);
                }
                gtk::gdk::EventType::ButtonRelease
                | gtk::gdk::EventType::TouchEnd
                | gtk::gdk::EventType::TouchCancel => {
                    ringer.let_go();
                }
                _ => {}
            }
            glib::Propagation::Proceed
        });
        self.volume.add_controller(pointer);
    }

    /// A pointer let go of the slider: its volume is saved, at once.
    fn let_go(&self) {
        if !self.held.replace(false) {
            return;
        }
        if let Some(waiting) = self.settling.take() {
            waiting.remove();
        }
        self.save_volume();
    }

    /// Saves the volume the slider shows, and plays the sound at it.
    fn save_volume(&self) {
        self.send(Input::SetRingerVolume(percent(self.volume.value())));
    }

    /// Asks for a sound file of the user's own, which then rings.
    fn choose_file(&self) {
        let text = self.text.borrow();
        let Some(text) = text.as_ref() else {
            return;
        };
        let audio = gtk::FileFilter::new();
        audio.set_name(Some(text.audio_files));
        audio.add_mime_type("audio/*");
        let filters = gio::ListStore::new::<gtk::FileFilter>();
        filters.append(&audio);
        let dialog = gtk::FileDialog::new();
        dialog.set_title(text.file_chooser);
        dialog.set_modal(true);
        dialog.set_filters(Some(&filters));
        dialog.set_default_filter(Some(&audio));
        let intents = self.intents.clone();
        dialog.open(
            Some(&self.window),
            None::<&gio::Cancellable>,
            move |chosen| {
                let Some(path) = chosen.ok().and_then(|file| file.path()) else {
                    return;
                };
                if let Some(path) = path.to_str() {
                    let _ = intents.unbounded_send(Input::ChooseRingerFile(path.to_owned()));
                }
            },
        );
    }

    fn send(&self, input: Input) {
        let _ = self.intents.unbounded_send(input);
    }

    pub(super) fn render(&self, view: Option<&RingerWindowView>) {
        let Some(view) = view else {
            self.opened.set(None);
            self.shown.replace(None);
            self.window.set_visible(false);
            return;
        };
        let opening = self.opened.replace(Some(view.opened)) != Some(view.opened);
        // The chip the user clicked shows what was saved, even when that
        // changed nothing.
        self.show_chosen(view.settings.sound());
        if !opening && self.shown.borrow().as_ref() == Some(view) {
            return;
        }
        self.shown.replace(Some(view.clone()));
        let text = messages::ringer_window(view);
        self.show(view, &text);
        *self.content.borrow_mut() = report(view, &text);
        self.text.replace(Some(text));
        if opening || !self.window.is_visible() {
            if let Some(chosen) = self.chip(view.settings.sound()) {
                GtkWindowExt::set_focus(&self.window, Some(chosen));
            }
            self.window.present();
        }
        self.publish();
    }

    fn show(&self, view: &RingerWindowView, text: &RingerWindowText) {
        self.window.set_title(Some(text.title));
        self.heading.set_text(text.heading);
        self.play.set_label(text.play);
        self.built_in.set_text(text.built_in);
        self.android.set_text(text.android);
        self.own_file.set_text(text.own_file);
        for chip in &self.chips {
            let name = text
                .built_in_sounds
                .iter()
                .chain(&text.android_sounds)
                .find(|(sound, _)| *sound == chip.sound)
                .map_or("", |(_, name)| name);
            chip.button.set_label(name);
        }
        if let Some(label) = self
            .file_chip
            .child()
            .and_then(|child| child.downcast::<gtk::Label>().ok())
        {
            label.set_text(text.file.as_deref().unwrap_or_default());
        }
        self.file_chip.set_visible(text.file.is_some());
        self.file_chip
            .update_property(&[gtk::accessible::Property::Label(text.own_file)]);
        self.choose_file.set_label(text.choose_file);
        self.notice_heading
            .set_text(text.notice.unwrap_or_default());
        self.notice_detail
            .set_text(text.notice_detail.unwrap_or_default());
        self.notice.set_visible(text.notice.is_some());
        self.volume_heading.set_text(text.volume_heading);
        self.volume
            .update_property(&[gtk::accessible::Property::Label(text.volume_heading)]);
        if !self.held.get() && self.settling.borrow().is_none() {
            let volume = f64::from(view.settings.volume());
            if (self.volume.value() - volume).abs() > f64::EPSILON {
                self.rendering.set(true);
                self.volume.set_value(volume);
                self.rendering.set(false);
            }
            self.volume_value.set_text(&text.volume);
        }
        self.show_keys(&text.keys);
    }

    /// Ticks the chip for `sound`, and only it.
    fn show_chosen(&self, sound: RingSound) {
        if let Some(chip) = self.chip(sound)
            && !chip.is_active()
        {
            chip.set_active(true);
        }
    }

    fn chip(&self, sound: RingSound) -> Option<&gtk::ToggleButton> {
        if sound == RingSound::File {
            return Some(&self.file_chip);
        }
        self.chips
            .iter()
            .find(|chip| chip.sound == sound)
            .map(|chip| &chip.button)
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
        if let Some(focus) = self.focus() {
            content.insert("focus".to_owned(), focus.to_owned());
        }
        self.report
            .borrow_mut()
            .set(NAME, Some(reported(&content, &self.window)));
    }

    /// The control with keyboard focus, as reported.
    fn focus(&self) -> Option<&'static str> {
        let focus = GtkWindowExt::focus(&self.window)?;
        let is = |widget: &gtk::Widget| focus == *widget || focus.is_ancestor(widget);
        if is(self.play.upcast_ref()) {
            return Some("play");
        }
        if is(self.choose_file.upcast_ref()) {
            return Some("choose-file");
        }
        if is(self.volume.upcast_ref()) {
            return Some("volume");
        }
        if is(self.file_chip.upcast_ref()) {
            return Some(RingSound::File.key());
        }
        self.chips
            .iter()
            .find(|chip| is(chip.button.upcast_ref()))
            .map(|chip| chip.sound.key())
    }
}

fn eyebrow() -> gtk::Label {
    let label = gtk::Label::new(None);
    label.add_css_class(class::EYEBROW);
    label.set_xalign(0.0);
    label
}

/// A grid of chips for `sounds`, three to a row, all in one group with
/// those in `chips` already, so choosing one unticks the rest.
fn chip_grid(sounds: &[RingSound], chips: &mut Vec<Chip>) -> gtk::Grid {
    let grid = gtk::Grid::new();
    grid.set_column_homogeneous(true);
    grid.set_column_spacing(space::S.unsigned_abs());
    grid.set_row_spacing(space::S.unsigned_abs());
    for (placed, &sound) in (0..).zip(sounds) {
        let button = gtk::ToggleButton::new();
        button.add_css_class("ring-sound");
        button.set_group(chips.first().map(|chip| &chip.button));
        grid.attach(&button, placed % COLUMNS, placed / COLUMNS, 1, 1);
        chips.push(Chip { sound, button });
    }
    grid
}

/// The user's own file's row: the row, the file's chip (in `group`,
/// hidden until there is a file) and "Choose a file…".
fn file_row(group: Option<&gtk::ToggleButton>) -> (gtk::Box, gtk::ToggleButton, gtk::Button) {
    let file_chip = gtk::ToggleButton::new();
    file_chip.add_css_class("ring-sound");
    file_chip.add_css_class("own-file");
    file_chip.set_group(group);
    let file_label = gtk::Label::new(None);
    file_label.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
    file_label.set_max_width_chars(18);
    file_chip.set_child(Some(&file_label));
    file_chip.set_visible(false);
    let choose_file = gtk::Button::new();
    choose_file.add_css_class("choose-file");
    let files = gtk::Box::new(gtk::Orientation::Horizontal, space::S);
    files.append(&file_chip);
    files.append(&choose_file);
    (files, file_chip, choose_file)
}

/// The volume's row: the row, the slider from the least volume to 100 %,
/// and its percentage.
fn volume_row() -> (gtk::Box, gtk::Scale, gtk::Label) {
    let volume = gtk::Scale::with_range(
        gtk::Orientation::Horizontal,
        f64::from(LEAST_VOLUME),
        100.0,
        5.0,
    );
    volume.set_increments(5.0, 10.0);
    volume.set_round_digits(0);
    volume.set_draw_value(false);
    volume.set_hexpand(true);
    let volume_value = gtk::Label::new(None);
    volume_value.add_css_class("volume-value");
    volume_value.set_xalign(1.0);
    let row = gtk::Box::new(gtk::Orientation::Horizontal, space::S);
    row.append(&volume);
    row.append(&volume_value);
    (row, volume, volume_value)
}

/// The notice that shows while the user's own file can't play, hidden: the
/// box, and its two lines.
fn notice() -> (gtk::Box, gtk::Label, gtk::Label) {
    let icon = gtk::Image::from_icon_name("dialog-warning-symbolic");
    icon.add_css_class("notice-icon");
    icon.set_valign(gtk::Align::Start);
    let heading = gtk::Label::new(None);
    heading.add_css_class("notice-heading");
    heading.set_xalign(0.0);
    let detail = gtk::Label::new(None);
    detail.set_xalign(0.0);
    let lines = gtk::Box::new(gtk::Orientation::Vertical, 2);
    lines.append(&heading);
    lines.append(&detail);
    let notice = gtk::Box::new(gtk::Orientation::Horizontal, space::S);
    notice.add_css_class(class::NOTICE);
    notice.append(&icon);
    notice.append(&lines);
    notice.set_visible(false);
    (notice, heading, detail)
}

/// The slider's value as a whole percentage.
fn percent(value: f64) -> u8 {
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "clamped to 0..=100 first"
    )]
    let percent = value.round().clamp(0.0, 100.0) as u8;
    percent
}

fn report(view: &RingerWindowView, text: &RingerWindowText) -> BTreeMap<String, String> {
    let names = |sounds: &[(RingSound, &str)]| {
        sounds
            .iter()
            .map(|(_, name)| *name)
            .collect::<Vec<_>>()
            .join("\n")
    };
    let keys: Vec<String> = text
        .keys
        .iter()
        .map(|(caps, does)| format!("{} {does}", caps.join(" ")))
        .collect();
    let mut content = BTreeMap::from([
        ("sound".to_owned(), view.settings.sound().key().to_owned()),
        ("built-in".to_owned(), names(&text.built_in_sounds)),
        ("android".to_owned(), names(&text.android_sounds)),
        ("volume".to_owned(), text.volume.clone()),
        ("play".to_owned(), text.play.to_owned()),
        ("keys".to_owned(), keys.join("\n")),
    ]);
    if let Some(file) = &text.file {
        content.insert("file".to_owned(), file.clone());
    }
    if let (Some(notice), Some(detail)) = (text.notice, text.notice_detail) {
        content.insert("notice".to_owned(), format!("{notice}\n{detail}"));
    }
    content
}

/// Adds the Ringer Window's styles to the display.
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
