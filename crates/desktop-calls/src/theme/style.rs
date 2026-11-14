//! The CSS every surface draws with: the colour roles as CSS variables, the
//! type roles and shared pieces as classes, and the corners.
//!
//! A surface takes its look only from here. It adds a class from [`class`]
//! to a widget and never names a colour, size of type or corner itself.

use std::fmt::Write as _;

use super::roles::Roles;

/// The CSS classes a surface puts on its widgets.
pub(crate) mod class {
    /// Who the Call is with: sans 20/650. With [`NUMBER`] too, a bare
    /// number in monospace 18.
    pub(crate) const WHO: &str = "type-who";
    /// A bare number where a name would be: monospace with tabular figures,
    /// at the size its other classes give.
    pub(crate) const NUMBER: &str = "number";
    /// A window's title: sans 18/650.
    pub(crate) const HEADING: &str = "type-heading";
    /// Buttons, rows and instructions: sans 13.5/400.
    #[expect(
        dead_code,
        reason = "for the surfaces spec #221 rebuilds; the CSS is in place"
    )]
    pub(crate) const BODY: &str = "type-body";
    /// Numbers, labels, times and counts: monospace 12/400 with tabular
    /// figures, in secondary text.
    pub(crate) const META: &str = "type-meta";
    /// A Call's running time: monospace 18/500 with tabular figures; 13 in
    /// secondary text on the Call Pill. With [`WORDS`] too, the same line
    /// says something other than a time, in sans, so it keeps its height.
    pub(crate) const TIMER: &str = "type-timer";
    /// With [`TIMER`]: the line holds words, not a time.
    pub(crate) const WORDS: &str = "words";
    /// A small heading over a group: monospace 10.5/600 in capitals.
    #[expect(
        dead_code,
        reason = "for the surfaces spec #221 rebuilds; the CSS is in place"
    )]
    pub(crate) const EYEBROW: &str = "type-eyebrow";
    /// A key, in a 1 px outlined box at 72%: monospace 10.5/500.
    pub(crate) const KEYCAP: &str = "type-keycap";
    /// One row of keycaps with what each does, at a surface's foot.
    #[expect(
        dead_code,
        reason = "for the surfaces spec #221 rebuilds; the CSS is in place"
    )]
    pub(crate) const FOOTER_LINE: &str = "footer-line";
    /// On a surface: it has the keyboard, so its border takes the accent.
    pub(crate) const HAS_KEYBOARD: &str = "has-keyboard";
    /// On a surface: the microphone is muted, so its border takes the
    /// warning colour.
    pub(crate) const MUTED: &str = "muted";
    /// On a surface: the Desktop has lost contact with the Call, so its
    /// border takes the warning colour.
    pub(crate) const CONTACT_LOST: &str = "contact-lost";
    /// Red text: a Missed Call.
    pub(crate) const MISSED: &str = "missed";
    /// A strip that says the microphone is muted, in the warning colour.
    pub(crate) const WARNING_STRIP: &str = "warning-strip";
}

/// The gaps between things, in pixels: the scale the surfaces #224 to #228
/// rebuild lay out on.
#[expect(
    dead_code,
    reason = "the scale the surfaces spec #221 rebuilds lay out on"
)]
pub(crate) mod space {
    pub(crate) const XS: i32 = 4;
    pub(crate) const S: i32 = 8;
    pub(crate) const M: i32 = 12;
    pub(crate) const L: i32 = 16;
    pub(crate) const XL: i32 = 20;
}

/// The largest corner radius a control takes, and a keycap.
const CONTROL_RADIUS: u32 = 8;
const KEYCAP_RADIUS: u32 = 3;

