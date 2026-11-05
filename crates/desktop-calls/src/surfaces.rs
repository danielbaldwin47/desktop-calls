//! The Desktop surfaces: the Incoming Call Banner and the Call Window, as
//! GTK4 and libadwaita windows on the layer shell.
//!
//! They render what the Call state machine says to show, send the user's
//! clicks back as intents, and report what is actually on screen (a surface
//! counts as shown once the compositor has mapped it) for the control
//! interface's `Surfaces`.
//!
//! - The Banner is an overlay surface, so it shows above fullscreen windows.
//!   It takes keyboard focus only when the keybinding asks; then A answers,
//!   D declines and Escape gives focus back. It sits top-right, below Omarchy's bar
//!   (the compositor keeps it out of the bar's exclusive zone) and to the
//!   left of Omarchy's notification-toast column, as wide as Hyprland says
//!   its `omarchy-notifications` layer is, or [`FALLBACK_TOAST_COLUMN_WIDTH`].
//!   It is unmapped whenever nothing rings: a mapped overlay surface stops
//!   Hyprland scanning a fullscreen window out directly.
//! - The Call Window is on the top layer, so fullscreen windows cover it, in
//!   the same corner, and takes keyboard focus only when clicked or when the
//!   keybinding asks; Escape gives it back. Each of its actions has a
//!   shortcut, shown on its button. Digits, `*` and `#` typed or pasted
//!   anywhere in it are keypad tones.
//! - Escape only ever gives keyboard focus back. It never ends or declines a
//!   Call.
//! - Each surface reports `focused` while it has keyboard focus, as the
//!   compositor says.
//! - No output is chosen, so the compositor puts each on the focused output.
//! - Where the compositor has no layer shell, each is an ordinary window.

use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;
use std::rc::Rc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use desktop_calls_core::messages::{self, Failure, Label};
use desktop_calls_core::{
    AudioLocation, BannerView, CallAudio, CallId, CallWindowPhase, CallWindowView, Caller,
    ContactMatch, Input, Mute, Timestamp, keypad_tones,
};
use futures_channel::mpsc::UnboundedSender;
use gtk::glib::{self, SourceId};
use gtk::prelude::*;
use gtk4_layer_shell::{Edge, KeyboardMode, Layer, LayerShell as _};

use crate::control::ShownSurfaces;
use crate::hyprland::{self, Unknown};
use crate::logging::EventKind;
use crate::theme::Theme;

/// How wide Omarchy's notification-toast column is taken to be when
/// Hyprland can't say. Omarchy 4.5's is 394.
const FALLBACK_TOAST_COLUMN_WIDTH: i32 = 400;
/// Space between a surface and what it sits next to.
const GAP: i32 = 12;
const WIDTH: i32 = 340;

/// The Call Window's shortcuts, as GTK accelerators. Hang up needs Control,
/// so a stray key can't end a Call.
const MUTE_SHORTCUT: &str = "m";
const KEYPAD_SHORTCUT: &str = "k";
const HANG_UP_SHORTCUT: &str = "<Control>h";
const HANDOFF_SHORTCUT: &str = "a";

/// The keypad's buttons, row by row, as on a phone.
const KEYPAD: [[&str; 3]; 4] = [
    ["1", "2", "3"],
    ["4", "5", "6"],
    ["7", "8", "9"],
    ["*", "0", "#"],
];

