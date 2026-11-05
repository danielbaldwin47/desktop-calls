//! The Desktop surfaces: the Incoming Call Banner, the Call Window and the
//! Dialer, as GTK4 and libadwaita windows on the layer shell.
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
//!   Hyprland scanning a fullscreen window out directly. While the Call
//!   Window shows too, for a waiting Call, the Banner sits below it.
//! - The Call Window is on the top layer, so fullscreen windows cover it, in
//!   the same corner, and takes keyboard focus only when clicked or when the
//!   keybinding asks; Escape gives it back. Each of its actions has a
//!   shortcut, shown on its button. Digits, `*` and `#` typed or pasted
//!   anywhere in it are keypad tones.
//! - The Dialer is an overlay surface centred in the space the bar leaves,
//!   like a launcher, with the keyboard to itself while open. Typing searches;
//!   Up and Down move the selection; Enter or a click Calls the selected
//!   row; Escape closes it.
//! - Escape only ever gives keyboard focus back, or closes the Dialer. It
//!   never ends or declines a Call.
//! - Each surface reports `focused` while it has keyboard focus, as the
//!   compositor says.
//! - No output is chosen, so the compositor puts each on the focused output.
//! - Where the compositor has no layer shell, each is an ordinary window.

use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;
use std::rc::Rc;
use std::time::{SystemTime, UNIX_EPOCH};

use desktop_calls_core::messages::{self, Label};
use desktop_calls_core::{CallId, Caller, ContactMatch, Input, Timestamp};
use futures_channel::mpsc::UnboundedSender;
use gtk::glib;
use gtk::prelude::*;
use gtk4_layer_shell::{Edge, KeyboardMode, Layer, LayerShell as _};

use crate::control::{Published, ShownSurfaces};
use crate::hyprland::{self, Unknown};
use crate::logging::EventKind;
use crate::theme::Theme;

mod banner;
mod call_window;
mod dialer;
mod first_frames;
mod setup_window;

use banner::Banner;
use call_window::CallWindow;
use dialer::Dialer;
use first_frames::FirstFrames;
use setup_window::SetupWindow;

/// How wide Omarchy's notification-toast column is taken to be when
/// Hyprland can't say. Omarchy 4.5's is 394.
const FALLBACK_TOAST_COLUMN_WIDTH: i32 = 400;
/// Space between a surface and what it sits next to.
const GAP: i32 = 12;
const WIDTH: i32 = 340;
const DIALER_WIDTH: i32 = 560;
/// The Dialer's list stays this tall, so the Dialer doesn't jump about as
/// the matches change.
const DIALER_LIST_HEIGHT: i32 = 420;
/// How far Page Up and Page Down move the Dialer's selection.
const DIALER_PAGE: i32 = 8;

/// The Call Window's shortcuts, as GTK accelerators. Hang up needs Control,
/// so a stray key can't end a Call.
const MUTE_SHORTCUT: &str = "m";
const KEYPAD_SHORTCUT: &str = "k";
const HANG_UP_SHORTCUT: &str = "<Control>h";
const HANDOFF_SHORTCUT: &str = "a";
/// With two Calls, Swap, and Hang up for the one on hold.
const SWAP_SHORTCUT: &str = "s";
const HANG_UP_HELD_SHORTCUT: &str = "<Control><Shift>h";

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
.desktop-calls-surface .dialer-entry { font-size: 1.2em; }
.desktop-calls-surface .dialer-row { padding: 6px 8px; }
.desktop-calls-surface .dialer-row .title { font-weight: bold; }
.desktop-calls-surface .hd {
    font-size: 0.8em; font-weight: bold; padding: 0 6px;
    border: 1px solid currentColor; border-radius: 4px;
}
";

