//! Scenarios: a dev run of the app (`cargo run -- service`, or a Real-Phone
//! tool's hook) and the installed `desktop-calls.service` never share
//! runtime state, and never run side by side (ticket #123, ADR-0006).
//!
//! The installed service runs under the harness's stand-in for the systemd
//! user manager, which starts it from the installed unit file as systemd
//! does. The user's real systemd is never touched.

use std::time::Duration;

use test_harness::{AppEvent, AppState, Harness};

/// How long the app may take to notice a change on the Phone.
const DEADLINE: Duration = Duration::from_secs(5);
/// How long a second app may take to give up.
const REFUSAL_DEADLINE: Duration = Duration::from_secs(10);

#[test]
fn a_second_app_beside_a_running_one_refuses_and_leaves_it_running() {
    let harness = Harness::builder().phone_connected().start();
    harness.wait_for_app("ConnectionState is connected", DEADLINE, |state| {
        state.connection_state == "connected"
    });
    let first = harness.app_process_id();

    let status = harness.run_second_app(REFUSAL_DEADLINE);

    assert!(
        status.is_some_and(|status| !status.success()),
        "the second app gave up with a failure, not {status:?}"
    );
    assert!(
        harness.app_logged(&AppEvent::kind("service-failed").with("error_kind", "already-running")),
        "the second app logged why it gave up"
    );
    assert_eq!(
        harness.app_process_id(),
        first,
        "the first app still owns the control interface"
    );
    harness.wait_for_app("the first app still follows the Phone", DEADLINE, |state| {
        state.connection_state == "connected"
    });
}

const NUMBER: &str = "+447700900123";

/// A Call answered on the Phone, live, as the app shows it. Returns when
/// it connected, by the app.
fn live_call(harness: &Harness) -> u64 {
    let path = harness.simulated_phone().ring(NUMBER);
    harness.wait_for_app("the Call rings", DEADLINE, |state| state.ringing);
    harness.simulated_phone().answer_on_phone(&path);
    let state = harness.wait_for_app("the Call is live", DEADLINE, is_live);
    state.calls[0].active_since_ms
}

fn is_live(state: &AppState) -> bool {
    state.calls.len() == 1 && state.calls[0].state == "active"
}

/// Whether the app logged picking up Calls from an earlier run since its
/// log was `from` bytes long.
fn picked_up_since(harness: &Harness, from: usize) -> bool {
    harness.app_log()[from..]
        .lines()
        .any(|line| line.contains(r#""event_kind":"earlier-run-loaded""#))
}

#[test]
fn a_dev_run_stopped_mid_call_leaves_nothing_for_the_installed_service() {
    let mut harness = Harness::builder().phone_connected().installed().start();
    harness.start_app_by_hand();
    live_call(&harness);
    // As on 2026-11-05 at 05:57: a signal stops the dev run mid-Call.
    harness.kill_app_by_hand();

    let before_service = harness.app_log().len();
    harness.start_service();
    harness.wait_for_app("the installed service shows the Call", DEADLINE, is_live);
    assert!(
        !picked_up_since(&harness, before_service),
        "the installed service picked up the dev run's Calls"
    );

    // The dev run's own record is still there for the next dev run.
    harness.stop_service();
    let before_dev_run = harness.app_log().len();
    harness.start_app_by_hand();
    assert!(
        picked_up_since(&harness, before_dev_run),
        "the next dev run picked up the Call the last one left"
    );
}

#[test]
fn the_installed_service_picks_up_its_own_calls_after_a_crash() {
    let harness = Harness::builder().phone_connected().installed().start();
    harness.start_service();
    let active_since_ms = live_call(&harness);
    let crashed = harness.service_process_id();

    harness.kill_service();

    harness.wait_for_service_restart(crashed);
    let state = harness.wait_for_app("the restarted service shows the Call", DEADLINE, is_live);
    assert_eq!(
        state.calls[0].active_since_ms, active_since_ms,
        "the restarted service timed the Call from its start"
    );
}
