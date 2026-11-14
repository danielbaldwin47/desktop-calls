//! Scenarios: Calls Off for 1 hour or until tomorrow morning (ticket
//! #203). A deadline is an instant, kept in `calls-off.json`: when it
//! passes, calls come back on by themselves and the Phone connects within
//! the scorecard's 10 s. One that passes while the app is stopped takes
//! effect at the next start, and one that passes during a suspend takes
//! effect on the resume. The Indicator's menu and status, and the CLI,
//! show the end times in the Desktop's time zone, across a
//! daylight-saving change.

use std::fs;
use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use test_harness::{AppEvent, AppState, Harness, MenuEntry, TrayItem};

/// How long the app may take to notice a change on the Phone.
const DEADLINE: Duration = Duration::from_secs(5);
/// How soon the Phone is connected again once calls come back on (spec
/// #198, the polish scorecard's 10 s).
const RECONNECT_DEADLINE: Duration = Duration::from_secs(10);
const PHONE_NAME: &str = "My Pixel";
const DAY: u64 = 86_400;

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

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_else(|error| panic!("the clock is before 1970: {error}"))
        .as_secs()
}

/// Waits, by the wall clock, until `instant` has passed.
fn wait_until_past(instant: u64) {
    while now() <= instant {
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// `HH:MM` for the Unix time `instant`, `offset` seconds east of UTC.
fn clock_time(instant: u64, offset: i64) -> String {
    let local = instant
        .checked_add_signed(offset)
        .unwrap_or_else(|| panic!("{instant} is too early to show"));
    let minutes = local % DAY / 60;
    format!("{:02}:{:02}", minutes / 60, minutes % 60)
}

fn record(harness: &Harness) -> PathBuf {
    harness
        .directory()
        .join("state/desktop-calls/calls-off.json")
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

/// How long the Phone took to connect after `from`, by the app's log.
fn connected_after(harness: &Harness, from: &'static str) -> Duration {
    harness.app_took(
        &AppEvent::kind(from),
        &AppEvent::kind("connection-state-changed").with("connection_state", "connected"),
        RECONNECT_DEADLINE,
    )
}

fn cli_ok(harness: &Harness, arguments: &[&str]) -> String {
    let output = harness.cli(arguments);
    assert!(output.status.success(), "{arguments:?} failed: {output:?}");
    String::from_utf8_lossy(&output.stdout).into_owned()
}

/// Waits for calls to come back on and the Phone to connect, and checks
/// it happened within the scorecard's 10 s of `until`, by the wall clock.
fn connected_within_10_s_of(harness: &Harness, until: u64) {
    let left = Duration::from_secs(until.saturating_sub(now()));
    wait_connected(harness, left + RECONNECT_DEADLINE);
    assert!(now() >= until, "calls came back on before the deadline");
}

#[test]
fn a_deadline_replaced_by_a_later_one_doesnt_delay_the_later_one() {
    let harness = harness_in("UTC");
    let first = now() + 3;
    harness.turn_calls_off(first).expect("calls turn off");
    wait_off_until(&harness, first);
    cli_ok(&harness, &["calls", "on"]);
    wait_connected(&harness, RECONNECT_DEADLINE);

    let later = now() + 8;
    harness.turn_calls_off(later).expect("calls turn off again");
    wait_off_until(&harness, later);

    // The first deadline passes while calls are off until the later one:
    // nothing happens then, and the later one still turns calls on in time.
    wait_until_past(first);
    harness.app_holds("calls stay off past the replaced deadline", Duration::from_secs(1), |state| {
        state.calls_off == "off"
    });
    connected_within_10_s_of(&harness, later);
}

#[test]
fn turning_calls_off_again_while_they_are_off_sets_the_new_end_time() {
    let harness = harness_in("UTC");
    let first = now() + 3_600;
    harness.turn_calls_off(first).expect("calls turn off");
    wait_off_until(&harness, first);

    let sooner = now() + 6;
    harness.turn_calls_off(sooner).expect("calls turn off until sooner");

    wait_off_until(&harness, sooner);
    let record_text = fs::read_to_string(record(&harness)).expect("read the record");
    assert!(
        record_text.contains(&format!(r#""until":{sooner}"#)),
        "{record_text}"
    );
    connected_within_10_s_of(&harness, sooner);
}

#[test]
fn a_deadline_a_few_seconds_ahead_turns_calls_on_by_itself_and_reconnects_within_10_s() {
    let harness = harness_in("UTC");
    let until = now() + 6;

    harness.turn_calls_off(until).expect("calls turn off");

    wait_off_until(&harness, until);
    let item = indicator_says(&harness, &format!("Calls off until {}", clock_time(until, 0)));
    assert_eq!(item.status.as_deref(), Some("Active"), "no attention asked");
    let record_text = fs::read_to_string(record(&harness)).expect("read the record");
    assert!(
        record_text.contains(&format!(r#""until":{until}"#)),
        "{record_text}"
    );
    assert!(now() < until, "the setup took too long to test the deadline");
    harness.app_holds("calls stay off until the deadline", Duration::from_secs(1), |state| {
        state.calls_off == "off"
    });

    wait_connected(&harness, Duration::from_secs(6) + RECONNECT_DEADLINE);
    let took = connected_after(&harness, "calls-off-deadline-passed");
    assert!(took <= RECONNECT_DEADLINE, "connected {took:?} after the deadline");
    assert!(now() >= until, "calls came back on before the deadline");
    assert!(!record(&harness).exists(), "the record is cleared");
    assert!(!harness.simulated_phone().blocked());
    assert_eq!(harness.simulated_phone().connects_while_blocked(), 0);
    indicator_says(&harness, "My Pixel connected");
}

#[test]
fn a_deadline_that_passes_during_a_suspend_takes_effect_on_the_resume() {
    let harness = harness_in("UTC");
    let phone = harness.simulated_phone();
    let until = now() + 3;
    harness.turn_calls_off(until).expect("calls turn off");
    wait_off_until(&harness, until);

    phone.prepare_for_sleep(true);
    harness.wait_for_app_log("desktop-suspending", DEADLINE);
    wait_until_past(until);
    // Asleep, the Desktop doesn't act on the deadline.
    harness.app_holds("calls stay off while suspended", Duration::from_secs(2), |state| {
        state.calls_off == "off"
    });
    assert!(phone.blocked(), "the Phone is still blocked");

    phone.prepare_for_sleep(false);

    wait_connected(&harness, RECONNECT_DEADLINE);
    let took = connected_after(&harness, "desktop-resumed");
    assert!(took <= RECONNECT_DEADLINE, "connected {took:?} after the resume");
    harness.wait_for_app_log("calls-off-deadline-passed", DEADLINE);
    assert_eq!(phone.connects_while_blocked(), 0);
}

#[test]
fn a_deadline_that_passed_while_the_app_was_stopped_takes_effect_at_the_next_start() {
    let mut harness = harness_in("UTC");
    let until = now() + 2;
    harness.turn_calls_off(until).expect("calls turn off");
    wait_off_until(&harness, until);

    harness.kill_app_by_hand();
    wait_until_past(until);
    harness.start_app_by_hand();

    wait_connected(&harness, RECONNECT_DEADLINE);
    let took = connected_after(&harness, "calls-off-loaded");
    assert!(took <= RECONNECT_DEADLINE, "connected {took:?} after starting");
    assert!(!harness.simulated_phone().blocked());
    assert!(!record(&harness).exists(), "the record is cleared");
    assert_eq!(harness.simulated_phone().connects_while_blocked(), 0);
}

#[test]
fn a_deadline_still_ahead_at_the_next_start_keeps_calls_off_until_it_passes() {
    let mut harness = harness_in("UTC");
    let until = now() + 8;
    harness.turn_calls_off(until).expect("calls turn off");
    wait_off_until(&harness, until);

    harness.restart_app();

    wait_off_until(&harness, until);
    indicator_says(&harness, &format!("Calls off until {}", clock_time(until, 0)));
    assert!(harness.simulated_phone().blocked(), "the Phone is blocked");
    wait_connected(&harness, Duration::from_secs(8) + RECONNECT_DEADLINE);
    assert!(now() >= until, "calls came back on before the deadline");
    let took = connected_after(&harness, "calls-off-deadline-passed");
    assert!(took <= RECONNECT_DEADLINE, "connected {took:?} after the deadline");
}

#[test]
fn turning_calls_off_until_a_time_that_has_passed_is_refused() {
    let harness = harness_in("UTC");

    let refused = harness
        .turn_calls_off(now() - 120)
        .expect_err("a time that has passed is refused");

    assert_eq!(
        refused.message,
        "That time has already passed, so calls stay on; choose a time ahead."
    );
    assert_eq!(harness.app_state().calls_off, "on");
    assert!(!harness.simulated_phone().blocked());
}

#[test]
fn the_cli_turns_calls_off_for_a_length_or_until_a_time_and_says_until_when() {
    // Half an hour off the hour, east of UTC, with no daylight saving.
    let harness = harness_in("<+0530>-5:30");
    let offset = 19_800;

    let before = now();
    let said = cli_ok(&harness, &["calls", "off", "--for", "90m"]);
    let state = harness.wait_for_app("calls are off", DEADLINE, |state| state.calls_off == "off");
    let until = state.calls_off_until;
    assert!(
        (before + 5_400..=now() + 5_400).contains(&until),
        "{until} is not 90 minutes on"
    );
    let shown = clock_time(until, offset);
    assert_eq!(
        said,
        format!("Calls are off until {shown}: your phone stays disconnected, and connects again then.\n")
    );
    assert_eq!(
        cli_ok(&harness, &["status"]),
        format!("Calls are off until {shown}: your phone stays disconnected until then.\n")
    );
    cli_ok(&harness, &["calls", "on"]);
    wait_connected(&harness, RECONNECT_DEADLINE);

    // Two minutes on, to the minute, in local time.
    let wanted = (now() / 60 + 2) * 60;
    let shown = clock_time(wanted, offset);
    cli_ok(&harness, &["calls", "off", "--until", &shown]);
    wait_off_until(&harness, wanted);
    cli_ok(&harness, &["calls", "on"]);
    wait_connected(&harness, RECONNECT_DEADLINE);

    for (arguments, refusal) in [
        (
            ["calls", "off", "--for", "25h"],
            "Calls can be turned off for 1 minute to 24 hours, such as --for 90m or --for 2h; for longer, run desktop-calls calls off and turn them on yourself.\n",
        ),
        (
            ["calls", "off", "--until", "9:30"],
            "Give the time calls come back on as HH:MM on the 24-hour clock, such as --until 21:30.\n",
        ),
    ] {
        let output = harness.cli(&arguments);
        assert!(!output.status.success(), "{output:?}");
        assert_eq!(String::from_utf8_lossy(&output.stderr), refusal);
    }
    assert_eq!(harness.app_state().calls_off, "on");
}

/// Days since 1970-01-01 of the civil date `year`-`month`-`day`.
fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = year.div_euclid(400);
    let year_of_era = year - era * 400;
    let day_of_year = (153 * (month + if month > 2 { -3 } else { 9 }) + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

/// The year of the civil date `days` after 1970-01-01.
fn civil_year(days: i64) -> i64 {
    let mut year = 1970 + days / 366;
    while days_from_civil(year + 1, 1, 1) <= days {
        year += 1;
    }
    year
}

/// A POSIX time zone in which the local time is now 22:00 standard time,
/// whose daylight-saving time (an hour on) starts tomorrow at 02:00, with
/// its offset east of UTC in seconds and the Unix time of tomorrow's local
/// midnight.
fn evening_before_clocks_go_forward() -> (String, i64, u64) {
    let now = i64::try_from(now()).unwrap_or_else(|error| panic!("the clock: {error}"));
    let since_midnight = now.rem_euclid(86_400) / 60;
    // Whole minutes, from -12 h to +12 h.
    let offset_minutes = (22 * 60 - since_midnight + 720).rem_euclid(1_440) - 720;
    let offset = offset_minutes * 60;
    let tomorrow = (now + offset).div_euclid(86_400) + 1;
    let day_of_year = tomorrow - days_from_civil(civil_year(tomorrow), 1, 1);
    let ends = (day_of_year + 100) % 365;
    let (sign, minutes) = if offset_minutes >= 0 {
        ('-', offset_minutes)
    } else {
        ('+', -offset_minutes)
    };
    // POSIX counts offsets west of UTC; the names only need to be unique.
    let zone = format!(
        "<STD>{sign}{}:{:02}<DST>,{day_of_year}/2,{ends}/2",
        minutes / 60,
        minutes % 60
    );
    let midnight = u64::try_from(tomorrow * 86_400 - offset)
        .unwrap_or_else(|error| panic!("midnight before 1970: {error}"));
    (zone, offset, midnight)
}

#[test]
fn the_menu_offers_an_hour_and_tomorrow_morning_with_their_end_times_across_a_clock_change() {
    let (zone, offset, midnight) = evening_before_clocks_go_forward();
    let harness = harness_in(&zone);
    indicator_says(&harness, "My Pixel connected");

    let menu = harness.tray().menu();
    let choices = &MenuEntry::find(&menu, "Turn calls off")
        .expect("Turn calls off")
        .children;
    let labels = MenuEntry::labels(choices);
    // Local time is 22:00, or 22:01 if a minute turned meanwhile.
    assert_eq!(labels[0], "Until I turn them on");
    assert!(
        ["For 1 hour (until 23:00)", "For 1 hour (until 23:01)"].contains(&labels[1].as_str()),
        "{labels:?}"
    );
    assert_eq!(labels[2], "Until tomorrow morning (07:00)", "{labels:?}");

    harness.tray().click("Until tomorrow morning (07:00)");

    // 07:00 is daylight-saving time by then: an hour sooner than it would
    // be on standard time.
    let morning = midnight + 6 * 3_600;
    wait_off_until(&harness, morning);
    assert_eq!(clock_time(morning, offset + 3_600), "07:00");
    indicator_says(&harness, "Calls off until 07:00");
    assert_eq!(
        cli_ok(&harness, &["status"]),
        "Calls are off until 07:00: your phone stays disconnected until then.\n"
    );

    cli_ok(&harness, &["calls", "on"]);
    wait_connected(&harness, RECONNECT_DEADLINE);
    let menu = harness.tray().menu();
    let for_an_hour = MenuEntry::labels(
        &MenuEntry::find(&menu, "Turn calls off")
            .expect("Turn calls off")
            .children,
    )[1]
    .clone();
    let before = now();
    harness.tray().click(&for_an_hour);
    let state = harness.wait_for_app("calls are off", DEADLINE, |state| state.calls_off == "off");
    assert!(
        (before + 3_540..=now() + 3_600).contains(&state.calls_off_until),
        "{} is not an hour on",
        state.calls_off_until
    );
    let shown = clock_time(state.calls_off_until, offset);
    assert_eq!(for_an_hour, format!("For 1 hour (until {shown})"));
    indicator_says(&harness, &format!("Calls off until {shown}"));
}
