//! Scenarios: the Quiet Hours Window (ticket #209), opened from the
//! Indicator's Quiet Hours… entry and worked with synthetic keys alone.
//! Each change of its switch or its times is saved at once, and kept
//! across a restart; equal times, or a time that can't be read, save
//! nothing.
//!
//! The app runs with a `TZ` putting local time at noon, so the sentence
//! says "tonight" for Quiet Hours from 22:00. Hours saved later, such as
//! 06:15 to 06:00, are under way at noon and turn calls off; the window
//! works the same either way.

use std::collections::BTreeMap;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use test_harness::{AppState, Harness, Key};

/// How long the app may take to notice a change.
const DEADLINE: Duration = Duration::from_secs(5);
const PHONE_NAME: &str = "My Pixel";
const NAME: &str = "quiet-hours-window";
const DAY: u64 = 86_400;

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_else(|error| panic!("the clock is before 1970: {error}"))
        .as_secs()
}

/// A POSIX `TZ` in which the local clock reads 12:00 now.
fn noon_now() -> String {
    let utc = i64::try_from(now() % DAY).unwrap_or(0);
    let day = i64::try_from(DAY).unwrap_or(0);
    let mut east = (12 * 3_600 - utc).rem_euclid(day);
    if east > day / 2 {
        east -= day;
    }
    // POSIX counts the offset westwards.
    let sign = if east > 0 { '-' } else { '+' };
    let seconds = east.unsigned_abs();
    format!(
        "<LOC>{sign}{:02}:{:02}:{:02}",
        seconds / 3_600,
        seconds % 3_600 / 60,
        seconds % 60
    )
}

/// The Phone connected, calls on, at noon.
fn connected() -> Harness {
    let harness = Harness::builder()
        .headless_screen()
        .phone_name(PHONE_NAME)
        .time_zone(&noon_now())
        .start();
    harness.simulated_phone().connect();
    harness.wait_for_app("the Phone is connected, calls on", DEADLINE, |state| {
        state.connection_state == "connected" && state.calls_off == "on"
    });
    harness
}

/// Minutes after midnight.
fn minutes(hour: u16, minute: u16) -> u16 {
    hour * 60 + minute
}

fn window(state: &AppState) -> Option<&BTreeMap<String, String>> {
    state.surface(NAME)
}

fn field<'state>(state: &'state AppState, name: &str) -> Option<&'state str> {
    window(state)?.get(name).map(String::as_str)
}

/// Waits until the window shows each of `fields`, with keyboard focus.
fn wait_for_window(harness: &Harness, fields: &[(&str, &str)]) -> AppState {
    harness.wait_for_app(
        &format!("the Quiet Hours Window shows {fields:?}, focused"),
        DEADLINE,
        |state| {
            field(state, "focused") == Some("true")
                && fields
                    .iter()
                    .all(|(name, value)| field(state, name) == Some(*value))
        },
    )
}

fn wait_for_hours(harness: &Harness, hours: Option<(u16, u16)>) -> AppState {
    harness.wait_for_app(
        &format!("Quiet Hours are {hours:?}"),
        DEADLINE,
        |state| state.quiet_hours == hours,
    )
}

/// Chooses Quiet Hours… in the Indicator's menu, whatever times its hint
/// shows.
fn open(harness: &Harness) {
    let menu = harness.tray().menu();
    let entry = menu
        .iter()
        .find(|entry| entry.label.starts_with("Quiet Hours…"))
        .unwrap_or_else(|| panic!("the menu has Quiet Hours…: {menu:#?}"));
    harness.tray().click(&entry.label);
}

#[test]
fn opened_from_the_indicator_it_shows_hours_set_from_the_command_line_and_escape_closes_it() {
    let harness = connected();
    harness
        .set_quiet_hours(minutes(22, 0), minutes(7, 0))
        .unwrap_or_else(|error| panic!("SetQuietHours: {error:?}"));
    wait_for_hours(&harness, Some((minutes(22, 0), minutes(7, 0))));

    // The menu shows the times as a hint.
    harness
        .tray()
        .click_when_shown("Quiet Hours…  22:00–07:00", DEADLINE);
    let state = wait_for_window(
        &harness,
        &[
            ("switch", "on"),
            ("start", "22:00"),
            ("end", "07:00"),
            ("focus", "switch"),
        ],
    );
    assert_eq!(
        field(&state, "sentence"),
        Some("Saved. Calls go off tonight at 22:00 and come back at 07:00, every day.")
    );
    assert_eq!(field(&state, "bar"), Some("00:00-07:00 22:00-24:00"));
    assert_eq!(field(&state, "keys"), Some("Tab Next\n↑ ↓ 15 min\nEsc Close"));
    assert_eq!(field(&state, "notice"), None);

    harness.keyboard().press(Key::Escape);
    harness.wait_for_app("the Quiet Hours Window is closed", DEADLINE, |state| {
        window(state).is_none()
    });
    assert_eq!(
        harness.app_state().quiet_hours,
        Some((minutes(22, 0), minutes(7, 0))),
        "closing loses nothing"
    );

    // Hours cleared from the command line show as off the next time.
    harness
        .clear_quiet_hours()
        .unwrap_or_else(|error| panic!("ClearQuietHours: {error:?}"));
    wait_for_hours(&harness, None);
    open(&harness);
    let state = wait_for_window(
        &harness,
        &[("switch", "off"), ("start", "22:00"), ("end", "07:00")],
    );
    assert_eq!(
        field(&state, "sentence"),
        Some("Off. Calls stay on unless you turn them off from the Indicator.")
    );
    assert_eq!(field(&state, "keys"), Some("Space Switch on\nEsc Close"));
}

