//! The Desktop surfaces: the Incoming Call Banner, the Call Window and the
//! Dialer, as GTK4 and libadwaita windows on the layer shell, and the Setup
//! Window and the Quiet Hours Window, as ordinary windows.
//!
//! They render what the Call state machine says to show, send the user's
//! clicks back as intents, and report what is actually on screen (a surface
//! counts as shown once the compositor has mapped it) for the control
//! interface's `Surfaces`.
//!
//! Placement has one rule: a surface is centred like a launcher (the
//! Dialer) or flush in a corner, anchored to its two edges with the standard
//! gap, Omarchy's own edge gap (half of Hyprland's `general:gaps_out`, read
//! by [`Theme`], or [`FALLBACK_GAP`]). No surface sits part-way across the
//! screen to keep clear of another app's. At the top, the gap is measured
//! from the bar: the compositor keeps a surface out of the bar's exclusive
//! zone. Each surface reports its `anchor` (`top-right`, `bottom-right` or
//! `centre`) and, in a corner, its `margin`.
//!
//! - The Banner is an overlay surface, so it shows above fullscreen windows.
//!   It takes keyboard focus only when the keybinding asks; then A answers,
//!   D declines and Escape gives focus back. It sits flush top-right. It is
//!   unmapped whenever nothing rings: a mapped overlay surface stops
//!   Hyprland scanning a fullscreen window out directly.
//! - The Banner may cover Omarchy's top notification card while it rings,
//!   and that is meant: a ringing Call outranks a notification. Omarchy's
//!   notification column is an overlay surface that ignores exclusive zones
//!   and offers no way to reserve room beside it, so nothing can push it
//!   aside. The Banner is drawn above it, since Hyprland draws one layer's
//!   surfaces in the order they were mapped and the Banner is mapped afresh
//!   for each ring (inferred, judged at Real-Phone Checklist step 7.2). Each
//!   takes only its own clicks, and the card is there once the Banner goes.
//! - A live Call has one home, the bottom-right corner. The Call Window is on
//!   the top layer, so fullscreen windows cover it, flush in that corner, and
//!   takes keyboard focus only when clicked or when the keybinding asks;
//!   Escape gives it back. Each of its actions has a shortcut, shown on its
//!   button. Digits, `*` and `#` typed or pasted anywhere in it are keypad
//!   tones. Anchored at the bottom, it grows upwards: whatever comes and goes
//!   during a Call sits above its controls, so they and Hang up never move.
//!   With a Waiting Call, the Banner is top right and the Call Window bottom
//!   right, so neither covers the other.
//! - A few seconds after a Call connects, the Call Window collapses to the
//!   Call Pill, as the Call state machine decides: a strip on the top layer,
//!   flush in the same corner, sharing the Call Window's bottom-right
//!   corner, which never takes the keyboard. A click on its body, the
//!   keybinding or the Indicator expands the Call Window again, in place;
//!   Escape on the Call Window collapses it. The Call Window reports the
//!   pointer, clicks, keys, the keyboard and its keypad for the decision.
//! - The Dialer is an overlay surface centred in the space the bar leaves,
//!   like a launcher, with the keyboard to itself while open. Typing searches;
//!   Up and Down move the selection; Enter or a click Calls the selected
//!   row; Escape closes it.
//! - Escape only ever gives keyboard focus back, collapses the Call Window
//!   or closes the Dialer. It never ends or declines a Call.
//! - The Setup Window and the Quiet Hours Window are ordinary windows, not
//!   on the layer shell: tasks the user works through, which tile, take
//!   focus when they open and can be moved aside. Escape closes either.
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
use crate::logging::EventKind;
use crate::theme::{Theme, class};

mod banner;
mod call_window;
mod dialer;
mod first_frames;
mod icons;
mod quiet_hours_window;
mod setup_problems;
mod setup_window;

use banner::Banner;
use call_window::CallWindow;
use dialer::Dialer;
use first_frames::FirstFrames;
use quiet_hours_window::QuietHoursWindow;
use setup_problems::SetupProblems;
use setup_window::SetupWindow;

/// The standard gap between a surface and the screen's edges when Hyprland
/// can't say what Omarchy's is.
const FALLBACK_GAP: i32 = 12;
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

