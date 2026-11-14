//! A tray item's menu drawn as Omarchy's tray draws it, for the gallery: a
//! stand-in, since the gallery runs no Omarchy shell.
//!
//! It draws the menu layout the app publishes over dbusmenu, read back as
//! [`MenuEntry`]s, in the style of Omarchy 4.5's tray menu
//! (`plugins/bar/widgets/Tray.qml`): a card 232 logical pixels wide inside
//! 8 of padding, with a 1 px border in the bar's foreground at 45%; rows
//! 30 high in the monospace font at 11 px, the label 28 in from the left;
//! a separator an 11-high gap with a 1 px line in the popup border colour
//! (the theme's accent) at 45%, 10 in from each side; a disabled entry at
//! 45% opacity; `›` at the right of an entry with a submenu, and a tick in
//! the first 22 pixels of a ticked one. At the root it hides what Omarchy
//! hides: a first entry with a submenu named as the item is, and a
//! separator among the first two rows. Omarchy opens a submenu in place
//! of the menu, under a header row of `‹` and the entry's label and a
//! separator line, which [`draw_tray_menu`] draws for `submenu`. Nothing
//! is pointed at, so no row has Omarchy's hover fill. Drawn with
//! `rsvg-convert`, as [`crate::draw_tray_icon`] draws the icon.

use std::fmt::Write as _;
use std::fs;
use std::io::Write as _;
use std::path::Path;
use std::process::{Command, Stdio};

use crate::stand_ins::MenuEntry;

/// Pixels to the logical pixel, as [`crate::draw_tray_icon`] draws.
const SCALE: u32 = 2;
/// The card's content width and padding, in logical pixels.
const CONTENT_WIDTH: u32 = 232;
const PADDING: u32 = 8;
/// A row's height, and a separator's.
const ROW: u32 = 30;
const SEPARATOR: u32 = 11;
/// The label's font size, and its distance from the row's left edge.
const FONT_SIZE: u32 = 11;
const LABEL_LEFT: u32 = 28;
/// How far a separator's line, and the submenu glyph, keep from the sides.
const INSET: u32 = 10;
/// The room a label leaves on its right: 8, and with a submenu also the
/// glyph and its 10 from the edge.
const LABEL_RIGHT: u32 = 8;
const LABEL_RIGHT_WITH_SUBMENU: u32 = 26;
/// One character of the monospace font at 11 px is about 6.6 wide: a
/// label is elided to what fits at that width.
const ADVANCE_TENTHS: u32 = 66;
/// The opacity Omarchy gives a disabled entry, the card's border and a
/// separator's line.
const DIM: f64 = 0.45;

/// The colours the menu is drawn in.
#[derive(Clone, Copy, Debug)]
pub struct MenuColours {
    /// The popup's background: the theme's background.
    pub background: [u8; 3],
    /// The bar's foreground: labels and the card's border.
    pub foreground: [u8; 3],
    /// The popup border colour, for separator lines: the theme's accent.
    pub separator: [u8; 3],
}

/// Draws the menu `entries` of the tray item titled `title` as Omarchy's
/// tray shows it: at the root, or with `submenu`, the entry with that
/// label opened. Into the PNG at `output`.
///
/// # Errors
///
/// If `submenu` names no entry with a submenu, `rsvg-convert` can't draw
/// it, or the PNG can't be written.
pub fn draw_tray_menu(
    entries: &[MenuEntry],
    title: &str,
    submenu: Option<&str>,
    colours: MenuColours,
    output: &Path,
) -> Result<(), String> {
    let mut rows = String::new();
    let mut y = PADDING;
    let shown: Vec<&MenuEntry> = if let Some(label) = submenu {
        let entry = MenuEntry::find(entries, label)
            .filter(|entry| !entry.children.is_empty())
            .ok_or_else(|| format!("the menu has no submenu {label:?}: {entries:#?}"))?;
        draw_row(&mut rows, y, "\u{2039}", label, true, false, colours);
        y += ROW;
        draw_line(&mut rows, y, colours);
        y += SEPARATOR;
        entry.children.iter().collect()
    } else {
        entries
            .iter()
            .enumerate()
            .filter(|(index, entry)| {
                let title_entry = *index == 0
                    && !entry.children.is_empty()
                    && entry.label.eq_ignore_ascii_case(title);
                let leading_separator = entry.separator && *index <= 1;
                !title_entry && !leading_separator
            })
            .map(|(_, entry)| entry)
            .collect()
    };
    for entry in shown {
        if entry.separator {
            draw_line(&mut rows, y, colours);
            y += SEPARATOR;
        } else {
            let tick = if entry.ticked == Some(true) {
                "\u{2713}"
            } else {
                ""
            };
            let submenu = !entry.children.is_empty();
            draw_row(
                &mut rows,
                y,
                tick,
                &entry.label,
                entry.enabled,
                submenu,
                colours,
            );
            y += ROW;
        }
    }
    let width = CONTENT_WIDTH + 2 * PADDING;
    let height = y + PADDING;
    let svg = format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{w}\" height=\"{h}\" \
         viewBox=\"0 0 {width} {height}\" font-family=\"monospace\" font-size=\"{FONT_SIZE}\">\
         <rect width=\"{width}\" height=\"{height}\" fill=\"{background}\"/>\
         <rect x=\"0.5\" y=\"0.5\" width=\"{inner_w}\" height=\"{inner_h}\" fill=\"none\" \
         stroke=\"{foreground}\" stroke-opacity=\"{DIM}\"/>{rows}</svg>",
        w = width * SCALE,
        h = height * SCALE,
        inner_w = width - 1,
        inner_h = height - 1,
        background = hex(colours.background),
        foreground = hex(colours.foreground),
    );
    render(&svg, output)
}

