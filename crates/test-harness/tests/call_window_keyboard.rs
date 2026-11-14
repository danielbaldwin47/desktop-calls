//! Scenario 13 on screen: Mute and the keypad in the Call Window, by click,
//! by the shortcuts the window shows, by typing and by pasting, checked on
//! screenshots, through `Surfaces` and against the Simulated Phone.
//!
//! The deadlines allow for CI's slower runners: see
//! docs/research/2026-11-04-ci-banner-latency.md.

use std::collections::BTreeMap;
use std::time::Duration;

use test_harness::{AppState, BAR_COLOUR, Harness, Key, RecordedMethodCall, Region, Screenshot};

/// How long the app may take to notice a change on the Phone, or a key.
const DEADLINE: Duration = Duration::from_secs(5);
/// How long the screen may take to show a change. One capture takes up to a
/// second on a slow runner.
const SCREEN_DEADLINE: Duration = Duration::from_secs(10);
/// How long something the Phone must not be asked is watched for, after the
/// app has settled.
const HOLD: Duration = Duration::from_millis(300);

const NUMBER: &str = "+447700900123";
const AUDIO_GATEWAY: &str = "org.pipewire.Telephony.AudioGateway1";

/// The Hang up button's red, and the orange that marks the microphone muted.
const RED: [u8; 3] = [0xc0, 0x1c, 0x28];
const ORANGE: [u8; 3] = [0xe6, 0x61, 0x00];

/// The space between the Call Window's rows, and round its edge.
const ROW_GAP: f64 = 8.0;
const PADDING: f64 = 16.0;

fn connected() -> Harness {
    let harness = Harness::builder().headless_screen().omarchy_bar().start();
    harness.simulated_phone().connect();
    harness.wait_for_app("ConnectionState is connected", DEADLINE, |state| {
        state.connection_state == "connected"
    });
    harness
}

fn call_window(state: &AppState) -> Option<&BTreeMap<String, String>> {
    state.surface("call-window")
}

fn shown(state: &AppState, key: &str) -> Option<String> {
    call_window(state).and_then(|window| window.get(key).cloned())
}

/// Answers on the Desktop and waits until the Call Window offers Mute.
fn live_on_desktop(harness: &Harness) -> String {
    let path = harness.simulated_phone().ring(NUMBER);
    let state = harness.wait_for_app("the Call rings", DEADLINE, |state| {
        state.ringing && state.calls.len() == 1
    });
    harness.answer(state.calls[0].id);
    harness.wait_for_app("the Call Window offers Mute", DEADLINE, |state| {
        state.audio_location == "desktop" && shown(state, "mute").as_deref() == Some("Mute")
    });
    path
}

fn close_to(colour: [u8; 3], target: [u8; 3]) -> bool {
    colour
        .iter()
        .zip(target)
        .all(|(channel, wanted)| channel.abs_diff(wanted) <= 16)
}

fn find(screenshot: &Screenshot, colour: [u8; 3]) -> Option<Region> {
    screenshot.region_where(|pixel| close_to(pixel, colour))
}

/// Where the app's surfaces are: everything that is neither the black
/// background nor the bar stand-in.
fn app_region(screenshot: &Screenshot) -> Option<Region> {
    screenshot.region_where(|pixel| pixel != [0, 0, 0] && pixel != BAR_COLOUR)
}

/// Where the Hang up button is, once the screen shows it.
fn hang_up_button(harness: &Harness) -> Region {
    let screenshot = harness.screenshot_when("call-window", SCREEN_DEADLINE, |screenshot| {
        find(screenshot, RED).is_some()
    });
    find(&screenshot, RED).unwrap_or_else(|| panic!("Hang up left the screen"))
}

/// The Mute button: the left half of the row just above Hang up.
fn mute_button_centre(hang_up: Region) -> (f64, f64) {
    let height = hang_up.bottom - hang_up.top;
    (
        hang_up.left + (hang_up.right - hang_up.left) / 4.0,
        hang_up.top - ROW_GAP - height / 2.0,
    )
}

/// Clicks the Call Window's padding, beside Hang up, which gives the window
/// the keyboard without pressing anything.
fn focus_call_window(harness: &Harness) {
    let hang_up = hang_up_button(harness);
    harness.pointer().click(
        hang_up.left - 8.0,
        f64::midpoint(hang_up.top, hang_up.bottom),
    );
}

