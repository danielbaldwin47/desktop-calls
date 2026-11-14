//! Scenarios: turning calls off never cuts a live Call (ticket #204).
//! Turned off from the Indicator or the CLI during a Call, calls stay on,
//! the Indicator says they turn off when this Call ends and offers Keep
//! calls on, and once the last Call ends the Simulated Phone is blocked.
//! Keep calls on cancels it; the waiting turn-off survives an app restart
//! mid-Call; a deadline that passes during the Call leaves calls on.

use std::fs;
use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use test_harness::{AppState, Harness};

/// How long the app may take to notice a change on the Phone.
const DEADLINE: Duration = Duration::from_secs(5);
/// Longer than the reconnect waits and a blocked `Connect`'s slow failure.
const STAYS: Duration = Duration::from_secs(4);
const PHONE_NAME: &str = "My Pixel";
const NUMBER: &str = "+447700900123";
const PENDING: &str = "Calls turn off when this Call ends";

fn connected() -> Harness {
    let harness = Harness::builder().phone_name(PHONE_NAME).start();
    harness.simulated_phone().connect();
    harness.wait_for_app("the Phone is connected", DEADLINE, |state| {
        state.connection_state == "connected" && state.calls_off == "on"
    });
    harness
}

/// A Call answered on the Phone, live; returns its object path there.
fn live_call(harness: &Harness) -> String {
    let path = harness.simulated_phone().ring(NUMBER);
    harness.wait_for_app("the Call rings", DEADLINE, |state| state.ringing);
    harness.simulated_phone().answer_on_phone(&path);
    harness.wait_for_app("the Call is live", DEADLINE, is_live);
    path
}

fn is_live(state: &AppState) -> bool {
    state.calls.len() == 1 && state.calls[0].state == "active"
}

/// Calls are on, with a turn-off waiting, and the Call carries on.
fn waiting(state: &AppState) -> bool {
    state.calls_off == "on"
        && state.calls_off_pending
        && state.connection_state == "connected"
        && !state.calls.is_empty()
}

fn wait_waiting(harness: &Harness) -> AppState {
    harness.wait_for_app(
        "a turn-off waits, and the Call carries on",
        DEADLINE,
        waiting,
    )
}

fn wait_off(harness: &Harness) -> AppState {
    harness.wait_for_app(
        "calls are off and the Phone disconnected",
        DEADLINE,
        |state| {
            state.calls_off == "off"
                && !state.calls_off_pending
                && state.connection_state == "disconnected"
        },
    )
}

/// Calls stay on and the Phone connected, with nothing waiting.
fn stays_on(harness: &Harness) {
    harness.app_holds("calls stay on and the Phone connected", STAYS, |state| {
        state.calls_off == "on" && !state.calls_off_pending && state.connection_state == "connected"
    });
    assert!(!harness.simulated_phone().blocked());
}

fn record(harness: &Harness) -> PathBuf {
    harness
        .directory()
        .join("state/desktop-calls/calls-off.json")
}

fn cli_ok(harness: &Harness, arguments: &[&str]) -> String {
    let output = harness.cli(arguments);
    assert!(output.status.success(), "{arguments:?} failed: {output:?}");
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_else(|error| panic!("the clock is before 1970: {error}"))
        .as_secs()
}