/// One entry's row at `y`: `lead` centred in the first 22 pixels (a tick,
/// or the submenu header's `‹`), the label, elided to fit, and `›` for an
/// entry with a submenu.
fn draw_row(
    svg: &mut String,
    y: u32,
    lead: &str,
    label: &str,
    enabled: bool,
    submenu: bool,
    colours: MenuColours,
) {
    let opacity = if enabled { 1.0 } else { DIM };
    // Text sits on its baseline: about a third of the font size below the
    // row's middle centres it.
    let baseline = y + ROW / 2 + 4;
    let right = if submenu {
        LABEL_RIGHT_WITH_SUBMENU
    } else {
        LABEL_RIGHT
    };
    let _ = write!(
        svg,
        "<g fill=\"{foreground}\" opacity=\"{opacity}\">\
         <text x=\"{lead_x}\" y=\"{baseline}\" text-anchor=\"middle\">{lead}</text>\
         <text x=\"{label_x}\" y=\"{baseline}\">{label}</text>",
        foreground = hex(colours.foreground),
        lead_x = PADDING + 11,
        label_x = PADDING + LABEL_LEFT,
        lead = escape(lead),
        label = escape(&elide(label, CONTENT_WIDTH - LABEL_LEFT - right)),
    );
    if submenu {
        let _ = write!(
            svg,
            "<text x=\"{x}\" y=\"{baseline}\" text-anchor=\"end\">\u{203a}</text>",
            x = PADDING + CONTENT_WIDTH - INSET,
        );
    }
    svg.push_str("</g>");
}

/// A separator's line, centred in the gap that starts at `y`.
fn draw_line(svg: &mut String, y: u32, colours: MenuColours) {
    let _ = write!(
        svg,
        "<rect x=\"{x}\" y=\"{line_y}\" width=\"{width}\" height=\"1\" fill=\"{colour}\" \
         fill-opacity=\"{DIM}\"/>",
        x = PADDING + INSET,
        line_y = y + SEPARATOR / 2,
        width = CONTENT_WIDTH - 2 * INSET,
        colour = hex(colours.separator),
    );
}

/// `label` cut short with `…` to fit `room` logical pixels, as Omarchy's
/// `elide: Text.ElideRight` does.
fn elide(label: &str, room: u32) -> String {
    let fits = usize::try_from(room * 10 / ADVANCE_TENTHS).unwrap_or(usize::MAX);
    if label.chars().count() <= fits {
        return label.to_owned();
    }
    label
        .chars()
        .take(fits.saturating_sub(1))
        .chain(['\u{2026}'])
        .collect()
}

fn hex([red, green, blue]: [u8; 3]) -> String {
    format!("#{red:02x}{green:02x}{blue:02x}")
}

/// `text` with the characters XML reads as markup escaped.
fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// Draws `svg` with `rsvg-convert` into the PNG at `output`.
fn render(svg: &str, output: &Path) -> Result<(), String> {
    if let Some(parent) = output.parent() {
        fs::create_dir_all(parent).map_err(|error| format!("{}: {error}", parent.display()))?;
    }
    let mut child = Command::new("rsvg-convert")
        .args(["--format", "png", "--output"])
        .arg(output)
        .stdin(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| format!("run rsvg-convert: {error}"))?;
    child
        .stdin
        .take()
        .ok_or("rsvg-convert has no stdin")?
        .write_all(svg.as_bytes())
        .map_err(|error| format!("write to rsvg-convert: {error}"))?;
    let finished = child
        .wait_with_output()
        .map_err(|error| format!("wait for rsvg-convert: {error}"))?;
    if finished.status.success() {
        Ok(())
    } else {
        Err(format!(
            "rsvg-convert {}: {}",
            output.display(),
            String::from_utf8_lossy(&finished.stderr)
        ))
    }
}