fn microphone_volumes_set(recorded: &[RecordedMethodCall]) -> Vec<String> {
    recorded
        .iter()
        .filter(|call| {
            call.member == "Set"
                && call.arguments.contains(AUDIO_GATEWAY)
                && call.arguments.contains("MicrophoneVolume")
        })
        .map(|call| call.arguments.clone())
        .collect()
}

fn set_to(volume: u8) -> String {
    format!("(\"{AUDIO_GATEWAY}\", \"MicrophoneVolume\", <byte 0x{volume:02x}>)")
}

fn tones_sent(recorded: &[RecordedMethodCall]) -> Vec<String> {
    recorded
        .iter()
        .filter(|call| call.member == "SendTones")
        .map(|call| {
            call.arguments
                .trim_start_matches("(\"")
                .trim_end_matches("\",)")
                .to_owned()
        })
        .collect()
}

#[test]
fn clicking_mute_mutes_the_microphone_and_the_window_shows_it_unmistakably() {
    let harness = connected();
    harness.simulated_phone().set_microphone_volume(9);
    live_on_desktop(&harness);
    let hang_up = hang_up_button(&harness);
    let (x, y) = mute_button_centre(hang_up);

    harness.pointer().click(x, y);

    // The Call Window reports what it shows on its own, after the state.
    let state = harness.wait_for_app(
        "Muted is true and the Call Window shows it",
        DEADLINE,
        |state| state.muted && shown(state, "muted").is_some(),
    );
    assert_eq!(shown(&state, "muted").as_deref(), Some("Microphone muted"));
    assert_eq!(shown(&state, "mute").as_deref(), Some("Unmute"));
    // Orange on the Mute button, and on a strip below Hang up as wide as
    // the window.
    let screenshot = harness.screenshot("call-window-muted");
    let marked = find(&screenshot, ORANGE).expect("orange is on screen");
    assert!(
        marked.top < y && marked.bottom > hang_up.bottom,
        "orange marks neither Mute nor a strip below Hang up: {marked:?}"
    );
    assert!(
        marked.right - marked.left >= hang_up.right - hang_up.left - 2.0,
        "the muted mark is too small to miss: {marked:?}"
    );
    harness.simulated_phone().wait_for_recorded(
        "MicrophoneVolume is set to 0",
        DEADLINE,
        |recorded| microphone_volumes_set(recorded) == [set_to(0)],
    );

    harness.pointer().click(x, y);

    harness.wait_for_app("Muted is false", DEADLINE, |state| {
        !state.muted && shown(state, "muted").is_none()
    });
    let unmuted = harness.screenshot("call-window-unmuted");
    assert_eq!(find(&unmuted, ORANGE), None, "{}", unmuted.path.display());
    harness.simulated_phone().wait_for_recorded(
        "MicrophoneVolume is set back to 9",
        DEADLINE,
        |recorded| microphone_volumes_set(recorded) == [set_to(0), set_to(9)],
    );
}

#[test]
fn every_action_in_the_call_window_is_reachable_by_the_shortcut_it_shows() {
    let harness = connected();
    live_on_desktop(&harness);
    let state = harness.app_state();
    assert_eq!(shown(&state, "mute-shortcut").as_deref(), Some("M"));
    assert_eq!(shown(&state, "keypad-shortcut").as_deref(), Some("K"));
    assert_eq!(shown(&state, "hang-up-shortcut").as_deref(), Some("Ctrl+H"));
    focus_call_window(&harness);

    harness.keyboard().type_text("m");
    harness.wait_for_app("M mutes", DEADLINE, |state| state.muted);
    harness.keyboard().type_text("m");
    harness.wait_for_app("M unmutes", DEADLINE, |state| !state.muted);

    harness.keyboard().type_text("k");
    harness.wait_for_app("K shows the keypad", DEADLINE, |state| {
        shown(state, "keypad").as_deref() == Some("shown")
    });
    harness.keyboard().type_text("k");
    harness.wait_for_app("K hides the keypad", DEADLINE, |state| {
        shown(state, "keypad").is_none()
    });

    harness.keyboard().press_with_control('h');
    harness.wait_for_app("Ctrl+H hangs up", DEADLINE, |state| state.calls.is_empty());
    harness
        .simulated_phone()
        .wait_for_recorded("the Call is hung up", DEADLINE, |recorded| {
            recorded.iter().any(|recorded| recorded.member == "Hangup")
        });
}

