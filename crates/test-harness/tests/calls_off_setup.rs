//! Scenarios: the Setup Window while calls are off (ticket #207). It says
//! so and offers Turn calls on, which Enter presses: the Simulated Phone is
//! unblocked and connects, and the window carries on as usual. Pairing
//! waits while calls are off. A Phone paired during Quiet Hours isn't
//! blocked the moment it pairs (the note PR #260 left for this ticket).

use std::collections::BTreeMap;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use test_harness::{AppState, Harness, Key};

/// How long the app may take to show a change.
const DEADLINE: Duration = Duration::from_secs(5);
/// How long pairing and connecting may take after Confirm.
const CONNECT_DEADLINE: Duration = Duration::from_secs(15);
/// Longer than the reconnect waits and a blocked `Connect`'s slow failure.
const STAYS: Duration = Duration::from_secs(4);
/// Longer than BlueZ takes to block the Phone once the app asks.
const SETTLES: Duration = Duration::from_millis(500);
const PHONE_NAME: &str = "My Pixel";
const DAY: u64 = 86_400;

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_else(|error| panic!("the clock is before 1970: {error}"))
        .as_secs()
}

/// A POSIX `TZ` in which the local clock reads `hour:minute` exactly at
/// the Unix time `instant`.
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

fn setup_window(state: &AppState) -> Option<&BTreeMap<String, String>> {
    state.surface("setup-window")
}

fn field<'state>(state: &'state AppState, name: &str) -> Option<&'state str> {
    setup_window(state)?.get(name).map(String::as_str)
}

fn cli_ok(harness: &Harness, arguments: &[&str]) {
    let output = harness.cli(arguments);
    assert!(output.status.success(), "{arguments:?} failed: {output:?}");
}

fn is_off(state: &AppState) -> bool {
    state.calls_off == "off" && state.connection_state == "disconnected"
}

/// The Phone is connected, then calls are turned off until turned on, and
/// stay off while BlueZ blocks the Simulated Phone.
fn calls_off() -> Harness {
    let harness = Harness::builder()
        .headless_screen()
        .phone_name(PHONE_NAME)
        .start();
    harness.simulated_phone().connect();
    harness.wait_for_app("the Phone is connected", DEADLINE, |state| {
        state.connection_state == "connected" && state.calls_off == "on"
    });
    cli_ok(&harness, &["calls", "off"]);
    harness.wait_for_app("calls are off", DEADLINE, is_off);
    harness.app_holds("calls stay off", SETTLES, is_off);
    assert!(harness.simulated_phone().blocked(), "the Phone is blocked");
    harness
}

/// Opens the Setup Window, which says calls are off and has the keyboard
/// on Turn calls on.
fn opened_while_off(harness: &Harness) -> AppState {
    cli_ok(harness, &["connect"]);
    harness.wait_for_app(
        "the Setup Window says calls are off, with keyboard focus",
        DEADLINE,
        |state| field(state, "calls-off").is_some() && field(state, "focused") == Some("true"),
    )
}

#[test]
fn while_calls_are_off_the_setup_window_says_so_and_enter_turns_them_on() {
    let harness = calls_off();

    let state = opened_while_off(&harness);

    assert_eq!(field(&state, "calls-off"), Some("Calls are off"));
    assert_eq!(field(&state, "calls-off-button"), Some("Turn calls on"));
    assert!(
        field(&state, "calls-off-text")
            .is_some_and(|text| text.contains("until you turn calls on")),
        "{state:?}"
    );
    assert_eq!(field(&state, "dimmed"), Some("true"), "the step waits");
    assert_eq!(field(&state, "phones").unwrap_or_default(), "", "no Phone to choose");

    harness.keyboard().press(Key::Return);

    let state = harness.wait_for_app(
        "the Phone reconnects and the card is gone",
        Duration::from_secs(15),
        |state| {
            state.calls_off == "on"
                && state.connection_state == "connected"
                && field(state, "calls-off").is_none()
        },
    );
    assert!(!harness.simulated_phone().blocked(), "the Phone is unblocked");
    // Ticket #231: Turn calls on confirms, as from the Indicator.
    let notification = harness.notifications().wait_for(
        "the confirmation that calls are on",
        DEADLINE,
        |notification| notification.summary == "Calls on",
    );
    assert_eq!(notification.body, "My Pixel will reconnect.");
    // The window carries on as usual: it still lists to choose from.
    assert_eq!(field(&state, "step"), Some("choose"), "{state:?}");
    assert_eq!(field(&state, "dimmed"), None);
    harness.keyboard().press(Key::Escape);
    harness.wait_for_app("the Setup Window closes", DEADLINE, |state| {
        setup_window(state).is_none()
    });
}

#[test]
fn pairing_waits_while_calls_are_off() {
    let harness = calls_off();
    opened_while_off(&harness);

    // The Phone tries to pair from its own screen: the Desktop turns it
    // down, shows no code, and nothing connects.
    let _ = harness.simulated_phone().pair_from_phone();
    let state = harness.app_holds("the window keeps waiting", STAYS, |state| {
        is_off(state)
            && field(state, "calls-off").is_some()
            && field(state, "step") == Some("choose")
    });
    assert_eq!(field(&state, "code"), None);
    assert!(
        harness
            .simulated_phone()
            .pairing_requests()
            .iter()
            .all(|(_, outcome)| outcome != "confirmed"),
        "no pairing was confirmed"
    );
    assert!(harness.simulated_phone().blocked(), "the Phone stays blocked");
    assert_eq!(harness.simulated_phone().connects_while_blocked(), 0);
}

#[test]
fn a_phone_paired_during_quiet_hours_is_not_blocked_the_moment_it_pairs() {
    // 23:00, inside Quiet Hours from 22:00 to 07:00, with no Phone yet.
    let harness = Harness::builder()
        .headless_screen()
        .phone_name(PHONE_NAME)
        .phone_unpaired()
        .time_zone(&time_zone_reading(23, 0, now()))
        .start();
    harness
        .set_quiet_hours(22 * 60, 7 * 60)
        .expect("Quiet Hours set");
    harness.wait_for_app("the Setup Window lists the Phone", Duration::from_secs(10), |state| {
        field(state, "phones") == Some(PHONE_NAME) && field(state, "focused") == Some("true")
    });
    assert_eq!(harness.app_state().calls_off, "on", "no Phone to turn off yet");

    harness.keyboard().press(Key::Return);
    harness.wait_for_app("the same code on both screens", CONNECT_DEADLINE, |state| {
        field(state, "step") == Some("confirm")
    });
    harness.keyboard().press(Key::Return);

    let state = harness.wait_for_app(
        "the new Phone is connected, calls on",
        CONNECT_DEADLINE,
        |state| state.connection_state == "connected" && state.calls_off == "on",
    );
    assert_eq!(
        state.phone.as_ref().map(|phone| phone.name.as_str()),
        Some(PHONE_NAME)
    );
    harness.app_holds("calls stay on", STAYS, |state| {
        state.calls_off == "on" && state.connection_state == "connected"
    });
    assert!(!harness.simulated_phone().blocked(), "the Phone isn't blocked");
    assert_eq!(harness.simulated_phone().connects_while_blocked(), 0);
}
