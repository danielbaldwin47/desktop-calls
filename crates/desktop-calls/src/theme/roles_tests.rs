//! The colour roles read at 4.5:1 or better for the gallery's themes, stock
//! Adwaita and real Omarchy palettes.

use super::*;

/// The gallery's and the screen scenarios' Omarchy themes.
const GALLERY_LIGHT: &str =
    include_str!("../../../test-harness/fixtures/omarchy-themes/light/colors.toml");
const GALLERY_DARK: &str =
    include_str!("../../../test-harness/fixtures/omarchy-themes/dark/colors.toml");

/// Real Omarchy themes' colours, from `/usr/share/omarchy/themes/*/colors.toml`
/// (Omarchy 4.5), cut to the keys Desktop Calls reads.
const CATPPUCCIN_LATTE: &str = r##"
mode = "light"
accent = "#1e66f5"
background = "#eff1f5"
lighter_background = "#dce0e8"
foreground = "#4c4f69"
red = "#d20f39"
yellow = "#df8e1d"
orange = "#d84e2b"
green = "#40a02b"
"##;

const TOKYO_NIGHT: &str = r##"
mode = "dark"
accent = "#7aa2f7"
background = "#1a1b26"
lighter_background = "#24283b"
foreground = "#a9b1d6"
red = "#f7768e"
yellow = "#e0af68"
orange = "#eb927b"
green = "#9ece6a"
"##;

const GRUVBOX: &str = r##"
mode = "dark"
accent = "#7daea3"
background = "#282828"
lighter_background = "#3c3836"
foreground = "#d4be98"
red = "#ea6962"
yellow = "#d8a657"
orange = "#e1875c"
green = "#a9b665"
"##;

const FLEXOKI_LIGHT: &str = r##"
mode = "light"
accent = "#205EA6"
background = "#FFFCF0"
lighter_background = "#E6E4D9"
foreground = "#100F0F"
red = "#D14D41"
yellow = "#D0A215"
orange = "#d0772b"
green = "#879A39"
"##;

const ROSE_PINE: &str = r##"
mode = "light"
accent = "#56949f"
background = "#faf4ed"
lighter_background = "#f2e9e1"
foreground = "#575279"
red = "#b4637a"
yellow = "#ea9d34"
orange = "#cf8057"
green = "#286983"
"##;

/// Monochrome, and with no orange.
const WHITE_THEME: &str = r##"
mode = "light"
accent = "#6e6e6e"
background = "#ffffff"
lighter_background = "#c0c0c0"
foreground = "#000000"
red = "#2a2a2a"
yellow = "#4a4a4a"
green = "#3a3a3a"
"##;

fn palette(colors_toml: &str) -> Palette {
    let table = colors_toml.parse::<toml::Table>().expect("valid TOML");
    Palette::from_table(&table).expect("a palette")
}

/// Every theme the roles must hold for, by name.
fn every_theme() -> Vec<(&'static str, Roles)> {
    let mut themes = vec![
        ("stock light", Roles::stock(false)),
        ("stock dark", Roles::stock(true)),
    ];
    for (name, colors_toml) in [
        ("gallery light", GALLERY_LIGHT),
        ("gallery dark", GALLERY_DARK),
        ("catppuccin-latte", CATPPUCCIN_LATTE),
        ("tokyo-night", TOKYO_NIGHT),
        ("gruvbox", GRUVBOX),
        ("flexoki-light", FLEXOKI_LIGHT),
        ("rose-pine", ROSE_PINE),
        ("white", WHITE_THEME),
    ] {
        themes.push((name, Roles::omarchy(&palette(colors_toml))));
    }
    themes
}

fn assert_reads(theme: &str, role: &str, text: Rgb, backgrounds: &[(&str, Rgb)]) {
    for (behind, background) in backgrounds {
        let contrast = text.contrast(*background);
        assert!(
            contrast >= 4.5,
            "{theme}: {role} {text} on the {behind} {background} is {contrast:.2}:1"
        );
    }
}