#[test]
fn with_keys_alone_each_change_is_saved_at_once_and_kept_across_a_restart() {
    let mut harness = connected();
    open(&harness);
    wait_for_window(&harness, &[("switch", "off"), ("focus", "switch")]);

    // Space switches on, offering 22:00 to 07:00.
    harness.keyboard().type_text(" ");
    wait_for_hours(&harness, Some((minutes(22, 0), minutes(7, 0))));
    wait_for_window(&harness, &[("switch", "on")]);

    // Tab to the start; Up steps it 15 minutes.
    harness.keyboard().press(Key::Tab);
    wait_for_window(&harness, &[("focus", "start")]);
    harness.keyboard().press(Key::Up);
    wait_for_hours(&harness, Some((minutes(22, 15), minutes(7, 0))));

    // Tab to the end; four digits type 00:00, then Down steps back round
    // midnight to 23:45.
    harness.keyboard().press(Key::Tab);
    wait_for_window(&harness, &[("focus", "end")]);
    harness.keyboard().type_text("0000");
    wait_for_hours(&harness, Some((minutes(22, 15), minutes(0, 0))));
    harness.keyboard().press(Key::Down);
    wait_for_hours(&harness, Some((minutes(22, 15), minutes(23, 45))));

    // Two digits, then Tab: 06:00. Tab went round to the switch.
    harness.keyboard().type_text("06");
    wait_for_window(&harness, &[("end", "06:__")]);
    harness.keyboard().press(Key::Tab);
    wait_for_hours(&harness, Some((minutes(22, 15), minutes(6, 0))));
    wait_for_window(&harness, &[("focus", "switch"), ("end", "06:00")]);

    // Shift+Tab goes round the other way, to the end; a time that can't be
    // read saves nothing, and the field shows the last time again.
    harness.keyboard().press_with_shift(Key::Tab);
    wait_for_window(&harness, &[("focus", "end")]);
    harness.keyboard().type_text("2960");
    wait_for_window(&harness, &[("end", "29:60")]);
    harness.keyboard().press_with_shift(Key::Tab);
    wait_for_window(&harness, &[("focus", "start"), ("end", "06:00")]);
    assert_eq!(
        harness.app_state().quiet_hours,
        Some((minutes(22, 15), minutes(6, 0)))
    );

    // A start typed equal to the end is shown with the notice and not
    // saved.
    harness.keyboard().type_text("0600");
    let state = wait_for_window(
        &harness,
        &[("start", "06:00"), ("notice", "Start and end must differ")],
    );
    assert_eq!(state.quiet_hours, Some((minutes(22, 15), minutes(6, 0))));
    harness.keyboard().press(Key::Up);
    wait_for_hours(&harness, Some((minutes(6, 15), minutes(6, 0))));
    let state = wait_for_window(&harness, &[("start", "06:15")]);
    assert_eq!(field(&state, "notice"), None);

    // Equal again, then switched off: the last saved times come back, so
    // switching on saves them and the switch shows on.
    harness.keyboard().press(Key::Down);
    wait_for_window(&harness, &[("notice", "Start and end must differ")]);
    harness.keyboard().press_with_shift(Key::Tab);
    wait_for_window(&harness, &[("focus", "switch")]);
    harness.keyboard().type_text(" ");
    wait_for_hours(&harness, None);
    let state = wait_for_window(
        &harness,
        &[("switch", "off"), ("start", "06:15"), ("end", "06:00")],
    );
    assert_eq!(field(&state, "notice"), None);
    harness.keyboard().type_text(" ");
    wait_for_hours(&harness, Some((minutes(6, 15), minutes(6, 0))));
    wait_for_window(&harness, &[("switch", "on")]);

    harness.restart_app();
    let state = wait_for_hours(&harness, Some((minutes(6, 15), minutes(6, 0))));
    assert!(window(&state).is_none(), "the window opens only when asked");
    open(&harness);
    wait_for_window(
        &harness,
        &[("switch", "on"), ("start", "06:15"), ("end", "06:00")],
    );

    // Space on the switch switches off, keeping the times shown.
    harness.keyboard().type_text(" ");
    wait_for_hours(&harness, None);
    wait_for_window(
        &harness,
        &[("switch", "off"), ("start", "06:15"), ("end", "06:00")],
    );
    harness.restart_app();
    wait_for_hours(&harness, None);
}
