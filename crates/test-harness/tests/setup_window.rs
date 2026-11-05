//! Scenarios: the Setup Window pairs a Phone from nothing, with the same
//! code on both screens, from either side; checks the Phone shares Calls;
//! opens again from the Indicator, the control interface and the command
//! line; pairs again after the Phone forgot the pairing; and switches to a
//! second Phone (ticket #21; polish scorecard items 4, 5, 7 and 8). Every
//! step is taken with synthetic keys alone.

use std::collections::BTreeMap;
use std::time::Duration;

use test_harness::{AppEvent, AppState, Bus, Harness, Key, RecordedMethodCall};

/// How long the app may take to start and show the Setup Window by itself.
const STARTUP: Duration = Duration::from_secs(10);
/// How long the app may take to notice a change.
const DEADLINE: Duration = Duration::from_secs(5);
/// How soon a nearby Phone is listed once the window shows (the ticket).
const LISTED_WITHIN: Duration = Duration::from_secs(2);
/// How long pairing and connecting may take after Confirm.
const CONNECT_DEADLINE: Duration = Duration::from_secs(15);
const PHONE_NAME: &str = "My Pixel";
const HEADPHONES: &str = "WH-1000XM5";
const OLD_PHONE: &str = "Old Pixel";
const HANDS_FREE_AUDIO_GATEWAY: &str = "0000111f-0000-1000-8000-00805f9b34fb";

/// A fresh install, with the Phone and a pair of headphones nearby.
fn fresh_install() -> Harness {
    Harness::builder()
        .headless_screen()
        .phone_name(PHONE_NAME)
        .phone_unpaired()
        .headphones_nearby(HEADPHONES)
        .start()
}

fn setup_window(state: &AppState) -> Option<&BTreeMap<String, String>> {
    state.surface("setup-window")
}

fn field<'state>(state: &'state AppState, name: &str) -> Option<&'state str> {
    setup_window(state)?.get(name).map(String::as_str)
}

/// Waits until the Setup Window is at `step`, and returns the app's state.
fn wait_for_step(harness: &Harness, step: &str, deadline: Duration) -> AppState {
    harness.wait_for_app(
        &format!("the Setup Window is at step {step:?}"),
        deadline,
        |state| field(state, "step") == Some(step),
    )
}

/// Waits until the Setup Window lists the Phone, with keyboard focus.
fn wait_for_phone_listed(harness: &Harness, deadline: Duration) -> AppState {
    harness.wait_for_app(
        "the Setup Window lists the Phone and has keyboard focus",
        deadline,
        |state| {
            field(state, "phones") == Some(PHONE_NAME) && field(state, "focused") == Some("true")
        },
    )
}

fn closed(harness: &Harness) {
    harness.wait_for_app("the Setup Window is closed", DEADLINE, |state| {
        setup_window(state).is_none()
    });
}

fn recorded(harness: &Harness) -> Vec<RecordedMethodCall> {
    harness
        .simulated_phone()
        .wait_for_recorded("the record", DEADLINE, |_| true)
}

fn adapter_discoverable(harness: &Harness) -> bool {
    let reply = harness.call_phone_method(
        Bus::System,
        "org.bluez",
        "/org/bluez/hci0",
        "org.freedesktop.DBus.Properties",
        "Get",
        &("org.bluez.Adapter1", "Discoverable"),
    );
    let value: zbus::OwnedVariant = reply
        .body()
        .deserialize()
        .unwrap_or_else(|error| panic!("Discoverable isn't a variant: {error}"));
    bool::try_from(value).unwrap_or_else(|error| panic!("Discoverable isn't a boolean: {error}"))
}

/// Waits until the window names the Desktop as Phones list it, which it
/// does once the Desktop is discoverable, and checks BlueZ agrees.
fn wait_until_discoverable(harness: &Harness) {
    harness.wait_for_app("the Setup Window names the Desktop", DEADLINE, |state| {
        field(state, "desktop").is_some()
    });
    assert!(adapter_discoverable(harness), "the adapter is discoverable");
}

