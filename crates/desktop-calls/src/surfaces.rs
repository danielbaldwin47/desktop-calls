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
//!   keybinding asks; Escape gives it back.
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

use desktop_calls_core::messages::{self, Label};
use desktop_calls_core::{
    AudioLocation, BannerView, CallId, CallWindowPhase, CallWindowView, Input, Timestamp,
};
use futures_channel::mpsc::UnboundedSender;
use gtk::glib::{self, SourceId};
use gtk::prelude::*;
use gtk4_layer_shell::{Edge, KeyboardMode, Layer, LayerShell as _};

use crate::control::ShownSurfaces;
use crate::hyprland::{self, Unknown};
use crate::logging::EventKind;

/// How wide Omarchy's notification-toast column is taken to be when
/// Hyprland can't say. Omarchy 4.5's is 394.
const FALLBACK_TOAST_COLUMN_WIDTH: i32 = 400;
/// Space between a surface and what it sits next to.
const GAP: i32 = 12;
const WIDTH: i32 = 340;

/// Answer is green and Decline and Hang up are red, as on a phone, in light
/// and dark alike.
const CSS: &str = "
.desktop-calls-surface { padding: 16px; }
.desktop-calls-surface .caller { font-size: 1.4em; font-weight: bold; }
.desktop-calls-surface .hd {
    font-size: 0.8em; font-weight: bold; padding: 0 6px;
    border: 1px solid currentColor; border-radius: 4px;
}
.desktop-calls-surface button.answer { background: #26a269; color: #ffffff; }
.desktop-calls-surface button.decline,
.desktop-calls-surface button.hang-up { background: #c01c28; color: #ffffff; }
";

/// The surfaces, once GTK is up.
pub struct Surfaces {
    banner: Banner,
    call_window: Rc<CallWindow>,
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
        if let Some(display) = gtk::gdk::Display::default() {
            let provider = gtk::CssProvider::new();
            provider.load_from_string(CSS);
            gtk::style_context_add_provider_for_display(
                &display,
                &provider,
                gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
            );
        }
        let report = Rc::new(RefCell::new(Report {
            shown: ShownSurfaces::new(),
            reports,
        }));
        let surfaces = Self {
            banner: Banner::new(intents, report.clone()),
            call_window: CallWindow::new(intents, report),
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
        Unknown::NotHyprland | Unknown::NoToastLayer => tracing::debug!(
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
        let decline = surface.button(Label::Decline, "decline", intents, Input::Decline);
        let answer = surface.button(Label::Answer, "answer", intents, Input::Answer);
        let buttons = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        buttons.append(&decline);
        buttons.append(&answer);
        let content = gtk::Box::new(gtk::Orientation::Vertical, 8);
        content.append(&heading);
        content.append(&caller);
        content.append(&buttons);
        surface.window.set_child(Some(&content));
        let banner = Self {
            surface,
            heading,
            caller,
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
        let caller = messages::caller(&view.caller);
        self.caller.set_text(&caller);
        // While focused, the heading says which keys do what.
        let heading = messages::label(if view.focused {
            Label::BannerKeys
        } else {
            Label::IncomingCall
        });
        self.heading.set_text(heading);
        self.surface.call.set(Some(view.call));
        self.surface.show(BTreeMap::from([
            ("call".to_owned(), view.call.to_string()),
            ("heading".to_owned(), heading.to_owned()),
            ("caller".to_owned(), caller),
        ]));
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
    caller: gtk::Label,
    status: gtk::Label,
    audio_location: gtk::Label,
    hd: gtk::Label,
    hang_up: gtk::Button,
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
        let status = gtk::Label::new(None);
        status.add_css_class("numeric");
        status.set_xalign(0.0);
        let audio_location = gtk::Label::new(None);
        audio_location.add_css_class("dim-label");
        audio_location.set_xalign(0.0);
        let hang_up = surface.button(Label::HangUp, "hang-up", intents, Input::HangUp);
        let content = gtk::Box::new(gtk::Orientation::Vertical, 8);
        content.append(&heading);
        content.append(&status);
        content.append(&audio_location);
        content.append(&hang_up);
        surface.window.set_child(Some(&content));
        Rc::new(Self {
            surface,
            caller,
            status,
            audio_location,
            hd,
            hang_up,
            ticker: RefCell::default(),
            view: Rc::default(),
        })
    }
}

impl CallWindow {
    fn render(self: &Rc<Self>, view: Option<&CallWindowView>) {
        *self.view.borrow_mut() = view.cloned();
        let Some(view) = view else {
            self.stop_ticker();
            self.surface.hide();
            return;
        };
        self.surface.call.set(Some(view.call));
        if matches!(view.phase, CallWindowPhase::Live { .. }) {
            self.start_ticker();
        } else {
            self.stop_ticker();
        }
        self.update();
        self.surface.focus(view.focused);
    }

    /// Brings every label up to date with the view and the clock.
    fn update(&self) {
        let Some(view) = self.view.borrow().clone() else {
            return;
        };
        let caller = messages::caller(&view.caller);
        self.caller.set_text(&caller);
        let (status, audio_location, wideband) = match view.phase {
            CallWindowPhase::Connecting => {
                (messages::label(Label::Connecting).to_owned(), None, false)
            }
            CallWindowPhase::Live {
                active_since,
                audio_location,
                wideband,
            } => (
                messages::elapsed(seconds_since(active_since)),
                Some(messages::label(match audio_location {
                    AudioLocation::Desktop => Label::AudioOnDesktop,
                    AudioLocation::Phone => Label::AudioOnPhone,
                })),
                wideband,
            ),
            CallWindowPhase::Ended => (messages::label(Label::CallEnded).to_owned(), None, false),
        };
        self.status.set_text(&status);
        // The row stays, empty, while there is nothing to say, so Hang up
        // never moves under the pointer when the Call connects.
        self.audio_location
            .set_text(audio_location.unwrap_or_default());
        self.hd.set_visible(wideband);
        self.hang_up
            .set_sensitive(view.phase != CallWindowPhase::Ended);
        let mut content = BTreeMap::from([
            ("call".to_owned(), view.call.to_string()),
            ("caller".to_owned(), caller),
            ("status".to_owned(), status),
        ]);
        if let Some(audio_location) = audio_location {
            content.insert("audio-location".to_owned(), audio_location.to_owned());
        }
        if wideband {
            content.insert("hd".to_owned(), messages::label(Label::Hd).to_owned());
        }
        self.surface.show(content);
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
