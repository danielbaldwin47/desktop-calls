//! Scenarios: the Dialer's look, checked on screenshots under the light and
//! the dark Omarchy fixture theme (ticket #227). The selected row is
//! Omarchy's cursor row, an accent bar over a 10% fill, and its detail line
//! reads on that fill at 4.5:1, in red for a Missed Call. A notice is a
//! strip in the warning tint, the Dialer's full width, under the search row.

use std::time::Duration;

use test_harness::vcards::{self, CallKind};
use test_harness::{AppState, BAR_COLOUR, Harness, Key, OmarchyTheme, Region, Screenshot};

/// How long the app may take to show a change.
const DEADLINE: Duration = Duration::from_secs(5);
/// How long a whole sync may take.
const SYNC_DEADLINE: Duration = Duration::from_secs(15);
/// How long a surface may take to be drawn on screen. Each capture and its
/// scan take over a second on a busy CI runner, so this allows several.
const SCREEN_DEADLINE: Duration = Duration::from_secs(10);
/// The contrast every text on the Dialer reaches (WCAG AA).
const READABLE: f64 = 4.5;
/// How far short of [`READABLE`] a glyph's most inked pixel may fall on a
/// screenshot. The text's colour role reads at 4.5:1 (the theme's unit tests
/// hold that); a thin stroke, such as the CI image's fallback monospace at
/// 12.5 px, never paints that full colour into any one pixel, and measured
/// 4.49:1 there for a role at 4.5:1.
const ANTIALIASING: f64 = 0.1;
/// How much foreground a selected row's fill takes, and how much of the
/// warning colour a notice strip's, over the background, by the spec (#221).
const SELECTION_PERCENT: u16 = 10;
const WARNING_TINT_PERCENT: u16 = 14;
/// The search row's height, under the Dialer's 2 px padding.
const SEARCH_ROW: f64 = 52.0;

/// A connected Phone with Contacts, and a Call History whose newest Call is
/// a Missed Call, on a screen with Omarchy's bar, under `theme`.
fn synced(theme: OmarchyTheme) -> Harness {
    let harness = Harness::builder()
        .theme(theme.mode())
        .omarchy_theme(theme)
        .headless_screen()
        .omarchy_bar()
        .start();
    let phone = harness.simulated_phone();
    phone.set_pbap_folder(
        "pb",
        &[
            vcards::contact("José Smith", &[("CELL", "+447700900001")]),
            vcards::contact("Alice Smith", &[("CELL", "+447700900123")]),
        ]
        .concat(),
    );
    phone.set_pbap_folder(
        "cch",
        &[
            vcards::call(CallKind::Missed, Some("+447700900999"), "20261104T110000"),
            vcards::call(CallKind::Dialed, Some("+447700900123"), "20261104T100000"),
        ]
        .concat(),
    );
    phone.connect();
    harness.wait_for_app(
        "the Phone is connected, and Contacts and Call History synced",
        SYNC_DEADLINE,
        |state| {
            state.connection_state == "connected"
                && state.contacts.state == "ready"
                && state.contacts.call_history == 2
        },
    );
    harness
}

fn shown<'a>(state: &'a AppState, key: &str) -> Option<&'a str> {
    state
        .surface("dialer")
        .and_then(|dialer| dialer.get(key))
        .map(String::as_str)
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

fn near(colour: [u8; 3], target: [u8; 3], within: u8) -> bool {
    colour
        .iter()
        .zip(target)
        .all(|(channel, wanted)| channel.abs_diff(wanted) <= within)
}