/// Waits until BlueZ says the adapter isn't discoverable.
fn wait_until_not_discoverable(harness: &Harness) {
    let give_up = std::time::Instant::now() + DEADLINE;
    while adapter_discoverable(harness) {
        assert!(
            std::time::Instant::now() < give_up,
            "the adapter is still discoverable after {DEADLINE:?}"
        );
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// The window shows the code the Simulated Phone's latest pairing shows,
/// as six digits; returns it.
fn same_code_shown(harness: &Harness) -> String {
    let state = wait_for_step(harness, "confirm", CONNECT_DEADLINE);
    let shown = field(&state, "code").unwrap_or_default().to_owned();
    let (code, outcome) = harness
        .simulated_phone()
        .pairing_requests()
        .pop()
        .unwrap_or_else(|| panic!("the Phone was never asked to pair"));
    assert_eq!(outcome, "waiting");
    assert_eq!(shown, format!("{code:06}"), "the same six digits");
    shown
}

fn connected(harness: &Harness) -> AppState {
    harness.wait_for_app("ConnectionState is connected", CONNECT_DEADLINE, |state| {
        state.connection_state == "connected"
    })
}

#[test]
fn on_a_fresh_install_the_setup_window_opens_by_itself_and_lists_only_the_phone() {
    let harness = fresh_install();

    wait_for_phone_listed(&harness, STARTUP);
    harness.assert_app_within(
        "the Setup Window to list the Phone once shown",
        &AppEvent::kind("surface-shown").with("surface", "setup-window"),
        &AppEvent::kind("setup-phone-listed"),
        LISTED_WITHIN,
    );
    // Discovery found the headphones too, after the Phone.
    let found = recorded(&harness);
    assert!(
        found.iter().any(|call| call.member == "StartDiscovery"),
        "{found:?}"
    );
    std::thread::sleep(Duration::from_secs(1));
    assert!(harness.surfaces_seen().iter().all(|shown| {
        shown
            .get("setup-window")
            .and_then(|window| window.get("phones"))
            .is_none_or(|phones| !phones.contains(HEADPHONES))
    }));
}

#[test]
fn choosing_the_phone_pairs_with_the_same_code_trusts_and_connects_it() {
    let harness = fresh_install();
    wait_for_phone_listed(&harness, STARTUP);

    harness.keyboard().press(Key::Return);
    same_code_shown(&harness);
    harness.keyboard().press(Key::Return);

    let state = connected(&harness);
    assert_eq!(
        state.phone.as_ref().map(|phone| phone.name.as_str()),
        Some(PHONE_NAME)
    );
    let calls = recorded(&harness);
    assert!(calls.iter().any(|call| call.member == "Pair"), "{calls:?}");
    assert!(
        calls.iter().any(|call| call.member == "Set"
            && call.arguments.contains("Trusted")
            && call.arguments.contains("true")),
        "Trusted is set: {calls:?}"
    );
    assert_eq!(
        harness
            .simulated_phone()
            .pairing_requests()
            .pop()
            .map(|(_, outcome)| outcome),
        Some("confirmed".to_owned())
    );
    wait_for_step(&harness, "done", CONNECT_DEADLINE);
    for step in [
        "choose",
        "pairing",
        "confirm",
        "confirmed",
        "connecting",
        "contacts-warning",
        "done",
    ] {
        harness.wait_for_app_log(&format!(r#""step":"{step}""#), DEADLINE);
    }
}

#[test]
fn a_phone_pairing_from_its_own_screen_shows_the_same_code_while_the_window_is_open() {
    let harness = fresh_install();
    wait_for_phone_listed(&harness, STARTUP);
    wait_until_discoverable(&harness);

    harness
        .simulated_phone()
        .pair_from_phone()
        .expect("the Phone finds the Desktop");
    same_code_shown(&harness);
    harness.keyboard().press(Key::Return);
    connected(&harness);
    wait_for_step(&harness, "done", CONNECT_DEADLINE);

    harness.keyboard().press(Key::Return);
    closed(&harness);
    wait_until_not_discoverable(&harness);
    assert!(
        harness.simulated_phone().pair_from_phone().is_err(),
        "with the window closed the Phone can't find the Desktop"
    );
}

#[test]
fn a_phone_connecting_media_without_hands_free_is_asked_for_hands_free() {
    let harness = fresh_install();
    harness.simulated_phone().set_hands_free("when-asked");
    wait_for_phone_listed(&harness, STARTUP);

    harness.keyboard().press(Key::Return);
    same_code_shown(&harness);
    harness.keyboard().press(Key::Return);

    connected(&harness);
    let calls = recorded(&harness);
    assert!(
        calls.iter().any(|call| call.member == "ConnectProfile"
            && call.arguments.contains(HANDS_FREE_AUDIO_GATEWAY)),
        "ConnectProfile for hands-free: {calls:?}"
    );
    wait_for_step(&harness, "done", CONNECT_DEADLINE);
}

#[test]
fn a_phone_refusing_hands_free_shows_where_to_switch_phone_calls_on() {
    let harness = fresh_install();
    harness.simulated_phone().set_hands_free("refused");
    wait_for_phone_listed(&harness, STARTUP);
    harness.keyboard().press(Key::Return);
    same_code_shown(&harness);
    harness.keyboard().press(Key::Return);

    let state = wait_for_step(&harness, "phone-calls-off", CONNECT_DEADLINE);
    let instructions = field(&state, "instructions").unwrap_or_default();
    assert!(
        instructions.contains("On your phone: switch Phone calls on."),
        "{instructions}"
    );
    assert!(
        instructions.contains("tap the settings icon next to “desktop”"),
        "{instructions}"
    );

    // The user switches Phone calls on, and tries again.
    harness.simulated_phone().set_hands_free("when-asked");
    harness.keyboard().press(Key::Return);
    connected(&harness);
    wait_for_step(&harness, "done", CONNECT_DEADLINE);
}

#[test]
fn the_indicator_the_control_interface_and_the_command_line_each_open_the_window() {
    let harness = Harness::builder()
        .headless_screen()
        .phone_name(PHONE_NAME)
        .start();
    harness.simulated_phone().connect();
    connected(&harness);
    assert!(setup_window(&harness.app_state()).is_none());

    harness.tray().click("Connect your phone…");
    wait_for_step(&harness, "choose", DEADLINE);
    harness.keyboard().press(Key::Escape);
    closed(&harness);

    harness.open_connect_window();
    wait_for_step(&harness, "choose", DEADLINE);
    harness.keyboard().press(Key::Escape);
    closed(&harness);

    let output = harness.cli(&["connect"]);
    assert!(output.status.success(), "{output:?}");
    wait_for_step(&harness, "choose", DEADLINE);
}

#[test]
fn pairing_a_second_phone_switches_to_it() {
    let harness = Harness::builder()
        .headless_screen()
        .phone_name(PHONE_NAME)
        .phone_unpaired()
        .old_phone_paired(OLD_PHONE)
        .start();
    harness.wait_for_app("the old Phone is followed", STARTUP, |state| {
        state
            .phone
            .as_ref()
            .is_some_and(|phone| phone.name == OLD_PHONE)
    });
    assert!(setup_window(&harness.app_state()).is_none());

    let output = harness.cli(&["connect"]);
    assert!(output.status.success(), "{output:?}");
    wait_for_phone_listed(&harness, DEADLINE);
    harness.keyboard().press(Key::Return);
    same_code_shown(&harness);
    harness.keyboard().press(Key::Return);

    let state = connected(&harness);
    assert_eq!(
        state.phone.as_ref().map(|phone| phone.name.as_str()),
        Some(PHONE_NAME)
    );
}

#[test]
fn pair_again_after_the_phone_forgot_the_pairing() {
    let harness = Harness::builder()
        .headless_screen()
        .phone_name(PHONE_NAME)
        .start();
    harness.simulated_phone().connect();
    connected(&harness);
    harness.simulated_phone().forget_pairing();
    harness.wait_for_app(
        "the pairing-lost Problem shows",
        CONNECT_DEADLINE,
        |state| {
            state
                .problems
                .iter()
                .any(|problem| problem.kind == "pairing-lost")
        },
    );

    harness.tray().click("Pairing lost: pair again…");
    wait_for_step(&harness, "choose", DEADLINE);
    wait_until_discoverable(&harness);
    harness
        .simulated_phone()
        .pair_from_phone()
        .expect("the Phone finds the Desktop");
    same_code_shown(&harness);
    harness.keyboard().press(Key::Return);

    connected(&harness);
    wait_for_step(&harness, "done", CONNECT_DEADLINE);
    assert!(harness.app_state().problems.is_empty());
}
