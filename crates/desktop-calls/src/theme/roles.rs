//! The colour roles every surface draws with, computed from an Omarchy
//! theme's `colors.toml`, or from stock Adwaita without one.
//!
//! Each role that carries text is chosen to read at 4.5:1 or better against
//! everything it is drawn on, so no theme can make a label unreadable.

use super::colour::{BLACK, READABLE, Rgb, WHITE};

/// Stock Adwaita's colours (libadwaita 1.10), for a Desktop without
/// Omarchy, and for any colour an Omarchy theme lacks.
struct Stock {
    background: Rgb,
    /// Adwaita's text, `rgb(0 0 6 / 80%)` over the background in light.
    foreground: Rgb,
    accent: Rgb,
    warning: Rgb,
}

const STOCK_LIGHT: Stock = Stock {
    background: Rgb(0xfa, 0xfa, 0xfb),
    foreground: Rgb(0x32, 0x32, 0x37),
    accent: Rgb(0x35, 0x84, 0xe4),
    warning: Rgb(0xe5, 0xa5, 0x0a),
};

const STOCK_DARK: Stock = Stock {
    background: Rgb(0x22, 0x22, 0x26),
    foreground: Rgb(0xff, 0xff, 0xff),
    accent: Rgb(0x35, 0x84, 0xe4),
    warning: Rgb(0xcd, 0x93, 0x09),
};

/// The Answer button's green and the Decline and Hang up buttons' red, with
/// white labels, as on a phone, when there is no Omarchy theme. The green is
/// Adwaita's `#26a269` deepened until white reads on it at 4.5:1.
const STOCK_GREEN: Rgb = Rgb(0x20, 0x86, 0x57);
const STOCK_RED: Rgb = Rgb(0xc0, 0x1c, 0x28);

/// How much foreground a selected row's fill takes, over the background.
const SELECTION_PERCENT: u8 = 10;
/// How much foreground a surface's border takes, over the background.
const BORDER_PERCENT: u8 = 30;
/// How much of Answer's green rings the Banner's ringing dot.
const ANSWER_RING_PERCENT: u8 = 30;
/// How much of the warning colour a warning strip's fill takes.
const WARNING_TINT_PERCENT: u8 = 14;
/// How much text libadwaita mixes into a checked toggle's fill, hovered
/// and pressed (libadwaita 1.10's `button:checked`). A checked fill takes
/// no more than its label still reads on at 4.5:1.
const CHECKED_PERCENT: u8 = 30;
const CHECKED_HOVER_PERCENT: u8 = 35;
const CHECKED_PRESSED_PERCENT: u8 = 40;

/// What Desktop Calls takes from an Omarchy theme's `colors.toml`. A colour
/// the theme lacks, or gives in a form other than `#rrggbb` or `#rgb`,
/// stays stock.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Palette {
    pub(super) dark: bool,
    accent: Option<Rgb>,
    background: Option<Rgb>,
    lighter_background: Option<Rgb>,
    foreground: Option<Rgb>,
    red: Option<Rgb>,
    green: Option<Rgb>,
    yellow: Option<Rgb>,
    orange: Option<Rgb>,
}

impl Palette {
    /// `None` if the table has no colour Desktop Calls uses.
    pub(super) fn from_table(table: &toml::Table) -> Option<Self> {
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
            lighter_background: colour("lighter_background"),
            foreground: colour("foreground"),
            red: colour("red"),
            green: colour("green"),
            yellow: colour("yellow"),
            orange: colour("orange"),
        };
        let any = [
            palette.accent,
            palette.background,
            palette.lighter_background,
            palette.foreground,
            palette.red,
            palette.green,
            palette.yellow,
            palette.orange,
        ]
        .iter()
        .any(Option::is_some);
        any.then_some(palette)
    }

    /// The theme's warning colour: its orange, else its yellow.
    fn warning(&self) -> Option<Rgb> {
        self.orange.or(self.yellow)
    }

    /// libadwaita's colour variables, without the leading `--`, and their
    /// colours, so its own widgets follow the theme too. Borders and shades
    /// are left to libadwaita, which mixes them from these.
    pub(super) fn adwaita_variables(&self) -> Vec<(&'static str, Rgb)> {
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
            ("warning-bg-color", "warning-fg-color", self.warning()),
        ] {
            if let Some(colour) = colour {
                variables.push((background, colour));
                variables.push((foreground, self.labels().on(&[colour])));
            }
        }
        variables
    }

    /// Label choices from the theme's background and foreground, stock
    /// where it lacks either.
    fn labels(&self) -> Labels {
        let stock = if self.dark { &STOCK_DARK } else { &STOCK_LIGHT };
        Labels {
            surface: self.background.unwrap_or(stock.background),
            text: self.foreground.unwrap_or(stock.foreground),
        }
    }
}

