//! Scenarios: the Dialer while calls are off (ticket #206). It says so,
//! and Enter places that one Call: the Simulated Phone is unblocked and
//! connected, the Call placed once it connects, and calls go off again
//! when it ends, with no `Connect` while blocked. Hang up first, or no
//! connection in 15 s, puts calls back off; Keep calls on cancels the
//! return to off; and the return to off survives an app restart mid-Call.

use std::fs;
use std::path::PathBuf;
use std::time::Duration;

use test_harness::{AppState, Harness, Key};

/// How long the app may take to show a change.
const DEADLINE: Duration = Duration::from_secs(5);
/// Longer than the 15 s the Phone has to connect for the Call.
const PHONE_FOR_DIAL: Duration = Duration::from_secs(25);
/// Longer than the reconnect waits and a blocked `Connect`'s slow failure.
const STAYS: Duration = Duration::from_secs(4);
/// Longer than BlueZ takes to block the Phone once the app asks.
const SETTLES: Duration = Duration::from_millis(500);
const TYPED: &str = "07700 900888";
const DIALLED: &str = "07700900888";
const AGAIN: &str = "Calls off again when this Call ends";
const CONNECTING: &str = "Connecting to your phone…";

fn on_screen() -> Harness {
    let harness = Harness::builder().headless_screen().omarchy_bar().start();
    harness.simulated_phone().connect();
    harness.wait_for_app("the Phone is connected", DEADLINE, |state| {
        state.connection_state == "connected" && state.calls_off == "on"
    });
    harness
}

fn cli_ok(harness: &Harness, arguments: &[&str]) -> String {
    let output = harness.cli(arguments);
    assert!(output.status.success(), "{arguments:?} failed: {output:?}");
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn is_off(state: &AppState) -> bool {
    state.calls_off == "off" && !state.calls_off_pending && state.connection_state == "disconnected"
}

/// Calls are off, and stay off while BlueZ blocks the Simulated Phone.
fn wait_off(harness: &Harness, why: &str) -> AppState {
    harness.wait_for_app(why, DEADLINE, is_off);
    let state = harness.app_holds("calls stay off", SETTLES, is_off);
    assert!(harness.simulated_phone().blocked(), "the Phone is blocked");
    state
}

/// Calls turned off until turned on, from the CLI.
fn calls_off(harness: &Harness) {
    cli_ok(harness, &["calls", "off"]);
    wait_off(harness, "calls are off");
}

fn shown<'a>(state: &'a AppState, surface: &str, key: &str) -> Option<&'a str> {
    state
        .surface(surface)
        .and_then(|content| content.get(key))
        .map(String::as_str)
}

/// Opens the Dialer and types the number: it says calls are off, and that
/// Enter calls anyway.
fn dialer_typed(harness: &Harness) -> AppState {
    cli_ok(harness, &["dialer"]);
    harness.wait_for_app("the Dialer has keyboard focus", DEADLINE, |state| {
        shown(state, "dialer", "focused") == Some("true")
    });
    harness.keyboard().type_text(TYPED);
    harness.wait_for_app("the Dialer searched for the number", DEADLINE, |state| {
        shown(state, "dialer", "query") == Some(TYPED)
    })
}

/// The Phone dials the number, and the person called answers.
fn answered(harness: &Harness) {
    dialled(harness);
    let _ = harness.simulated_phone().alert();
    harness.wait_for_app("the Call rings at the other end", DEADLINE, |state| {
        state.calls.len() == 1 && state.calls[0].state == "alerting"
    });
    harness.simulated_phone().callee_answers();
}

fn dialled(harness: &Harness) {
    let expected = format!("(\"{DIALLED}\",)");
    harness
        .simulated_phone()
        .wait_for_recorded("the Phone is asked to dial", DEADLINE, |calls| {
            calls
                .iter()
                .any(|call| call.member == "Dial" && call.arguments == expected)
        });
}

/// The Call placed from the Dialer while calls were off, answered: returns
/// its path on the Phone.
fn placed_anyway(harness: &Harness) -> String {
    calls_off(harness);
    dialer_typed(harness);
    harness.keyboard().press(Key::Return);
    answered(harness);
    harness.wait_for_app(
        "the Call is live, calls on until it ends",
        DEADLINE,
        |state| {
            state.calls.len() == 1
                && state.calls[0].state == "active"
                && state.calls_off == "on"
                && state.calls_off_pending
        },
    );
    let calls = harness.simulated_phone().calls_on_phone();
    let Some((path, _)) = calls.first() else {
        panic!("the Phone has no Call");
    };
    path.clone()
}

fn record(harness: &Harness) -> PathBuf {
    harness
        .directory()
        .join("state/desktop-calls/calls-off.json")
}

fn says_again(harness: &Harness) {
    harness.tray().wait_for("the Indicator says calls go off again", DEADLINE, |items| {
        items.iter().any(|item| item.tooltip.as_deref() == Some(AGAIN))
    });
}

