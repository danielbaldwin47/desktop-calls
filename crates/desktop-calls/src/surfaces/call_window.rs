//! The Call Window: the Call's caller, status and audio, and its actions with
//! their shortcuts.

use super::{
    HANDOFF_SHORTCUT, HANG_UP_HELD_SHORTCUT, HANG_UP_SHORTCUT, KEYPAD, KEYPAD_SHORTCUT,
    MUTE_SHORTCUT, SWAP_SHORTCUT, SharedReport, Surface, Who, class, detail_label, keycap,
    seconds_since, set_class,
};
use desktop_calls_core::messages::{self, Failure, Label};
use desktop_calls_core::{
    AudioLocation, CallAudio, CallWindowPhase, CallWindowView, Input, Mute, keypad_tones,
};
use futures_channel::mpsc::UnboundedSender;
use gtk::glib::{self, SourceId};
use gtk::prelude::*;
use gtk4_layer_shell::{KeyboardMode, Layer};
use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;
use std::rc::Rc;
use std::time::Duration;

mod held;
mod pill;

use held::HeldRow;
use pill::CallPill;

/// The Call Window.
pub(super) struct CallWindow {
    pub(super) surface: Surface,
    intents: UnboundedSender<Input>,
    caller: gtk::Label,
    detail: gtk::Label,
    status: gtk::Label,
    /// Where the audio is, and the Handoff button.
    audio: AudioRow,
    hd: gtk::Label,
    mute: gtk::Button,
    mute_label: gtk::Label,
    keypad_shown: gtk::ToggleButton,
    hang_up: gtk::Button,
    hang_up_label: gtk::Label,
    /// The other Call, on hold, with Swap.
    held: HeldRow,
    /// Why a Call placed here failed or was refused.
    notice: gtk::Label,
    /// The strip that says the microphone is muted.
    microphone_muted: gtk::Label,
    /// Why Mute is disabled, while it is.
    mute_unavailable: gtk::Label,
    /// Where keypad tones are typed or pasted, and which shows those sent.
    tones: gtk::Entry,
    keypad: gtk::Grid,
    /// Each shortcut as the window shows it, by the action's report key,
    /// with the hint that shows it on its button.
    shortcuts: BTreeMap<String, (String, Option<gtk::Widget>)>,
    /// Set while the window writes the tones it sent into `tones`, so the
    /// write isn't taken for typing.
    writing_tones: Cell<bool>,
    /// The once-a-second update of the Call timer, while it runs.
    ticker: RefCell<Option<SourceId>>,
    view: Rc<RefCell<Option<CallWindowView>>>,
    /// Called whenever the window may have changed height, so the Banner
    /// stays below it.
    resized: RefCell<Option<Box<dyn Fn()>>>,
    /// What the window shows as while it is collapsed.
    pill: CallPill,
}

