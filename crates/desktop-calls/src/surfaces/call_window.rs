//! The Call Window: who the Call is with and for how long, one state line
//! that says where it is heard, three tiles (Mute, Keypad and the audio
//! move) and Hang up, each with its shortcut.
//!
//! Anchored at the bottom, it grows upwards. From the bottom up, Hang up,
//! then the tiles, then the state line and who are fixed; whatever comes and
//! goes during a Call (the Held Call's row, a notice, the keypad and its
//! tone entry) sits above the tiles, so they and Hang up never move under
//! the pointer. Muting recolours the Mute tile, the state line and the
//! border, and moves nothing. Placing a Call, after it ends and once contact
//! with it is lost, the window shows only what works.

use super::{
    Anchor, HANDOFF_SHORTCUT, HANG_UP_HELD_SHORTCUT, HANG_UP_SHORTCUT, KEYPAD, KEYPAD_SHORTCUT,
    MUTE_SHORTCUT, SWAP_SHORTCUT, SharedReport, Surface, Who, class, detail_label, keycap,
    seconds_since, set_class,
};
use crate::theme::space;
use desktop_calls_core::messages::{self, Failure, Label};
use desktop_calls_core::{
    AudioLocation, CallWindowPhase, CallWindowView, Input, Mute, keypad_tones,
};
use futures_channel::mpsc::UnboundedSender;
use gtk::gdk::ModifierType;
use gtk::glib::{self, SourceId};
use gtk::prelude::*;
use gtk4_layer_shell::{KeyboardMode, Layer};
use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;
use std::rc::Rc;
use std::time::Duration;

mod held;
mod icons;
mod pill;
mod render;

use held::HeldRow;
use icons::{Icon, IconView};
use pill::CallPill;

/// Each shortcut as the window shows it, by the action's report key, with
/// the keycap that shows it.
type Shortcuts = BTreeMap<String, (String, Option<gtk::Widget>)>;

/// The Call Window.
pub(super) struct CallWindow {
    pub(super) surface: Surface,
    intents: UnboundedSender<Input>,
    caller: gtk::Label,
    timer: gtk::Label,
    /// The number's label and the number, or "Not in Contacts".
    meta: gtk::Label,
    state: StateRow,
    notice: Notice,
    /// Where keypad tones are typed or pasted, and which shows those sent;
    /// there only while the keypad is open.
    tones: gtk::Entry,
    keypad: gtk::Grid,
    tiles: gtk::Box,
    mute: Tile<gtk::Button>,
    keypad_shown: Tile<gtk::ToggleButton>,
    /// Moves the audio to where the Call isn't heard.
    handoff: Tile<gtk::Button>,
    /// Makes the only Call, on hold, live again.
    resume: gtk::Button,
    hang_up: gtk::Button,
    /// The other Call, on hold, with Swap.
    held: HeldRow,
    /// Each shortcut as the window shows it, by the action's report key,
    /// with the keycap that shows it.
    shortcuts: Shortcuts,
    /// Set while the window writes the tones it sent into `tones`, so the
    /// write isn't taken for typing.
    writing_tones: Cell<bool>,
    /// The once-a-second update of the Call timer, while it runs.
    ticker: RefCell<Option<SourceId>>,
    /// The timer as it read when contact with the Call was lost, struck
    /// through and still while it is.
    lost_timer: RefCell<Option<String>>,
    view: Rc<RefCell<Option<CallWindowView>>>,
    /// What the window shows as while it is collapsed.
    pill: CallPill,
}

/// The state line: an icon, what it says, and the HD mark.
struct StateRow {
    row: gtk::Box,
    icon: IconView,
    text: gtk::Label,
    hd: gtk::Label,
}

/// A notice in the warning tint: a heading, if it has one, over what to do.
struct Notice {
    row: gtk::Box,
    heading: gtk::Label,
    text: gtk::Label,
}

/// One of the three tiles: an icon over a label, and its keycap in the
/// corner.
struct Tile<B> {
    button: B,
    icon: IconView,
    label: gtk::Label,
}

