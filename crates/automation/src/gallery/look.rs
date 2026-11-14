//! How the Desktop looks in each column of the gallery, and the colours
//! its checks look for under each look.

use test_harness::{Builder, Harness, OmarchyTheme, Theme};

/// The stock Answer button's green, the stock Decline and Hang up buttons'
/// red, and stock Adwaita's warning colour, light and dark, which marks the
/// microphone muted without Omarchy, and stock Adwaita's accent.
const GREEN: [u8; 3] = [0x20, 0x86, 0x57];
const RED: [u8; 3] = [0xc0, 0x1c, 0x28];
const STOCK_WARNING_LIGHT: [u8; 3] = [0xe5, 0xa5, 0x0a];
const STOCK_WARNING_DARK: [u8; 3] = [0xcd, 0x93, 0x09];
const STOCK_ACCENT: [u8; 3] = [0x35, 0x84, 0xe4];

/// How the Desktop looks in one column of the gallery.
#[derive(Clone, Copy)]
pub(super) enum Look {
    /// No Omarchy, light or dark as the settings portal says.
    Stock(Theme),
    Omarchy(OmarchyTheme),
}

pub(super) const LOOKS: [Look; 4] = [
    Look::Stock(Theme::Light),
    Look::Stock(Theme::Dark),
    Look::Omarchy(OmarchyTheme::Light),
    Look::Omarchy(OmarchyTheme::Dark),
];

impl Look {
    /// Its name, for file names and captions.
    pub(super) fn as_str(self) -> &'static str {
        match self {
            Self::Stock(theme) => theme.as_str(),
            Self::Omarchy(theme) => theme.as_str(),
        }
    }

    /// The Answer button's colour.
    pub(super) fn green(self) -> [u8; 3] {
        match self {
            Self::Stock(_) => GREEN,
            Self::Omarchy(theme) => theme.colour("green"),
        }
    }

    /// The Decline and Hang up buttons' colour.
    pub(super) fn red(self) -> [u8; 3] {
        match self {
            Self::Stock(_) => RED,
            Self::Omarchy(theme) => theme.colour("red"),
        }
    }

    /// The colour that marks the microphone muted: the theme's orange, or
    /// stock Adwaita's warning colour.
    pub(super) fn warning(self) -> [u8; 3] {
        match self {
            Self::Stock(Theme::Light) => STOCK_WARNING_LIGHT,
            Self::Stock(Theme::Dark) => STOCK_WARNING_DARK,
            Self::Omarchy(theme) => theme.colour("orange"),
        }
    }

    /// The colour of a surface's border while it has the keyboard.
    pub(super) fn accent(self) -> [u8; 3] {
        match self {
            Self::Stock(_) => STOCK_ACCENT,
            Self::Omarchy(theme) => theme.colour("accent"),
        }
    }

    /// Sets the Desktop up to look this way. Omarchy also tells the settings
    /// portal whether its theme is light or dark.
    pub(super) fn builder(self) -> Builder {
        match self {
            Self::Stock(theme) => Harness::builder().theme(theme),
            Self::Omarchy(theme) => Harness::builder().theme(theme.mode()).omarchy_theme(theme),
        }
    }
}
