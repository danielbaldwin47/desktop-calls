//! Every surface follows the Desktop's look, live.
//!
//! - Palette, and light or dark: from Omarchy's current theme,
//!   `~/.local/state/omarchy/current/theme/colors.toml` (flat
//!   `key = "#rrggbb"` pairs, and `mode`). Its colours override libadwaita's
//!   colour variables through one CSS provider at APPLICATION priority, and
//!   its `mode` sets libadwaita's colour scheme, so the palette and the light
//!   or dark style change in the same frame. (Omarchy also tells the settings
//!   portal, but only later in its switch.)
//! - Corner radius: Hyprland's `decoration:rounding`, for the surfaces and
//!   their buttons, as Omarchy's own shell does. It is read again whenever
//!   Hyprland announces `configreloaded`, which a theme switch also causes.
//! - Live: a theme switch deletes `current/theme` and moves the new theme
//!   into its place. A monitor on `current/` sees that, and once it has been
//!   quiet for [`DEBOUNCE`] the theme is read again and the provider's CSS
//!   replaced.
//! - Without Omarchy, stock Adwaita, light or dark as the settings portal
//!   says. Without Hyprland, stock corners.
//! - Every surface draws with the colour roles in [`roles`], computed from
//!   the palette so that text reads at 4.5:1 or better, and with the type
//!   roles and pieces in [`style`]. Surfaces name a class, never a colour.
//! - Once a switch has been applied, the hook given to
//!   [`Theme::when_switched`] runs, so the surfaces can log when they reach
//!   the screen in the new theme (ticket #99).

mod colour;
mod roles;
mod style;

pub(crate) use style::class;

use std::cell::{Cell, RefCell};
use std::env;
use std::fmt;
use std::io;
use std::path::{Path, PathBuf};
use std::rc::{Rc, Weak};
use std::time::Duration;

use adw::ColorScheme;
use gtk::gio::{self, prelude::*};
use gtk::glib::{self, SourceId};

use crate::hyprland::{self, Unknown};
use crate::logging::EventKind;

use roles::{Palette, Roles};

/// How long `current/` must be quiet before the theme is read again. A
/// switch is a delete and a move a moment apart.
const DEBOUNCE: Duration = Duration::from_millis(100);

/// Omarchy's state directory for the current theme, under the home directory.
const OMARCHY_CURRENT: &str = ".local/state/omarchy/current";

/// Follows the look for as long as it lives.
pub struct Theme {
    shared: Rc<Shared>,
    /// Dropping the monitor would stop it.
    _monitor: Option<gio::FileMonitor>,
}

struct Shared {
    provider: gtk::CssProvider,
    /// Omarchy's `current/`, if there is a home directory.
    current: Option<PathBuf>,
    palette: RefCell<Option<Palette>>,
    rounding: Cell<Option<u32>>,
    /// The CSS the provider holds now.
    css: RefCell<String>,
    /// The debounced reread, while one is due.
    reread: RefCell<Option<SourceId>>,
    /// What runs once a theme switch has been applied.
    switched: RefCell<Option<Switched>>,
}

