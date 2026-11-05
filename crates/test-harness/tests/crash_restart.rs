//! Scenarios: the app starts while the Phone has a live Call, as after
//! logging in mid-Call or after a crash that systemd restarts it from.
//! The Call Window shows the Call with its Audio Location, and after a
//! crash with the direction, timer and Mute the Call had before it.

use std::thread;
use std::time::Duration;

use test_harness::{AppCall, AppState, Builder, Harness};

/// How long the app may take to notice a change on the Phone.
const DEADLINE: Duration = Duration::from_secs(5);
/// How long the Call audio may take to reach the Desktop after Answer.
const TRANSPORT_DEADLINE: Duration = Duration::from_millis(1_500);
/// How long the Call runs before the crash, so its timer shows it.
const TALKING: Duration = Duration::from_millis(1_200);

const NUMBER: &str = "+447700900123";
const TRANSPORT: &str = "org.pipewire.Telephony.AudioGatewayTransport1";

fn connected(builder: Builder) -> Harness {
    let harness = builder.start();
    harness.simulated_phone().connect();
    harness.wait_for_app("ConnectionState is connected", DEADLINE, |state| {
        state.connection_state == "connected"
    });
    harness
}

/// A Call answered on the Desktop, live with its audio here. Returns it as
/// the app lists it.
fn answered_here(harness: &Harness) -> AppCall {
    let _path = harness.simulated_phone().ring(NUMBER);
    let state = harness.wait_for_app("the Call rings", DEADLINE, |state| {
        state.ringing && state.calls.len() == 1
    });
    let call = state.calls[0].id;
    harness.answer(call);
    let state = harness.wait_for_app(
        "the Call is live with its audio on the Desktop",
        TRANSPORT_DEADLINE,
        |state| is_live(state) && state.audio_location == "desktop",
    );
    state.calls[0].clone()
}

fn is_live(state: &AppState) -> bool {
    state.calls.len() == 1 && state.calls[0].state == "active"
}

/// The Call Window's timer, in seconds, if it shows one.
fn timer_seconds(state: &AppState) -> Option<u64> {
    let status = state.surface("call-window")?.get("status")?;
    let (minutes, seconds) = status.split_once(':')?;
    Some(minutes.parse::<u64>().ok()? * 60 + seconds.parse::<u64>().ok()?)
}

#[test]
fn after_a_crash_mid_call_the_call_window_shows_the_same_call_audio_and_timer() {
    let mut harness = connected(Harness::builder().headless_screen());
    let before = answered_here(&harness);
    thread::sleep(TALKING);

    harness.restart_app();

    let state = harness.wait_for_app(
        "the Call Window shows the Call, timed from its start",
        DEADLINE,
        |state| {
            is_live(state)
                && state.audio_location == "desktop"
                && timer_seconds(state).is_some_and(|seconds| seconds >= 1)
        },
    );
    let after = &state.calls[0];
    assert_eq!(after.number, before.number);
    assert_eq!(after.direction, "incoming");
    assert_eq!(after.active_since_ms, before.active_since_ms);
    harness.hang_up(after.id);
    harness.wait_for_app("the Call ended", DEADLINE, |state| state.calls.is_empty());
}

#[test]
fn after_a_crash_a_call_whose_audio_was_on_the_phone_stays_there() {
    let mut harness = connected(Harness::builder());
    let path = harness.simulated_phone().ring(NUMBER);
    harness.wait_for_app("the Call rings", DEADLINE, |state| state.ringing);
    harness.simulated_phone().answer_on_phone(&path);
    let state = harness.wait_for_app("the Call is live on the Phone", DEADLINE, is_live);
    let before = state.calls[0].clone();

    harness.restart_app();

    let state = harness.wait_for_app("the Call is back, on the Phone", DEADLINE, |state| {
        is_live(state) && state.connection_state == "connected"
    });
    assert_eq!(state.audio_location, "phone");
    assert_eq!(state.calls[0].active_since_ms, before.active_since_ms);
    harness.simulated_phone().recorded_holds(
        "the Desktop never asked for the audio",
        Duration::from_millis(300),
        |recorded| {
            !recorded
                .iter()
                .any(|call| call.interface == TRANSPORT && call.member == "Activate")
        },
    );
}

#[test]
fn after_a_crash_while_muted_the_call_shows_muted_and_unmutes() {
    let mut harness = connected(Harness::builder());
    answered_here(&harness);
    harness.set_muted(true).expect("mute");
    harness.wait_for_app("muted", DEADLINE, |state| state.muted);

    harness.restart_app();

    harness.wait_for_app("the Call is back, muted", DEADLINE, |state| {
        is_live(state) && state.audio_location == "desktop" && state.muted
    });
    harness.set_muted(false).expect("unmute");
    harness.wait_for_app("unmuted", DEADLINE, |state| !state.muted);
}

#[test]
fn started_while_a_call_is_live_on_the_phone_the_app_shows_it() {
    let harness = Harness::builder().phone_connected().installed().start();
    let path = harness.simulated_phone().ring(NUMBER);
    harness.simulated_phone().answer_on_phone(&path);

    // The CLI starts the service, as at login.
    let status = harness.cli(&["status"]);
    assert!(status.status.success(), "{status:?}");

    let state = harness.wait_for_app("the Call shows", DEADLINE, |state| {
        is_live(state) && state.connection_state == "connected"
    });
    assert_eq!(state.audio_location, "phone");
    assert_eq!(state.calls[0].number, NUMBER);
    // The Phone doesn't say when a Call started, so a Call this Desktop
    // never saw is timed from when it first sees it.
    assert!(state.calls[0].active_since_ms > 0, "{state:?}");
}