/// The surfaces' layout, and the orange that marks the microphone muted,
/// unlike any green or red. The rest of their colours and their corners come
/// from [`Theme`].
const CSS: &str = "
.desktop-calls-surface { padding: 16px; }
.desktop-calls-surface .microphone-muted {
    background: #e66100; color: #ffffff; font-weight: bold;
    padding: 8px; border-radius: 6px;
}
.desktop-calls-surface button.mute.muted { background: #e66100; color: #ffffff; }
.desktop-calls-surface .shortcut { font-size: 0.85em; opacity: 0.75; }
.desktop-calls-surface .caller { font-size: 1.4em; font-weight: bold; }
.desktop-calls-surface .hd {
    font-size: 0.8em; font-weight: bold; padding: 0 6px;
    border: 1px solid currentColor; border-radius: 4px;
}
";

/// The surfaces, once GTK is up.
pub struct Surfaces {
    banner: Banner,
    call_window: Rc<CallWindow>,
    _theme: Option<Theme>,
}

/// What the surfaces show, and where it is reported.
struct Report {
    shown: ShownSurfaces,
    reports: UnboundedSender<ShownSurfaces>,
}

impl Report {
    fn set(&mut self, surface: &str, content: Option<BTreeMap<String, String>>) {
        let changed = match content {
            Some(content) => {
                self.shown.insert(surface.to_owned(), content.clone()) != Some(content)
            }
            None => self.shown.remove(surface).is_some(),
        };
        if changed {
            let _ = self.reports.unbounded_send(self.shown.clone());
        }
    }
}

type SharedReport = Rc<RefCell<Report>>;

impl Surfaces {
    /// Builds the surfaces, hidden. Clicks go to `intents`, and what is shown
    /// to `reports`.
    pub fn new(intents: &UnboundedSender<Input>, reports: UnboundedSender<ShownSurfaces>) -> Self {
        let theme = gtk::gdk::Display::default().map(|display| {
            let provider = gtk::CssProvider::new();
            provider.load_from_string(CSS);
            gtk::style_context_add_provider_for_display(
                &display,
                &provider,
                gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
            );
            Theme::follow(&display)
        });
        let report = Rc::new(RefCell::new(Report {
            shown: ShownSurfaces::new(),
            reports,
        }));
        let surfaces = Self {
            banner: Banner::new(intents, report.clone()),
            call_window: CallWindow::new(intents, report),
            _theme: theme,
        };
        surfaces.place_beside_toasts();
        surfaces
    }

    /// Shows, updates or hides each surface to match `view`.
    pub fn render(&self, view: &desktop_calls_core::Surfaces) {
        if view.banner.is_some() && !self.banner.surface.window.is_visible() {
            // The toast column may have changed since; the Banner moves if so.
            self.place_beside_toasts();
        }
        self.banner.render(view.banner.as_ref());
        self.call_window.render(view.call_window.as_ref());
    }

    /// Asks Hyprland, off the main thread, how wide the toast column is, and
    /// moves both surfaces to its left.
    fn place_beside_toasts(&self) {
        let windows = [
            self.banner.surface.window.clone(),
            self.call_window.surface.window.clone(),
        ];
        glib::MainContext::default().spawn_local(async move {
            let width = gtk::gio::spawn_blocking(hyprland::toast_column_width)
                .await
                .unwrap_or(Err(Unknown::NotHyprland));
            let width = match width {
                Ok(width) => i32::try_from(width).unwrap_or(FALLBACK_TOAST_COLUMN_WIDTH),
                Err(unknown) => {
                    toast_column_unknown(&unknown);
                    FALLBACK_TOAST_COLUMN_WIDTH
                }
            };
            if gtk4_layer_shell::is_supported() {
                for window in windows {
                    window.set_margin(Edge::Right, width + GAP);
                }
            }
        });
    }
}

fn toast_column_unknown(unknown: &Unknown) {
    match unknown {
        Unknown::NotHyprland | Unknown::NoToastLayer | Unknown::NoRounding => tracing::debug!(
            event_kind = %EventKind::ToastColumnUnknown,
            error_kind = unknown.as_str(),
            "No toast column to keep clear of; allowing {FALLBACK_TOAST_COLUMN_WIDTH} px"
        ),
        Unknown::Unreachable(error) => tracing::warn!(
            event_kind = %EventKind::ToastColumnUnknown,
            error_kind = unknown.as_str(),
            detail = %error,
            "Hyprland didn't say how wide the toast column is; allowing {FALLBACK_TOAST_COLUMN_WIDTH} px"
        ),
        Unknown::Unreadable(error) => tracing::warn!(
            event_kind = %EventKind::ToastColumnUnknown,
            error_kind = unknown.as_str(),
            detail = %error,
            "Couldn't read Hyprland's layers; allowing {FALLBACK_TOAST_COLUMN_WIDTH} px"
        ),
    }
}

/// A surface's window, with what it shows and whether it is on screen.
struct Surface {
    name: &'static str,
    window: gtk::Window,
    content: Rc<RefCell<BTreeMap<String, String>>>,
    mapped: Rc<Cell<bool>>,
    report: SharedReport,
    call: Rc<Cell<Option<CallId>>>,
    /// How the surface takes keyboard focus when nobody asked it to.
    keyboard: KeyboardMode,
    /// Escape gave keyboard focus back, and the compositor hasn't yet taken
    /// it: until then the surface takes no focus at all, not even on demand.
    releasing: Rc<Cell<bool>>,
}

/// What a surface reports: its content, and `focused` while it has keyboard
/// focus.
fn reported(content: &BTreeMap<String, String>, window: &gtk::Window) -> BTreeMap<String, String> {
    let mut reported = content.clone();
    if window.is_active() {
        reported.insert("focused".to_owned(), "true".to_owned());
    }
    reported
}

impl Surface {
    fn new(
        name: &'static str,
        layer: Layer,
        keyboard: KeyboardMode,
        intents: &UnboundedSender<Input>,
        report: SharedReport,
    ) -> Self {
        let window = gtk::Window::new();
        window.set_title(Some("Desktop Calls"));
        window.set_resizable(false);
        window.set_default_size(WIDTH, -1);
        window.add_css_class("desktop-calls-surface");
        if gtk4_layer_shell::is_supported() {
            window.init_layer_shell();
            window.set_namespace(Some(&format!("desktop-calls-{name}")));
            window.set_layer(layer);
            window.set_keyboard_mode(keyboard);
            window.set_anchor(Edge::Top, true);
            window.set_anchor(Edge::Right, true);
            // The default exclusive zone keeps the surface out of the bar's;
            // the margin is only the gap below it.
            window.set_margin(Edge::Top, GAP);
            window.set_margin(Edge::Right, FALLBACK_TOAST_COLUMN_WIDTH + GAP);
        }
        let surface = Self {
            name,
            window,
            content: Rc::default(),
            mapped: Rc::default(),
            report,
            call: Rc::default(),
            keyboard,
            releasing: Rc::default(),
        };
        let (content, mapped, report) = (
            surface.content.clone(),
            surface.mapped.clone(),
            surface.report.clone(),
        );
        surface.window.connect_map(move |window| {
            mapped.set(true);
            report
                .borrow_mut()
                .set(name, Some(reported(&content.borrow(), window)));
        });
        let (content, mapped, report, releasing) = (
            surface.content.clone(),
            surface.mapped.clone(),
            surface.report.clone(),
            surface.releasing.clone(),
        );
        surface.window.connect_is_active_notify(move |window| {
            if !window.is_active() && releasing.replace(false) && gtk4_layer_shell::is_supported() {
                window.set_keyboard_mode(keyboard);
            }
            if mapped.get() {
                report
                    .borrow_mut()
                    .set(name, Some(reported(&content.borrow(), window)));
            }
        });
        surface.on_escape(intents);
        let (mapped, report) = (surface.mapped.clone(), surface.report.clone());
        surface.window.connect_unmap(move |_| {
            mapped.set(false);
            report.borrow_mut().set(name, None);
        });
        // Pre-warm: realising creates the window's renderer, which costs
        // GTK most of a second the first time (it probes Vulkan, then GL).
        // Doing it now, hidden, lets the Banner show within a second of the
        // Phone ringing.
        WidgetExt::realize(&surface.window);
        surface
    }

    /// Shows the window with `content`, or updates what it reports.
    fn show(&self, content: BTreeMap<String, String>) {
        let unchanged = *self.content.borrow() == content;
        *self.content.borrow_mut() = content;
        if !self.window.is_visible() {
            self.window.present();
        } else if self.mapped.get() && !unchanged {
            self.report.borrow_mut().set(
                self.name,
                Some(reported(&self.content.borrow(), &self.window)),
            );
        }
    }

    fn hide(&self) {
        self.call.set(None);
        self.releasing.set(false);
        self.window.set_visible(false);
    }

    /// Takes keyboard focus while `focused`, as the keybinding asked;
    /// otherwise takes it only as the surface usually does.
    fn focus(&self, focused: bool) {
        if !gtk4_layer_shell::is_supported() {
            if focused && self.window.is_visible() {
                self.window.present();
            }
            return;
        }
        let mode = if focused {
            self.releasing.set(false);
            KeyboardMode::Exclusive
        } else if self.releasing.get() {
            KeyboardMode::None
        } else {
            self.keyboard
        };
        if self.window.keyboard_mode() != mode {
            self.window.set_keyboard_mode(mode);
        }
    }

    /// Escape gives keyboard focus back, however the surface got it, and
    /// tells the Call state machine so.
    fn on_escape(&self, intents: &UnboundedSender<Input>) {
        let keys = gtk::EventControllerKey::new();
        keys.set_propagation_phase(gtk::PropagationPhase::Capture);
        let (window, releasing, intents) =
            (self.window.clone(), self.releasing.clone(), intents.clone());
        keys.connect_key_pressed(move |_, key, _, modifiers| {
            if key != gtk::gdk::Key::Escape || has_command_modifier(modifiers) {
                return glib::Propagation::Proceed;
            }
            if gtk4_layer_shell::is_supported() {
                releasing.set(true);
                window.set_keyboard_mode(KeyboardMode::None);
            }
            let _ = intents.unbounded_send(Input::ReleaseFocus);
            glib::Propagation::Stop
        });
        self.window.add_controller(keys);
    }

    /// A button that sends the intent `make` builds for the surface's Call.
    fn button(
        &self,
        label: Label,
        class: &str,
        intents: &UnboundedSender<Input>,
        make: fn(CallId) -> Input,
    ) -> gtk::Button {
        let button = gtk::Button::with_label(messages::label(label));
        button.add_css_class(class);
        button.set_hexpand(true);
        let (call, intents) = (self.call.clone(), intents.clone());
        button.connect_clicked(move |_| {
            if let Some(call) = call.get() {
                let _ = intents.unbounded_send(make(call));
            }
        });
        button
    }
}

/// What the surfaces show about the other party: the Contact's name or the
/// number, and, for a Contact, a line with the number's label and the
/// number. The content they report gets `caller`, and for a Contact
/// `number-label` and `number`.
struct Who {
    caller: String,
    detail: Option<(String, String)>,
}

impl Who {
    fn of(caller: &Caller, contact: Option<&ContactMatch>) -> Self {
        let named = contact.filter(|contact| !contact.name.trim().is_empty());
        Self {
            caller: messages::caller_name(caller, named),
            detail: named.map(|contact| {
                (
                    messages::number_label(&contact.label),
                    messages::caller(caller),
                )
            }),
        }
    }

    /// Shows the detail line on `label`, or hides it.
    fn show_detail(&self, label: &gtk::Label) {
        match &self.detail {
            Some((number_label, number)) => {
                label.set_text(&messages::caller_detail(number_label, number));
                label.set_visible(true);
            }
            None => label.set_visible(false),
        }
    }

    fn report(&self, content: &mut BTreeMap<String, String>) {
        content.insert("caller".to_owned(), self.caller.clone());
        if let Some((number_label, number)) = &self.detail {
            content.insert("number-label".to_owned(), number_label.clone());
            content.insert("number".to_owned(), number.clone());
        }
    }
}

fn detail_label() -> gtk::Label {
    let label = gtk::Label::new(None);
    label.add_css_class("dim-label");
    label.set_xalign(0.0);
    label.set_visible(false);
    label
}

/// Whether a key press holds Control, Alt or Super, so it is a shortcut of
/// something else and not a plain letter.
fn has_command_modifier(modifiers: gtk::gdk::ModifierType) -> bool {
    modifiers.intersects(
        gtk::gdk::ModifierType::CONTROL_MASK
            | gtk::gdk::ModifierType::ALT_MASK
            | gtk::gdk::ModifierType::SUPER_MASK,
    )
}

/// The Incoming Call Banner.
struct Banner {
    surface: Surface,
    heading: gtk::Label,
    caller: gtk::Label,
    detail: gtk::Label,
    answer: gtk::Button,
}

impl Banner {
    fn new(intents: &UnboundedSender<Input>, report: SharedReport) -> Self {
        let surface = Surface::new(
            "banner",
            Layer::Overlay,
            KeyboardMode::None,
            intents,
            report,
        );
        let heading = gtk::Label::new(Some(messages::label(Label::IncomingCall)));
        heading.add_css_class("dim-label");
        heading.set_xalign(0.0);
        let caller = gtk::Label::new(None);
        caller.add_css_class("caller");
        caller.set_xalign(0.0);
        let detail = detail_label();
        let decline = surface.button(Label::Decline, "decline", intents, Input::Decline);
        let answer = surface.button(Label::Answer, "answer", intents, Input::Answer);
        let buttons = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        buttons.append(&decline);
        buttons.append(&answer);
        let content = gtk::Box::new(gtk::Orientation::Vertical, 8);
        content.append(&heading);
        content.append(&caller);
        content.append(&detail);
        content.append(&buttons);
        surface.window.set_child(Some(&content));
        let banner = Self {
            surface,
            heading,
            caller,
            detail,
            answer,
        };
        banner.on_letters(intents);
        banner
    }

    /// While the Banner has keyboard focus, A answers and D declines.
    fn on_letters(&self, intents: &UnboundedSender<Input>) {
        let keys = gtk::EventControllerKey::new();
        keys.set_propagation_phase(gtk::PropagationPhase::Capture);
        let (call, intents) = (self.surface.call.clone(), intents.clone());
        keys.connect_key_pressed(move |_, key, _, modifiers| {
            let make: fn(CallId) -> Input = match key.to_lower() {
                gtk::gdk::Key::a => Input::Answer,
                gtk::gdk::Key::d => Input::Decline,
                _ => return glib::Propagation::Proceed,
            };
            if has_command_modifier(modifiers) {
                return glib::Propagation::Proceed;
            }
            if let Some(call) = call.get() {
                let _ = intents.unbounded_send(make(call));
            }
            glib::Propagation::Stop
        });
        self.surface.window.add_controller(keys);
    }

    fn render(&self, view: Option<&BannerView>) {
        let Some(view) = view else {
            self.surface.hide();
            return;
        };
        let who = Who::of(&view.caller, view.contact.as_ref());
        self.caller.set_text(&who.caller);
        who.show_detail(&self.detail);
        // While focused, the heading says which keys do what.
        let heading = messages::label(if view.focused {
            Label::BannerKeys
        } else {
            Label::IncomingCall
        });
        self.heading.set_text(heading);
        self.surface.call.set(Some(view.call));
        let mut content = BTreeMap::from([
            ("call".to_owned(), view.call.to_string()),
            ("heading".to_owned(), heading.to_owned()),
        ]);
        who.report(&mut content);
        self.surface.show(content);
        self.surface.focus(view.focused);
        if view.focused {
            // Enter and Space then answer, never decline.
            self.answer.grab_focus();
        }
    }
}

/// The Call Window.
struct CallWindow {
    surface: Surface,
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
    /// Why a Call placed here failed or was refused.
    notice: gtk::Label,
    /// The strip that says the microphone is muted.
    microphone_muted: gtk::Label,
    /// Why Mute is disabled, while it is.
    mute_unavailable: gtk::Label,
    /// Where keypad tones are typed or pasted, and which shows those sent.
    tones: gtk::Entry,
    keypad: gtk::Grid,
    /// Each shortcut as the window shows it, by the action's report key.
    shortcuts: BTreeMap<String, String>,
    /// Set while the window writes the tones it sent into `tones`, so the
    /// write isn't taken for typing.
    writing_tones: Cell<bool>,
    /// The once-a-second update of the Call timer, while it runs.
    ticker: RefCell<Option<SourceId>>,
    view: Rc<RefCell<Option<CallWindowView>>>,
}

impl CallWindow {
    fn new(intents: &UnboundedSender<Input>, report: SharedReport) -> Rc<Self> {
        let surface = Surface::new(
            "call-window",
            Layer::Top,
            KeyboardMode::OnDemand,
            intents,
            report,
        );
        let caller = gtk::Label::new(None);
        caller.add_css_class("caller");
        caller.set_xalign(0.0);
        let hd = gtk::Label::new(Some(messages::label(Label::Hd)));
        hd.add_css_class("hd");
        let heading = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        heading.append(&caller);
        heading.append(&hd);
        let detail = detail_label();
        let status = gtk::Label::new(None);
        status.add_css_class("numeric");
        status.set_xalign(0.0);
        let mut shortcuts = BTreeMap::new();
        let audio = AudioRow::new();
        shortcuts.insert("handoff-shortcut".to_owned(), audio.shortcut.clone());
        let mute = gtk::Button::new();
        mute.add_css_class("mute");
        let mute_label = gtk::Label::new(Some(messages::label(Label::Mute)));
        shortcuts.insert(
            "mute-shortcut".to_owned(),
            show_shortcut(&mute, &mute_label, MUTE_SHORTCUT),
        );
        let keypad_shown = gtk::ToggleButton::new();
        shortcuts.insert(
            "keypad-shortcut".to_owned(),
            show_shortcut(
                &keypad_shown,
                &gtk::Label::new(Some(messages::label(Label::Keypad))),
                KEYPAD_SHORTCUT,
            ),
        );
        let controls = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        controls.set_homogeneous(true);
        controls.append(&mute);
        controls.append(&keypad_shown);
        let hang_up = hang_up_button(&surface, intents);
        shortcuts.insert(
            "hang-up-shortcut".to_owned(),
            show_shortcut(
                &hang_up,
                &gtk::Label::new(Some(messages::label(Label::HangUp))),
                HANG_UP_SHORTCUT,
            ),
        );

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
            notice,
            microphone_muted,
            mute_unavailable,
            tones,
            keypad,
            shortcuts,
            writing_tones: Cell::new(false),
            ticker: RefCell::default(),
            view: Rc::default(),
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
                window.keypad.set_visible(toggle.is_active());
                window.update();
            }
        });
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
        let actions: [(&str, gtk::Widget); 4] = [
            (MUTE_SHORTCUT, self.mute.clone().upcast()),
            (KEYPAD_SHORTCUT, self.keypad_shown.clone().upcast()),
            (HANDOFF_SHORTCUT, self.audio.handoff.clone().upcast()),
            (HANG_UP_SHORTCUT, self.hang_up.clone().upcast()),
        ];
        for (accelerator, control) in actions {
            // Disabled controls ignore it, and the key is still taken.
            let action = gtk::CallbackAction::new(move |_, _| {
                control.activate();
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

/// Why a Call placed here failed or was refused, hidden.
fn notice_label() -> gtk::Label {
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
    microphone_muted.add_css_class("microphone-muted");
    microphone_muted.set_visible(false);
    let mute_unavailable = gtk::Label::new(Some(messages::failure(Failure::MuteUnavailable).text));
    mute_unavailable.add_css_class("dim-label");
    mute_unavailable.set_wrap(true);
    mute_unavailable.set_xalign(0.0);
    mute_unavailable.set_visible(false);
    (microphone_muted, mute_unavailable)
}

/// Puts `text` and `accelerator`'s label on `button`, and returns the label
/// as shown.
fn show_shortcut(button: &impl IsA<gtk::Button>, text: &gtk::Label, accelerator: &str) -> String {
    let shown = gtk::accelerator_parse(accelerator)
        .map(|(key, modifiers)| gtk::accelerator_get_label(key, modifiers).to_string())
        .unwrap_or_default();
    let shortcut = gtk::Label::new(Some(&shown));
    shortcut.add_css_class("shortcut");
    let label = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    label.set_halign(gtk::Align::Center);
    label.append(text);
    label.append(&shortcut);
    button.as_ref().set_child(Some(&label));
    button.as_ref().set_hexpand(true);
    shown
}

impl CallWindow {
    fn render(self: &Rc<Self>, view: Option<&CallWindowView>) {
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
            return;
        };
        self.surface.call.set(view.call);
        let live = matches!(view.phase, CallWindowPhase::Live { .. });
        if live {
            self.start_ticker();
        } else {
            self.stop_ticker();
        }
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
        let who = Who::of(&view.caller, view.contact.as_ref());
        self.caller.set_text(&who.caller);
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
        self.hang_up.set_sensitive(!matches!(
            view.phase,
            CallWindowPhase::Ended | CallWindowPhase::NotPlaced
        ));
        let notice = view.notice.map(|failure| messages::failure(failure).text);
        self.notice.set_text(notice.unwrap_or_default());
        self.notice.set_visible(notice.is_some());
        let mute_text = self.show_controls(mute);

        let mut content = BTreeMap::from([("status".to_owned(), status)]);
        if let Some(call) = view.call {
            content.insert("call".to_owned(), call.to_string());
        }
        if let Some(notice) = notice {
            content.insert("notice".to_owned(), notice.to_owned());
        }
        who.report(&mut content);
        content.extend(self.shortcuts.clone());
        if let Some(audio_location) = audio_location {
            content.insert("audio-location".to_owned(), audio_location.to_owned());
            content.insert("handoff".to_owned(), handoff.to_owned());
        }
        if wideband {
            content.insert("hd".to_owned(), messages::label(Label::Hd).to_owned());
        }
        if let Some(mute) = mute {
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
        if self.keypad_shown.is_active() {
            content.insert("keypad".to_owned(), "shown".to_owned());
        }
        let tones = self.tones.text();
        if !tones.is_empty() {
            content.insert("tones".to_owned(), tones.to_string());
        }
        self.surface.show(content);
    }

    /// Brings the Handoff button up to date with the Call's audio, `None`
    /// with no live Call, and returns its label. It names where the audio
    /// will go, and is disabled while the audio is on its way.
    fn show_handoff(&self, audio: Option<CallAudio>) -> &'static str {
        let label = messages::label(match audio {
            Some(CallAudio {
                moving_here: true, ..
            }) => Label::MovingAudioHere,
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
            .set_sensitive(audio.is_some_and(|audio| !audio.moving_here));
        label
    }

    /// Brings Mute, its notices and the keypad up to date with `mute`, which
    /// is `None` with no live Call, and returns Mute's label.
    fn show_controls(&self, mute: Option<Mute>) -> &'static str {
        let live = mute.is_some();
        let mute_text = messages::label(if mute == Some(Mute::On) {
            Label::Unmute
        } else {
            Label::Mute
        });
        self.mute_label.set_text(mute_text);
        self.mute
            .set_sensitive(matches!(mute, Some(Mute::Off | Mute::On)));
        if mute == Some(Mute::On) {
            self.mute.add_css_class("muted");
        } else {
            self.mute.remove_css_class("muted");
        }
        self.microphone_muted.set_visible(mute == Some(Mute::On));
        self.mute_unavailable
            .set_visible(mute == Some(Mute::Unavailable));
        self.keypad_shown.set_sensitive(live);
        self.tones.set_sensitive(live);
        self.keypad.set_sensitive(live);
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

fn seconds_since(since: Timestamp) -> u64 {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_millis());
    let now = u64::try_from(now).unwrap_or(u64::MAX);
    now.saturating_sub(since.0) / 1_000
}
