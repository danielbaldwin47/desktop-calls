//! Scenarios: Quiet Hours (ticket #205), daily times when calls are off,
//! set from the CLI and the control interface and kept in
//! `$XDG_CONFIG_HOME/desktop-calls/quiet-hours`. They turn calls off when
//! they begin and on when they end, the Phone connecting within the
//! scorecard's 10 s. Calls turned on during them stay on until they next
//! begin, across a restart; a timed break ending inside them stays off
//! until they end.
//!
//! An edge is reached without waiting for it: the app starts with a `TZ`
//! whose offset, to the second, puts local time a few seconds before it.

use std::fs;
use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use test_harness::{AppEvent, AppState, Harness, TrayItem};

/// How long the app may take to notice a change on the Phone.
const DEADLINE: Duration = Duration::from_secs(5);
/// How soon the Phone is connected again once calls come back on (spec
/// #198, the polish scorecard's 10 s).
const RECONNECT_DEADLINE: Duration = Duration::from_secs(10);
/// How far ahead of the start of a scenario its Quiet Hours edge is: time
/// for the app to start, connect the Phone and take the Quiet Hours.
const EDGE_AHEAD: u64 = 12;
const PHONE_NAME: &str = "My Pixel";
const DAY: u64 = 86_400;

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_else(|error| panic!("the clock is before 1970: {error}"))
        .as_secs()
}