#[test]
fn enter_in_the_dialer_while_calls_are_off_places_that_call_and_calls_go_off_again_after() {
    let harness = on_screen();
    calls_off(&harness);

    let state = dialer_typed(&harness);
    assert_eq!(
        shown(&state, "dialer", "calls-off"),
        Some("Calls are off. Enter calls anyway; calls go off again after.")
    );
    assert!(
        shown(&state, "dialer", "keys").is_some_and(|keys| keys.starts_with("Enter Call anyway")),
        "{:?}",
        state.surface("dialer")
    );
    assert_eq!(shown(&state, "dialer", "notice"), None);

    harness.keyboard().press(Key::Return);
    answered(&harness);
    harness.wait_for_app("the Call is live", DEADLINE, |state| {
        state.calls.len() == 1 && state.calls[0].state == "active" && state.calls_off_pending
    });
    says_again(&harness);
    let menu = harness.tray().menu();
    assert!(
        menu.iter().any(|entry| entry.label == "Keep calls on" && entry.enabled),
        "{menu:?}"
    );
    let record_text = fs::read_to_string(record(&harness)).expect("read the record");
    assert!(
        record_text.contains(r#""state":"pending""#) && record_text.contains(r#""exception":true"#),
        "{record_text}"
    );
    harness.wait_for_app_log("call-placed-while-calls-off", DEADLINE);

    let calls = harness.simulated_phone().calls_on_phone();
    let Some((path, _)) = calls.first() else {
        panic!("the Phone has no Call");
    };
    harness.simulated_phone().caller_hangs_up(path);

    wait_off(&harness, "calls are off again after the Call");
    assert_eq!(harness.simulated_phone().connects_while_blocked(), 0);
    assert!(
        cli_ok(&harness, &["status"])
            .contains("Calls are off until you turn them on: your phone stays disconnected.")
    );
}

#[test]
fn the_phone_not_connecting_in_15_s_says_out_of_range_and_calls_go_back_off() {
    let harness = on_screen();
    harness.simulated_phone().disconnect();
    harness.wait_for_app("the Phone is out of range", DEADLINE, |state| {
        state.connection_state == "disconnected"
    });
    calls_off(&harness);
    dialer_typed(&harness);

    harness.keyboard().press(Key::Return);

    harness.wait_for_app("the Call Window says it is connecting", DEADLINE, |state| {
        shown(state, "call-window", "state") == Some(CONNECTING)
    });
    let state = harness.wait_for_app(
        "the Call Window says the Phone is out of range",
        PHONE_FOR_DIAL,
        |state| shown(state, "call-window", "notice").is_some(),
    );
    assert_eq!(
        shown(&state, "call-window", "notice"),
        Some("Phone not connected: it's out of range or has Bluetooth off.")
    );
    wait_off(&harness, "calls are back off");
    assert_eq!(harness.simulated_phone().connects_while_blocked(), 0);
}

#[test]
fn hanging_up_before_the_phone_connects_cancels_the_call_and_calls_go_back_off() {
    let harness = on_screen();
    harness.simulated_phone().disconnect();
    harness.wait_for_app("the Phone is out of range", DEADLINE, |state| {
        state.connection_state == "disconnected"
    });
    calls_off(&harness);
    dialer_typed(&harness);
    harness.keyboard().press(Key::Return);
    harness.wait_for_app("the Call Window says it is connecting", DEADLINE, |state| {
        shown(state, "call-window", "state") == Some(CONNECTING)
    });

    harness.keyboard().press_with_control('h');

    let state = wait_off(&harness, "calls are back off");
    assert_eq!(shown(&state, "call-window", "state"), Some("Call ended"));
    harness.app_holds("calls stay off", STAYS, is_off);
    assert_eq!(harness.simulated_phone().connects_while_blocked(), 0);
}

#[test]
fn keep_calls_on_during_the_call_leaves_calls_on_after_it() {
    let harness = on_screen();
    let call = placed_anyway(&harness);

    harness.tray().click("Keep calls on");

    harness.wait_for_app("nothing waits", DEADLINE, |state| !state.calls_off_pending);
    harness.simulated_phone().caller_hangs_up(&call);
    harness.wait_for_app("the Call ended", DEADLINE, |state| state.calls.is_empty());
    harness.app_holds("calls stay on and the Phone connected", STAYS, |state| {
        state.calls_off == "on" && !state.calls_off_pending && state.connection_state == "connected"
    });
    assert!(!harness.simulated_phone().blocked());
}

#[test]
fn calls_go_off_again_after_the_call_even_across_an_app_restart() {
    let mut harness = on_screen();
    let call = placed_anyway(&harness);

    harness.restart_app();

    harness.wait_for_app(
        "the Call shows again, and calls still go off after it",
        DEADLINE,
        |state| state.calls.len() == 1 && state.calls_off == "on" && state.calls_off_pending,
    );
    says_again(&harness);
    harness.simulated_phone().caller_hangs_up(&call);
    wait_off(&harness, "calls are off again after the Call");
}
