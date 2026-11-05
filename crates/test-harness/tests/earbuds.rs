//! Scenario 20: earbuds connected to the Phone beside the Desktop. The
//! Incoming Call Banner still appears; answering on the Desktop brings the
//! audio here, or says plainly that the Phone kept it; a Call answered on
//! the earbuds shows its audio on the Phone. Checked through the control
//! interface, the surfaces' `Surfaces` and what the Simulated Phone was
//! asked.

use std::collections::BTreeMap;
use std::time::Duration;

use test_harness::{AppState, Harness, RecordedMethodCall};

/// How long the app may take to notice a change on the Phone.
const DEADLINE: Duration = Duration::from_secs(5);
/// How long the Desktop waits for each attempt to bring the audio here,
/// twice, plus slack.
const GIVE_UP: Duration = Duration::from_secs(10);
/// How long something that must not happen is watched for.
const QUIET: Duration = Duration::from_secs(1);

const NUMBER: &str = "+447700900123";
const AUDIO_NOT_MOVED: &str = "Your phone kept the audio; to hear the Call here, tap the audio button on your phone's call screen.";

/// The app on a screen of its own, the Phone connected, earbuds connected
/// to the Phone, and a Call ringing; returns the Call's path on the Phone
/// and its id in the app.
fn ringing_with_earbuds() -> (Harness, String, u32) {
    let harness = Harness::builder().headless_screen().omarchy_bar().start();
    harness.simulated_phone().connect();
    harness.wait_for_app("ConnectionState is connected", DEADLINE, |state| {
        state.connection_state == "connected"
    });
    harness.simulated_phone().set_earbuds(true);
    let path = harness.simulated_phone().ring(NUMBER);
    let state = harness.wait_for_app("the Banner shows the Call", DEADLINE, |state| {
        state.ringing && state.surface("banner").is_some()
    });
    let call = state.calls[0].id;
    (harness, path, call)
}

fn call_window(state: &AppState) -> Option<&BTreeMap<String, String>> {
    state.surface("call-window")
}

fn shown(state: &AppState, key: &str) -> Option<String> {
    call_window(state).and_then(|window| window.get(key).cloned())
}

fn is_live(state: &AppState) -> bool {
    state
        .calls
        .first()
        .is_some_and(|call| call.state == "active")
}

fn hang_up_asked(calls: &[RecordedMethodCall]) -> bool {
    calls.iter().any(|call| call.member == "Hangup")
}

#[test]
fn with_earbuds_connected_the_banner_appears_and_answering_here_brings_the_audio_here() {
    let (harness, _, call) = ringing_with_earbuds();

    harness.answer(call);

    let state = harness.wait_for_app(
        "the audio is on the Desktop and the Call Window says so",
        DEADLINE,
        |state| {
            state.audio_location == "desktop"
                && shown(state, "audio-location").as_deref() == Some("Audio on this computer")
        },
    );
    assert!(is_live(&state), "{state:?}");
    assert_eq!(shown(&state, "notice"), None);
}

#[test]
fn a_phone_that_keeps_the_audio_on_the_earbuds_gets_a_plain_message_and_the_call_stays_live() {
    let (harness, _, call) = ringing_with_earbuds();
    harness.simulated_phone().enable_quirk("earbuds-keep-audio");

    harness.answer(call);

    let state = harness.wait_for_app(
        "the Call Window says the Phone kept the audio",
        GIVE_UP,
        |state| shown(state, "notice").is_some(),
    );
    assert_eq!(shown(&state, "notice").as_deref(), Some(AUDIO_NOT_MOVED));
    assert_eq!(state.audio_location, "phone");
    assert_eq!(
        shown(&state, "audio-location").as_deref(),
        Some("Audio on phone")
    );
    assert_eq!(shown(&state, "handoff").as_deref(), Some("Move audio here"));
    assert_eq!(harness.wait_for_app_error(DEADLINE).kind, "audio-not-moved");
    harness.app_holds("the Call stays live", QUIET, is_live);
    harness.simulated_phone().recorded_holds(
        "the Phone is never asked to hang up",
        QUIET,
        |calls| !hang_up_asked(calls),
    );
}

#[test]
fn a_call_answered_on_the_earbuds_shows_its_audio_on_the_phone() {
    let (harness, path, _) = ringing_with_earbuds();

    harness.simulated_phone().answer_on_phone(&path);

    let state = harness.wait_for_app(
        "the Call Window shows the audio on the Phone",
        DEADLINE,
        |state| is_live(state) && shown(state, "handoff").is_some(),
    );
    assert_eq!(state.audio_location, "phone");
    assert_eq!(
        shown(&state, "audio-location").as_deref(),
        Some("Audio on phone")
    );
    assert_eq!(shown(&state, "handoff").as_deref(), Some("Move audio here"));
    assert_eq!(state.surface("banner"), None);
    harness.app_holds("the audio stays on the earbuds", QUIET, |state| {
        state.audio_location == "phone"
    });
}

#[test]
fn moving_the_audio_off_the_earbuds_says_plainly_how_when_the_phone_refuses() {
    let (harness, path, _) = ringing_with_earbuds();
    harness.simulated_phone().answer_on_phone(&path);
    harness.wait_for_app("the Call is live on the earbuds", DEADLINE, |state| {
        is_live(state) && shown(state, "handoff").as_deref() == Some("Move audio here")
    });

    harness
        .move_audio("desktop")
        .expect("MoveAudio(\"desktop\") is accepted");

    let state = harness.wait_for_app(
        "the Call Window says the Phone kept the audio",
        GIVE_UP,
        |state| shown(state, "notice").is_some(),
    );
    assert_eq!(shown(&state, "notice").as_deref(), Some(AUDIO_NOT_MOVED));
    assert!(is_live(&state), "{state:?}");
    assert_eq!(state.audio_location, "phone");
}
