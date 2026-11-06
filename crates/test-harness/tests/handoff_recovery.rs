//! Scenarios for ticket #101: hands-free doesn't come back after a Handoff
//! to the Phone, because BlueZ fails `ConnectProfile` once, or every time.
//! The Call carries on on the Phone, so the app never shows it as ended.
//! The Call Window says the connection to the Phone is lost, the Desktop
//! keeps asking BlueZ for hands-free, and it picks the Call back up when
//! hands-free returns. Checked through the control interface, the Call
//! Window's `Surfaces`, the notifications stand-in, and what the Simulated
//! Phone was asked and still has.
//!
//! Ticket #170 adds the Phone listing its Calls late once hands-free is
//! back: the Call is settled by what the Phone lists, not by the grace that
//! started when it vanished (ADR-0007).

use std::cell::Cell;
use std::time::Duration;

use test_harness::{AppCall, AppState, Harness, RecordedMethodCall};

/// How long the app may take to notice a change on the Phone.
const DEADLINE: Duration = Duration::from_secs(5);
/// How long the Desktop may take to ask BlueZ for hands-free twice more
/// after the drop's own `ConnectProfile` failed: it waits 1 s, then 2 s,
/// and BlueZ takes 0.15 s to fail each.
const RETRIES_DEADLINE: Duration = Duration::from_secs(8);
/// How long the Call Window says the connection was lost before it closes,
/// with some slack.
const LOST_WINDOW_DEADLINE: Duration = Duration::from_secs(14);

const NUMBER: &str = "+447700900123";
const DEVICE: &str = "org.bluez.Device1";
const LOST: &str = "Connection to phone lost: the Call continues on your phone.";

/// The app on a screen of its own, so its surfaces show, with the Phone
/// connected.
fn connected() -> Harness {
    let harness = Harness::builder().headless_screen().start();
    harness.simulated_phone().connect();
    harness.wait_for_app("ConnectionState is connected", DEADLINE, |state| {
        state.connection_state == "connected"
    });
    harness
}

fn shown(state: &AppState, key: &str) -> Option<String> {
    state
        .surface("call-window")
        .and_then(|window| window.get(key).cloned())
}

/// Whether the Call Window says the Call is over.
fn shows_ended(state: &AppState) -> bool {
    shown(state, "status").as_deref() == Some("Call ended")
}

/// A Call placed on the Phone, live, with its audio brought to the Desktop.
fn live_here(harness: &Harness) -> AppCall {
    let phone = harness.simulated_phone();
    let _call = phone.dial_on_phone(NUMBER);
    phone.callee_answers();
    harness.wait_for_app("the Call is live", DEADLINE, |state| {
        state
            .calls
            .first()
            .is_some_and(|call| call.state == "active")
    });
    if let Err(error) = harness.move_audio("desktop") {
        panic!("MoveAudio(\"desktop\") was refused: {error:?}");
    }
    let state = harness.wait_for_app("the audio is on the Desktop", DEADLINE, |state| {
        state.audio_location == "desktop"
            && shown(state, "handoff").as_deref() == Some("Move audio to phone")
    });
    state.calls[0].clone()
}

/// The `ConnectProfile`s the Desktop asked BlueZ for.
fn connect_profiles(calls: &[RecordedMethodCall]) -> usize {
    calls
        .iter()
        .filter(|call| call.interface == DEVICE && call.member == "ConnectProfile")
        .count()
}

/// Checks the Desktop only ever dropped hands-free once and asked for it
/// back, and never ended the Call or the Phone's connection.
fn assert_only_asked_for_hands_free(calls: &[RecordedMethodCall]) {
    let device: Vec<&str> = calls
        .iter()
        .filter(|call| call.interface == DEVICE)
        .map(|call| call.member.as_str())
        .collect();
    assert_eq!(device.first(), Some(&"DisconnectProfile"), "{calls:#?}");
    assert!(
        device[1..].iter().all(|member| *member == "ConnectProfile"),
        "{calls:#?}"
    );
    assert!(
        !calls.iter().any(|call| matches!(
            call.member.as_str(),
            "Hangup" | "Disconnect" | "RemoveDevice"
        )),
        "{calls:#?}"
    );
}