#[test]
fn calls_off_from_the_indicator_during_a_call_waits_for_it_to_end() {
    let harness = connected();
    let call = live_call(&harness);

    harness.tray().click("Until I turn them on");

    wait_waiting(&harness);
    harness.app_holds("the Call carries on, calls on", STAYS, waiting);
    assert!(!harness.simulated_phone().blocked(), "the Phone isn't blocked");
    let items = harness.tray().wait_for("the Indicator says calls turn off", DEADLINE, |items| {
        items.iter().any(|item| item.tooltip.as_deref() == Some(PENDING))
    });
    assert_eq!(
        items[0].icon_name.as_deref(),
        Some("desktop-calls-in-call-symbolic"),
        "{items:?}"
    );
    let menu = harness.tray().menu();
    assert!(
        menu.iter().any(|entry| entry.label == "Keep calls on" && entry.enabled),
        "{menu:?}"
    );
    assert!(!menu.iter().any(|entry| entry.label == "Turn calls off"), "{menu:?}");
    let record_text = fs::read_to_string(record(&harness)).expect("read the record");
    assert!(record_text.contains(r#""state":"pending""#), "{record_text}");

    harness.simulated_phone().caller_hangs_up(&call);

    wait_off(&harness);
    assert!(harness.simulated_phone().blocked(), "the Phone is blocked");
    harness.wait_for_app_log("calls-off-pending", DEADLINE);
    assert_eq!(harness.simulated_phone().connects_while_blocked(), 0);
}

#[test]
fn calls_off_from_the_cli_while_a_call_rings_waits_and_calls_on_keeps_them_on() {
    let harness = connected();
    let call = harness.simulated_phone().ring(NUMBER);
    harness.wait_for_app("the Call rings", DEADLINE, |state| state.ringing);

    let said = cli_ok(&harness, &["calls", "off"]);
    assert_eq!(
        said,
        "Calls turn off when this Call ends, until you turn them on. Your Call carries on; run desktop-calls calls on to keep calls on.\n"
    );
    let state = wait_waiting(&harness);
    assert!(state.ringing, "the Call still rings");
    assert!(cli_ok(&harness, &["status"]).ends_with(
        "Calls turn off when this Call ends, until you turn them on.\n"
    ));

    let said = cli_ok(&harness, &["calls", "on"]);
    assert_eq!(said, "Calls stay on: they won't turn off when this Call ends.\n");
    harness.wait_for_app("nothing waits", DEADLINE, |state| !state.calls_off_pending);
    assert!(!record(&harness).exists(), "the record is cleared");

    harness.simulated_phone().caller_hangs_up(&call);
    harness.wait_for_app("the Call ended", DEADLINE, |state| state.calls.is_empty());
    stays_on(&harness);
}

#[test]
fn keep_calls_on_from_the_indicator_cancels_the_waiting_turn_off() {
    let harness = connected();
    let call = live_call(&harness);
    harness.tray().click("Until I turn them on");
    wait_waiting(&harness);

    harness.tray().click("Keep calls on");

    harness.wait_for_app("nothing waits", DEADLINE, |state| !state.calls_off_pending);
    harness.wait_for_app_log("calls-kept-on", DEADLINE);
    harness.simulated_phone().caller_hangs_up(&call);
    harness.wait_for_app("the Call ended", DEADLINE, |state| state.calls.is_empty());
    stays_on(&harness);
}

#[test]
fn a_waiting_turn_off_survives_an_app_restart_mid_call() {
    let mut harness = connected();
    let call = live_call(&harness);
    cli_ok(&harness, &["calls", "off"]);
    wait_waiting(&harness);

    harness.restart_app();

    harness.wait_for_app(
        "the Call shows again, and the turn-off still waits",
        DEADLINE,
        |state| waiting(state) && is_live(state),
    );
    assert!(!harness.simulated_phone().blocked(), "the Phone isn't blocked");

    harness.simulated_phone().caller_hangs_up(&call);
    wait_off(&harness);
    assert!(harness.simulated_phone().blocked(), "the Phone is blocked");
}

#[test]
fn a_deadline_that_passes_during_the_call_leaves_calls_on() {
    let harness = connected();
    let call = live_call(&harness);
    let until = now() + 3;

    harness.turn_calls_off(until).expect("the turn-off waits");
    let state = wait_waiting(&harness);
    assert_eq!(state.calls_off_until, until, "it keeps the deadline it was given");

    harness.wait_for_app(
        "past the deadline, nothing waits and the Call carries on",
        Duration::from_secs(8),
        |state| !state.calls_off_pending && is_live(state),
    );
    assert!(!record(&harness).exists(), "the record is cleared");
    harness.simulated_phone().caller_hangs_up(&call);
    harness.wait_for_app("the Call ended", DEADLINE, |state| state.calls.is_empty());
    stays_on(&harness);
}

/// How long the Call Window may take to collapse by itself: the 5 s, and a
/// margin for a loaded runner.
const COLLAPSE_DEADLINE: Duration = Duration::from_secs(9);
const TAG: &str = "Calls off after";

/// The app on a headless screen, so its surfaces are reported, with the
/// Phone connected.
fn on_screen() -> Harness {
    let harness = Harness::builder()
        .headless_screen()
        .omarchy_bar()
        .phone_name(PHONE_NAME)
        .start();
    harness.simulated_phone().connect();
    harness.wait_for_app("the Phone is connected", DEADLINE, |state| {
        state.connection_state == "connected" && state.calls_off == "on"
    });
    harness
}

/// What `surface` says its "Calls off after" tag reads, if it shows one.
fn tag<'a>(state: &'a AppState, surface: &str) -> Option<&'a str> {
    state
        .surface(surface)
        .and_then(|content| content.get("calls-off-after"))
        .map(String::as_str)
}

/// What the Call Window, or once it has collapsed the Call Pill, says its
/// "Calls off after" tag reads.
fn tagged(state: &AppState) -> Option<&str> {
    tag(state, "call-window").or_else(|| tag(state, "call-pill"))
}

/// Whether the Call Window or the Call Pill is on screen.
fn shown(state: &AppState) -> bool {
    state.surface("call-window").is_some() || state.surface("call-pill").is_some()
}

#[test]
fn the_call_window_is_tagged_calls_off_after_while_a_turn_off_waits_and_the_tag_goes_on_keep_calls_on()
 {
    let harness = on_screen();
    let call = live_call(&harness);
    // The Call Window collapses to the Call Pill 5 s after the Call
    // connects, so on a slow runner either surface may be the one shown.
    harness.wait_for_app("the Call shows, untagged", DEADLINE, |state| {
        shown(state) && tagged(state).is_none()
    });

    cli_ok(&harness, &["calls", "off"]);

    harness.wait_for_app("the Call is tagged", DEADLINE, |state| {
        state.calls_off_pending && tagged(state) == Some(TAG)
    });

    harness.tray().click("Keep calls on");

    harness.wait_for_app("the tag goes, the Call still shown", DEADLINE, |state| {
        !state.calls_off_pending && shown(state) && tagged(state).is_none()
    });
    harness.simulated_phone().caller_hangs_up(&call);
}

#[test]
fn the_call_pill_is_tagged_too_and_the_tag_goes_when_the_call_ends() {
    let harness = on_screen();
    let call = live_call(&harness);
    cli_ok(&harness, &["calls", "off"]);
    wait_waiting(&harness);

    harness.wait_for_app(
        "the Call Window collapses to a tagged Call Pill",
        COLLAPSE_DEADLINE,
        |state| state.surface("call-window").is_none() && tag(state, "call-pill") == Some(TAG),
    );

    harness.simulated_phone().caller_hangs_up(&call);

    wait_off(&harness);
    // The Ended window lasts 2 s, which `wait_off` may outlast, so this
    // holds once it has gone too; the core tests cover the Ended window.
    harness.wait_for_app("no surface is tagged", DEADLINE, |state| {
        tagged(state).is_none()
    });
}