impl<B: IsA<gtk::Button>> Tile<B> {
    /// A tile on `button`, with `icon`, saying `text`, for `accelerator`.
    /// Returns it and its key as shown, with the keycap.
    fn new(button: B, icon: Icon, text: Label, accelerator: &str) -> (Self, String, gtk::Widget) {
        let icon = IconView::new(icon);
        let label = gtk::Label::new(Some(messages::label(text)));
        label.set_ellipsize(gtk::pango::EllipsizeMode::End);
        let face = gtk::Box::new(gtk::Orientation::Vertical, 3);
        face.set_valign(gtk::Align::Center);
        face.append(&icon.area);
        face.append(&label);
        let (cap, shown) = keycap(accelerator);
        cap.set_halign(gtk::Align::End);
        cap.set_valign(gtk::Align::Start);
        let overlay = gtk::Overlay::new();
        overlay.set_child(Some(&face));
        overlay.add_overlay(&cap);
        let tile = button.as_ref();
        tile.add_css_class(class::TILE);
        tile.set_child(Some(&overlay));
        tile.set_hexpand(true);
        tile.set_focus_on_click(false);
        (
            Self {
                button,
                icon,
                label,
            },
            shown,
            cap.upcast(),
        )
    }
}

impl CallWindow {
    pub(super) fn new(intents: &UnboundedSender<Input>, report: SharedReport) -> Rc<Self> {
        // Where the Call Pill sits: the window expands from it and collapses
        // back into it, in place.
        let surface = Surface::new(
            "call-window",
            Anchor::BottomRight,
            Layer::Top,
            KeyboardMode::OnDemand,
            Some(Input::CollapseCallWindow),
            intents,
            report.clone(),
        );
        let (top, caller, timer) = top_line();
        let meta = detail_label();
        let mut shortcuts = BTreeMap::new();
        let (tiles, mute, keypad_shown, handoff) = tiles(&mut shortcuts);

        let resume = gtk::Button::new();
        resume.add_css_class("resume");
        let resume_label = gtk::Label::new(Some(messages::label(Label::Resume)));
        let shown = show_shortcut(&resume, None, &resume_label, SWAP_SHORTCUT);
        shortcuts.insert("swap-shortcut".to_owned(), (shown, hint(&resume_label)));
        let hang_up = hang_up_button(&surface, intents);
        let shown = show_shortcut(
            &hang_up,
            Some(Icon::HangUp),
            &gtk::Label::new(Some(messages::label(Label::HangUp))),
            HANG_UP_SHORTCUT,
        );
        if let Some(cap) = hang_up_keycap(&hang_up) {
            shortcuts.insert("hang-up-shortcut".to_owned(), (shown, Some(cap)));
        }

        let held = HeldRow::new(intents);
        let state = StateRow::new();
        let notice = Notice::new();
        let (tones, keypad) = keypad();

        let who = gtk::Box::new(gtk::Orientation::Vertical, 2);
        who.append(&top);
        who.append(&meta);
        let content = gtk::Box::new(gtk::Orientation::Vertical, space::M);
        content.append(&held.row);
        content.append(&who);
        content.append(&state.row);
        content.append(&notice.row);
        content.append(&tones);
        content.append(&keypad);
        content.append(&tiles);
        content.append(&resume);
        content.append(&hang_up);
        surface.window.set_child(Some(&content));
        for button in [&resume, &hang_up] {
            // Clicking leaves the keyboard with the tone entry.
            button.set_focus_on_click(false);
        }

        let window = Rc::new(Self {
            surface,
            intents: intents.clone(),
            caller,
            timer,
            meta,
            state,
            notice,
            tones,
            keypad,
            tiles,
            mute,
            keypad_shown,
            handoff,
            resume,
            hang_up,
            held,
            shortcuts,
            writing_tones: Cell::new(false),
            ticker: RefCell::default(),
            lost_timer: RefCell::default(),
            view: Rc::default(),
            pill: CallPill::new(intents, report),
        });
        window.connect();
        window
    }