#[test]
fn with_connect_profile_failing_once_the_call_is_picked_back_up_when_hands_free_returns() {
    let harness = connected();
    let before = live_here(&harness);
    harness
        .simulated_phone()
        .enable_quirk("connect-profile-fails-once");
    let ended = Cell::new(false);
    let watch = |state: &AppState| {
        ended.set(ended.get() || shows_ended(state));
    };

    harness
        .move_audio("phone")
        .expect("MoveAudio(\"phone\") is accepted");

    harness.wait_for_app(
        "the Call Window says the connection is lost",
        DEADLINE,
        |state| {
            watch(state);
            shown(state, "notice").as_deref() == Some(LOST)
        },
    );
    let state = harness.wait_for_app(
        "the Call is back, live, with its audio on the Phone",
        DEADLINE,
        |state| {
            watch(state);
            state.connection_state == "connected"
                && state.calls.len() == 1
                && shown(state, "notice").is_none()
                && shown(state, "audio-location").as_deref() == Some("Audio on phone")
                && shown(state, "handoff").as_deref() == Some("Move audio here")
        },
    );
    assert!(!ended.get(), "the Call Window said Call ended");
    let call = &state.calls[0];
    assert_eq!(call.state, "active");
    assert_eq!(call.number, before.number);
    assert_eq!(
        call.active_since_ms, before.active_since_ms,
        "the same timer"
    );
    assert_eq!(shown(&state, "call"), Some(call.id.to_string()));
    let calls = harness.simulated_phone().wait_for_recorded(
        "BlueZ was asked for hands-free once more",
        DEADLINE,
        |calls| connect_profiles(calls) >= 2,
    );
    assert_only_asked_for_hands_free(&calls);
    // The Desktop can move the audio here again.
    harness
        .move_audio("desktop")
        .expect("MoveAudio(\"desktop\") is accepted");
    harness.wait_for_app("the audio is back here", DEADLINE, |state| {
        state.audio_location == "desktop"
    });
    assert_eq!(harness.notifications().received(), []);
}

#[test]
fn with_connect_profile_failing_always_the_call_is_never_shown_as_ended() {
    let harness = connected();
    let before = live_here(&harness);
    harness
        .simulated_phone()
        .enable_quirk("connect-profile-fails");
    let ended = Cell::new(false);
    let watch = |state: &AppState| {
        ended.set(ended.get() || shows_ended(state));
    };

    harness
        .move_audio("phone")
        .expect("MoveAudio(\"phone\") is accepted");

    let state = harness.wait_for_app(
        "the Call Window says the connection is lost",
        DEADLINE,
        |state| {
            watch(state);
            shown(state, "notice").as_deref() == Some(LOST)
        },
    );
    assert_eq!(shown(&state, "call"), Some(before.id.to_string()));
    assert_eq!(shown(&state, "audio-location"), None);
    assert_eq!(state.connection_state, "disconnected");
    let refused = harness
        .move_audio("desktop")
        .expect_err("moving the audio is refused while the connection is lost");
    assert_eq!(refused.message, LOST, "{refused:?}");

    let calls = harness.simulated_phone().wait_for_recorded(
        "the Desktop keeps asking BlueZ for hands-free",
        RETRIES_DEADLINE,
        |calls| connect_profiles(calls) >= 3,
    );
    assert_only_asked_for_hands_free(&calls);
    assert_eq!(
        harness.simulated_phone().calls_on_phone().len(),
        1,
        "the Call carries on on the Phone"
    );

    // The Call Window says so for a while, then closes without "Call ended".
    harness.wait_for_app("the Call Window closes", LOST_WINDOW_DEADLINE, |state| {
        watch(state);
        if state.surface("call-window").is_some() {
            assert_eq!(shown(state, "notice").as_deref(), Some(LOST));
        }
        state.surface("call-window").is_none()
    });
    assert!(!ended.get(), "the Call Window said Call ended");
    let on_phone = harness.simulated_phone().calls_on_phone();
    assert_eq!(on_phone.len(), 1, "{on_phone:?}");
    assert_eq!(on_phone[0].1, "active");
    // Not a Missed Call, nor anything else to notify.
    assert_eq!(harness.notifications().received(), []);
}

#[test]
fn with_the_calls_listed_late_after_hands_free_returns_the_call_keeps_its_id_and_timer() {
    let harness = connected();
    let before = live_here(&harness);
    // Hands-free is back 0.6 s after the drop, long before the 2 s grace
    // for the vanished Call runs out; the Phone lists the Call 2 s later,
    // after it has.
    harness.simulated_phone().enable_quirk("calls-listed-late");
    let ended = Cell::new(false);
    let watch = |state: &AppState| {
        ended.set(ended.get() || shows_ended(state));
    };

    harness
        .move_audio("phone")
        .expect("MoveAudio(\"phone\") is accepted");

    let state = harness.wait_for_app(
        "the Call is back, live, with its audio on the Phone",
        DEADLINE,
        |state| {
            watch(state);
            state.connection_state == "connected"
                && state.calls.len() == 1
                && shown(state, "audio-location").as_deref() == Some("Audio on phone")
                && shown(state, "handoff").as_deref() == Some("Move audio here")
        },
    );
    assert!(!ended.get(), "the Call Window said Call ended");
    let call = &state.calls[0];
    assert_eq!(call.id, before.id, "the same Call: {state:?}");
    assert_eq!(call.state, "active");
    assert_eq!(
        call.active_since_ms, before.active_since_ms,
        "the same timer"
    );
    assert_eq!(shown(&state, "call"), Some(call.id.to_string()));
    assert_eq!(harness.notifications().received(), []);
}