/// The surfaces' layout. Their colours, type, borders and corners come from
/// [`Theme`] and its classes ([`class`]).
const CSS: &str = "
.desktop-calls-surface { padding: 16px; }
.desktop-calls-surface.dialer { padding: 2px; }
.desktop-calls-surface.dialer list { background: none; }
.desktop-calls-surface .dialer-row { padding: 6px 14px; }
.desktop-calls-surface.dialer .footer-line { padding-left: 14px; padding-right: 14px; padding-bottom: 12px; }
.desktop-calls-surface .dialer-row .title { font-weight: bold; }
.desktop-calls-surface.call-pill { padding: 6px 6px 6px 12px; }
.desktop-calls-surface.call-pill.quiet { padding-right: 12px; }
.desktop-calls-surface .hd {
    font-size: 0.8em; font-weight: bold; padding: 0 6px;
    border: 1px solid currentColor;
}
";

/// The surfaces, once GTK is up.
pub struct Surfaces {
    banner: Banner,
    call_window: Rc<CallWindow>,
    dialer: Rc<Dialer>,
    setup_window: Rc<SetupWindow>,
    setup_problems: Rc<SetupProblems>,
    quiet_hours_window: Rc<QuietHoursWindow>,
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
        let setup_window = SetupWindow::new(intents, report.clone());
        let setup_problems = SetupProblems::new(intents, report.clone());
        setup_window.put_on_top(&setup_problems.widget);
        let quiet_hours_window = QuietHoursWindow::new(intents, report.clone());
        let surfaces = Self {
            banner: Banner::new(intents, report.clone()),
            call_window: CallWindow::new(intents, report),
            setup_window,
            setup_problems,
            quiet_hours_window,
            first_frames: FirstFrames::new(dialer.surface.window.clone()),
            dialer,
            theme,
        };
        if let Some(theme) = &surfaces.theme {
            theme.when_switched(theme_shown_when_drawn(surfaces.windows()));
            let placed = surfaces.placed();
            theme.when_edge_gap_changed(move |gap| {
                for surface in &placed {
                    surface.place(gap.unwrap_or(FALLBACK_GAP));
                }
            });
        }
        surfaces
    }

    /// What follows the standard gap: each surface's placement.
    fn placed(&self) -> [Placed; 4] {
        [
            &self.banner.surface,
            &self.call_window.surface,
            self.call_window.pill_surface(),
            &self.dialer.surface,
        ]
        .map(Surface::placed)
    }

    /// Each surface's name and window.
    fn windows(&self) -> [(&'static str, gtk::Window); 4] {
        [
            &self.banner.surface,
            &self.call_window.surface,
            self.call_window.pill_surface(),
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
        self.banner.render(view.banner.as_ref());
        self.call_window.render(view.call_window.as_ref());
        self.dialer.render(view.dialer.as_ref());
        self.setup_window.top_keeps_focus(
            view.setup_problems
                .as_ref()
                .is_some_and(|problems| problems.takes_focus),
        );
        self.setup_window.render(view.setup_window.as_ref());
        // After the Setup Window, so its first button takes focus.
        self.setup_problems
            .render(view.setup_problems.as_ref(), view.setup_window.as_ref());
        // The Problems going may take keyboard focus with them.
        self.setup_window.focus_if_lost();
        self.quiet_hours_window
            .render(view.quiet_hours_window.as_ref());
    }
}

/// Where a surface sits on the screen.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Anchor {
    /// Centred in the space the bar leaves, like a launcher.
    Centre,
    /// Flush in the top-right corner, with the standard gap.
    TopRight,
    /// Flush in the bottom-right corner, with the standard gap.
    BottomRight,
}

impl Anchor {
    fn as_str(self) -> &'static str {
        match self {
            Self::Centre => "centre",
            Self::TopRight => "top-right",
            Self::BottomRight => "bottom-right",
        }
    }

    /// Anchors `window` here, `gap` from the corner's two edges.
    fn apply(self, window: &gtk::Window, gap: i32) {
        if !gtk4_layer_shell::is_supported() {
            return;
        }
        let corner: &[Edge] = match self {
            // Anchored to no edge, the compositor centres it in the space
            // other surfaces' exclusive zones (the bar's) leave.
            Self::Centre => &[],
            Self::TopRight => &[Edge::Top, Edge::Right],
            Self::BottomRight => &[Edge::Bottom, Edge::Right],
        };
        for edge in [Edge::Top, Edge::Right, Edge::Bottom, Edge::Left] {
            let anchored = corner.contains(&edge);
            window.set_anchor(edge, anchored);
            // The default exclusive zone keeps the surface out of the bar's;
            // the margin is only the gap from it.
            window.set_margin(edge, if anchored { gap } else { 0 });
        }
    }

    /// What a surface reports of where it sits: its anchor and, in a
    /// corner, its gap from both edges.
    fn report(self, gap: i32, content: &mut BTreeMap<String, String>) {
        content.insert("anchor".to_owned(), self.as_str().to_owned());
        if self != Self::Centre {
            content.insert("margin".to_owned(), gap.to_string());
        }
    }
}