    /// Wires the controls, the keypad and the shortcuts to the window.
    fn connect(self: &Rc<Self>) {
        let weak = Rc::downgrade(self);
        self.mute.button.connect_clicked(move |_| {
            if let Some(window) = weak.upgrade() {
                window.toggle_mute();
            }
        });
        let weak = Rc::downgrade(self);
        self.handoff.button.connect_clicked(move |_| {
            if let Some(window) = weak.upgrade() {
                window.hand_off();
            }
        });
        let intents = self.intents.clone();
        self.resume.connect_clicked(move |_| {
            let _ = intents.unbounded_send(Input::Swap);
        });
        let weak = Rc::downgrade(self);
        self.keypad_shown.button.connect_toggled(move |toggle| {
            if let Some(window) = weak.upgrade() {
                let _ = window
                    .intents
                    .unbounded_send(Input::KeypadShown(toggle.is_active()));
                window.update();
            }
        });
        self.report_use();
        for (row, tones) in (0..).zip(KEYPAD) {
            for (column, tone) in (0..).zip(tones) {
                let button = gtk::Button::with_label(tone);
                button.set_focus_on_click(false);
                let weak = Rc::downgrade(self);
                button.connect_clicked(move |_| {
                    if let Some(window) = weak.upgrade() {
                        window.press_tone(tone);
                    }
                });
                self.keypad.attach(&button, column, row, 1, 1);
            }
        }
        if let Some(text) = self.tones.delegate() {
            let weak = Rc::downgrade(self);
            text.connect_insert_text(move |text, inserted, position| {
                if let Some(window) = weak.upgrade()
                    && !window.writing_tones.get()
                    && !inserted.is_empty()
                {
                    text.stop_signal_emission_by_name("insert-text");
                    window.type_tones(text, inserted, position);
                }
            });
        }
        self.connect_shortcuts();
        self.connect_typing();
    }

    /// Each action's shortcut. A control that is hidden or disabled
    /// ignores it, and the key is still taken: `activate` would press a
    /// greyed-out button anyway.
    fn connect_shortcuts(&self) {
        let shortcuts = gtk::ShortcutController::new();
        // Before any control sees the key, so M and K never reach the tone
        // entry.
        shortcuts.set_propagation_phase(gtk::PropagationPhase::Capture);
        let actions: [(&str, Vec<gtk::Widget>); 6] = [
            (MUTE_SHORTCUT, vec![self.mute.button.clone().upcast()]),
            (
                KEYPAD_SHORTCUT,
                vec![self.keypad_shown.button.clone().upcast()],
            ),
            (HANDOFF_SHORTCUT, vec![self.handoff.button.clone().upcast()]),
            (HANG_UP_SHORTCUT, vec![self.hang_up.clone().upcast()]),
            (
                SWAP_SHORTCUT,
                vec![
                    self.held.swap.clone().upcast(),
                    self.resume.clone().upcast(),
                ],
            ),
            (
                HANG_UP_HELD_SHORTCUT,
                vec![self.held.hang_up.clone().upcast()],
            ),
        ];
        for (accelerator, controls) in actions {
            let action = gtk::CallbackAction::new(move |_, _| {
                if let Some(control) = controls
                    .iter()
                    .find(|control| control.is_sensitive() && shows(*control))
                {
                    control.activate();
                }
                glib::Propagation::Stop
            });
            shortcuts.add_shortcut(gtk::Shortcut::new(
                gtk::ShortcutTrigger::parse_string(accelerator),
                Some(action),
            ));
        }
        self.surface.window.add_controller(shortcuts);
    }

    /// A tone typed or pasted while the tone entry doesn't have the
    /// keyboard goes to the keypad too, and opens it, so the tones sent
    /// show.
    fn connect_typing(self: &Rc<Self>) {
        let keys = gtk::EventControllerKey::new();
        let weak = Rc::downgrade(self);
        keys.connect_key_pressed(move |_, key, _, modifiers| {
            let Some(window) = weak.upgrade() else {
                return glib::Propagation::Proceed;
            };
            let control = modifiers.contains(ModifierType::CONTROL_MASK);
            let plain = !modifiers.intersects(ModifierType::CONTROL_MASK | ModifierType::ALT_MASK);
            let tone = key
                .to_unicode()
                .filter(|character| plain && keypad_tones(&character.to_string()).is_some());
            if let Some(tone) = tone
                && window.open_keypad()
            {
                window.press_tone(&tone.to_string());
                return glib::Propagation::Stop;
            }
            if control
                && matches!(key, gtk::gdk::Key::v | gtk::gdk::Key::V)
                && window.open_keypad()
                && let Some(text) = window.tones.delegate()
            {
                let _ = text.activate_action("clipboard.paste", None);
                return glib::Propagation::Stop;
            }
            glib::Propagation::Proceed
        });
        self.surface.window.add_controller(keys);
    }

