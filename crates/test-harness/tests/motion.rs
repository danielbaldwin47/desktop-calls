//! Ticket #234: the motion that tells the user something is under way. The
//! Banner's ringing dot breathes, and the Call Window's audio tile shows a
//! Handoff's progress; where the Desktop asks for reduced motion, neither
//! moves smoothly. Observed through the app's log.

use std::time::Duration;

use test_harness::{AppEvent, AppState, Harness};

/// How long the app may take to notice a change on the Phone.
const DEADLINE: Duration = Duration::from_secs(5);

const NUMBER: &str = "+447700900123";

fn connected(harness: Harness) -> Harness {
    harness.simulated_phone().connect();
    harness.wait_for_app("ConnectionState is connected", DEADLINE, |state| {
        state.connection_state == "connected"
    });
    harness
}

fn handoff(state: &AppState) -> Option<&String> {
    state
        .surface("call-window")
        .and_then(|window| window.get("handoff"))
}

/// Rings, waits for the Banner and then for the dot's event; answers on the
/// Phone and moves the audio here, and waits for the bar's event.
fn ring_then_move_the_audio_here(harness: &Harness, animated: bool) {
    let call = harness.simulated_phone().ring(NUMBER);
    harness.wait_for_app("the Banner shows", DEADLINE, |state| {
        state.surface("banner").is_some()
    });
    harness.wait_for_app_log("banner-ringing-dot", DEADLINE);
    assert!(
        harness.app_logged(&AppEvent::kind("banner-ringing-dot").with("breathing", animated)),
        "the ringing dot's breathing wasn't {animated}:\n{}",
        harness.app_log()
    );

    harness.simulated_phone().answer_on_phone(&call);
    harness.wait_for_app(
        "the Call Window offers to move the audio here",
        DEADLINE,
        |state| handoff(state).is_some_and(|handoff| handoff == "Move here"),
    );
    harness
        .move_audio("desktop")
        .unwrap_or_else(|error| panic!("MoveAudio(\"desktop\") was refused: {error:?}"));
    harness.wait_for_app_log("handoff-progress-started", DEADLINE);
    assert!(
        harness.app_logged(&AppEvent::kind("handoff-progress-started").with("animated", animated)),
        "the Handoff's progress wasn't animated={animated}:\n{}",
        harness.app_log()
    );
    harness.wait_for_app("the audio is here", DEADLINE, |state| {
        state.audio_location == "desktop"
            && handoff(state).is_some_and(|handoff| handoff == "To phone")
    });
}

#[test]
fn the_ringing_dot_breathes_and_the_audio_tile_shows_a_handoffs_progress() {
    let harness = connected(Harness::builder().headless_screen().full_motion().start());

    ring_then_move_the_audio_here(&harness, true);
}

#[test]
fn with_animations_off_the_dot_holds_still_and_the_progress_moves_in_steps() {
    let harness = connected(
        Harness::builder()
            .headless_screen()
            .reduced_motion()
            .start(),
    );

    ring_then_move_the_audio_here(&harness, false);
}