/// What runs once a theme switch has been applied, given its mode:
/// `"dark"` or `"light"`, or `None` when there is no Omarchy theme any more.
type Switched = Box<dyn Fn(Option<&'static str>)>;

impl Theme {
    /// Gives `display` the current look, and follows it from now on.
    pub fn follow(display: &gtk::gdk::Display) -> Self {
        let provider = gtk::CssProvider::new();
        gtk::style_context_add_provider_for_display(
            display,
            &provider,
            gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );
        let current = env::var_os("HOME").map(|home| PathBuf::from(home).join(OMARCHY_CURRENT));
        let shared = Rc::new(Shared {
            provider,
            current,
            palette: RefCell::default(),
            rounding: Cell::default(),
            css: RefCell::default(),
            reread: RefCell::default(),
            switched: RefCell::default(),
        });
        let _ = shared.reread_palette();
        shared.apply();
        // Without Omarchy, stock Adwaita's roles follow its light or dark.
        let following = Rc::downgrade(&shared);
        adw::StyleManager::default().connect_dark_notify(move |_| {
            if let Some(shared) = following.upgrade() {
                shared.apply();
            }
        });
        let monitor = shared
            .current
            .as_deref()
            .and_then(|current| watch(current, &shared));
        follow_rounding(&shared);
        Self {
            shared,
            _monitor: monitor,
        }
    }

    /// Runs `then` each time a switch of Omarchy's theme has been applied:
    /// once the new CSS is loaded, before it is drawn. `then` is given the
    /// new theme's mode, `"dark"` or `"light"`, or `None` without one.
    pub fn when_switched(&self, then: impl Fn(Option<&'static str>) + 'static) {
        *self.shared.switched.borrow_mut() = Some(Box::new(then));
    }
}

impl Shared {
    /// Reads Omarchy's current theme again, and logs a change. Returns
    /// whether it changed.
    fn reread_palette(&self) -> bool {
        let palette = self.current.as_deref().and_then(read_palette);
        let changed = *self.palette.borrow() != palette;
        if changed && let Some(palette) = &palette {
            tracing::info!(
                event_kind = %EventKind::ThemeChanged,
                mode = if palette.dark { "dark" } else { "light" },
                "Following the Omarchy theme"
            );
        } else if changed {
            tracing::info!(
                event_kind = %EventKind::ThemeChanged,
                "No Omarchy theme; using stock Adwaita"
            );
        }
        *self.palette.borrow_mut() = palette;
        changed
    }

    /// The current theme's mode: `"dark"` or `"light"`, or `None` without
    /// an Omarchy theme.
    fn mode(&self) -> Option<&'static str> {
        self.palette
            .borrow()
            .as_ref()
            .map(|palette| if palette.dark { "dark" } else { "light" })
    }

    /// Brings the provider and the colour scheme up to date.
    fn apply(&self) {
        let palette = self.palette.borrow();
        let (roles, adwaita) = match palette.as_ref() {
            Some(palette) => (Roles::omarchy(palette), palette.adwaita_variables()),
            None => (
                Roles::stock(adw::StyleManager::default().is_dark()),
                Vec::new(),
            ),
        };
        let css = style::css(&roles, &adwaita, self.rounding.get());
        if *self.css.borrow() != css {
            self.provider.load_from_string(&css);
            *self.css.borrow_mut() = css;
        }
        adw::StyleManager::default().set_color_scheme(match palette.as_ref() {
            Some(palette) if palette.dark => ColorScheme::ForceDark,
            Some(_) => ColorScheme::ForceLight,
            None => ColorScheme::Default,
        });
    }

    /// Rereads the theme once `current/` has been quiet for [`DEBOUNCE`].
    fn reread_soon(self: &Rc<Self>) {
        if let Some(due) = self.reread.borrow_mut().take() {
            due.remove();
        }
        let shared = Rc::downgrade(self);
        let due = glib::timeout_add_local_once(DEBOUNCE, move || {
            let Some(shared) = shared.upgrade() else {
                return;
            };
            // It has fired, so there is nothing left to remove.
            shared.reread.borrow_mut().take();
            let changed = shared.reread_palette();
            shared.apply();
            if changed && let Some(switched) = &*shared.switched.borrow() {
                switched(shared.mode());
            }
        });
        *self.reread.borrow_mut() = Some(due);
    }
}

/// Watches `current`, which a theme switch changes. It need not exist yet.
fn watch(current: &Path, shared: &Rc<Shared>) -> Option<gio::FileMonitor> {
    let monitor = gio::File::for_path(current)
        .monitor_directory(gio::FileMonitorFlags::WATCH_MOVES, gio::Cancellable::NONE);
    match monitor {
        Ok(monitor) => {
            let shared = Rc::downgrade(shared);
            monitor.connect_changed(move |_, _, _, _| {
                if let Some(shared) = shared.upgrade() {
                    shared.reread_soon();
                }
            });
            Some(monitor)
        }
        Err(error) => {
            tracing::warn!(
                event_kind = %EventKind::ThemeUnwatched,
                error_kind = "theme-unwatched",
                detail = %error,
                "Can't watch for Omarchy theme switches; following only the theme at startup"
            );
            None
        }
    }
}

/// Omarchy's current palette, or `None` without one.
fn read_palette(current: &Path) -> Option<Palette> {
    let path = current.join("theme/colors.toml");
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        // No Omarchy, or mid-switch; a switch's move brings the next read.
        Err(error) if error.kind() == io::ErrorKind::NotFound => return None,
        Err(error) => {
            unreadable("theme-unreadable", &error);
            return None;
        }
    };
    match text.parse::<toml::Table>() {
        Ok(table) => Palette::from_table(&table),
        Err(error) => {
            unreadable("theme-malformed", &error);
            None
        }
    }
}