impl CallWindow {
    pub(super) fn new(intents: &UnboundedSender<Input>, report: SharedReport) -> Rc<Self> {
        let surface = Surface::new(
            "call-window",
            Layer::Top,
            KeyboardMode::OnDemand,
            Some(Input::CollapseCallWindow),
            intents,
            report.clone(),
        );
        let (heading, caller, hd) = heading();
        let detail = detail_label();
        let status = gtk::Label::new(None);
        status.add_css_class(class::TIMER);
        status.set_xalign(0.0);
        let mut shortcuts = BTreeMap::new();
        let audio = AudioRow::new();
        shortcuts.insert(
            "handoff-shortcut".to_owned(),
            (audio.shortcut.clone(), hint(&audio.handoff_label)),
        );
        let mute = gtk::Button::new();
        mute.add_css_class("mute");
        let mute_label = gtk::Label::new(Some(messages::label(Label::Mute)));
        let shown = show_shortcut(&mute, &mute_label, MUTE_SHORTCUT);
        shortcuts.insert("mute-shortcut".to_owned(), (shown, hint(&mute_label)));
        let keypad_shown = gtk::ToggleButton::new();
        let keypad_label = gtk::Label::new(Some(messages::label(Label::Keypad)));
        let shown = show_shortcut(&keypad_shown, &keypad_label, KEYPAD_SHORTCUT);
        shortcuts.insert("keypad-shortcut".to_owned(), (shown, hint(&keypad_label)));
        let controls = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        controls.set_homogeneous(true);
        controls.append(&mute);
        controls.append(&keypad_shown);
        let hang_up = hang_up_button(&surface, intents);
        let hang_up_label = gtk::Label::new(Some(messages::label(Label::HangUp)));
        let shown = show_shortcut(&hang_up, &hang_up_label, HANG_UP_SHORTCUT);
        shortcuts.insert("hang-up-shortcut".to_owned(), (shown, hint(&hang_up_label)));

        let held = HeldRow::new(intents);
        let notice = notice_label();
        let (microphone_muted, mute_unavailable) = mute_notices();
        let (tones, keypad) = keypad();

        let content = gtk::Box::new(gtk::Orientation::Vertical, 8);
        content.append(&heading);
        content.append(&detail);
        content.append(&status);
        content.append(&audio.row);
        // Everything that comes and goes sits below Hang up, so Hang up
        // never moves under the pointer.
        content.append(&controls);
        content.append(&hang_up);
        content.append(&held.row);
        content.append(&notice);
        content.append(&microphone_muted);
        content.append(&mute_unavailable);
        content.append(&tones);
        content.append(&keypad);
        surface.window.set_child(Some(&content));
        for button in [
            mute.upcast_ref::<gtk::Button>(),
            keypad_shown.upcast_ref(),
            &audio.handoff,
            &hang_up,
        ] {
            // Clicking leaves the keyboard with the tone entry.
            button.set_focus_on_click(false);
        }

        let window = Rc::new(Self {
            surface,
            intents: intents.clone(),
            caller,
            detail,
            status,
            audio,
            hd,
            mute,
            mute_label,
            keypad_shown,
            hang_up,
            hang_up_label,
            held,
            notice,
            microphone_muted,
            mute_unavailable,
            tones,
            keypad,
            shortcuts,
            writing_tones: Cell::new(false),
            ticker: RefCell::default(),
            view: Rc::default(),
            resized: RefCell::default(),
            pill: CallPill::new(intents, report),
        });
        window.connect();
        window
    }

