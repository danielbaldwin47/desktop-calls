//! The Desktop surfaces: the Incoming Call Banner and the Call Window, as
//! GTK4 and libadwaita windows on the layer shell.
//!
//! They render what the Call state machine says to show, send the user's
//! clicks back as intents, and report what is actually on screen (a surface
//! counts as shown once the compositor has mapped it) for the control
//! interface's `Surfaces`.
//!
//! - The Banner is an overlay surface, so it shows above fullscreen windows.
//!   It never takes keyboard focus. It sits top-right, below Omarchy's bar
//!   and to the left of Omarchy's notification toasts, which are laid out in
//!   a column of [`TOAST_COLUMN_WIDTH`] at the top-right.
//! - The Call Window is on the top layer, so fullscreen windows cover it, in
//!   the same corner, and takes keyboard focus only when clicked.
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

/// The height of Omarchy's bar, which the surfaces sit below.
const BAR_HEIGHT: i32 = 26;
/// The width of Omarchy's notification-toast column at the top-right.
const TOAST_COLUMN_WIDTH: i32 = 380;
/// Space between a surface and what it sits next to.
const GAP: i32 = 12;
const TOP_MARGIN: i32 = BAR_HEIGHT + GAP;
const RIGHT_MARGIN: i32 = TOAST_COLUMN_WIDTH + GAP;
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
        Self {
            banner: Banner::new(intents, report.clone()),
            call_window: CallWindow::new(intents, report),
        }
    }

    /// Shows, updates or hides each surface to match `view`.
    pub fn render(&self, view: &desktop_calls_core::Surfaces) {
        self.banner.render(view.banner.as_ref());
        self.call_window.render(view.call_window.as_ref());
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
}

impl Surface {
    fn new(name: &'static str, layer: Layer, keyboard: KeyboardMode, report: SharedReport) -> Self {
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
            window.set_margin(Edge::Top, TOP_MARGIN);
            window.set_margin(Edge::Right, RIGHT_MARGIN);
        }
        let surface = Self {
            name,
            window,
            content: Rc::default(),
            mapped: Rc::default(),
            report,
            call: Rc::default(),
        };
        let (content, mapped, report) = (
            surface.content.clone(),
            surface.mapped.clone(),
            surface.report.clone(),
        );
        surface.window.connect_map(move |_| {
            mapped.set(true);
            report
                .borrow_mut()
                .set(name, Some(content.borrow().clone()));
        });
        let (mapped, report) = (surface.mapped.clone(), surface.report.clone());
        surface.window.connect_unmap(move |_| {
            mapped.set(false);
            report.borrow_mut().set(name, None);
        });
        surface
    }

    /// Shows the window with `content`, or updates what it reports.
    fn show(&self, content: BTreeMap<String, String>) {
        let unchanged = *self.content.borrow() == content;
        *self.content.borrow_mut() = content;
        if !self.window.is_visible() {
            self.window.present();
        } else if self.mapped.get() && !unchanged {
            self.report
                .borrow_mut()
                .set(self.name, Some(self.content.borrow().clone()));
        }
    }

    fn hide(&self) {
        self.call.set(None);
        self.window.set_visible(false);
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

/// The Incoming Call Banner.
struct Banner {
    surface: Surface,
    caller: gtk::Label,
}

impl Banner {
    fn new(intents: &UnboundedSender<Input>, report: SharedReport) -> Self {
        let surface = Surface::new("banner", Layer::Overlay, KeyboardMode::None, report);
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
        Self { surface, caller }
    }

    fn render(&self, view: Option<&BannerView>) {
        let Some(view) = view else {
            self.surface.hide();
            return;
        };
        let caller = messages::caller(&view.caller);
        self.caller.set_text(&caller);
        self.surface.call.set(Some(view.call));
        self.surface.show(BTreeMap::from([
            ("call".to_owned(), view.call.to_string()),
            (
                "heading".to_owned(),
                messages::label(Label::IncomingCall).to_owned(),
            ),
            ("caller".to_owned(), caller),
        ]));
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
        let surface = Surface::new("call-window", Layer::Top, KeyboardMode::OnDemand, report);
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
        self.audio_location
            .set_text(audio_location.unwrap_or_default());
        self.audio_location.set_visible(audio_location.is_some());
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
