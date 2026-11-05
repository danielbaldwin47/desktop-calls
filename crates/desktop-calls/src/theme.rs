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

use std::cell::{Cell, RefCell};
use std::env;
use std::fmt::{self, Write as _};
use std::io;
use std::path::{Path, PathBuf};
use std::rc::{Rc, Weak};
use std::time::Duration;

use adw::ColorScheme;
use gtk::gio::{self, prelude::*};
use gtk::glib::{self, SourceId};

use crate::hyprland::{self, Unknown};
use crate::logging::EventKind;

/// How long `current/` must be quiet before the theme is read again. A
/// switch is a delete and a move a moment apart.
const DEBOUNCE: Duration = Duration::from_millis(100);

/// Omarchy's state directory for the current theme, under the home directory.
const OMARCHY_CURRENT: &str = ".local/state/omarchy/current";

/// The Answer button's green and the Decline and Hang up buttons' red, with
/// white labels, as on a phone, when there is no Omarchy theme.
const STOCK_GREEN: Rgb = Rgb(0x26, 0xa2, 0x69);
const STOCK_RED: Rgb = Rgb(0xc0, 0x1c, 0x28);
const WHITE: Rgb = Rgb(0xff, 0xff, 0xff);
const BLACK: Rgb = Rgb(0x00, 0x00, 0x00);

/// The contrast a label needs against its background (WCAG AA).
const READABLE: f64 = 4.5;

/// Follows the look for as long as it lives.
pub struct Theme {
    _shared: Rc<Shared>,
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
}

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
        });
        shared.reread_palette();
        shared.apply();
        let monitor = shared
            .current
            .as_deref()
            .and_then(|current| watch(current, &shared));
        follow_rounding(&shared);
        Self {
            _shared: shared,
            _monitor: monitor,
        }
    }
}

impl Shared {
    /// Reads Omarchy's current theme again, and logs a change.
    fn reread_palette(&self) {
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
    }