/// What a surface needs to move to a new standard gap.
struct Placed {
    name: &'static str,
    window: gtk::Window,
    anchor: Anchor,
    gap: Rc<Cell<i32>>,
    content: Rc<RefCell<BTreeMap<String, String>>>,
    mapped: Rc<Cell<bool>>,
    report: SharedReport,
}

impl Placed {
    /// Moves the surface `gap` from its corner's edges, and reports it.
    fn place(&self, gap: i32) {
        if self.gap.replace(gap) == gap {
            return;
        }
        self.anchor.apply(&self.window, gap);
        if self.mapped.get() {
            self.report.borrow_mut().set(
                self.name,
                Some(reported_placed(
                    &self.content.borrow(),
                    &self.window,
                    self.anchor,
                    gap,
                )),
            );
        }
    }
}

/// A surface's window, with what it shows and whether it is on screen.
struct Surface {
    name: &'static str,
    window: gtk::Window,
    anchor: Anchor,
    /// The standard gap it sits at now.
    gap: Rc<Cell<i32>>,
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
    surfaces: [(&'static str, gtk::Window); 4],
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

/// What a window reports: its content, and `focused` while it has keyboard
/// focus.
fn reported(content: &BTreeMap<String, String>, window: &gtk::Window) -> BTreeMap<String, String> {
    let mut reported = content.clone();
    if window.is_active() {
        reported.insert("focused".to_owned(), "true".to_owned());
    }
    reported
}

/// What a surface reports: as [`reported`], and where it sits.
fn reported_placed(
    content: &BTreeMap<String, String>,
    window: &gtk::Window,
    anchor: Anchor,
    gap: i32,
) -> BTreeMap<String, String> {
    let mut reported = reported(content, window);
    anchor.report(gap, &mut reported);
    reported
}

impl Surface {
    /// A surface named `name`, placed at `anchor`. Escape sends `escape`,
    /// if the surface takes the keyboard at all.
    fn new(
        name: &'static str,
        anchor: Anchor,
        layer: Layer,
        keyboard: KeyboardMode,
        escape: Option<Input>,
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
        }
        anchor.apply(&window, FALLBACK_GAP);
        let surface = Self {
            name,
            window,
            anchor,
            gap: Rc::new(Cell::new(FALLBACK_GAP)),
            content: Rc::default(),
            mapped: Rc::default(),
            report,
            call: Rc::default(),
            keyboard,
            releasing: Rc::default(),
        };
        let (content, mapped, report, gap) = (
            surface.content.clone(),
            surface.mapped.clone(),
            surface.report.clone(),
            surface.gap.clone(),
        );
        surface.window.connect_map(move |window| {
            mapped.set(true);
            report.borrow_mut().set(
                name,
                Some(reported_placed(
                    &content.borrow(),
                    window,
                    anchor,
                    gap.get(),
                )),
            );
            surface_mapped(name);
            after_next_frame(window, move || surface_shown(name));
        });
        let (content, mapped, report, releasing, gap) = (
            surface.content.clone(),
            surface.mapped.clone(),
            surface.report.clone(),
            surface.releasing.clone(),
            surface.gap.clone(),
        );
        surface.window.connect_is_active_notify(move |window| {
            set_class(window, class::HAS_KEYBOARD, window.is_active());
            if !window.is_active() && releasing.replace(false) && gtk4_layer_shell::is_supported() {
                window.set_keyboard_mode(keyboard);
            }
            if mapped.get() {
                report.borrow_mut().set(
                    name,
                    Some(reported_placed(
                        &content.borrow(),
                        window,
                        anchor,
                        gap.get(),
                    )),
                );
            }
        });
        if let Some(escape) = escape {
            surface.on_escape(escape, intents);
        }
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
                Some(reported_placed(
                    &self.content.borrow(),
                    &self.window,
                    self.anchor,
                    self.gap.get(),
                )),
            );
        }
    }