fn unreadable(error_kind: &str, error: &dyn fmt::Display) {
    tracing::warn!(
        event_kind = %EventKind::ThemeUnreadable,
        error_kind,
        detail = %error,
        "Couldn't read the Omarchy theme's colours; using stock Adwaita"
    );
}

/// Reads Hyprland's rounding now, and again after each configuration reload.
fn follow_rounding(shared: &Rc<Shared>) {
    reread_rounding(Rc::downgrade(shared));
    let Ok(socket) = hyprland::event_socket() else {
        return;
    };
    let shared = Rc::downgrade(shared);
    glib::MainContext::default().spawn_local(async move {
        let address = gio::UnixSocketAddress::new(&socket);
        let connection = match gio::SocketClient::new().connect_future(&address).await {
            Ok(connection) => connection,
            Err(error) => {
                rounding_unknown(&Unknown::Unreachable(io::Error::other(error)));
                return;
            }
        };
        let events = gio::DataInputStream::new(&connection.input_stream());
        while let Ok(Some(event)) = events.read_line_utf8_future(glib::Priority::DEFAULT).await {
            if event.starts_with("configreloaded>>") {
                reread_rounding(shared.clone());
            }
        }
    });
}

/// Asks Hyprland, off the main thread, for its rounding, and applies it. If
/// Hyprland can't say, the last rounding stays.
fn reread_rounding(shared: Weak<Shared>) {
    glib::MainContext::default().spawn_local(async move {
        let rounding = gio::spawn_blocking(hyprland::rounding)
            .await
            .unwrap_or(Err(Unknown::NotHyprland));
        let Some(shared) = shared.upgrade() else {
            return;
        };
        match rounding {
            Ok(rounding) => {
                shared.rounding.set(Some(rounding));
                shared.apply();
            }
            Err(unknown) => rounding_unknown(&unknown),
        }
    });
}

fn rounding_unknown(unknown: &Unknown) {
    match unknown {
        Unknown::NotHyprland => tracing::debug!(
            event_kind = %EventKind::RoundingUnknown,
            error_kind = unknown.as_str(),
            "Not on Hyprland; keeping stock corners"
        ),
        Unknown::Unreachable(error) => tracing::warn!(
            event_kind = %EventKind::RoundingUnknown,
            error_kind = unknown.as_str(),
            detail = %error,
            "Hyprland didn't say how round its corners are"
        ),
        Unknown::Unreadable(error) => tracing::warn!(
            event_kind = %EventKind::RoundingUnknown,
            error_kind = unknown.as_str(),
            detail = %error,
            "Couldn't read Hyprland's corner rounding"
        ),
        Unknown::NoToastLayer | Unknown::NoRounding => tracing::warn!(
            event_kind = %EventKind::RoundingUnknown,
            error_kind = unknown.as_str(),
            "Hyprland has no corner rounding to follow"
        ),
    }
}
