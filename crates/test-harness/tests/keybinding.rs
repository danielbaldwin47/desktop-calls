//! Scenarios: the Desktop Calls keybinding (`desktop-calls` with no
//! subcommand) focuses the Incoming Call Banner while a Call rings and the
//! Call Window while one is live, and the Call is then handled by keyboard
//! alone. Driven by the CLI and synthetic keys, observed through the control
//! interface and the test client that had focus before.

use std::time::Duration;

use test_harness::{AppState, Harness, Key, Screenshot, TestClient};

/// How long the app may take to notice a change.
const DEADLINE: Duration = Duration::from_secs(5);
/// How long the Banner may take to show after the Phone rings.
const BANNER_DEADLINE: Duration = Duration::from_secs(1);
/// How long a surface may take to be drawn on screen. Each capture and its
/// scan take over a second on a busy CI runner, so this allows several.
const SCREEN_DEADLINE: Duration = Duration::from_secs(10);
/// How long the Call audio may take to reach the Desktop after Answer.
const TRANSPORT_DEADLINE: Duration = Duration::from_millis(1_500);
/// How long something the Phone must not be asked is watched for, after the
/// app has settled.
const HOLD: Duration = Duration::from_millis(300);

const NUMBER: &str = "+447700900123";

/// The Answer button's green.
const GREEN: [u8; 3] = [0x26, 0xa2, 0x69];

/// A connected Phone, and the test client open with keyboard focus, as the
/// user's own window would be.
fn connected_with_a_focused_window() -> (Harness, TestClient) {
    let harness = Harness::builder().headless_screen().omarchy_bar().start();
    harness.simulated_phone().connect();
    harness.wait_for_app("ConnectionState is connected", DEADLINE, |state| {
        state.connection_state == "connected"
    });
    let client = harness.start_test_client();
    client.wait_for("the test client has keyboard focus", DEADLINE, |line| {
        line == "keyboard-enter"
    });
    (harness, client)
}

fn shows_green(screenshot: &Screenshot) -> bool {
    screenshot
        .region_where(|pixel| {
            pixel
                .iter()
                .zip(GREEN)
                .all(|(channel, wanted)| channel.abs_diff(wanted) <= 16)
        })
        .is_some()
}

/// Rings the Phone and waits until the Banner is drawn. Returns the Call's
/// object path and id.
fn ring_until_banner(harness: &Harness) -> (String, u32) {
    let path = harness.simulated_phone().ring(NUMBER);
    let state = harness.wait_for_app("the Banner shows", BANNER_DEADLINE, |state| {
        state.ringing && state.surface("banner").is_some()
    });
    let _ = harness.screenshot_when("banner", SCREEN_DEADLINE, shows_green);
    (path, state.calls[0].id)
}