#[test]
fn secondary_text_reads_on_the_surface_a_raised_control_and_a_selected_row() {
    for (theme, roles) in every_theme() {
        assert_reads(
            theme,
            "secondary text",
            roles.secondary,
            &[
                ("surface", roles.surface),
                ("raised control", roles.raised),
                ("selected row", roles.selection),
            ],
        );
    }
}

#[test]
fn secondary_text_is_weaker_than_text_where_the_text_reads_well_above_the_floor() {
    for (theme, colors_toml) in [
        ("gallery light", GALLERY_LIGHT),
        ("gallery dark", GALLERY_DARK),
        ("flexoki-light", FLEXOKI_LIGHT),
    ] {
        let roles = Roles::omarchy(&palette(colors_toml));
        let backgrounds = [roles.surface, roles.raised, roles.selection];
        let secondary = roles.secondary.worst_contrast(&backgrounds);
        assert!(
            secondary < 5.0 && secondary < roles.text.worst_contrast(&backgrounds),
            "{theme}: secondary text is {secondary:.2}:1, no weaker than it need be"
        );
    }
}

#[test]
fn the_warning_label_reads_on_the_warning_colour_and_warning_text_on_the_surface() {
    for (theme, roles) in every_theme() {
        assert_reads(
            theme,
            "the warning label",
            roles.warning_label,
            &[("warning", roles.warning)],
        );
        assert_reads(
            theme,
            "warning text",
            roles.warning_text,
            &[
                ("surface", roles.surface),
                ("warning strip", roles.warning_tint),
            ],
        );
    }
}

#[test]
fn missed_reads_in_red_on_a_selected_row() {
    for (theme, roles) in every_theme() {
        assert_reads(
            theme,
            "Missed",
            roles.missed,
            &[
                ("surface", roles.surface),
                ("selected row", roles.selection),
            ],
        );
    }
}

#[test]
fn the_accent_label_reads_on_the_accent() {
    for (theme, roles) in every_theme() {
        assert_reads(
            theme,
            "the accent label",
            roles.accent_label,
            &[("accent", roles.accent)],
        );
    }
}

#[test]
fn warnings_take_the_themes_orange_else_its_yellow() {
    assert_eq!(
        Roles::omarchy(&palette(GALLERY_DARK)).warning,
        Rgb(0xd1, 0x9a, 0x66),
        "the gallery's dark orange"
    );
    assert_eq!(
        Roles::omarchy(&palette(WHITE_THEME)).warning,
        Rgb(0x4a, 0x4a, 0x4a),
        "the white theme's yellow, as it has no orange"
    );
}

#[test]
fn a_red_that_already_reads_stays_the_themes_red() {
    let roles = Roles::omarchy(&palette(TOKYO_NIGHT));
    assert_eq!(roles.missed, Rgb(0xf7, 0x76, 0x8e));
}

#[test]
fn a_green_and_red_that_already_read_under_the_call_label_stay_the_themes_own() {
    let roles = Roles::omarchy(&palette(TOKYO_NIGHT));
    assert_eq!(roles.answer, Rgb(0x9e, 0xce, 0x6a));
    assert_eq!(roles.end, Rgb(0xf7, 0x76, 0x8e));
}

#[test]
fn without_omarchy_the_roles_are_stock_adwaitas() {
    let light = Roles::stock(false);
    assert_eq!(light.surface, Rgb(0xfa, 0xfa, 0xfb));
    assert_eq!(light.accent, Rgb(0x35, 0x84, 0xe4));
    assert_eq!(light.call_label, Rgb(0xff, 0xff, 0xff));
    assert_eq!(
        light.answer,
        Rgb(0x20, 0x86, 0x57),
        "Adwaita's green, deepened"
    );
    assert_eq!(Roles::stock(true).surface, Rgb(0x22, 0x22, 0x26));
}

#[test]
fn keycaps_read_at_full_strength_on_every_control_they_sit_on() {
    // A keycap is drawn in its label's colour with no fade (#222), so it
    // reads wherever that label does: on Answer, Decline, End and Hang up
    // it is their shared label.
    for (theme, roles) in every_theme() {
        assert_reads(
            theme,
            "a keycap on Answer, End or Hang up",
            roles.call_label,
            &[("Answer", roles.answer), ("End", roles.end)],
        );
        assert_reads(
            theme,
            "a keycap",
            roles.text,
            &[("surface", roles.surface), ("raised control", roles.raised)],
        );
        assert_reads(
            theme,
            "a keycap on Unmute",
            roles.warning_label,
            &[("warning", roles.warning)],
        );
    }
}

