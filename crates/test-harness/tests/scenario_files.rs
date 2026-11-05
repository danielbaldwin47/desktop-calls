//! Scenarios: the Simulated Phone acting out a declarative scenario file, as
//! `just demo` uses it.

use std::path::Path;
use std::time::Duration;

use test_harness::Harness;

/// How long the app may take to notice a change on the Phone.
const DEADLINE: Duration = Duration::from_secs(5);

#[test]
fn the_demo_scenario_file_brings_the_phone_into_range() {
    let scenario =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../simulated-phone/scenarios/demo.scenario");

    let harness = Harness::builder().scenario(scenario).start();

    harness.wait_for_app("ConnectionState is connected", DEADLINE, |state| {
        state.connection_state == "connected"
    });
}

#[test]
fn scenario_steps_run_in_order() {
    let directory = tempfile::tempdir().expect("create a directory for the scenario");
    let scenario = directory.path().join("in-and-out.scenario");
    std::fs::write(
        &scenario,
        "# In range, out of range, and back.\nconnect\ndisconnect\n\nconnect\n",
    )
    .expect("write the scenario");

    let harness = Harness::builder().scenario(scenario).start();

    harness.wait_for_app("ConnectionState is connected", DEADLINE, |state| {
        state.connection_state == "connected"
    });
    // Each connect registers the next AudioGateway, so the second one is live.
    assert_eq!(
        harness.simulated_phone().audio_gateway(),
        format!(
            "/org/pipewire/Telephony/ag{}",
            harness.audio_gateway_index() + 1
        )
    );
}
