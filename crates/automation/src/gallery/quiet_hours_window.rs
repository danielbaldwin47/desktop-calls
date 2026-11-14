//! The Quiet Hours Window's states in the gallery: switched off, switched
//! on with 22:00 to 07:00, and refusing a start equal to the end. The app
//! runs at noon local time, so the sentence says "tonight" and calls stay
//! on whenever the gallery runs.

use std::time::{SystemTime, UNIX_EPOCH};

use test_harness::{AppState, Builder, Harness, Key};

use super::{DEADLINE, State, app_region, connect};

const NAME: &str = "quiet-hours-window";
const DAY: u64 = 86_400;

pub(super) const QUIET_HOURS_WINDOW_STATES: &[State] = &[
    State {
        name: "quiet-hours-window-off",
        title: "The Quiet Hours Window, switched off: the times kept and dimmed, the switch focused",
        still: |_| true,
        prepare: at_noon,
        reach: |harness| {
            connect(harness);
            harness.tray().click("Quiet Hours…");
            wait_for(harness, &[("switch", "off"), ("focus", "switch")]);
        },
        shown: |screenshot, _| app_region(screenshot).is_some(),
    },
    State {
        name: "quiet-hours-window-on",
        title: "The Quiet Hours Window, on from 22:00 to 07:00: the saved sentence and the bar in two pieces, the start focused",
        still: |_| true,
        prepare: at_noon,
        reach: |harness| {
            connect(harness);
            if let Err(error) = harness.set_quiet_hours(22 * 60, 7 * 60) {
                panic!("SetQuietHours: {error:?}");
            }
            harness.tray().click("Quiet Hours…");
            wait_for(harness, &[("switch", "on"), ("focus", "switch")]);
            harness.keyboard().press(Key::Tab);
            wait_for(harness, &[("focus", "start"), ("start", "22:00")]);
        },
        shown: |screenshot, _| app_region(screenshot).is_some(),
    },
    State {
        name: "quiet-hours-window-refused",
        title: "The Quiet Hours Window, refusing a start typed equal to the end",
        still: |_| true,
        prepare: at_noon,
        reach: |harness| {
            connect(harness);
            if let Err(error) = harness.set_quiet_hours(22 * 60, 7 * 60) {
                panic!("SetQuietHours: {error:?}");
            }
            harness.tray().click("Quiet Hours…");
            wait_for(harness, &[("switch", "on"), ("focus", "switch")]);
            harness.keyboard().press(Key::Tab);
            wait_for(harness, &[("focus", "start")]);
            harness.keyboard().type_text("0700");
            wait_for(harness, &[("notice", "Start and end must differ")]);
        },
        shown: |screenshot, _| app_region(screenshot).is_some(),
    },
];

fn shows(state: &AppState, key: &str, value: &str) -> bool {
    state
        .surface(NAME)
        .and_then(|window| window.get(key))
        .is_some_and(|shown| shown == value)
}

/// Waits until the window shows each of `fields`, with keyboard focus.
fn wait_for(harness: &Harness, fields: &[(&str, &str)]) {
    harness.wait_for_app(
        &format!("the Quiet Hours Window shows {fields:?}"),
        DEADLINE,
        |state| {
            shows(state, "focused", "true")
                && fields.iter().all(|(key, value)| shows(state, key, value))
        },
    );
}

/// The app with local time at noon.
fn at_noon(builder: Builder) -> Builder {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| since.as_secs());
    let utc = i64::try_from(now % DAY).unwrap_or(0);
    let day = i64::try_from(DAY).unwrap_or(0);
    let mut east = (12 * 3_600 - utc).rem_euclid(day);
    if east > day / 2 {
        east -= day;
    }
    // POSIX counts the offset westwards.
    let sign = if east > 0 { '-' } else { '+' };
    let seconds = east.unsigned_abs();
    builder.time_zone(&format!(
        "<LOC>{sign}{:02}:{:02}:{:02}",
        seconds / 3_600,
        seconds % 3_600 / 60,
        seconds % 60
    ))
}