/// The colours every surface draws with. Each is a role, not a hue: a
/// surface asks for "secondary text" or "warning", never for a colour.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Roles {
    /// A surface's background.
    pub(super) surface: Rgb,
    /// A control or tile raised off the surface.
    pub(super) raised: Rgb,
    /// Text.
    pub(super) text: Rgb,
    /// Text that matters less: the weakest mix of text over the surface
    /// that still reads at 4.5:1 on the surface, a raised control and a
    /// selected row.
    pub(super) secondary: Rgb,
    /// A surface's 1 px edge.
    pub(super) border: Rgb,
    /// Selection and keyboard focus.
    pub(super) accent: Rgb,
    /// A label on the accent.
    pub(super) accent_label: Rgb,
    /// Answer: the green, in the least deeper or paler shade, if need be,
    /// that the call label reads on.
    pub(super) answer: Rgb,
    /// The ring round the Banner's ringing dot: 30% of Answer's green over
    /// the surface.
    pub(super) answer_ring: Rgb,
    /// Decline, Hang up and End: the red, shaded as Answer's green is.
    pub(super) end: Rgb,
    /// One label for both Answer and End, so the pair looks alike: whichever
    /// of the theme's surface and text colours contrasts more on its green
    /// and red, or white or black where neither reads at 4.5:1 and one of
    /// those contrasts more. White on stock Adwaita.
    pub(super) call_label: Rgb,
    /// "Missed", and any red text: the red, made readable on the surface
    /// and on a selected row.
    pub(super) missed: Rgb,
    /// Muted, warnings and the Calls Off strip.
    pub(super) warning: Rgb,
    /// A label on the warning colour.
    pub(super) warning_label: Rgb,
    /// Warning text on the surface or on a warning strip.
    pub(super) warning_text: Rgb,
    /// A warning strip's fill.
    pub(super) warning_tint: Rgb,
    /// A selected row's fill, under a 3 px accent bar.
    pub(super) selection: Rgb,
    /// A checked toggle's fill, such as Keypad's: libadwaita's 30% of text
    /// over the surface, or less where its label and keycap would read
    /// under 4.5:1.
    pub(super) checked: Rgb,
    /// A checked toggle's fill under the pointer: libadwaita's 35%, capped
    /// as [`Self::checked`] is, so where the cap bites it equals it.
    pub(super) checked_hover: Rgb,
    /// A checked toggle's fill while pressed: libadwaita's 40%, capped
    /// likewise.
    pub(super) checked_pressed: Rgb,
}

impl Roles {
    /// The roles for an Omarchy theme. A colour it lacks is stock
    /// Adwaita's, in the theme's mode.
    pub(super) fn omarchy(palette: &Palette) -> Self {
        let stock = if palette.dark {
            &STOCK_DARK
        } else {
            &STOCK_LIGHT
        };
        let surface = palette.background.unwrap_or(stock.background);
        let text = palette.foreground.unwrap_or(stock.foreground);
        let answer = palette.green.unwrap_or(STOCK_GREEN);
        let end = palette.red.unwrap_or(STOCK_RED);
        Self::from_base(Base {
            surface,
            raised: palette.lighter_background,
            text,
            accent: palette.accent.unwrap_or(stock.accent),
            answer,
            end,
            call_label: None,
            warning: palette.warning().unwrap_or(stock.warning),
        })
    }

    /// The roles for stock Adwaita, light or dark. Answer and End keep
    /// white labels, as on a phone.
    pub(super) fn stock(dark: bool) -> Self {
        let stock = if dark { &STOCK_DARK } else { &STOCK_LIGHT };
        Self::from_base(Base {
            surface: stock.background,
            raised: None,
            text: stock.foreground,
            accent: stock.accent,
            answer: STOCK_GREEN,
            end: STOCK_RED,
            call_label: Some(WHITE),
            warning: stock.warning,
        })
    }