/// Runs the keybinding's command and checks it succeeded.
fn press_the_keybinding(harness: &Harness) {
    let output = harness.cli(&[]);
    assert!(
        output.status.success(),
        "desktop-calls failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn focused(state: &AppState, surface: &str) -> bool {
    state
        .surface(surface)
        .is_some_and(|content| content.get("focused").map(String::as_str) == Some("true"))
}

fn is_live(state: &AppState, call: u32) -> bool {
    state
        .calls
        .iter()
        .any(|known| known.id == call && known.state == "active")
}

#[test]
fn while_ringing_the_keybinding_focuses_the_banner_and_a_answers_on_the_desktop() {
    let (harness, _client) = connected_with_a_focused_window();
    let (path, call) = ring_until_banner(&harness);

    press_the_keybinding(&harness);
    harness.wait_for_app("the Banner has keyboard focus", DEADLINE, |state| {
        focused(state, "banner")
    });
    harness.keyboard().type_text("a");

    let state = harness.wait_for_app(
        "the Call is active with its audio on the Desktop",
        TRANSPORT_DEADLINE,
        |state| is_live(state, call) && state.audio_location == "desktop",
    );
    assert!(!state.ringing);
    harness
        .simulated_phone()
        .wait_for_recorded("the Call is answered", DEADLINE, |recorded| {
            recorded
                .iter()
                .any(|recorded| recorded.path == path && recorded.member == "Answer")
        });
}

#[test]
fn while_ringing_the_keybinding_focuses_the_banner_and_d_declines() {
    let (harness, _client) = connected_with_a_focused_window();
    let (path, _) = ring_until_banner(&harness);

    press_the_keybinding(&harness);
    harness.wait_for_app("the Banner has keyboard focus", DEADLINE, |state| {
        focused(state, "banner")
    });
    harness.keyboard().type_text("d");

    let state = harness.wait_for_app("the Call is declined", DEADLINE, |state| {
        state.calls.is_empty() && state.surface("banner").is_none()
    });
    assert!(!state.ringing);
    let calls =
        harness
            .simulated_phone()
            .wait_for_recorded("the Call is hung up", DEADLINE, |recorded| {
                recorded
                    .iter()
                    .any(|recorded| recorded.path == path && recorded.member == "Hangup")
            });
    assert!(
        !calls.iter().any(|recorded| recorded.member == "Answer"),
        "{calls:#?}"
    );
}

#[test]
fn escape_gives_focus_back_and_the_call_keeps_ringing() {
    let (harness, client) = connected_with_a_focused_window();
    ring_until_banner(&harness);
    press_the_keybinding(&harness);
    harness.wait_for_app("the Banner has keyboard focus", DEADLINE, |state| {
        focused(state, "banner")
    });
    client.wait_for("the test client lost keyboard focus", DEADLINE, |line| {
        line == "keyboard-leave"
    });
    let state = harness.app_state();
    assert_eq!(
        state
            .surface("banner")
            .and_then(|banner| banner.get("heading"))
            .map(String::as_str),
        Some("Press A to answer or D to decline"),
        "the focused Banner says which keys work"
    );
    let _ = harness.screenshot("banner-focused");

    harness.keyboard().press(Key::Escape);

    let state = harness.wait_for_app("the Banner lets go of the keyboard", DEADLINE, |state| {
        state.surface("banner").is_some() && !focused(state, "banner")
    });
    assert!(state.ringing);
    // Typing reaches the user's window again, and A answers nothing.
    harness.keyboard().type_text("a");
    client.wait_for("A reaches the test client", DEADLINE, |line| {
        line == "key 30 released"
    });
    let lines = client.lines();
    let left = lines
        .iter()
        .position(|line| line == "keyboard-leave")
        .unwrap_or_default();
    assert!(
        lines[left..].iter().any(|line| line == "keyboard-enter"),
        "the test client never got keyboard focus back: {lines:?}"
    );
    let state = harness.app_holds(
        "the Call keeps ringing",
        Duration::from_millis(500),
        |state| state.ringing && state.surface("banner").is_some(),
    );
    assert_eq!(state.calls.len(), 1, "{state:?}");
}

#[test]
fn during_a_live_call_the_keybinding_focuses_the_call_window_and_escape_leaves_the_call_live() {
    let (harness, _client) = connected_with_a_focused_window();
    let (_, call) = ring_until_banner(&harness);
    harness.answer(call);
    harness.wait_for_app("the Call Window shows the live Call", DEADLINE, |state| {
        is_live(state, call)
            && state
                .surface("call-window")
                .is_some_and(|window| window.contains_key("audio-location"))
    });

    press_the_keybinding(&harness);
    harness.wait_for_app("the Call Window has keyboard focus", DEADLINE, |state| {
        focused(state, "call-window")
    });
    harness.keyboard().press(Key::Escape);

    harness.wait_for_app(
        "the Call Window lets go of the keyboard",
        DEADLINE,
        |state| state.surface("call-window").is_some() && !focused(state, "call-window"),
    );
    harness.app_holds("the Call stays live", Duration::from_millis(500), |state| {
        is_live(state, call)
    });
    harness
        .simulated_phone()
        .recorded_holds("the Desktop never hangs up", HOLD, |recorded| {
            !recorded.iter().any(|recorded| recorded.member == "Hangup")
        });
}