/// A POSIX `TZ` in which the local clock reads `hour:minute` exactly at
/// the Unix time `instant`: an offset from UTC to the second, within 12
/// hours either way.
fn time_zone_reading(hour: u64, minute: u64, instant: u64) -> String {
    let wanted = i64::try_from((hour * 60 + minute) * 60).unwrap_or(0);
    let utc = i64::try_from(instant % DAY).unwrap_or(0);
    let day = i64::try_from(DAY).unwrap_or(0);
    let mut east = (wanted - utc).rem_euclid(day);
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

fn harness_in(time_zone: &str) -> Harness {
    let harness = Harness::builder()
        .phone_name(PHONE_NAME)
        .time_zone(time_zone)
        .start();
    harness.simulated_phone().connect();
    wait_connected(&harness, DEADLINE);
    harness
}

fn wait_connected(harness: &Harness, deadline: Duration) -> AppState {
    harness.wait_for_app("the Phone is connected, calls on", deadline, |state| {
        state.connection_state == "connected" && state.calls_off == "on"
    })
}

fn wait_off_until(harness: &Harness, until: u64) -> AppState {
    harness.wait_for_app(
        &format!("calls are off until {until} and the Phone disconnected"),
        DEADLINE,
        |state| {
            state.calls_off == "off"
                && state.calls_off_until == until
                && state.connection_state == "disconnected"
        },
    )
}

/// Waits, by the wall clock, until `instant` has passed.
fn wait_until_past(instant: u64) {
    while now() <= instant {
        std::thread::sleep(Duration::from_millis(100));
    }
}

fn record(harness: &Harness) -> PathBuf {
    harness
        .directory()
        .join("state/desktop-calls/calls-off.json")
}

fn quiet_hours_file(harness: &Harness) -> PathBuf {
    harness
        .directory()
        .join("config/desktop-calls/quiet-hours")
}

fn record_text(harness: &Harness) -> String {
    fs::read_to_string(record(harness)).unwrap_or_else(|error| panic!("read the record: {error}"))
}

/// Waits until the Indicator, the only tray item, says `tooltip`.
fn indicator_says(harness: &Harness, tooltip: &str) -> TrayItem {
    let items = harness.tray().wait_for(
        &format!("the Indicator says {tooltip:?}"),
        DEADLINE,
        |items| {
            items
                .iter()
                .any(|item| item.tooltip.as_deref() == Some(tooltip))
        },
    );
    items.into_iter().next().unwrap_or_else(|| unreachable!())
}

fn cli_ok(harness: &Harness, arguments: &[&str]) -> String {
    let output = harness.cli(arguments);
    assert!(output.status.success(), "{arguments:?} failed: {output:?}");
    String::from_utf8_lossy(&output.stdout).into_owned()
}

/// Minutes after midnight.
fn minutes(hour: u16, minute: u16) -> u16 {
    hour * 60 + minute
}

#[test]
fn quiet_hours_across_midnight_turn_calls_off_when_they_begin() {
    let start = now() + EDGE_AHEAD;
    let harness = harness_in(&time_zone_reading(23, 59, start));

    harness
        .set_quiet_hours(minutes(23, 59), minutes(6, 0))
        .expect("Quiet Hours set");

    harness.wait_for_app("Quiet Hours are set", DEADLINE, |state| {
        state.quiet_hours == Some((minutes(23, 59), minutes(6, 0)))
    });
    assert!(now() < start, "the setup took too long to test the start");
    harness.app_holds("calls stay on until Quiet Hours begin", Duration::from_secs(1), |state| {
        state.calls_off == "on"
    });

    // 06:00 the next morning: 6 hours and a minute after 23:59.
    let end = start + 6 * 3_600 + 60;
    let left = Duration::from_secs(start.saturating_sub(now()));
    harness.wait_for_app("calls go off as Quiet Hours begin", left + DEADLINE, |state| {
        state.calls_off == "off"
    });
    assert!(now() >= start, "calls went off before Quiet Hours began");
    wait_off_until(&harness, end);
    let item = indicator_says(&harness, "Calls off until 06:00 (quiet hours)");
    assert_eq!(item.status.as_deref(), Some("Active"), "no attention asked");
    let kept = record_text(&harness);
    assert!(kept.contains(r#""quiet_hours":true"#), "{kept}");
    assert!(kept.contains(&format!(r#""until":{end}"#)), "{kept}");
    assert!(harness.simulated_phone().blocked());
    assert_eq!(harness.simulated_phone().connects_while_blocked(), 0);
}

#[test]
fn quiet_hours_across_midnight_turn_calls_on_when_they_end_and_the_phone_connects_within_10_s() {
    let end = now() + EDGE_AHEAD;
    let harness = harness_in(&time_zone_reading(0, 0, end));

    // Set while under way: calls go off at once, until midnight.
    harness
        .set_quiet_hours(minutes(23, 0), minutes(0, 0))
        .expect("Quiet Hours set");

    wait_off_until(&harness, end);
    indicator_says(&harness, "Calls off until 00:00 (quiet hours)");
    assert!(now() < end, "the setup took too long to test the end");

    let left = Duration::from_secs(end.saturating_sub(now()));
    wait_connected(&harness, left + RECONNECT_DEADLINE);
    assert!(now() >= end, "calls came back on before Quiet Hours ended");
    let took = harness.app_took(
        &AppEvent::kind("calls-off-deadline-passed"),
        &AppEvent::kind("connection-state-changed").with("connection_state", "connected"),
        RECONNECT_DEADLINE,
    );
    assert!(took <= RECONNECT_DEADLINE, "connected {took:?} after Quiet Hours ended");
    assert!(!record(&harness).exists(), "the record is cleared");
    assert!(!harness.simulated_phone().blocked());
    assert_eq!(harness.simulated_phone().connects_while_blocked(), 0);
    indicator_says(&harness, "My Pixel connected");
}

#[test]
fn turned_on_during_quiet_hours_calls_stay_on_across_a_restart_and_a_timed_break_ending_inside_them_stays_off()
 {
    let mut harness = harness_in(&time_zone_reading(23, 0, now()));
    harness
        .set_quiet_hours(minutes(22, 0), minutes(7, 0))
        .expect("Quiet Hours set");
    let state = harness.wait_for_app("calls go off for Quiet Hours", DEADLINE, |state| {
        state.calls_off == "off"
    });
    // 07:00 is 8 hours on.
    let end = state.calls_off_until;
    assert!(
        (now() + 8 * 3_600 - 60..=now() + 8 * 3_600 + 60).contains(&end),
        "{end} is not 07:00"
    );
    indicator_says(&harness, "Calls off until 07:00 (quiet hours)");

    cli_ok(&harness, &["calls", "on"]);
    wait_connected(&harness, RECONNECT_DEADLINE);
    let kept = record_text(&harness);
    assert!(kept.contains(r#""state":"kept-on""#), "{kept}");

    harness.restart_app();
    wait_connected(&harness, RECONNECT_DEADLINE);
    harness.app_holds("calls stay on after the restart", Duration::from_secs(2), |state| {
        state.calls_off == "on"
    });

    // A break by hand replaces Quiet Hours; ending inside them, it stays
    // off until they end.
    let until = now() + 3;
    harness.turn_calls_off(until).expect("calls turn off");
    wait_off_until(&harness, until);
    wait_until_past(until);
    wait_off_until(&harness, end);
    indicator_says(&harness, "Calls off until 07:00 (quiet hours)");
    assert!(harness.simulated_phone().blocked());
    assert_eq!(harness.simulated_phone().connects_while_blocked(), 0);
}

#[test]
fn the_cli_sets_and_clears_quiet_hours_and_they_persist_across_a_restart() {
    let mut harness = harness_in(&time_zone_reading(12, 0, now()));

    let said = cli_ok(&harness, &["quiet-hours", "22:00-06:30"]);
    assert_eq!(
        said,
        "Quiet Hours are 22:00 to 06:30: calls are off then every day, and on again after.\n"
    );
    harness.wait_for_app("Quiet Hours are set", DEADLINE, |state| {
        state.quiet_hours == Some((minutes(22, 0), minutes(6, 30)))
    });
    assert_eq!(
        fs::read_to_string(quiet_hours_file(&harness)).expect("the Quiet Hours file"),
        "22:00-06:30\n"
    );
    let address = harness
        .app_state()
        .phone
        .map(|phone| phone.address)
        .unwrap_or_default();
    assert_eq!(
        cli_ok(&harness, &["status"]),
        format!(
            "Connected to My Pixel ({address}).\nQuiet Hours: calls are off every day from 22:00 to 06:30.\n"
        )
    );
    assert_eq!(harness.app_state().calls_off, "on", "outside Quiet Hours");

    harness.restart_app();
    let state = wait_connected(&harness, RECONNECT_DEADLINE);
    assert_eq!(state.quiet_hours, Some((minutes(22, 0), minutes(6, 30))));

    // Until tomorrow morning ends when Quiet Hours end.
    let said = cli_ok(&harness, &["calls", "off", "--until-morning"]);
    assert_eq!(
        said,
        "Calls are off until 06:30: your phone stays disconnected, and connects again then.\n"
    );
    cli_ok(&harness, &["calls", "on"]);
    wait_connected(&harness, RECONNECT_DEADLINE);

    let refused = harness.cli(&["quiet-hours", "07:00-07:00"]);
    assert!(!refused.status.success());
    assert_eq!(
        String::from_utf8_lossy(&refused.stderr),
        "Give Quiet Hours as HH:MM-HH:MM on the 24-hour clock, with different start and end times, such as 22:00-07:00; or off to stop them.\n"
    );

    assert_eq!(
        cli_ok(&harness, &["quiet-hours", "off"]),
        "Quiet Hours are off.\n"
    );
    harness.wait_for_app("Quiet Hours are off", DEADLINE, |state| {
        state.quiet_hours.is_none()
    });
    assert!(!quiet_hours_file(&harness).exists());
    harness.restart_app();
    let state = wait_connected(&harness, RECONNECT_DEADLINE);
    assert_eq!(state.quiet_hours, None);
}

#[test]
fn set_quiet_hours_refuses_a_time_past_23_59() {
    let harness = harness_in("UTC");

    let refused = harness
        .set_quiet_hours(minutes(24, 0), minutes(7, 0))
        .expect_err("24:00 is refused");

    assert!(refused.message.starts_with("Give Quiet Hours as HH:MM-HH:MM"), "{refused:?}");
    assert_eq!(harness.app_state().quiet_hours, None);
}
