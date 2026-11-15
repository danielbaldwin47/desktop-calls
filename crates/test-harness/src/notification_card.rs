//! A notification drawn as Omarchy's notification card draws it, for the
//! gallery: a stand-in, since the gallery runs no Omarchy shell.
//!
//! It draws the [`Notification`] the app actually sent, as the
//! notification server stand-in received it, in the style of Omarchy
//! 4.5's card (`plugins/notifications/Toast.qml`): 380 logical pixels wide
//! with a 2 px border in the theme's accent; inside 12 at the sides and 10
//! at the top, a 40 px icon slot holding the `omarchy-glyph` hint's glyph
//! at 28 px in the text colour; beside it the summary in Liberation Sans
//! 14 bold, "now" at the right in the monospace font at 11 in the dim
//! colour (the text colour darkened by 1.4), and the body in Liberation
//! Sans 14 in the body colour (darkened by 1.15), on one line here. The
//! front card's buttons run along its bottom edge: every action but
//! `default` (the card's click), in equal segments 34 high split by
//! hairlines in the dim colour at 35%, labels in the monospace font at 12
//! in the body colour. Nothing is pointed at, so nothing has the hover
//! fill. Drawn with `rsvg-convert`, as [`crate::draw_tray_menu`] draws.

use std::fmt::Write as _;
use std::path::Path;

use crate::stand_ins::Notification;
use crate::tray_menu::{escape, hex, render};

/// Pixels to the logical pixel, as the other stand-ins draw.
const SCALE: u32 = 2;
const WIDTH: u32 = 380;
const BORDER: u32 = 2;
const PADDING_X: u32 = 12;
const PADDING_Y: u32 = 10;
/// The icon slot's side, the glyph's size in it, and the gap after it.
const SLOT: u32 = 40;
const GLYPH_SIZE: u32 = 28;
const SLOT_GAP: u32 = 12;
/// The summary's and body's font size, and the height of a line of it.
const TEXT_SIZE: u32 = 14;
const LINE: u32 = 18;
/// The timestamp's font size, and the buttons'.
const STAMP_SIZE: u32 = 11;
const BUTTON_SIZE: u32 = 12;
/// The room the timestamp takes at the summary's right, and about how wide
/// a character of Liberation Sans 14 is, in tenths of a pixel.
const STAMP_ROOM: u32 = 32;
const ADVANCE_TENTHS: u32 = 78;
/// A button segment's height: Omarchy's control height and 6.
const SEGMENT: u32 = 34;
/// Omarchy's darkening of the text colour for the body, and for the
/// timestamp and hairlines.
const BODY_DARKER: f64 = 1.15;
const DIM_DARKER: f64 = 1.4;
const RULE_OPACITY: f64 = 0.35;
/// The fonts: Omarchy's card names Liberation Sans for its text, and the
/// glyph comes from the Nerd Font Omarchy's monospace font is.
const TEXT_FONT: &str = "Liberation Sans, sans-serif";
const GLYPH_FONT: &str = "Symbols Nerd Font Mono, JetBrainsMono Nerd Font, monospace";

/// The colours the card is drawn in.
#[derive(Clone, Copy, Debug)]
pub struct CardColours {
    /// The card's background: the theme's background.
    pub background: [u8; 3],
    /// The text colour: the theme's foreground.
    pub text: [u8; 3],
    /// The border: the theme's accent.
    pub border: [u8; 3],
}