/// WCAG's relative luminance.
fn luminance(colour: [u8; 3]) -> f64 {
    let linear = |channel: u8| {
        let value = f64::from(channel) / 255.0;
        if value <= 0.039_28 {
            value / 12.92
        } else {
            ((value + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * linear(colour[0]) + 0.7152 * linear(colour[1]) + 0.0722 * linear(colour[2])
}

/// WCAG's contrast ratio between two colours.
fn contrast(one: [u8; 3], other: [u8; 3]) -> f64 {
    let (one, other) = (luminance(one), luminance(other));
    (one.max(other) + 0.05) / (one.min(other) + 0.05)
}

/// Whether `colour` is a red: the theme's, or one mixed towards the text to
/// read.
fn reddish(colour: [u8; 3]) -> bool {
    let [red, green, blue] = colour.map(i16::from);
    red - green > 40 && red - blue > 40
}

/// Where the Dialer is: everything that is neither the black background nor
/// the bar stand-in.
fn app_region(screenshot: &Screenshot) -> Option<Region> {
    screenshot.region_where(|pixel| pixel != [0, 0, 0] && pixel != BAR_COLOUR)
}

/// The band of the Dialer filled with `fill` edge to edge, such as the
/// selected row or the notice strip, found down a column in the Dialer's
/// left padding, where no text's antialiasing can pass for the fill. Returns
/// the Dialer and the band, from just inside the Dialer's 2 px padding to
/// its right edge.
fn band(screenshot: &Screenshot, fill: [u8; 3]) -> Option<(Region, Region)> {
    let dialer = app_region(screenshot)?;
    let column = Region {
        left: dialer.left + 7.0,
        right: dialer.left + 11.0,
        ..dialer
    };
    let found = screenshot.region_within(column, |pixel| near(pixel, fill, 2))?;
    Some((
        dialer,
        Region {
            left: dialer.left + 2.0,
            right: dialer.right - 2.0,
            ..found
        },
    ))
}

/// Where a row's or the strip's content starts, past the accent bar: its
/// 14 px padding, at the screenshots' scale of 1.6, less a little.
const CONTENT_INSET: f64 = 20.0;
/// The notice's icon, 16 across at 1.6, and the gap after it.
const NOTICE_ICON: f64 = 26.0;
const NOTICE_GAP: f64 = 16.0;

/// The selected row's detail line: the lower part of its fill, past the
/// accent bar, which would otherwise pass for its text.
fn detail_line(row: Region) -> Region {
    Region {
        top: row.top + (row.bottom - row.top) * 0.55,
        left: row.left + CONTENT_INSET,
        ..row
    }
}

/// The selected row in `screenshot`, whose fill is `selection`, is Omarchy's
/// cursor row, and its detail line has text at 4.5:1 on that fill, red or
/// not as `red`.
fn assert_selected_row_reads(
    theme: OmarchyTheme,
    harness: &Harness,
    name: &str,
    selection: [u8; 3],
    red: bool,
) {
    let screenshot = harness.screenshot_when(name, SCREEN_DEADLINE, |screenshot| {
        band(screenshot, selection).is_some()
    });
    let see = screenshot.path.display();
    let (_, row) = band(&screenshot, selection)
        .unwrap_or_else(|| panic!("no selected row: {see}"));
    // The 3 px bar, inside the Dialer's 2 px padding; the row below has none.
    let middle = f64::midpoint(row.top, row.bottom);
    let bar = screenshot.colour_at(row.left + 1.5, middle);
    assert!(
        near(bar, theme.colour("accent"), 8),
        "the selected row's bar is {bar:02x?}, not the accent; see {see}"
    );
    let fill = screenshot.colour_at(row.right - 4.0, middle);
    assert!(
        near(fill, selection, 2),
        "the selected row's fill stops short of the right edge: {fill:02x?}; see {see}"
    );
    let below = screenshot.colour_at(row.left + 1.5, row.bottom + 12.0);
    assert!(
        !near(below, theme.colour("accent"), 24),
        "the row below the selection has a bar too; see {see}"
    );
    let ink = screenshot.region_within(detail_line(row), |pixel| {
        contrast(pixel, selection) >= READABLE - ANTIALIASING && reddish(pixel) == red
    });
    assert!(
        ink.is_some(),
        "the selected row's detail line has no {} text at {READABLE}:1 on {selection:02x?}; see {see}",
        if red { "red" } else { "secondary" },
    );
}

fn the_selected_row_reads_and_a_notice_is_a_strip(theme: OmarchyTheme) {
    let harness = synced(theme);
    let (background, foreground) = (theme.colour("background"), theme.colour("foreground"));
    let selection = mix(background, foreground, SELECTION_PERCENT);
    let output = harness.cli(&["dialer"]);
    assert!(output.status.success(), "desktop-calls dialer failed");
    let state = harness.wait_for_app("the Dialer has the keyboard", DEADLINE, |state| {
        shown(state, "focused") == Some("true") && shown(state, "selected") == Some("0")
    });
    assert!(
        shown(&state, "row-0").is_some_and(|row| row.contains("Missed")),
        "{:?}",
        state.surface("dialer")
    );

    // The Missed Call, selected: its detail line in red at 4.5:1.
    assert_selected_row_reads(theme, &harness, "dialer-missed-selected", selection, true);

    // An Outgoing Call, selected: its detail line in secondary text.
    harness.keyboard().press(Key::Down);
    harness.wait_for_app("the second row is selected", DEADLINE, |state| {
        shown(state, "selected") == Some("1")
    });
    assert_selected_row_reads(theme, &harness, "dialer-outgoing-selected", selection, false);

    // Enter with the Phone out of range: the notice is a tinted strip, the
    // Dialer's full width, between the search row and the rows.
    harness.simulated_phone().disconnect();
    harness.wait_for_app("the footer says out of range", DEADLINE, |state| {
        shown(state, "phone") == Some("Out of range")
    });
    harness.keyboard().press(Key::Return);
    harness.wait_for_app("the Dialer says why not", DEADLINE, |state| {
        shown(state, "notice").is_some()
    });
    let tint = mix(
        background,
        theme.colour("orange"),
        WARNING_TINT_PERCENT,
    );
    let screenshot = harness.screenshot_when("dialer-notice", SCREEN_DEADLINE, |screenshot| {
        band(screenshot, tint).is_some() && band(screenshot, selection).is_some()
    });
    let see = screenshot.path.display();
    let (dialer, strip) = band(&screenshot, tint).unwrap_or_else(|| panic!("no notice strip: {see}"));
    let (_, row) = band(&screenshot, selection).unwrap_or_else(|| panic!("no selected row: {see}"));
    let right = screenshot.colour_at(strip.right - 4.0, f64::midpoint(strip.top, strip.bottom));
    assert!(
        near(right, tint, 2),
        "the strip stops short of the Dialer's right edge {dialer:?}: {right:02x?}; see {see}"
    );
    let below_search = strip.top - dialer.top;
    assert!(
        (SEARCH_ROW..SEARCH_ROW + 6.0).contains(&below_search),
        "the strip starts {below_search} below the Dialer's top, not under the search row; see {see}"
    );
    assert!(
        strip.bottom <= row.top,
        "the strip {strip:?} isn't above the rows {row:?}; see {see}"
    );
    // The warning icon, a graphic, at 3:1 (WCAG's non-text contrast); then
    // the words, right of it, at 4.5:1.
    let icon = Region {
        left: strip.left + CONTENT_INSET,
        right: strip.left + CONTENT_INSET + NOTICE_ICON,
        ..strip
    };
    assert!(
        screenshot
            .region_within(icon, |pixel| contrast(pixel, tint) >= 3.0)
            .is_some(),
        "the strip has no icon at its left; see {see}"
    );
    let words = Region {
        left: icon.right + NOTICE_GAP,
        ..strip
    };
    assert!(
        screenshot
            .region_within(words, |pixel| contrast(pixel, tint) >= READABLE - ANTIALIASING)
            .is_some(),
        "the strip has no words at {READABLE}:1 on {tint:02x?}; see {see}"
    );
}

#[test]
fn under_a_light_theme_the_selected_row_reads_and_a_notice_is_a_strip() {
    the_selected_row_reads_and_a_notice_is_a_strip(OmarchyTheme::Light);
}

#[test]
fn under_a_dark_theme_the_selected_row_reads_and_a_notice_is_a_strip() {
    the_selected_row_reads_and_a_notice_is_a_strip(OmarchyTheme::Dark);
}
