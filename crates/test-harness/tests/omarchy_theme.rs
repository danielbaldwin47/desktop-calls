//! Scenarios: the surfaces follow the live Omarchy theme (its palette, light
//! or dark) and Hyprland's corner rounding, with no restart, and are stock
//! Adwaita on a Desktop without Omarchy. Checked on screenshots.
//!
//! A theme switch is held to its target by the app's own log, up to the
//! frame drawn in the new theme. The timed screen checks capture only a few
//! pixels at a time, so a slow CI runner spends its time on the app, not on
//! the harness, and are held to a looser round trip.

use std::time::{Duration, Instant, SystemTime};

use test_harness::{
    AppEvent, AppState, BAR_COLOUR, Builder, Harness, Key, OmarchyTheme, Region, Screenshot,
};

/// How long the app may take to notice a change on the Phone.
const DEADLINE: Duration = Duration::from_secs(5);
/// How long an open surface may take to follow a theme switch: from the
/// switch starting to the surface's first frame in the new theme, by the
/// app's log (ticket #99).
const THEME_DEADLINE: Duration = Duration::from_secs(1);
/// How long the tests' own round trip may take: the switch, and the new
/// colours captured off the screen. Looser than [`THEME_DEADLINE`] because
/// a loaded CI runner stretches the captures even when the app is fast
/// (tickets #83 and #99); the target is asserted on the app's log.
const ROUND_TRIP: Duration = Duration::from_secs(3);

const NUMBER: &str = "+447700900123";

/// Stock Adwaita's light window background (libadwaita 1.10).
const ADWAITA_LIGHT_WINDOW: [u8; 3] = [0xfa, 0xfa, 0xfb];
/// The stock Answer button's green.
const STOCK_GREEN: [u8; 3] = [0x26, 0xa2, 0x69];

/// The Banner's corner radius once Hyprland rounds windows.
const ROUNDING: u32 = 12;

fn close_to(colour: [u8; 3], target: [u8; 3]) -> bool {
    colour
        .iter()
        .zip(target)
        .all(|(channel, wanted)| channel.abs_diff(wanted) <= 6)
}

/// Whether every pixel of `screenshot` is `colour`.
fn shows_only(screenshot: &Screenshot, colour: [u8; 3]) -> bool {
    screenshot
        .region_where(|pixel| !close_to(pixel, colour))
        .is_none()
}

/// Whether `colour`, give or take antialiasing, is anywhere on screen.
fn shows(screenshot: &Screenshot, colour: [u8; 3]) -> bool {
    screenshot
        .region_where(|pixel| {
            pixel
                .iter()
                .zip(colour)
                .all(|(channel, wanted)| channel.abs_diff(wanted) <= 16)
        })
        .is_some()
}

/// Where the app's surfaces are: everything that is neither the black
/// background nor the bar stand-in.
fn app_region(screenshot: &Screenshot) -> Option<Region> {
    screenshot.region_where(|pixel| pixel != [0, 0, 0] && pixel != BAR_COLOUR)
}

/// Starts `builder` on a screen, connects the Phone and rings it, and waits
/// until the screen shows the Banner with its Answer button in `green`.
/// Returns where the Banner is.
fn ringing(builder: Builder, green: [u8; 3]) -> (Harness, Region) {
    let harness = builder.headless_screen().omarchy_bar().start();
    harness.simulated_phone().connect();
    harness.wait_for_app("ConnectionState is connected", DEADLINE, |state| {
        state.connection_state == "connected"
    });
    let _call = harness.simulated_phone().ring(NUMBER);
    harness.wait_for_app("the Banner shows", DEADLINE, |state| {
        state.surface("banner").is_some()
    });
    let screenshot =
        harness.screenshot_when("banner", DEADLINE, |screenshot| shows(screenshot, green));
    let banner = app_region(&screenshot)
        .unwrap_or_else(|| panic!("the Banner isn't on screen: {}", screenshot.path.display()));
    (harness, banner)
}