/// Draws `notification` as Omarchy's front notification card shows it,
/// into the PNG at `output`.
///
/// # Errors
///
/// If `rsvg-convert` can't draw it, or the PNG can't be written.
pub fn draw_notification_card(
    notification: &Notification,
    colours: CardColours,
    output: &Path,
) -> Result<(), String> {
    let text = hex(colours.text);
    let body_colour = hex(darker(colours.text, BODY_DARKER));
    let dim = hex(darker(colours.text, DIM_DARKER));
    let left = BORDER + PADDING_X;
    let top = BORDER + PADDING_Y;
    let has_glyph = !notification.glyph.is_empty();
    let text_left = if has_glyph {
        left + SLOT + SLOT_GAP
    } else {
        left
    };
    let right = WIDTH - BORDER - PADDING_X;
    let lines = if notification.body.is_empty() { 1 } else { 2 };
    let content = SLOT.max(lines * LINE);
    let buttons: Vec<&str> = notification
        .actions
        .iter()
        .filter(|(key, _)| key != "default")
        .map(|(_, label)| label.as_str())
        .collect();
    let bar = if buttons.is_empty() { 0 } else { SEGMENT + 1 };
    let height = top + content + PADDING_Y + bar + BORDER;

    let mut svg = String::new();
    if has_glyph {
        let _ = write!(
            svg,
            "<text x=\"{x}\" y=\"{y}\" text-anchor=\"middle\" font-family=\"{GLYPH_FONT}\" \
             font-size=\"{GLYPH_SIZE}\" fill=\"{text}\">{glyph}</text>",
            x = left + SLOT / 2,
            y = top + SLOT / 2 + GLYPH_SIZE / 3,
            glyph = escape(&notification.glyph),
        );
    }
    let _ = write!(
        svg,
        "<text x=\"{text_left}\" y=\"{y}\" font-family=\"{TEXT_FONT}\" font-size=\"{TEXT_SIZE}\" \
         font-weight=\"bold\" fill=\"{text}\">{summary}</text>\
         <text x=\"{right}\" y=\"{y}\" text-anchor=\"end\" font-family=\"monospace\" \
         font-size=\"{STAMP_SIZE}\" fill=\"{dim}\">now</text>",
        y = top + LINE - 4,
        summary = escape(&elide(
            &notification.summary,
            right - text_left - STAMP_ROOM
        )),
    );
    if !notification.body.is_empty() {
        let _ = write!(
            svg,
            "<text x=\"{text_left}\" y=\"{y}\" font-family=\"{TEXT_FONT}\" \
             font-size=\"{TEXT_SIZE}\" fill=\"{body_colour}\">{body}</text>",
            y = top + 2 * LINE - 4,
            body = escape(&elide(&notification.body, right - text_left)),
        );
    }
    if !buttons.is_empty() {
        let bar_top = height - BORDER - bar;
        let inner = WIDTH - 2 * BORDER;
        let _ = write!(
            svg,
            "<rect x=\"{BORDER}\" y=\"{bar_top}\" width=\"{inner}\" height=\"1\" fill=\"{dim}\" \
             fill-opacity=\"{RULE_OPACITY}\"/>"
        );
        let count = u32::try_from(buttons.len()).unwrap_or(1);
        for (index, label) in (0..).zip(&buttons) {
            let start = BORDER + index * inner / count;
            let end = BORDER + (index + 1) * inner / count;
            if index > 0 {
                let _ = write!(
                    svg,
                    "<rect x=\"{start}\" y=\"{bar_top}\" width=\"1\" height=\"{bar}\" \
                     fill=\"{dim}\" fill-opacity=\"{RULE_OPACITY}\"/>"
                );
            }
            let _ = write!(
                svg,
                "<text x=\"{x}\" y=\"{y}\" text-anchor=\"middle\" font-family=\"monospace\" \
                 font-size=\"{BUTTON_SIZE}\" fill=\"{body_colour}\">{label}</text>",
                x = u32::midpoint(start, end),
                y = bar_top + 1 + SEGMENT / 2 + 4,
                label = escape(label),
            );
        }
    }
    let document = format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{w}\" height=\"{h}\" \
         viewBox=\"0 0 {WIDTH} {height}\">\
         <rect width=\"{WIDTH}\" height=\"{height}\" fill=\"{background}\"/>\
         <rect x=\"1\" y=\"1\" width=\"{inner_w}\" height=\"{inner_h}\" fill=\"none\" \
         stroke=\"{border}\" stroke-width=\"{BORDER}\"/>{svg}</svg>",
        w = WIDTH * SCALE,
        h = height * SCALE,
        inner_w = WIDTH - BORDER,
        inner_h = height - BORDER,
        background = hex(colours.background),
        border = hex(colours.border),
    );
    render(&document, output)
}

/// `text` cut short with `…` to fit `room` logical pixels at
/// [`ADVANCE_TENTHS`], so a long name stays on the card. Omarchy wraps a
/// long summary instead; the stand-in keeps to one line.
fn elide(text: &str, room: u32) -> String {
    let fits = usize::try_from(room * 10 / ADVANCE_TENTHS).unwrap_or(usize::MAX);
    if text.chars().count() <= fits {
        return text.to_owned();
    }
    text.chars()
        .take(fits.saturating_sub(1))
        .chain(['\u{2026}'])
        .collect()
}

/// `colour` darkened as Qt's `darker(colour, factor)` does: its value
/// divided by `factor`, hue and saturation kept.
fn darker(colour: [u8; 3], factor: f64) -> [u8; 3] {
    colour.map(|channel| {
        // At most 255 and at least 0, so the cast is exact.
        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "the value is rounded and within 0 to 255"
        )]
        let darkened = (f64::from(channel) / factor).round() as u8;
        darkened
    })
}