#[test]
fn escape_collapses_the_call_window_and_leaves_the_call_live() {
    let harness = connected();
    live_on_desktop(&harness);
    focus_call_window(&harness);

    harness.keyboard().press(Key::Escape);

    harness.wait_for_app("Escape collapses the Call Window", DEADLINE, |state| {
        state.surface("call-pill").is_some()
    });
    harness.app_holds(
        "the Call stays live on the Call Pill",
        Duration::from_secs(2),
        |state| {
            state.calls.iter().any(|call| call.state == "active")
                && state.surface("call-pill").is_some()
        },
    );
    harness
        .simulated_phone()
        .recorded_holds("the Desktop never hangs up", HOLD, |recorded| {
            !recorded.iter().any(|recorded| recorded.member == "Hangup")
        });
}

#[test]
fn typed_and_pasted_tones_reach_the_phone_as_typed() {
    let harness = connected();
    live_on_desktop(&harness);
    focus_call_window(&harness);

    harness.keyboard().type_text("1*#");
    let pasted = harness.set_clipboard("1234#");
    harness.keyboard().press_with_control('v');

    // One SendTones a tone: a Pixel takes one tone per AT+VTS
    // (docs/research/2026-11-04-mute-and-keypad.md).
    let recorded =
        harness
            .simulated_phone()
            .wait_for_recorded("eight tones are sent", DEADLINE, |recorded| {
                tones_sent(recorded).len() >= 8
            });
    drop(pasted);
    assert_eq!(
        tones_sent(&recorded),
        ["1", "*", "#", "1", "2", "3", "4", "#"]
    );
    harness.wait_for_app("the keypad shows what was sent", DEADLINE, |state| {
        shown(state, "tones").as_deref() == Some("1*#1234#")
    });
    assert!(
        harness.app_errors().is_empty(),
        "{:?}",
        harness.app_errors()
    );
}

#[test]
fn clicking_a_keypad_button_sends_its_tone() {
    let harness = connected();
    live_on_desktop(&harness);
    focus_call_window(&harness);
    let before = app_region(&harness.screenshot("keypad-hidden")).expect("the Call Window shows");

    harness.keyboard().type_text("k");

    harness.wait_for_app("K shows the keypad", DEADLINE, |state| {
        shown(state, "keypad").as_deref() == Some("shown")
    });
    // The screenshot waits for the Call Window to finish growing (#53).
    let after = app_region(&harness.screenshot("keypad-shown")).expect("the Call Window shows");
    assert!(after.bottom > before.bottom, "the keypad isn't drawn");
    // The keypad is the new last row: 3 buttons across, 4 down, 1 2 3 first.
    let top = before.bottom - PADDING + ROW_GAP;
    let (left, right, bottom) = (
        after.left + PADDING,
        after.right - PADDING,
        after.bottom - PADDING,
    );
    harness
        .pointer()
        .click(left + (right - left) / 6.0, top + (bottom - top) / 8.0);

    let recorded =
        harness
            .simulated_phone()
            .wait_for_recorded("a tone is sent", DEADLINE, |recorded| {
                !tones_sent(recorded).is_empty()
            });
    assert_eq!(tones_sent(&recorded), ["1"]);
}

#[test]
fn with_the_audio_on_the_phone_mute_is_disabled_and_says_why() {
    let harness = connected();
    let path = harness.simulated_phone().ring(NUMBER);
    harness.wait_for_app("the Call rings", DEADLINE, |state| state.ringing);
    harness.simulated_phone().answer_on_phone(&path);
    let state = harness.wait_for_app(
        "the Call Window says Mute is unavailable",
        DEADLINE,
        |state| shown(state, "mute-unavailable").is_some(),
    );
    assert_eq!(
        shown(&state, "mute-unavailable").as_deref(),
        Some(
            "Mute works only while the Call's audio is on this computer; mute on your phone instead."
        )
    );
    focus_call_window(&harness);

    harness.keyboard().type_text("m");

    harness.app_holds("Muted stays false", Duration::from_secs(1), |state| {
        !state.muted
    });
    harness
        .simulated_phone()
        .recorded_holds("no MicrophoneVolume is set", HOLD, |recorded| {
            microphone_volumes_set(recorded).is_empty()
        });
    let screenshot = harness.screenshot("call-window-mute-unavailable");
    assert_eq!(find(&screenshot, ORANGE), None);
}