    /// Opens the keypad if tones can be sent, and returns whether they can.
    fn open_keypad(&self) -> bool {
        let can = self.tones.is_sensitive() && shows(&self.keypad_shown.button);
        if can && !self.keypad_shown.button.is_active() {
            self.keypad_shown.button.set_active(true);
        }
        can
    }

    /// Reports the pointer entering and leaving the window, each click and
    /// key press in it, and the window getting and losing the keyboard, so
    /// the Call state machine knows when the user is using it.
    fn report_use(&self) {
        let pointer = gtk::EventControllerMotion::new();
        let intents = self.intents.clone();
        pointer.connect_enter(move |_, _, _| {
            let _ = intents.unbounded_send(Input::PointerOverCallWindow(true));
        });
        let intents = self.intents.clone();
        pointer.connect_leave(move |_| {
            let _ = intents.unbounded_send(Input::PointerOverCallWindow(false));
        });
        self.surface.window.add_controller(pointer);
        // Before any control takes the click or the key.
        let click = gtk::GestureClick::new();
        click.set_propagation_phase(gtk::PropagationPhase::Capture);
        let intents = self.intents.clone();
        click.connect_pressed(move |_, _, _, _| {
            let _ = intents.unbounded_send(Input::CallWindowUsed);
        });
        self.surface.window.add_controller(click);
        let keys = gtk::EventControllerKey::new();
        keys.set_propagation_phase(gtk::PropagationPhase::Capture);
        let intents = self.intents.clone();
        keys.connect_key_pressed(move |_, _, _, _| {
            let _ = intents.unbounded_send(Input::CallWindowUsed);
            glib::Propagation::Proceed
        });
        self.surface.window.add_controller(keys);
        let intents = self.intents.clone();
        self.surface.window.connect_is_active_notify(move |window| {
            let _ = intents.unbounded_send(Input::CallWindowKeyboard(window.is_active()));
        });
    }

    fn toggle_mute(&self) {
        let muted = matches!(
            self.view.borrow().as_ref().map(|view| &view.phase),
            Some(CallWindowPhase::Live { mute: Mute::On, .. })
        );
        let _ = self.intents.unbounded_send(Input::SetMuted(!muted));
    }

    /// Asks for the audio where it isn't.
    fn hand_off(&self) {
        let audio = self.view.borrow().as_ref().and_then(|view| view.audio);
        if let Some(audio) = audio {
            let _ = self
                .intents
                .unbounded_send(Input::MoveAudio(match audio.location {
                    AudioLocation::Phone => AudioLocation::Desktop,
                    AudioLocation::Desktop => AudioLocation::Phone,
                }));
        }
    }

    /// Types `tone` at the end of the tone entry, as if from the keyboard.
    fn press_tone(&self, tone: &str) {
        if !self.tones.is_sensitive() {
            return;
        }
        let mut end = i32::try_from(self.tones.text().chars().count()).unwrap_or(i32::MAX);
        self.tones.insert_text(tone, &mut end);
        self.tones.set_position(end);
    }

    /// Sends what was typed or pasted into the tone entry as keypad tones,
    /// and writes the tones into the entry; anything else is refused with
    /// the error bell.
    fn type_tones(&self, text: &gtk::Editable, inserted: &str, position: &mut i32) {
        let Some(tones) = keypad_tones(inserted) else {
            text.error_bell();
            return;
        };
        let _ = self
            .intents
            .unbounded_send(Input::SendTones(inserted.to_owned()));
        self.writing_tones.set(true);
        text.insert_text(&tones, position);
        self.writing_tones.set(false);
        self.update();
    }

    fn start_ticker(self: &Rc<Self>) {
        if self.ticker.borrow().is_some() {
            return;
        }
        let window = Rc::downgrade(self);
        let ticker = glib::timeout_add_local(Duration::from_secs(1), move || {
            let Some(window) = window.upgrade() else {
                return glib::ControlFlow::Break;
            };
            window.update();
            glib::ControlFlow::Continue
        });
        *self.ticker.borrow_mut() = Some(ticker);
    }