/// The roles, types and pieces that don't depend on the theme. Colours come
/// only from the `--dc-*` variables [`css`] defines.
const STATIC: &str = r#"
.number { font-family: monospace; font-feature-settings: "tnum"; }
.type-who { font-size: 20px; font-weight: 650; }
.type-who.number { font-size: 18px; font-weight: 500; }
.type-heading { font-size: 18px; font-weight: 650; }
.type-body { font-size: 13.5px; font-weight: 400; }
.type-meta {
    font-family: monospace; font-size: 12px; font-weight: 400;
    font-feature-settings: "tnum"; color: var(--dc-secondary);
}
.type-timer {
    font-family: monospace; font-size: 18px; font-weight: 500;
    font-feature-settings: "tnum";
}
.type-timer.words { font-family: unset; font-feature-settings: unset; }
.call-pill .type-timer { font-size: 13px; color: var(--dc-secondary); }
.type-eyebrow {
    font-family: monospace; font-size: 10.5px; font-weight: 600;
    text-transform: uppercase; letter-spacing: 0.7px; color: var(--dc-secondary);
}
.type-keycap {
    font-family: monospace; font-size: 10.5px; font-weight: 500;
    padding: 1px 4px; border: 1px solid currentColor; opacity: 0.72;
}
.footer-line {
    border-spacing: 16px; padding-top: 12px; border-top: 1px solid var(--dc-border);
    font-size: 11.5px; color: var(--dc-secondary);
}
.footer-line > box { border-spacing: 6px; }

.dim-label { opacity: 1; color: var(--dc-secondary); }
.missed { color: var(--dc-missed); }

window.desktop-calls-surface { box-shadow: inset 0 0 0 1px var(--dc-border); }
window.desktop-calls-surface.has-keyboard { box-shadow: inset 0 0 0 2px var(--dc-accent); }
window.desktop-calls-surface.muted,
window.desktop-calls-surface.contact-lost { box-shadow: inset 0 0 0 2px var(--dc-warning); }

.desktop-calls-surface button.answer { background: var(--dc-answer); color: var(--dc-call-label); }
.desktop-calls-surface button.decline,
.desktop-calls-surface button.hang-up { background: var(--dc-end); color: var(--dc-call-label); }
.desktop-calls-surface button.mute.muted,
.desktop-calls-surface .warning-strip {
    background: var(--dc-warning); color: var(--dc-warning-label);
}
.desktop-calls-surface .warning-strip { font-weight: bold; padding: 8px; }

.desktop-calls-surface row:selected,
.desktop-calls-surface row:selected:hover {
    background: var(--dc-selection); color: inherit;
    box-shadow: inset 3px 0 0 var(--dc-accent);
}
"#;

/// The CSS for `roles`, the libadwaita colour variables to override
/// (`name`, colour) for an Omarchy theme, and Hyprland's `rounding` (stock
/// corners without one).
pub(super) fn css(
    roles: &Roles,
    adwaita: &[(&'static str, super::colour::Rgb)],
    rounding: Option<u32>,
) -> String {
    let mut css = String::from(":root {\n");
    for (variable, colour) in adwaita {
        let _ = writeln!(css, "  --{variable}: {colour};");
    }
    for (variable, colour) in [
        ("surface", roles.surface),
        ("raised", roles.raised),
        ("text", roles.text),
        ("secondary", roles.secondary),
        ("border", roles.border),
        ("accent", roles.accent),
        ("accent-label", roles.accent_label),
        ("answer", roles.answer),
        ("end", roles.end),
        ("call-label", roles.call_label),
        ("missed", roles.missed),
        ("warning", roles.warning),
        ("warning-label", roles.warning_label),
        ("warning-text", roles.warning_text),
        ("warning-tint", roles.warning_tint),
        ("selection", roles.selection),
    ] {
        let _ = writeln!(css, "  --dc-{variable}: {colour};");
    }
    css.push_str("}\n");
    css.push_str(STATIC);
    let keycap = rounding.map_or(KEYCAP_RADIUS, |rounding| rounding.min(KEYCAP_RADIUS));
    let _ = writeln!(
        css,
        ".type-keycap, .desktop-calls-surface .hd {{ border-radius: {keycap}px; }}"
    );
    if let Some(rounding) = rounding {
        let control = rounding.min(CONTROL_RADIUS);
        let _ = writeln!(
            css,
            ".desktop-calls-surface {{ border-radius: {rounding}px; }}\n\
             .desktop-calls-surface button,\n\
             .desktop-calls-surface entry,\n\
             .desktop-calls-surface .warning-strip {{ border-radius: {control}px; }}"
        );
    }
    css
}