/// A few pixels of a surface's background: in its left padding, halfway
/// down, clear of its border, its corners and its content.
fn background_patch(surface: Region) -> Region {
    let middle = f64::midpoint(surface.top, surface.bottom);
    Region {
        left: surface.left + 4.0,
        top: middle - 3.0,
        right: surface.left + 10.0,
        bottom: middle + 3.0,
    }
}

/// One layout pixel inside the surface's top-left corner, clear of its
/// border: background where the corner is square, black where it is
/// rounded. At scale 1.6 the surface's edges fall between screen pixels, so
/// its outermost pixels are blends.
fn corner(surface: Region) -> Region {
    let (left, top) = (surface.left.ceil() + 2.0, surface.top.ceil() + 2.0);
    Region {
        left,
        top,
        right: left + 1.0,
        bottom: top + 1.0,
    }
}

/// Captures `patch` until it shows only `colour`, and returns how long that
/// took from `since`.
///
/// # Panics
///
/// If it hasn't within [`DEADLINE`].
fn time_until(harness: &Harness, name: &str, patch: Region, colour: [u8; 3]) -> Duration {
    let since = Instant::now();
    loop {
        let screenshot = harness.screenshot_of(name, patch);
        if shows_only(&screenshot, colour) {
            return since.elapsed();
        }
        assert!(
            since.elapsed() < DEADLINE,
            "{name}: the screen never showed {colour:02x?} within {DEADLINE:?}; see {}",
            screenshot.path.display()
        );
    }
}

/// Checks the app drew the Banner in `theme` within [`THEME_DEADLINE`] of
/// `since`, just before the harness started switching to it, by the app's
/// log. Nothing but the app sits between the switch and the frame: no test
/// process starting, no capture of the screen.
fn assert_banner_follows_within(harness: &Harness, since: SystemTime, theme: OmarchyTheme) {
    harness.assert_app_within_since(
        &format!("the Banner to show the {} theme", theme.mode().as_str()),
        since,
        &AppEvent::kind("theme-shown")
            .with("surface", "banner")
            .with("mode", theme.mode().as_str()),
        THEME_DEADLINE,
    );
}

#[test]
fn an_open_banner_follows_a_switch_between_light_and_dark_omarchy_themes_within_a_second() {
    let dark = OmarchyTheme::Dark;
    let light = OmarchyTheme::Light;
    let (harness, banner) = ringing(
        Harness::builder().theme(dark.mode()).omarchy_theme(dark),
        dark.colour("green"),
    );
    let patch = background_patch(banner);
    assert!(
        shows_only(
            &harness.screenshot_of("dark-background", patch),
            dark.colour("background")
        ),
        "the Banner's background isn't the dark theme's"
    );

    let since = SystemTime::now();
    harness.switch_omarchy_theme(light);
    let to_light = time_until(&harness, "to-light", patch, light.colour("background"));
    assert_banner_follows_within(&harness, since, light);

    let screenshot = harness.screenshot("light");
    assert!(
        shows(&screenshot, light.colour("green")),
        "Answer isn't the light theme's green"
    );
    assert!(
        shows(&screenshot, light.colour("red")),
        "Decline isn't the light theme's red"
    );
    let since = SystemTime::now();
    harness.switch_omarchy_theme(dark);
    let to_dark = time_until(&harness, "to-dark", patch, dark.colour("background"));
    assert_banner_follows_within(&harness, since, dark);

    assert!(
        to_light <= ROUND_TRIP,
        "to light took {to_light:?} on screen"
    );
    assert!(to_dark <= ROUND_TRIP, "to dark took {to_dark:?} on screen");
    // The same Banner, still ringing: the app followed without a restart.
    let state = harness.app_state();
    assert!(
        state.ringing && state.surface("banner").is_some(),
        "{state:?}"
    );
}

#[test]
fn the_call_window_takes_its_colours_from_the_omarchy_theme() {
    let light = OmarchyTheme::Light;
    let (harness, _) = ringing(
        Harness::builder().omarchy_theme(light),
        light.colour("green"),
    );
    let call = harness.app_state().calls[0].id;

    harness.answer(call);

    let screenshot = harness.screenshot_when("call-window", DEADLINE, |screenshot| {
        shows(screenshot, light.colour("red")) && !shows(screenshot, light.colour("green"))
    });
    let window = app_region(&screenshot).expect("the Call Window is on screen");
    let patch = harness.screenshot_of("call-window-background", background_patch(window));
    assert!(
        shows_only(&patch, light.colour("background")),
        "the Call Window's background isn't the theme's; see {}",
        patch.path.display()
    );
}