    fn stop_ticker(&self) {
        if let Some(ticker) = self.ticker.borrow_mut().take() {
            ticker.remove();
        }
    }

    /// The Call Pill's surface.
    pub(super) fn pill_surface(&self) -> &Surface {
        &self.pill.surface
    }
}

impl StateRow {
    fn new() -> Self {
        let icon = IconView::new(Icon::Computer);
        let text = gtk::Label::new(None);
        text.set_xalign(0.0);
        text.set_wrap(true);
        text.set_max_width_chars(40);
        let hd = gtk::Label::new(Some(messages::label(Label::Hd)));
        hd.add_css_class("hd");
        hd.set_valign(gtk::Align::Center);
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        row.add_css_class(class::STATE_LINE);
        row.append(&icon.area);
        row.append(&text);
        row.append(&hd);
        Self {
            row,
            icon,
            text,
            hd,
        }
    }
}

impl Notice {
    fn new() -> Self {
        let icon = IconView::new(Icon::Warning);
        icon.area.set_valign(gtk::Align::Start);
        icon.area.add_css_class("notice-icon");
        let heading = gtk::Label::new(None);
        heading.add_css_class("notice-heading");
        heading.set_xalign(0.0);
        heading.set_wrap(true);
        heading.set_max_width_chars(36);
        let text = gtk::Label::new(None);
        text.set_xalign(0.0);
        text.set_wrap(true);
        text.set_max_width_chars(36);
        let words = gtk::Box::new(gtk::Orientation::Vertical, 2);
        words.set_hexpand(true);
        words.append(&heading);
        words.append(&text);
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 10);
        row.add_css_class(class::NOTICE);
        row.append(&icon.area);
        row.append(&words);
        row.set_visible(false);
        Self { row, heading, text }
    }
}

/// Whether `widget` and every widget it is in, short of its window, are
/// visible: shown whenever its window is.
fn shows(widget: &impl IsA<gtk::Widget>) -> bool {
    let mut widget = widget.as_ref().clone();
    while let Some(parent) = widget.parent() {
        if !widget.is_visible() {
            return false;
        }
        widget = parent;
    }
    true
}

/// The Call Window's status for a phase without a live Call, whose status
/// is its timer instead, as the Call Pill shows it.
fn status_label(phase: &CallWindowPhase) -> Label {
    match phase {
        CallWindowPhase::Connecting | CallWindowPhase::Live { .. } => Label::Connecting,
        CallWindowPhase::Calling => Label::Calling,
        CallWindowPhase::Ringing => Label::Ringing,
        CallWindowPhase::NotPlaced => Label::CallNotPlaced,
        CallWindowPhase::Ended => Label::CallEnded,
        CallWindowPhase::OnHold { .. } => Label::OnHold,
    }
}

/// Hang up. With no Call yet, hanging up a Call placed here cancels the
/// dial.
fn hang_up_button(surface: &Surface, intents: &UnboundedSender<Input>) -> gtk::Button {
    let hang_up = gtk::Button::new();
    hang_up.add_css_class("hang-up");
    let (call, intents) = (surface.call.clone(), intents.clone());
    hang_up.connect_clicked(move |_| {
        let intent = call.get().map_or(Input::CancelDial, Input::HangUp);
        let _ = intents.unbounded_send(intent);
    });
    hang_up
}

/// The keycap [`show_shortcut`] put on `button`: the last thing on it.
fn hang_up_keycap(button: &gtk::Button) -> Option<gtk::Widget> {
    button.child()?.last_child()
}

