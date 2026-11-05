//! Scenarios: a ringing Call is answered or declined on the Phone, or the
//! caller gives up, and the Incoming Call Banner and the Call Window keep up,
//! on a screen of their own, observed through the control interface's
//! `Surfaces` and on screenshots.

use std::time::Duration;

use test_harness::{AppState, Harness};

/// How long the app may take to notice a change on the Phone.
const DEADLINE: Duration = Duration::from_secs(5);
/// How long the Banner may take to show after the Phone rings.
const BANNER_DEADLINE: Duration = Duration::from_secs(1);
/// How long a surface may take to be drawn on screen. Each capture and its
/// scan take over a second on a busy CI runner, so this allows several.
const SCREEN_DEADLINE: Duration = Duration::from_secs(10);
/// How long the Banner may stay up after the Phone confirms the Call is gone.
const KEEP_UP_DEADLINE: Duration = Duration::from_millis(200);

const NUMBER: &str = "+447700900123";

/// The Answer button's green.
const GREEN: [u8; 3] = [0x26, 0xa2, 0x69];

fn connected_on_screen() -> Harness {
    let harness = Harness::builder().headless_screen().omarchy_bar().start();
    harness.simulated_phone().connect();
    harness.wait_for_app("ConnectionState is connected", DEADLINE, |state| {
        state.connection_state == "connected"
    });
    harness
}

/// Rings the Phone and waits until the Banner is on screen, drawn. Returns
/// the Call's object path.
///
/// The person holding the Phone reacts seconds after it rings, by which
/// time the Banner is drawn. Its first frame is the one slow moment: on a
/// CI-like machine, with software rendering and one CPU shared by four
/// scenarios, it held the app for about 0.6 s after the Banner mapped, and a
/// change on the Phone that arrived then waited behind it.
fn ring_until_banner(harness: &Harness) -> String {
    let path = harness.simulated_phone().ring(NUMBER);
    harness.wait_for_app("the Banner shows", BANNER_DEADLINE, |state| {
        state.ringing && state.surface("banner").is_some()
    });
    let _ = harness.screenshot_when("banner", SCREEN_DEADLINE, shows_green);
    path
}

fn shows_green(screenshot: &test_harness::Screenshot) -> bool {
    screenshot
        .region_where(|pixel| {
            pixel
                .iter()
                .zip(GREEN)
                .all(|(channel, wanted)| channel.abs_diff(wanted) <= 16)
        })
        .is_some()
}

fn banner_closed(state: &AppState) -> bool {
    !state.ringing && state.surface("banner").is_none()
}

#[test]
fn answered_on_the_phone_the_banner_closes_and_the_call_window_says_audio_on_phone() {
    let harness = connected_on_screen();
    let path = ring_until_banner(&harness);

    harness.simulated_phone().answer_on_phone(&path);

    let state = harness.wait_for_app(
        "the Call Window says the audio is on the Phone",
        DEADLINE,
        |state| {
            banner_closed(state)
                && state.surface("call-window").is_some_and(|window| {
                    window.get("audio-location").map(String::as_str) == Some("Audio on phone")
                })
        },
    );
    assert_eq!(state.audio_location, "phone");
    // The Banner's Answer button is gone from the screen too.
    let _ = harness.screenshot_when(
        "call-window-audio-on-phone",
        SCREEN_DEADLINE,
        |screenshot| !shows_green(screenshot),
    );
}

#[test]
fn declined_on_the_phone_the_banner_closes_at_once() {
    let harness = connected_on_screen();
    let path = ring_until_banner(&harness);

    harness.simulated_phone().decline_on_phone(&path);

    let state = harness.wait_for_app("the Banner closes", KEEP_UP_DEADLINE, banner_closed);
    assert!(state.calls.is_empty(), "{state:?}");
    // A Call that never connected has no Call Window to show "Call ended".
    assert_eq!(state.surface("call-window"), None);
}

#[test]
fn the_caller_hanging_up_while_ringing_closes_the_banner_at_once() {
    let harness = connected_on_screen();
    let path = ring_until_banner(&harness);

    harness.simulated_phone().caller_hangs_up(&path);

    let state = harness.wait_for_app("the Banner closes", KEEP_UP_DEADLINE, banner_closed);
    assert!(state.calls.is_empty(), "{state:?}");
    assert_eq!(state.surface("call-window"), None);
}