#[test]
fn without_omarchy_the_banner_is_stock_adwaita_and_the_app_logs_no_error() {
    let (harness, banner) = ringing(Harness::builder(), STOCK_GREEN);

    let patch = harness.screenshot_of("background", background_patch(banner));

    assert!(
        shows_only(&patch, ADWAITA_LIGHT_WINDOW),
        "the Banner's background isn't stock Adwaita's; see {}",
        patch.path.display()
    );
    let log = harness.app_log();
    let errors: Vec<_> = log
        .lines()
        .filter(|line| line.contains("\"level\":\"ERROR\""))
        .collect();
    assert!(errors.is_empty(), "the app logged errors: {errors:#?}");
    let theme_warnings: Vec<_> = log
        .lines()
        .filter(|line| {
            line.contains("\"level\":\"WARN\"") && line.contains("\"event_kind\":\"theme-")
        })
        .collect();
    assert!(
        theme_warnings.is_empty(),
        "the app warned about the theme: {theme_warnings:#?}"
    );
}

#[test]
fn an_open_banner_rounds_its_corners_when_hyprland_reloads_with_rounding() {
    let (harness, banner) = ringing(Harness::builder().hyprland_rounding(0), STOCK_GREEN);
    assert!(
        shows_only(
            &harness.screenshot_of("square", corner(banner)),
            ADWAITA_LIGHT_WINDOW
        ),
        "the Banner's corners aren't square while Hyprland's rounding is 0"
    );

    harness.set_hyprland_rounding(ROUNDING);

    time_until(&harness, "rounded", corner(banner), [0, 0, 0]);
    // The rest of the Banner stays put.
    let screenshot = harness.screenshot("rounded-banner");
    assert_eq!(app_region(&screenshot), Some(banner));
}

/// `percent` of `top` over `bottom`, as CSS mixes colours.
fn mix(bottom: [u8; 3], top: [u8; 3], percent: u16) -> [u8; 3] {
    let channel = |at: usize| {
        let mixed =
            (u16::from(bottom[at]) * (100 - percent) + u16::from(top[at]) * percent + 50) / 100;
        u8::try_from(mixed).unwrap_or(u8::MAX)
    };
    [channel(0), channel(1), channel(2)]
}

/// The colours across a surface's left edge, halfway down: from just
/// outside it to three layout pixels in.
fn left_edge(screenshot: &Screenshot, surface: Region) -> Vec<[u8; 3]> {
    let middle = f64::midpoint(surface.top, surface.bottom);
    (0..16)
        .map(|step| screenshot.colour_at(surface.left - 0.5 + f64::from(step) * 0.25, middle))
        .collect()
}

/// Whether `surface`'s left edge, halfway down, is a border in `border`
/// rather than its `background` running to the edge.
///
/// At scale 1.6 the outermost screen pixel is a blend of the black outside
/// and whatever is at the edge, and a blend of black and a light
/// background can match a light theme's border colour. A border at least
/// 1 layout pixel wide covers 1.6 screen pixels, so at least two pixels
/// from the edge in differ from the background; with no border only the
/// blend does.
fn has_border(
    screenshot: &Screenshot,
    surface: Region,
    border: [u8; 3],
    background: [u8; 3],
) -> bool {
    const SCREEN_PIXEL: f64 = 1.0 / 1.6;
    let middle = f64::midpoint(surface.top, surface.bottom);
    let edge: Vec<[u8; 3]> = (0..6)
        .map(|pixel| {
            screenshot.colour_at(
                surface.left + (f64::from(pixel) + 0.5) * SCREEN_PIXEL,
                middle,
            )
        })
        .take_while(|colour| !near(*colour, background, 4))
        .collect();
    edge.len() >= 2 && edge.iter().any(|colour| near(*colour, border, 8))
}