/// The three tiles: Mute, Keypad and the audio move, in a row of equal
/// widths. Adds each one's shortcut to `shortcuts`.
fn tiles(
    shortcuts: &mut Shortcuts,
) -> (
    gtk::Box,
    Tile<gtk::Button>,
    Tile<gtk::ToggleButton>,
    Tile<gtk::Button>,
) {
    let (mute, shown, cap) = Tile::new(
        gtk::Button::new(),
        Icon::Microphone,
        Label::Mute,
        MUTE_SHORTCUT,
    );
    mute.button.add_css_class("mute");
    shortcuts.insert("mute-shortcut".to_owned(), (shown, Some(cap)));
    let (keypad_shown, shown, cap) = Tile::new(
        gtk::ToggleButton::new(),
        Icon::Keypad,
        Label::Keypad,
        KEYPAD_SHORTCUT,
    );
    shortcuts.insert("keypad-shortcut".to_owned(), (shown, Some(cap)));
    let (handoff, shown, cap) = Tile::new(
        gtk::Button::new(),
        Icon::Phone,
        Label::ToPhone,
        HANDOFF_SHORTCUT,
    );
    handoff.button.add_css_class("handoff");
    shortcuts.insert("handoff-shortcut".to_owned(), (shown, Some(cap)));
    let tiles = gtk::Box::new(gtk::Orientation::Horizontal, space::S);
    tiles.set_homogeneous(true);
    tiles.append(&mute.button);
    tiles.append(&keypad_shown.button);
    tiles.append(&handoff.button);
    (tiles, mute, keypad_shown, handoff)
}

/// The top line: who, and the timer at the right. Returns the line, who and
/// the timer.
fn top_line() -> (gtk::Box, gtk::Label, gtk::Label) {
    let caller = gtk::Label::new(None);
    caller.add_css_class(class::WHO);
    caller.set_xalign(0.0);
    caller.set_hexpand(true);
    caller.set_ellipsize(gtk::pango::EllipsizeMode::End);
    caller.set_valign(gtk::Align::BaselineFill);
    let timer = gtk::Label::new(None);
    timer.add_css_class(class::TIMER);
    timer.set_xalign(1.0);
    timer.set_valign(gtk::Align::BaselineFill);
    let line = gtk::Box::new(gtk::Orientation::Horizontal, space::S);
    line.append(&caller);
    line.append(&timer);
    (line, caller, timer)
}

/// Why a Call placed here failed or was refused, hidden.
pub(super) fn notice_label() -> gtk::Label {
    let notice = gtk::Label::new(None);
    notice.set_xalign(0.0);
    notice.set_wrap(true);
    notice.set_max_width_chars(36);
    notice.set_visible(false);
    notice
}

/// The tone entry, and the keypad's buttons, hidden until the keypad opens.
fn keypad() -> (gtk::Entry, gtk::Grid) {
    let tones = gtk::Entry::new();
    tones.add_css_class(class::TONES);
    tones.set_placeholder_text(Some(messages::label(Label::KeypadPlaceholder)));
    tones.set_input_purpose(gtk::InputPurpose::Phone);
    tones.set_visible(false);
    let keypad = gtk::Grid::new();
    keypad.add_css_class(class::KEYPAD);
    keypad.set_row_spacing(6);
    keypad.set_column_spacing(6);
    keypad.set_row_homogeneous(true);
    keypad.set_column_homogeneous(true);
    keypad.set_visible(false);
    (tones, keypad)
}

/// Adds what the Call Pill's Mute and its notices show for `mute`, with
/// Mute saying `mute_text`, to `content`.
fn report_mute(mute: Mute, mute_text: &str, content: &mut BTreeMap<String, String>) {
    content.insert("mute".to_owned(), mute_text.to_owned());
    match mute {
        Mute::On => {
            content.insert(
                "muted".to_owned(),
                messages::label(Label::MicrophoneMuted).to_owned(),
            );
        }
        Mute::Unavailable => {
            content.insert(
                "mute-unavailable".to_owned(),
                messages::failure(Failure::MuteUnavailable).text.to_owned(),
            );
        }
        Mute::Off => {}
    }
}

/// The shortcut hint [`show_shortcut`] put beside `text`, right after it.
fn hint(text: &gtk::Label) -> Option<gtk::Widget> {
    text.next_sibling()
}

/// Puts `icon`, if any, `text` and `accelerator`'s keycap on `button`, and
/// returns the key as shown.
fn show_shortcut(
    button: &impl IsA<gtk::Button>,
    icon: Option<Icon>,
    text: &gtk::Label,
    accelerator: &str,
) -> String {
    let (shortcut, shown) = keycap(accelerator);
    let label = gtk::Box::new(gtk::Orientation::Horizontal, space::S);
    label.set_halign(gtk::Align::Center);
    if let Some(icon) = icon {
        label.append(&IconView::new(icon).area);
    }
    label.append(text);
    label.append(&shortcut);
    button.as_ref().set_child(Some(&label));
    button.as_ref().set_hexpand(true);
    shown
}
