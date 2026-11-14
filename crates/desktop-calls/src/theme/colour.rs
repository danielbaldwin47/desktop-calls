//! sRGB colours, their WCAG contrast, and mixes of two of them.

use std::fmt;

/// The contrast text needs against what is behind it (WCAG AA).
pub(super) const READABLE: f64 = 4.5;

pub(super) const WHITE: Rgb = Rgb(0xff, 0xff, 0xff);
pub(super) const BLACK: Rgb = Rgb(0x00, 0x00, 0x00);

/// An sRGB colour.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Rgb(pub(super) u8, pub(super) u8, pub(super) u8);

impl Rgb {
    /// `#rrggbb` or `#rgb`, in either case; anything else is no colour.
    pub(super) fn parse(text: &str) -> Option<Self> {
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
    pub(super) fn contrast(self, other: Self) -> f64 {
        let (a, b) = (self.luminance(), other.luminance());
        (a.max(b) + 0.05) / (a.min(b) + 0.05)
    }

    /// The lowest contrast against any of `backgrounds`.
    pub(super) fn worst_contrast(self, backgrounds: &[Self]) -> f64 {
        backgrounds
            .iter()
            .map(|background| self.contrast(*background))
            .fold(f64::INFINITY, f64::min)
    }

    /// Whether it reads against every one of `backgrounds`.
    pub(super) fn reads_on(self, backgrounds: &[Self]) -> bool {
        self.worst_contrast(backgrounds) >= READABLE
    }

    pub(super) fn is_dark(self) -> bool {
        self.contrast(BLACK) < self.contrast(WHITE)
    }

    /// `percent` of `top` over this colour, as CSS's `color-mix` in sRGB:
    /// 0 is this colour, 100 is `top`.
    pub(super) fn under(self, top: Self, percent: u8) -> Self {
        let percent = u16::from(percent.min(100));
        let channel = |bottom: u8, top: u8| {
            let mixed = (u16::from(bottom) * (100 - percent) + u16::from(top) * percent + 50) / 100;
            u8::try_from(mixed).unwrap_or(u8::MAX)
        };
        Self(
            channel(self.0, top.0),
            channel(self.1, top.1),
            channel(self.2, top.2),
        )
    }

    /// The least of `top` over this colour, in whole percent, that reads on
    /// every one of `backgrounds`, or `None` if not even all of it does.
    pub(super) fn weakest_readable_mix(self, top: Self, backgrounds: &[Self]) -> Option<Self> {
        (0..=100)
            .map(|percent| self.under(top, percent))
            .find(|mixed| mixed.reads_on(backgrounds))
    }
}

impl fmt::Display for Rgb {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "#{:02x}{:02x}{:02x}", self.0, self.1, self.2)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn black_on_white_is_21_to_1_and_a_colour_on_itself_1_to_1() {
        assert!((BLACK.contrast(WHITE) - 21.0).abs() < 1e-9);
        let grey = Rgb(0x77, 0x77, 0x77);
        assert!((grey.contrast(grey) - 1.0).abs() < 1e-9);
    }

    #[test]
    fn a_mix_runs_from_the_bottom_colour_to_the_top_one() {
        let (bottom, top) = (Rgb(0, 0, 0), Rgb(200, 100, 50));
        assert_eq!(bottom.under(top, 0), bottom);
        assert_eq!(bottom.under(top, 100), top);
        assert_eq!(bottom.under(top, 50), Rgb(100, 50, 25));
    }

    #[test]
    fn the_weakest_readable_mix_of_black_on_white_is_about_54_percent() {
        // #767676 is the lightest grey that reads on white, 4.54 to 1; in
        // whole percent the first step past it is #757575.
        let mixed = WHITE.weakest_readable_mix(BLACK, &[WHITE]);
        assert_eq!(mixed, Some(Rgb(0x75, 0x75, 0x75)));
    }

    #[test]
    fn colours_parse_from_long_and_short_hex_only() {
        assert_eq!(Rgb::parse("#FFfC0a"), Some(Rgb(0xff, 0xfc, 0x0a)));
        assert_eq!(Rgb::parse("#abc"), Some(Rgb(0xaa, 0xbb, 0xcc)));
        assert_eq!(Rgb::parse("abc"), None);
        assert_eq!(Rgb::parse("#abcd"), None);
        assert_eq!(Rgb::parse("#ggg"), None);
    }
}