    fn from_base(base: Base) -> Self {
        let Base {
            surface,
            raised,
            text,
            accent,
            answer,
            end,
            call_label,
            warning,
        } = base;
        let labels = Labels { surface, text };
        let selection = surface.under(text, SELECTION_PERCENT);
        // libadwaita raises a control by mixing in a tenth of the text.
        let raised = raised.unwrap_or(selection);
        let warning_tint = surface.under(warning, WARNING_TINT_PERCENT);
        // The muted Call Pill keeps its timer in secondary text on the
        // warning strip's fill.
        let backgrounds = [surface, raised, selection, warning_tint];
        let secondary = surface
            .weakest_readable_mix(text, &backgrounds)
            .unwrap_or_else(|| labels.on(&backgrounds));
        let call_label = call_label.unwrap_or_else(|| labels.on(&[answer, end]));
        let answer = under_label(answer, call_label);
        Self {
            surface,
            raised,
            text,
            secondary,
            border: surface.under(text, BORDER_PERCENT),
            accent,
            accent_label: labels.on(&[accent]),
            answer,
            answer_ring: surface.under(answer, ANSWER_RING_PERCENT),
            end: under_label(end, call_label),
            call_label,
            missed: labels.readable(end, &[surface, selection]),
            warning,
            warning_label: labels.on(&[warning]),
            warning_text: labels.readable(warning, &[surface, warning_tint]),
            warning_tint,
            selection,
            checked: fill_under(surface, text, CHECKED_PERCENT),
            checked_hover: fill_under(surface, text, CHECKED_HOVER_PERCENT),
            checked_pressed: fill_under(surface, text, CHECKED_PRESSED_PERCENT),
        }
    }
}

/// Up to `percent` of `label` over `surface`, as much as the label still
/// reads on at 4.5:1: a control's fill under its own label.
fn fill_under(surface: Rgb, label: Rgb, percent: u8) -> Rgb {
    (0..=percent)
        .rev()
        .map(|percent| surface.under(label, percent))
        .find(|fill| label.reads_on(&[*fill]))
        .unwrap_or(surface)
}

/// `fill` as a button under `label`: itself if the label reads on it, else
/// the least shade of it, away from the label, that the label reads on.
/// Few greens and reds read at 4.5:1 under any one label, so Answer and End
/// keep one label and give way in shade instead.
fn under_label(fill: Rgb, label: Rgb) -> Rgb {
    let away = if label.is_dark() { WHITE } else { BLACK };
    fill.weakest_readable_mix(away, &[label]).unwrap_or(fill)
}

/// What the roles are computed from.
#[derive(Clone, Copy)]
struct Base {
    surface: Rgb,
    /// The theme's own raised colour, if it has one.
    raised: Option<Rgb>,
    text: Rgb,
    accent: Rgb,
    answer: Rgb,
    end: Rgb,
    /// A fixed label for Answer and End, else one chosen to read.
    call_label: Option<Rgb>,
    warning: Rgb,
}

/// Chooses label colours from the theme's own surface and text colours.
struct Labels {
    surface: Rgb,
    text: Rgb,
}

impl Labels {
    /// A label colour readable on every one of `colours`: the theme's
    /// surface or text colour if either reads, otherwise whichever of
    /// those, white or black reads best.
    fn on(&self, colours: &[Rgb]) -> Rgb {
        let best = |candidates: &[Rgb]| {
            candidates
                .iter()
                .copied()
                .max_by(|a, b| {
                    a.worst_contrast(colours)
                        .total_cmp(&b.worst_contrast(colours))
                })
                .unwrap_or(WHITE)
        };
        let themed = best(&[self.surface, self.text]);
        if themed.worst_contrast(colours) >= READABLE {
            themed
        } else {
            best(&[self.surface, self.text, WHITE, BLACK])
        }
    }

    /// `colour` as text on `backgrounds`: itself if it reads, else the
    /// least mix of it towards the text colour that does, else a label.
    fn readable(&self, colour: Rgb, backgrounds: &[Rgb]) -> Rgb {
        colour
            .weakest_readable_mix(self.text, backgrounds)
            .unwrap_or_else(|| self.on(backgrounds))
    }
}

#[cfg(test)]
#[path = "roles_tests.rs"]
mod tests;