    /// Wires the controls, the keypad and the shortcuts to the window.
    fn connect(self: &Rc<Self>) {
        let weak = Rc::downgrade(self);
        self.mute.connect_clicked(move |_| {
            if let Some(window) = weak.upgrade() {
                window.toggle_mute();
            }
        });
        let weak = Rc::downgrade(self);
        self.audio.handoff.connect_clicked(move |_| {
            if let Some(window) = weak.upgrade() {
                window.hand_off();
            }
        });
        let weak = Rc::downgrade(self);
        self.keypad_shown.connect_toggled(move |toggle| {
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

        let shortcuts = gtk::ShortcutController::new();
        // Before any control sees the key, so M and K never reach the tone
        // entry.
        shortcuts.set_propagation_phase(gtk::PropagationPhase::Capture);
        let actions: [(&str, gtk::Widget); 6] = [
            (MUTE_SHORTCUT, self.mute.clone().upcast()),
            (KEYPAD_SHORTCUT, self.keypad_shown.clone().upcast()),
            (HANDOFF_SHORTCUT, self.audio.handoff.clone().upcast()),
            (HANG_UP_SHORTCUT, self.hang_up.clone().upcast()),
            (SWAP_SHORTCUT, self.held.swap.clone().upcast()),
            (HANG_UP_HELD_SHORTCUT, self.held.hang_up.clone().upcast()),
        ];
        for (accelerator, control) in actions {
            // Disabled controls ignore it, and the key is still taken.
            // `activate` would press a greyed-out button anyway.
            let action = gtk::CallbackAction::new(move |_, _| {
                if control.is_sensitive() {
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

        // A tone typed while a button has the keyboard goes to the keypad
        // too.
        let keys = gtk::EventControllerKey::new();
        let weak = Rc::downgrade(self);
        keys.connect_key_pressed(move |_, key, _, modifiers| {
            let plain = !modifiers.intersects(
                gtk::gdk::ModifierType::CONTROL_MASK | gtk::gdk::ModifierType::ALT_MASK,
            );
            let tone = key
                .to_unicode()
                .filter(|character| plain && keypad_tones(&character.to_string()).is_some());
            match (weak.upgrade(), tone) {
                (Some(window), Some(tone)) => {
                    window.press_tone(&tone.to_string());
                    glib::Propagation::Stop
                }
                _ => glib::Propagation::Proceed,
            }
        });
        self.surface.window.add_controller(keys);
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
}

/// The Call Window's status for a phase without a live Call, whose status
/// is its timer instead.
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

/// The Call Window's row that says where the audio is, with the Handoff
/// button beside it.
struct AudioRow {
    row: gtk::Box,
    audio_location: gtk::Label,
    /// Moves the audio to where its label says.
    handoff: gtk::Button,
    handoff_label: gtk::Label,
    /// The Handoff button's shortcut, as shown.
    shortcut: String,
}

impl AudioRow {
    fn new() -> Self {
        let audio_location = gtk::Label::new(None);
        audio_location.add_css_class("dim-label");
        audio_location.set_xalign(0.0);
        audio_location.set_hexpand(true);
        // As wide as its widest text, and the Handoff button as wide as its
        // widest label, so the window never changes width under the pointer
        // as the audio moves.
        audio_location.set_width_chars(widest(&[Label::AudioOnPhone, Label::AudioOnDesktop]));
        let handoff = gtk::Button::new();
        handoff.add_css_class("handoff");
        let handoff_label = gtk::Label::new(Some(messages::label(Label::MoveAudioHere)));
        handoff_label.set_width_chars(widest(&[
            Label::MoveAudioHere,
            Label::MoveAudioToPhone,
            Label::MovingAudioHere,
            Label::MovingAudioToPhone,
        ]));
        let shortcut = show_shortcut(&handoff, &handoff_label, HANDOFF_SHORTCUT);
        handoff.set_hexpand(false);
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        row.append(&audio_location);
        row.append(&handoff);
        Self {
            row,
            audio_location,
            handoff,
            handoff_label,
            shortcut,
        }
    }
}

/// How many characters the longest of `labels` has.
fn widest(labels: &[Label]) -> i32 {
    labels
        .iter()
        .map(|label| messages::label(*label).chars().count())
        .max()
        .map_or(0, |widest| i32::try_from(widest).unwrap_or(i32::MAX))
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

/// The heading row: the caller, and HD beside it. Returns the row, the
/// caller and HD.
fn heading() -> (gtk::Box, gtk::Label, gtk::Label) {
    let caller = gtk::Label::new(None);
    caller.add_css_class(class::WHO);
    caller.set_xalign(0.0);
    let hd = gtk::Label::new(Some(messages::label(Label::Hd)));
    hd.add_css_class("hd");
    let heading = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    heading.append(&caller);
    heading.append(&hd);
    (heading, caller, hd)
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

/// The tone entry, and the keypad's buttons, hidden.
fn keypad() -> (gtk::Entry, gtk::Grid) {
    let tones = gtk::Entry::new();
    tones.set_placeholder_text(Some(messages::label(Label::KeypadPlaceholder)));
    tones.set_input_purpose(gtk::InputPurpose::Phone);
    let keypad = gtk::Grid::new();
    keypad.set_row_spacing(6);
    keypad.set_column_spacing(6);
    keypad.set_row_homogeneous(true);
    keypad.set_column_homogeneous(true);
    keypad.set_visible(false);
    (tones, keypad)
}

/// The strip that says the microphone is muted, and the note that says why
/// Mute is disabled, both hidden.
fn mute_notices() -> (gtk::Label, gtk::Label) {
    let microphone_muted = gtk::Label::new(Some(messages::label(Label::MicrophoneMuted)));
    microphone_muted.add_css_class(class::WARNING_STRIP);
    microphone_muted.set_visible(false);
    let mute_unavailable = gtk::Label::new(Some(messages::failure(Failure::MuteUnavailable).text));
    mute_unavailable.add_css_class("dim-label");
    mute_unavailable.set_wrap(true);
    mute_unavailable.set_xalign(0.0);
    mute_unavailable.set_visible(false);
    (microphone_muted, mute_unavailable)
}

/// Adds what Mute and its notices show for `mute`, with Mute saying
/// `mute_text`, to `content`.
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

/// Puts `text` and `accelerator`'s label on `button`, and returns the label
/// as shown.
fn show_shortcut(button: &impl IsA<gtk::Button>, text: &gtk::Label, accelerator: &str) -> String {
    let (shortcut, shown) = keycap(accelerator);
    let label = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    label.set_halign(gtk::Align::Center);
    label.append(text);
    label.append(&shortcut);
    button.as_ref().set_child(Some(&label));
    button.as_ref().set_hexpand(true);
    shown
}

impl CallWindow {
    pub(super) fn render(self: &Rc<Self>, view: Option<&CallWindowView>) {
        let was_live = matches!(
            self.view.borrow().as_ref().map(|view| &view.phase),
            Some(CallWindowPhase::Live { .. })
        );
        let new_call =
            self.view.borrow().as_ref().map(|view| view.call) != view.map(|view| view.call);
        *self.view.borrow_mut() = view.cloned();
        if new_call {
            self.tones.set_text("");
            self.keypad_shown.set_active(false);
        }
        let Some(view) = view else {
            self.stop_ticker();
            self.surface.hide();
            self.pill.hide();
            return;
        };
        let live = matches!(view.phase, CallWindowPhase::Live { .. });
        if live {
            self.start_ticker();
        } else {
            self.stop_ticker();
        }
        if view.collapsed {
            // The window keeps its keypad and tones for when it expands.
            if self.surface.window.is_visible() {
                self.surface.hide();
            }
            self.update();
            return;
        }
        self.pill.hide();
        self.surface.call.set(view.call);
        self.update();
        if live && !was_live {
            // Once the update has enabled it: a disabled entry can't take
            // the keyboard.
            self.tones.grab_focus();
        }
        self.surface.focus(view.focused);
    }

    /// Brings every label up to date with the view and the clock.
    fn update(&self) {
        let Some(view) = self.view.borrow().clone() else {
            return;
        };
        if view.collapsed {
            self.pill.render(&view);
            return;
        }
        let who = Who::of(&view.caller, view.contact.as_ref());
        who.show(&self.caller);
        who.show_detail(&self.detail);
        let (status, wideband, mute) = match view.phase {
            CallWindowPhase::Live {
                active_since,
                wideband,
                mute,
            } => (
                messages::elapsed(seconds_since(active_since)),
                wideband,
                Some(mute),
            ),
            phase => (
                messages::label(status_label(&phase)).to_owned(),
                false,
                None,
            ),
        };
        self.status.set_text(&status);
        set_class(&self.status, class::WORDS, mute.is_none());
        let audio_location = view.audio.map(|audio| {
            messages::label(match audio.location {
                AudioLocation::Desktop => Label::AudioOnDesktop,
                AudioLocation::Phone => Label::AudioOnPhone,
            })
        });
        // The row stays, empty, while there is nothing to say, so Hang up
        // never moves under the pointer when the Call connects.
        self.audio
            .audio_location
            .set_text(audio_location.unwrap_or_default());
        let handoff = self.show_handoff(view.audio);
        self.hd.set_visible(wideband);
        // While a Call is on hold and a Waiting Call rings, the Phone would
        // answer or decline the Waiting Call instead; the held row says so.
        self.hang_up.set_sensitive(
            !view.waiting_first
                && !view.connection_lost
                && !matches!(
                    view.phase,
                    CallWindowPhase::Ended | CallWindowPhase::NotPlaced
                ),
        );
        let hang_up_on_phone = self.show_hang_up(view.connection_lost);
        let notice = view.notice.map(|failure| messages::failure(failure).text);
        self.notice.set_text(notice.unwrap_or_default());
        self.notice.set_visible(notice.is_some());
        // With contact lost, nothing in the window works; the notice says
        // why, without Mute's hint beside it.
        let mute = mute.filter(|_| !view.connection_lost);
        let mute_text = self.show_controls(mute, view.connection_lost);
        set_class(&self.surface.window, class::MUTED, mute == Some(Mute::On));
        set_class(
            &self.surface.window,
            class::CONTACT_LOST,
            view.connection_lost,
        );

        let mut content = BTreeMap::from([("status".to_owned(), status)]);
        if let Some(call) = view.call {
            content.insert("call".to_owned(), call.to_string());
        }
        if let Some(notice) = notice {
            content.insert("notice".to_owned(), notice.to_owned());
        }
        who.report(&mut content);
        if let Some(hang_up_on_phone) = hang_up_on_phone {
            content.insert("hang-up".to_owned(), hang_up_on_phone.to_owned());
            content.insert("mute".to_owned(), messages::label(Label::Mute).to_owned());
        }
        // Each shortcut whose hint is on screen.
        content.extend(
            self.shortcuts
                .iter()
                .filter(|(_, (_, hint))| hint.as_ref().is_none_or(WidgetExt::is_visible))
                .map(|(key, (shown, _))| (key.clone(), shown.clone())),
        );
        if view.waiting_first {
            // Greyed out, Hang up has no shortcut to report.
            content.remove("hang-up-shortcut");
        }
        if let Some(audio_location) = audio_location {
            content.insert("audio-location".to_owned(), audio_location.to_owned());
            content.insert("handoff".to_owned(), handoff.to_owned());
        }
        if wideband {
            content.insert("hd".to_owned(), messages::label(Label::Hd).to_owned());
        }
        if let Some(mute) = mute {
            report_mute(mute, mute_text, &mut content);
        }
        self.report_tones(&mut content);
        self.held.update(&view, &mut content);
        self.surface.show(content);
        if let Some(resized) = self.resized.borrow().as_ref() {
            resized();
        }
    }

    /// Reports the tone entry, the keypad toggle and the keypad while they
    /// show, and the tones typed while the entry shows.
    fn report_tones(&self, content: &mut BTreeMap<String, String>) {
        // The toggle keeps its place while hidden, so it counts as hidden
        // only when it can't be seen, pressed or reached with the keyboard:
        // any one of those left is the toggle still there.
        let toggle_there = self.keypad_shown.opacity() > 0.0
            || self.keypad_shown.can_target()
            || self.keypad_shown.can_focus();
        for (key, shown) in [
            ("tone-entry", WidgetExt::is_visible(&self.tones)),
            ("keypad-toggle", toggle_there),
            ("keypad", WidgetExt::is_visible(&self.keypad)),
        ] {
            if shown {
                content.insert(key.to_owned(), "shown".to_owned());
            }
        }
        let tones = self.tones.text();
        if WidgetExt::is_visible(&self.tones) && !tones.is_empty() {
            content.insert("tones".to_owned(), tones.to_string());
        }
    }

    /// The Call Pill's surface.
    pub(super) fn pill_surface(&self) -> &Surface {
        &self.pill.surface
    }

    /// Calls `resized` whenever the window may have changed height.
    pub(super) fn on_resized(&self, resized: impl Fn() + 'static) {
        *self.resized.borrow_mut() = Some(Box::new(resized));
    }

    /// Brings the Handoff button up to date with the Call's audio, `None`
    /// with no live Call, and returns its label. It names where the audio
    /// will go, and is disabled while the audio is on its way.
    fn show_handoff(&self, audio: Option<CallAudio>) -> &'static str {
        let label = messages::label(match audio {
            Some(CallAudio {
                moving: Some(AudioLocation::Desktop),
                ..
            }) => Label::MovingAudioHere,
            Some(CallAudio {
                moving: Some(AudioLocation::Phone),
                ..
            }) => Label::MovingAudioToPhone,
            Some(CallAudio {
                location: AudioLocation::Desktop,
                ..
            }) => Label::MoveAudioToPhone,
            Some(CallAudio {
                location: AudioLocation::Phone,
                ..
            })
            | None => Label::MoveAudioHere,
        });
        self.audio.handoff_label.set_text(label);
        self.audio
            .handoff
            .set_sensitive(audio.is_some_and(|audio| audio.moving.is_none()));
        label
    }

    /// Brings Hang up's look up to date: while the Desktop has lost contact
    /// with the Call, a plain button saying to hang up on the Phone, and no
    /// control shows a shortcut, as none works. Returns that label then.
    fn show_hang_up(&self, connection_lost: bool) -> Option<&'static str> {
        let label = messages::label(if connection_lost {
            Label::HangUpOnPhone
        } else {
            Label::HangUp
        });
        self.hang_up_label.set_text(label);
        for hint in self
            .shortcuts
            .values()
            .filter_map(|(_, hint)| hint.as_ref())
        {
            hint.set_visible(!connection_lost);
        }
        // Not red: it isn't a Hang up that works.
        if connection_lost {
            self.hang_up.remove_css_class("hang-up");
        } else {
            self.hang_up.add_css_class("hang-up");
        }
        connection_lost.then_some(label)
    }

    /// Brings Mute, its notices and the keypad up to date with `mute`, which
    /// is `None` with no live Call, and returns Mute's label. While the
    /// Desktop has lost contact with the Call, the tone entry and the keypad
    /// are hidden, and so is the keypad toggle: no tone can be sent. The
    /// entry and the keypad sit below Hang up. The toggle shares Mute's row,
    /// above it, so it keeps its place, invisible, and Mute doesn't stretch.
    fn show_controls(&self, mute: Option<Mute>, connection_lost: bool) -> &'static str {
        let live = mute.is_some();
        let mute_text = messages::label(if mute == Some(Mute::On) {
            Label::Unmute
        } else {
            Label::Mute
        });
        self.mute_label.set_text(mute_text);
        self.mute
            .set_sensitive(matches!(mute, Some(Mute::Off | Mute::On)));
        set_class(&self.mute, class::MUTED, mute == Some(Mute::On));
        self.microphone_muted.set_visible(mute == Some(Mute::On));
        self.mute_unavailable
            .set_visible(mute == Some(Mute::Unavailable));
        let contact_back = !connection_lost && !WidgetExt::is_visible(&self.tones);
        self.keypad_shown
            .set_opacity(if connection_lost { 0.0 } else { 1.0 });
        self.keypad_shown.set_can_target(!connection_lost);
        self.keypad_shown.set_can_focus(!connection_lost);
        self.keypad_shown
            .update_state(&[gtk::accessible::State::Hidden(connection_lost)]);
        self.tones.set_visible(!connection_lost);
        self.keypad
            .set_visible(!connection_lost && self.keypad_shown.is_active());
        self.keypad_shown.set_sensitive(live);
        self.tones.set_sensitive(live);
        self.keypad.set_sensitive(live);
        if live && contact_back {
            // Once enabled, as when the Call went live: the keyboard
            // returns to the tone entry, without selecting the tones typed
            // so far, which the next one would replace.
            self.tones.grab_focus_without_selecting();
        }
        mute_text
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
}