fn near(colour: [u8; 3], target: [u8; 3], within: u8) -> bool {
    colour
        .iter()
        .zip(target)
        .all(|(channel, wanted)| channel.abs_diff(wanted) <= within)
}

fn focused(state: &AppState, surface: &str) -> bool {
    state
        .surface(surface)
        .is_some_and(|shown| shown.get("focused").map(String::as_str) == Some("true"))
}

#[test]
fn the_banner_has_a_plain_border_and_the_dialer_with_the_keyboard_an_accent_one() {
    let dark = OmarchyTheme::Dark;
    let (harness, banner) = ringing(
        Harness::builder().theme(dark.mode()).omarchy_theme(dark),
        dark.colour("green"),
    );
    // The foreground at 30% over the background, by the spec (#221).
    let background = dark.colour("background");
    let border = mix(background, dark.colour("foreground"), 30);
    let accent = dark.colour("accent");
    assert!(!focused(&harness.app_state(), "banner"), "the Banner took the keyboard");
    let screenshot = harness.screenshot("banner-border");
    let edge = left_edge(&screenshot, banner);
    assert!(
        has_border(&screenshot, banner, border, background),
        "the Banner's edge {edge:02x?} has no {border:02x?} border; see {}",
        screenshot.path.display()
    );
    assert!(
        !edge.iter().any(|colour| near(*colour, accent, 24)),
        "the Banner, without the keyboard, has an accent border; see {}",
        screenshot.path.display()
    );

    let call = harness.app_state().calls[0].id;
    harness.decline(call);
    harness.wait_for_app("the Banner goes", DEADLINE, |state| {
        state.surface("banner").is_none()
    });
    let output = harness.cli(&["dialer"]);
    assert!(output.status.success(), "desktop-calls dialer failed");
    harness.wait_for_app("the Dialer has the keyboard", DEADLINE, |state| {
        focused(state, "dialer")
    });

    let screenshot = harness.screenshot_when("dialer-border", DEADLINE, |screenshot| {
        app_region(screenshot)
            .is_some_and(|dialer| has_border(screenshot, dialer, accent, background))
    });
    let dialer = app_region(&screenshot).expect("the Dialer is on screen");
    assert!(
        !has_border(&screenshot, dialer, border, background),
        "the Dialer, with the keyboard, still has the plain border; see {}",
        screenshot.path.display()
    );
}

#[test]
fn the_call_window_with_the_keyboard_has_an_accent_border_and_the_call_pill_a_plain_one() {
    let light = OmarchyTheme::Light;
    let (harness, _) = ringing(
        Harness::builder().omarchy_theme(light),
        light.colour("green"),
    );
    // The foreground at 30% over the background, by the spec (#221).
    let background = light.colour("background");
    let border = mix(background, light.colour("foreground"), 30);
    let accent = light.colour("accent");
    let call = harness.app_state().calls[0].id;

    harness.answer(call);

    harness.wait_for_app("the Call Window has the keyboard", DEADLINE, |state| {
        focused(state, "call-window")
    });
    let screenshot = harness.screenshot_when("call-window-border", DEADLINE, |screenshot| {
        shows(screenshot, light.colour("red"))
            && app_region(screenshot)
                .is_some_and(|window| has_border(screenshot, window, accent, background))
    });
    assert!(app_region(&screenshot).is_some(), "the Call Window is on screen");

    harness.keyboard().press(Key::Escape);

    harness.wait_for_app("Escape collapses the Call Window", DEADLINE, |state| {
        state.surface("call-window").is_none() && state.surface("call-pill").is_some()
    });
    let screenshot = harness.screenshot_when("call-pill-border", DEADLINE, |screenshot| {
        app_region(screenshot).is_some_and(|pill| has_border(screenshot, pill, border, background))
    });
    let pill = app_region(&screenshot).expect("the Call Pill is on screen");
    assert!(
        !left_edge(&screenshot, pill)
            .iter()
            .any(|colour| near(*colour, accent, 24)),
        "the Call Pill, which never takes the keyboard, has an accent border; see {}",
        screenshot.path.display()
    );
}