/// How much text libadwaita 1.10 mixes into a button's fill at rest and
/// under the pointer, over the surface.
const ADWAITA_BUTTON_PERCENT: u8 = 10;
const ADWAITA_HOVER_PERCENT: u8 = 15;

#[test]
fn a_checked_or_hovered_control_keeps_its_label_and_keycap_at_4_5_to_1() {
    // A control's label and its keycap are drawn in text (#222), so both
    // read wherever text reads on the control's fill.
    for (theme, roles) in every_theme() {
        assert_reads(
            theme,
            "the label and keycap of a control",
            roles.text,
            &[
                ("checked fill", roles.checked),
                ("checked and hovered fill", roles.checked_hover),
                ("checked and pressed fill", roles.checked_pressed),
                (
                    "libadwaita's hovered fill",
                    roles.surface.under(roles.text, ADWAITA_HOVER_PERCENT),
                ),
            ],
        );
    }
}

#[test]
fn a_checked_toggle_still_reads_as_checked() {
    // It carries clearly more text than libadwaita's resting fill, more
    // even than its hovered one, wherever the floor caps it.
    for (theme, roles) in every_theme() {
        let rest = roles.surface.under(roles.text, ADWAITA_BUTTON_PERCENT);
        let hover = roles.surface.under(roles.text, ADWAITA_HOVER_PERCENT);
        let step = rest.contrast(hover);
        let checked = rest.contrast(roles.checked);
        assert!(
            checked >= step * step,
            "{theme}: the checked fill {} is {checked:.2}:1 off the resting {rest}, \
             under two hover steps ({step:.2}:1 each)",
            roles.checked
        );
    }
}

#[test]
fn a_checked_fill_is_libadwaitas_where_its_label_reads_and_less_where_not() {
    let gallery = Roles::omarchy(&palette(GALLERY_LIGHT));
    assert_eq!(
        gallery.checked,
        gallery.surface.under(gallery.text, 30),
        "libadwaita's 30%"
    );
    let gruvbox = Roles::omarchy(&palette(GRUVBOX));
    assert_eq!(gruvbox.checked, Rgb(0x53, 0x4e, 0x44), "gruvbox: 25%");
    assert_eq!(
        gruvbox.checked_hover, gruvbox.checked,
        "gruvbox: the floor caps hovered at the checked fill"
    );
}

#[test]
fn a_green_or_red_the_call_label_falls_short_on_takes_the_least_shade_that_reads() {
    // Measured on #222: each moves only as far as 4.5:1 under the label.
    let latte = Roles::omarchy(&palette(CATPPUCCIN_LATTE));
    assert_eq!(
        latte.call_label,
        Rgb(0x00, 0x00, 0x00),
        "black, as neither of latte's own reads"
    );
    assert_eq!(
        latte.end,
        Rgb(0xd9, 0x33, 0x57),
        "latte's red #d20f39, 15% paler"
    );
    assert_eq!(latte.answer, Rgb(0x40, 0xa0, 0x2b), "latte's own green");
    let rose_pine = Roles::omarchy(&palette(ROSE_PINE));
    assert_eq!(
        rose_pine.end,
        Rgb(0xab, 0x5e, 0x74),
        "rose-pine's red #b4637a, 5% deeper"
    );
    let gallery = Roles::omarchy(&palette(GALLERY_LIGHT));
    assert_eq!(
        gallery.answer,
        Rgb(0x2c, 0x86, 0x42),
        "the gallery's green #2f8f46, 6% deeper"
    );
    let stock = Roles::stock(false);
    assert_eq!(
        stock.answer,
        Rgb(0x20, 0x86, 0x57),
        "Adwaita's #26a269, 17% deeper"
    );
    assert_eq!(stock.end, Rgb(0xc0, 0x1c, 0x28), "Adwaita's own red");
}