    /// Brings the provider and the colour scheme up to date.
    fn apply(&self) {
        let palette = self.palette.borrow();
        let css = css(palette.as_ref(), self.rounding.get());
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
            shared.reread_palette();
            shared.apply();
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

/// The CSS for `palette` (stock Adwaita without one) and `rounding` (stock
/// corners without one).
fn css(palette: Option<&Palette>, rounding: Option<u32>) -> String {
    let mut css = String::new();
    let (green, red, label) = match palette {
        Some(palette) => {
            css.push_str(":root {\n");
            for (variable, colour) in palette.variables() {
                let _ = writeln!(css, "  --{variable}: {colour};");
            }
            css.push_str("}\n");
            palette.call_buttons()
        }
        None => (STOCK_GREEN, STOCK_RED, WHITE),
    };
    let _ = writeln!(
        css,
        ".desktop-calls-surface button.answer {{ background: {green}; color: {label}; }}\n\
         .desktop-calls-surface button.decline,\n\
         .desktop-calls-surface button.hang-up {{ background: {red}; color: {label}; }}"
    );
    if let Some(rounding) = rounding {
        let _ = writeln!(
            css,
            ".desktop-calls-surface,\n\
             .desktop-calls-surface button {{ border-radius: {rounding}px; }}"
        );
    }
    css
}

/// An sRGB colour.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Rgb(u8, u8, u8);

impl Rgb {
    /// `#rrggbb` or `#rgb`; anything else is no colour.
    fn parse(text: &str) -> Option<Self> {
        let hex = text.strip_prefix('#')?;
        if !hex.bytes().all(|digit| digit.is_ascii_hexdigit()) {
            return None;
        }
        let channel = |digits: &str| u8::from_str_radix(digits, 16).ok();
        match hex.len() {
            6 => Some(Self(
                channel(&hex[0..2])?,
                channel(&hex[2..4])?,
                channel(&hex[4..6])?,
            )),
            3 => {
                let short = |at: usize| channel(&hex[at..=at]).map(|value| value * 17);
                Some(Self(short(0)?, short(1)?, short(2)?))
            }
            _ => None,
        }
    }

    /// WCAG relative luminance, 0 for black to 1 for white.
    fn luminance(self) -> f64 {
        let linear = |channel: u8| {
            let value = f64::from(channel) / 255.0;
            if value <= 0.040_45 {
                value / 12.92
            } else {
                ((value + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * linear(self.0) + 0.7152 * linear(self.1) + 0.0722 * linear(self.2)
    }

    /// WCAG contrast ratio, from 1 to 21.
    fn contrast(self, other: Self) -> f64 {
        let (a, b) = (self.luminance(), other.luminance());
        (a.max(b) + 0.05) / (a.min(b) + 0.05)
    }

    fn is_dark(self) -> bool {
        self.contrast(BLACK) < self.contrast(WHITE)
    }
}

impl fmt::Display for Rgb {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "#{:02x}{:02x}{:02x}", self.0, self.1, self.2)
    }
}

/// What Desktop Calls takes from an Omarchy theme's `colors.toml`. A colour
/// the theme lacks, or gives in a form other than `#rrggbb`, stays stock.
#[derive(Clone, Debug, PartialEq)]
struct Palette {
    dark: bool,
    accent: Option<Rgb>,
    background: Option<Rgb>,
    foreground: Option<Rgb>,
    red: Option<Rgb>,
    green: Option<Rgb>,
    yellow: Option<Rgb>,
}

impl Palette {
    /// `None` if the table has no colour Desktop Calls uses.
    fn from_table(table: &toml::Table) -> Option<Self> {
        let colour = |key: &str| table.get(key)?.as_str().and_then(Rgb::parse);
        let background = colour("background");
        let dark = match table.get("mode").and_then(toml::Value::as_str) {
            Some("dark") => true,
            Some("light") => false,
            _ => background.is_some_and(Rgb::is_dark),
        };
        let palette = Self {
            dark,
            accent: colour("accent"),
            background,
            foreground: colour("foreground"),
            red: colour("red"),
            green: colour("green"),
            yellow: colour("yellow"),
        };
        let any = [
            palette.accent,
            palette.background,
            palette.foreground,
            palette.red,
            palette.green,
            palette.yellow,
        ]
        .iter()
        .any(Option::is_some);
        any.then_some(palette)
    }

    /// libadwaita's colour variables, without the leading `--`, and their
    /// colours. Borders and shades are left to libadwaita, which mixes them
    /// from these.
    fn variables(&self) -> Vec<(&'static str, Rgb)> {
        let mut variables = Vec::new();
        if let Some(background) = self.background {
            for variable in [
                "window-bg-color",
                "view-bg-color",
                "headerbar-bg-color",
                "headerbar-backdrop-color",
                "sidebar-bg-color",
                "sidebar-backdrop-color",
                "secondary-sidebar-bg-color",
                "secondary-sidebar-backdrop-color",
                "dialog-bg-color",
                "popover-bg-color",
                "overview-bg-color",
            ] {
                variables.push((variable, background));
            }
        }
        if let Some(foreground) = self.foreground {
            for variable in [
                "window-fg-color",
                "view-fg-color",
                "headerbar-fg-color",
                "sidebar-fg-color",
                "secondary-sidebar-fg-color",
                "dialog-fg-color",
                "popover-fg-color",
                "overview-fg-color",
                "card-fg-color",
            ] {
                variables.push((variable, foreground));
            }
        }
        for (background, foreground, colour) in [
            ("accent-bg-color", "accent-fg-color", self.accent),
            ("destructive-bg-color", "destructive-fg-color", self.red),
            ("error-bg-color", "error-fg-color", self.red),
            ("success-bg-color", "success-fg-color", self.green),
            ("warning-bg-color", "warning-fg-color", self.yellow),
        ] {
            if let Some(colour) = colour {
                variables.push((background, colour));
                variables.push((foreground, self.label_on(&[colour])));
            }
        }
        variables
    }

    /// The Answer button's colour and the Decline and Hang up buttons'
    /// colour, the theme's green and red, and one label colour for both, so
    /// the pair looks alike.
    fn call_buttons(&self) -> (Rgb, Rgb, Rgb) {
        let green = self.green.unwrap_or(STOCK_GREEN);
        let red = self.red.unwrap_or(STOCK_RED);
        (green, red, self.label_on(&[green, red]))
    }

    /// A label colour readable on every one of `colours`: the theme's own
    /// background or foreground if either is readable, otherwise whichever
    /// of those, white or black is the most readable.
    fn label_on(&self, colours: &[Rgb]) -> Rgb {
        let contrast = |label: Rgb| {
            colours
                .iter()
                .map(|colour| colour.contrast(label))
                .fold(f64::INFINITY, f64::min)
        };
        let most_readable = |candidates: &[Option<Rgb>]| {
            candidates
                .iter()
                .flatten()
                .copied()
                .max_by(|a, b| contrast(*a).total_cmp(&contrast(*b)))
        };
        let themed = most_readable(&[self.background, self.foreground])
            .filter(|label| contrast(*label) >= READABLE);
        themed
            .or_else(|| {
                most_readable(&[self.background, self.foreground, Some(WHITE), Some(BLACK)])
            })
            .unwrap_or(WHITE)
    }
}