    /// Reports the surface as shown with `content` before it is on screen:
    /// the Call Pill, while the Call Window shrinks into it, so the control
    /// interface reports the end of the change at once. Mapping it later
    /// reports the same.
    fn report_ahead(&self, content: BTreeMap<String, String>) {
        *self.content.borrow_mut() = content;
        self.publish();
    }

    /// Reports the surface as shown with what it shows now.
    fn publish(&self) {
        self.report.borrow_mut().set(
            self.name,
            Some(reported_placed(
                &self.content.borrow(),
                &self.window,
                self.anchor,
                self.gap.get(),
            )),
        );
    }

    /// Reports the surface as gone while it is still on screen: the Call
    /// Window, while it shrinks into the Call Pill.
    fn report_gone(&self) {
        self.report.borrow_mut().set(self.name, None);
    }

    /// What moves the surface to a new standard gap.
    fn placed(&self) -> Placed {
        Placed {
            name: self.name,
            window: self.window.clone(),
            anchor: self.anchor,
            gap: self.gap.clone(),
            content: self.content.clone(),
            mapped: self.mapped.clone(),
            report: self.report.clone(),
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

    /// Escape (or Super+W) gives keyboard focus back, however the surface got it, and
    /// sends the Call state machine `escape`.
    fn on_escape(&self, escape: Input, intents: &UnboundedSender<Input>) {
        let keys = gtk::EventControllerKey::new();
        keys.set_propagation_phase(gtk::PropagationPhase::Capture);
        let (window, releasing, intents) =
            (self.window.clone(), self.releasing.clone(), intents.clone());
        keys.connect_key_pressed(move |_, key, _, modifiers| {
            if !is_escape(key, modifiers) {
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
    /// Whether `caller` is a bare number, not a name.
    bare_number: bool,
}

impl Who {
    fn of(caller: &Caller, contact: Option<&ContactMatch>) -> Self {
        let named = contact.filter(|contact| !contact.name.trim().is_empty());
        Self {
            bare_number: named.is_none() && matches!(caller, Caller::Number(_)),
            caller: messages::caller_name(caller, named),
            detail: named.map(|contact| {
                (
                    messages::number_label(&contact.label),
                    messages::caller(caller),
                )
            }),
        }
    }

    /// Shows who on `label`: a bare number in monospace.
    fn show(&self, label: &gtk::Label) {
        label.set_text(&self.caller);
        set_class(label, class::NUMBER, self.bare_number);
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

/// Adds `class` to `widget`, or takes it away.
fn set_class(widget: &impl IsA<gtk::Widget>, class: &str, on: bool) {
    if on {
        widget.add_css_class(class);
    } else {
        widget.remove_css_class(class);
    }
}

/// A label showing `accelerator` (GTK's form, such as `<Control>m`) as a
/// keycap, and the key as shown.
fn keycap(accelerator: &str) -> (gtk::Label, String) {
    let shown = gtk::accelerator_parse(accelerator)
        .map(|(key, modifiers)| gtk::accelerator_get_label(key, modifiers).to_string())
        .unwrap_or_default();
    let label = gtk::Label::new(Some(&shown));
    label.add_css_class(class::KEYCAP);
    (label, shown)
}

fn detail_label() -> gtk::Label {
    let label = gtk::Label::new(None);
    label.add_css_class(class::META);
    label.set_xalign(0.0);
    label.set_visible(false);
    label
}

/// Escape, or Super+W: Omarchy's Close window. Hyprland's close never
/// reaches a layer surface, so a bind can hand Super+W to the surface
/// instead, which then closes as on Escape (ticket #125).
fn is_escape(key: gtk::gdk::Key, modifiers: gtk::gdk::ModifierType) -> bool {
    use gtk::gdk::ModifierType;
    let super_only = (modifiers
        & (ModifierType::CONTROL_MASK | ModifierType::ALT_MASK | ModifierType::SUPER_MASK))
        == ModifierType::SUPER_MASK;
    (key == gtk::gdk::Key::Escape && !has_command_modifier(modifiers))
        || (super_only && matches!(key, gtk::gdk::Key::w | gtk::gdk::Key::W))
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
