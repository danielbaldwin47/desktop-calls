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
    /// number in monospace 18/650.
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
    pub(crate) const EYEBROW: &str = "type-eyebrow";
    /// A key, in a 1 px outlined box: monospace 10.5/500, in its label's
    /// colour at full strength, so it reads at 4.5:1 like any text.
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
    /// One of the Call Window's three tiles: 58 high, its icon over its
    /// label in 12, its keycap in the corner. With [`MUTED`], in the warning
    /// colour; with [`ACCENT`], in the accent.
    pub(crate) const TILE: &str = "tile";
    /// A control filled with the accent: the one action that fixes things.
    pub(crate) const ACCENT: &str = "accent";
    /// A small button, 30 high.
    pub(crate) const SMALL: &str = "small";
    /// The Call Window's state line: 12.5, in secondary text; with
    /// [`MUTED`], in warning text.
    pub(crate) const STATE_LINE: &str = "state-line";
    /// A notice on the warning tint, its icon in warning text.
    pub(crate) const NOTICE: &str = "notice";
    /// A row above a rule: the Held Call's.
    pub(crate) const HELD_ROW: &str = "held-row";
    /// A timer struck through and in secondary text: it stopped counting.
    pub(crate) const STRUCK: &str = "struck";
    /// On a surface: the Call is over, so who is in secondary text.
    pub(crate) const ENDED: &str = "ended";
    /// The keypad's tone entry: monospace 15, spaced.
    pub(crate) const TONES: &str = "tones";
    /// The keypad's buttons: monospace 15/500, 34 high.
    pub(crate) const KEYPAD: &str = "keypad";
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
.type-who.number { font-size: 18px; }
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
    padding: 1px 4px; border: 1px solid currentColor;
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

.desktop-calls-surface button:checked { background-color: var(--dc-checked); }
.desktop-calls-surface button:checked:hover { background-color: var(--dc-checked-hover); }
.desktop-calls-surface button:checked:active,
.desktop-calls-surface button:checked.keyboard-activating {
    background-color: var(--dc-checked-pressed);
}

.desktop-calls-surface button.answer { background: var(--dc-answer); color: var(--dc-call-label); }
.desktop-calls-surface button.decline,
.desktop-calls-surface button.hang-up { background: var(--dc-end); color: var(--dc-call-label); }
.desktop-calls-surface button.mute.muted {
    background: var(--dc-warning); color: var(--dc-warning-label);
}

.desktop-calls-surface button.hang-up,
.desktop-calls-surface button.resume { min-height: 36px; padding-top: 0; padding-bottom: 0; }
.desktop-calls-surface button.tile {
    min-height: 58px; padding: 0 4px; font-size: 12px; font-weight: 600;
}
.desktop-calls-surface button.tile .type-keycap { border-color: transparent; padding: 0; }
.desktop-calls-surface button.tile:checked { box-shadow: inset 0 0 0 2px var(--dc-accent); }
.desktop-calls-surface button.tile.accent {
    background: var(--dc-accent); color: var(--dc-accent-label);
}
.desktop-calls-surface button.small {
    min-height: 30px; min-width: 0; padding: 0 10px; font-size: 12.5px;
}
.desktop-calls-surface button.hang-up-held {
    background: transparent; color: var(--dc-missed);
    box-shadow: inset 0 0 0 1px var(--dc-end);
}
.desktop-calls-surface button.hang-up-held:hover,
.desktop-calls-surface button.hang-up-held:focus-visible {
    background: var(--dc-end); color: var(--dc-call-label); box-shadow: none;
}
.state-line { font-size: 12.5px; color: var(--dc-secondary); }
.state-line.muted { color: var(--dc-warning-text); font-weight: 600; }
.notice { padding: 10px 12px; background: var(--dc-warning-tint); font-size: 12.5px; }
.notice .notice-icon { color: var(--dc-warning-text); }
.notice .notice-heading { font-weight: 600; }
.held-row { padding-bottom: 12px; border-bottom: 1px solid var(--dc-border); }
.held-caller { font-weight: 600; }
.held-row .dim-label { font-size: 12.5px; }
.type-timer.struck { text-decoration-line: line-through; color: var(--dc-secondary); }
window.desktop-calls-surface.ended .type-who,
window.desktop-calls-surface.ended .type-meta,
window.desktop-calls-surface.ended .state-line { color: var(--dc-secondary); }
entry.tones {
    font-family: monospace; font-size: 15px; letter-spacing: 1px; min-height: 34px;
}
.keypad button {
    font-family: monospace; font-size: 15px; font-weight: 500; min-height: 34px; padding: 0;
}

.pill-caller { font-size: 13.5px; font-weight: 600; }
.pill-tag {
    font-family: monospace; font-size: 11px; font-weight: 500; color: var(--dc-secondary);
}
.pill-state { color: var(--dc-secondary); }
.pill-state.muted,
window.call-pill.muted .pill-tag { color: var(--dc-warning-text); font-weight: 600; }
.live-dot {
    min-width: 8px; min-height: 8px; border-radius: 4px; background: var(--dc-answer);
}
.pill-divider { min-width: 1px; margin: 4px 2px; background: var(--dc-border); }
window.call-pill.hovered { background-color: var(--dc-selection); }
window.call-pill.muted { background-color: var(--dc-warning-tint); }
.desktop-calls-surface.call-pill button.pill-button {
    min-width: 32px; min-height: 32px; padding: 0;
}
.desktop-calls-surface.call-pill button.hang-up {
    background: transparent; color: var(--dc-text);
    box-shadow: inset 0 0 0 1px var(--dc-border);
}
.desktop-calls-surface.call-pill button.hang-up:hover,
.desktop-calls-surface.call-pill button.hang-up:focus-visible {
    background: var(--dc-end); color: var(--dc-call-label); box-shadow: none;
}

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
        ("checked", roles.checked),
        ("checked-hover", roles.checked_hover),
        ("checked-pressed", roles.checked_pressed),
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
             .desktop-calls-surface .notice {{ border-radius: {control}px; }}"
        );
    }
    css
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The declarations of the first rule for `selector` in `css`.
    fn rule<'a>(css: &'a str, selector: &str) -> &'a str {
        let start = css
            .find(&format!("{selector} {{"))
            .unwrap_or_else(|| panic!("no rule for {selector}"));
        let body = &css[start..];
        &body[..body.find('}').unwrap_or(body.len())]
    }

    #[test]
    fn a_keycap_is_drawn_at_full_strength_and_a_bare_number_as_heavy_as_a_name() {
        let css = css(&Roles::stock(false), &[], None);
        assert!(
            !rule(&css, ".type-keycap").contains("opacity"),
            "keycaps are text the user must read to act, so they meet 4.5:1 unfaded (#222)"
        );
        assert!(
            !rule(&css, ".type-who.number").contains("font-weight"),
            "a bare number keeps Who's weight, 650"
        );
    }

    #[test]
    fn a_checked_toggle_takes_the_checked_fills_not_libadwaitas() {
        // libadwaita's 30% text fill leaves a checked toggle's label under
        // 4.5:1 in some themes; the roles cap it (#222).
        let css = css(&Roles::stock(false), &[], None);
        for (selector, role) in [
            (".desktop-calls-surface button:checked", "--dc-checked)"),
            (
                ".desktop-calls-surface button:checked:hover",
                "--dc-checked-hover)",
            ),
            (
                ".desktop-calls-surface button:checked.keyboard-activating",
                "--dc-checked-pressed)",
            ),
        ] {
            assert!(
                rule(&css, selector).contains(role),
                "{selector} takes {role}"
            );
        }
    }
}