/// The surfaces, once GTK is up.
pub struct Surfaces {
    banner: Banner,
    call_window: Rc<CallWindow>,
    dialer: Rc<Dialer>,
    setup_window: Rc<SetupWindow>,
    first_frames: FirstFrames,
    theme: Option<Theme>,
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
        let dialer = Dialer::new(intents, report.clone());
        let surfaces = Self {
            banner: Banner::new(intents, report.clone()),
            call_window: CallWindow::new(intents, report.clone()),
            setup_window: SetupWindow::new(intents, report),
            first_frames: FirstFrames::new(dialer.surface.window.clone()),
            dialer,
            theme,
        };
        if let Some(theme) = &surfaces.theme {
            theme.when_switched(theme_shown_when_drawn(surfaces.windows()));
        }
        surfaces.place_beside_toasts();
        let (banner, call_window) = (
            surfaces.banner.surface.window.clone(),
            surfaces.call_window.surface.window.clone(),
        );
        surfaces
            .call_window
            .on_resized(move || stack(&banner, &call_window));
        surfaces
    }

    /// Each surface's name and window.
    fn windows(&self) -> [(&'static str, gtk::Window); 3] {
        [
            &self.banner.surface,
            &self.call_window.surface,
            &self.dialer.surface,
        ]
        .map(|surface| (surface.name, surface.window.clone()))
    }

    /// Does the surfaces' first-frame work once the app is `quiet` (the
    /// Phone Connected, no Call, nothing shown), and abandons it for good
    /// the moment it is not. Called before `render`, so nothing shows while
    /// a piece of it is still to run.
    pub fn prepare_first_frames(&self, quiet: bool) {
        self.first_frames.while_quiet(quiet);
    }

    /// Shows, updates or hides each surface to match `view`, once the state
    /// it follows from is `_published`, so `Surfaces` never runs ahead of
    /// the Calls on the control interface.
    pub fn render(&self, view: &desktop_calls_core::Surfaces, _published: &Published) {
        if view.banner.is_some() && !self.banner.surface.window.is_visible() {
            // The toast column may have changed since; the Banner moves if so.
            self.place_beside_toasts();
        }
        self.banner.render(view.banner.as_ref());
        self.call_window.render(view.call_window.as_ref());
        self.dialer.render(view.dialer.as_ref());
        self.setup_window.render(view.setup_window.as_ref());
        stack(
            &self.banner.surface.window,
            &self.call_window.surface.window,
        );
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

/// With call waiting the Banner and the Call Window show at once, in the
/// same corner: the Banner goes below the Call Window, so the Call Window
/// never moves under the pointer and the Banner never covers it.
fn stack(banner: &gtk::Window, call_window: &gtk::Window) {
    if !gtk4_layer_shell::is_supported() {
        return;
    }
    let top = if call_window.is_visible() {
        let (_, height, _, _) = call_window.measure(gtk::Orientation::Vertical, WIDTH);
        GAP + height + GAP
    } else {
        GAP
    };
    if banner.margin(Edge::Top) != top {
        banner.set_margin(Edge::Top, top);
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

/// Runs `then` once `window` has drawn its next frame and handed it to the
/// compositor, so the app can log when a change reached the screen (ticket
/// #83). An unrealized window draws nothing, so `then` never runs.
fn after_next_frame(window: &impl IsA<gtk::Widget>, then: impl FnOnce() + 'static) {
    let Some(clock) = window.frame_clock() else {
        return;
    };
    let then = Cell::new(Some(then));
    let handler = Rc::new(RefCell::new(None));
    let disconnect = handler.clone();
    let id = clock.connect_after_paint(move |clock| {
        if let Some(then) = then.take() {
            then();
        }
        if let Some(id) = disconnect.borrow_mut().take() {
            clock.disconnect(id);
        }
    });
    *handler.borrow_mut() = Some(id);
    // A frame is drawn even if nothing else asked for one.
    window.queue_draw();
}

/// When GTK mapped a surface: the moment the control interface's `Surfaces`
/// lists it, before its first frame is drawn. The gap to `surface-shown` is
/// GTK's first paint, which on a cold shader cache is most of the time
/// (ticket #83).
fn surface_mapped(name: &str) {
    tracing::debug!(
        event_kind = %EventKind::SurfaceMapped,
        surface = name,
        "Mapped the {name} surface"
    );
}

/// When a surface's first frame since it was mapped reached the compositor.
fn surface_shown(name: &str) {
    tracing::info!(
        event_kind = %EventKind::SurfaceShown,
        surface = name,
        "Drew the {name} surface's first frame"
    );
}

/// What to do once a theme switch has been applied: log `theme-shown` for
/// each surface on screen once it has drawn its next frame, the first in
/// the new theme (ticket #99). A hidden surface draws nothing, so it logs
/// nothing.
fn theme_shown_when_drawn(
    surfaces: [(&'static str, gtk::Window); 3],
) -> impl Fn(Option<&'static str>) + 'static {
    move |mode| {
        for &(name, ref window) in &surfaces {
            if window.is_mapped() {
                after_next_frame(window, move || theme_shown(name, mode));
            }
        }
    }
}

/// When a surface's first frame in a newly switched theme reached the
/// compositor.
fn theme_shown(name: &str, mode: Option<&str>) {
    tracing::info!(
        event_kind = %EventKind::ThemeShown,
        surface = name,
        mode,
        "Drew the {name} surface in the new theme"
    );
}

/// When GTK unmapped a surface: the moment the control interface's
/// `Surfaces` stops listing it. GTK has told the compositor to take it off
/// the screen by then, so a hidden surface has no frame left to wait for
/// (ticket #99).
fn surface_unmapped(name: &str) {
    tracing::debug!(
        event_kind = %EventKind::SurfaceUnmapped,
        surface = name,
        "Unmapped the {name} surface"
    );
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
    /// A surface named `name` in the top-right corner. Escape sends
    /// `escape`.
    fn new(
        name: &'static str,
        layer: Layer,
        keyboard: KeyboardMode,
        escape: Input,
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
            surface_mapped(name);
            after_next_frame(window, move || surface_shown(name));
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
        surface.on_escape(escape, intents);
        let (mapped, report) = (surface.mapped.clone(), surface.report.clone());
        surface.window.connect_unmap(move |_| {
            mapped.set(false);
            report.borrow_mut().set(name, None);
            surface_unmapped(name);
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
    /// sends the Call state machine `escape`.
    fn on_escape(&self, escape: Input, intents: &UnboundedSender<Input>) {
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
            let _ = intents.unbounded_send(escape.clone());
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

fn seconds_since(since: Timestamp) -> u64 {
    now().0.saturating_sub(since.0) / 1_000
}

pub(crate) fn now() -> Timestamp {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_millis());
    Timestamp(u64::try_from(now).unwrap_or(u64::MAX))
}

/// The Desktop's time zone's offset from UTC now, in seconds.
pub(crate) fn utc_offset_seconds() -> i64 {
    glib::DateTime::now_local().map_or(0, |now| now.utc_offset().as_seconds())
}
