//! Scenarios: the Phone connects and drops, observed through the control
//! interface and the CLI.

use std::time::Duration;

use test_harness::{AppState, Harness, PhoneIdentity};

/// How long the app may take to notice a change on the Phone.
const DEADLINE: Duration = Duration::from_secs(5);

#[test]
fn simulated_phone_connects() {
    let harness = Harness::start();

    harness.simulated_phone().connect();

    let state = harness.wait_for_app("ConnectionState is connected", DEADLINE, |state| {
        state.connection_state == "connected"
    });
    assert_eq!(
        state.phone,
        Some(PhoneIdentity {
            name: harness.phone().name.clone(),
            address: harness.phone().address.clone(),
        })
    );
    // The app found the AudioGateway at this run's index, never `ag0`.
    assert_ne!(harness.audio_gateway_index(), 0);
    assert_eq!(
        harness.simulated_phone().audio_gateway(),
        format!(
            "/org/pipewire/Telephony/ag{}",
            harness.audio_gateway_index()
        )
    );
}

#[test]
fn the_phone_link_finds_audio_gateways_only_through_the_object_manager() {
    let harness = Harness::start();
    harness.simulated_phone().connect();
    harness.wait_for_app("ConnectionState is connected", DEADLINE, |state| {
        state.connection_state == "connected"
    });

    let telephony_calls: Vec<_> = harness
        .simulated_phone()
        .recorded_method_calls()
        .into_iter()
        .filter(|call| call.bus == "session" && call.path.starts_with("/org/pipewire/Telephony"))
        .collect();

    assert!(
        telephony_calls
            .iter()
            .any(|call| call.path == "/org/pipewire/Telephony"
                && call.interface == "org.freedesktop.DBus.ObjectManager"
                && call.member == "GetManagedObjects"),
        "the app never listed the Telephony objects: {telephony_calls:#?}"
    );
    assert!(
        !telephony_calls
            .iter()
            .any(|call| call.member == "Introspect"),
        "PipeWire's Introspect lists no children, so walking it can't work: {telephony_calls:#?}"
    );
}

#[test]
fn hands_free_drops_and_returns() {
    let harness = Harness::start();
    harness.simulated_phone().connect();
    harness.wait_for_app("ConnectionState is connected", DEADLINE, |state| {
        state.connection_state == "connected"
    });

    harness.simulated_phone().disconnect();

    let state = harness.wait_for_app("ConnectionState is disconnected", DEADLINE, |state| {
        state.connection_state == "disconnected"
    });
    assert_eq!(
        state.phone.map(|phone| phone.address),
        Some(harness.phone().address.clone()),
        "the app still knows which Phone it is waiting for"
    );

    // PipeWire registers a fresh AudioGateway, at the next index, on reconnect.
    harness.simulated_phone().connect();

    harness.wait_for_app("ConnectionState is connected again", DEADLINE, |state| {
        state.connection_state == "connected"
    });
    assert_eq!(
        harness.simulated_phone().audio_gateway(),
        format!(
            "/org/pipewire/Telephony/ag{}",
            harness.audio_gateway_index() + 1
        )
    );
}

#[test]
fn status_json_matches_the_control_interface() {
    let harness = Harness::start();
    harness.simulated_phone().connect();
    let state = harness.wait_for_app("ConnectionState is connected", DEADLINE, |state| {
        state.connection_state == "connected"
    });

    let printed = harness.cli(&["status", "--json"]);

    assert!(
        printed.status.success(),
        "status --json failed: {printed:?}"
    );
    let json: serde_json::Value =
        serde_json::from_slice(&printed.stdout).expect("status --json prints JSON");
    assert_eq!(json, expected_json(&state));
}

#[test]
fn status_json_before_the_phone_connects() {
    let harness = Harness::start();
    let state = harness.wait_for_app("the Phone is known", DEADLINE, |state| {
        state.phone.is_some()
    });

    let printed = harness.cli(&["status", "--json"]);

    assert!(
        printed.status.success(),
        "status --json failed: {printed:?}"
    );
    let json: serde_json::Value =
        serde_json::from_slice(&printed.stdout).expect("status --json prints JSON");
    assert_eq!(json, expected_json(&state));
    assert_eq!(json["connection_state"], "connecting");
}

/// The JSON `status --json` prints for a state, written out by hand.
fn expected_json(state: &AppState) -> serde_json::Value {
    let phone = state.phone.as_ref().map_or(
        serde_json::Value::Null,
        |phone| serde_json::json!({ "name": phone.name, "address": phone.address }),
    );
    serde_json::json!({
        "connection_state": state.connection_state,
        "phone": phone,
    })
}
