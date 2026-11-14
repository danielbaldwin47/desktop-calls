//! Scenarios: the Desktop Calls keybinding (`desktop-calls` with no
//! subcommand) focuses the Incoming Call Banner while a Call rings and the
//! Call Window while one is live, and the Call is then handled by keyboard
//! alone. Driven by the CLI and synthetic keys, observed through the control
//! interface and the test client that had focus before.

use std::time::Duration;

use test_harness::{AppState, DESKTOP_CALLS_BINDING, Harness, Key, Screenshot, TestClient};

/// How long the app may take to notice a change.
const DEADLINE: Duration = Duration::from_secs(5);
/// How long the Banner may take to show after the Phone rings: from the
/// Call appearing to its first frame, by the app's log (scorecard item 1).
const BANNER_DEADLINE: Duration = Duration::from_secs(1);
/// How long a surface may take to be drawn on screen. Each capture and its
/// scan take over a second on a busy CI runner, so this allows several.
const SCREEN_DEADLINE: Duration = Duration::from_secs(10);
/// How long the Call audio may take to reach the Desktop after Answer
/// reaches the app, by its log (polish scorecard item 2).
const TRANSPORT_DEADLINE: Duration = Duration::from_millis(1_500);
/// How long the tests' own round trip may take: the request reaching the
/// app, and the change coming back on the control interface. Looser than
/// the scorecard targets because a loaded CI runner stretches it even when
/// the app is fast (ticket #83); the targets are asserted on the app's log.
const ROUND_TRIP: Duration = Duration::from_secs(3);
/// How long something the Phone must not be asked is watched for, after the
/// app has settled.
const HOLD: Duration = Duration::from_millis(300);

const NUMBER: &str = "+447700900123";

/// The Banner's footer on a Desktop with no Desktop Calls keybinding.
const NO_KEYBINDING: &str = "No keybinding set. Add one in Connect your phone…";

/// The Answer button's green.
const GREEN: [u8; 3] = [0x20, 0x86, 0x57];

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
    let state = harness.wait_for_app("the Banner shows", ROUND_TRIP, |state| {
        state.ringing && state.surface("banner").is_some()
    });
    harness.assert_banner_within(state.calls[0].id, BANNER_DEADLINE);
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
        ROUND_TRIP,
        |state| is_live(state, call) && state.audio_location == "desktop",
    );
    harness.assert_answered_audio_within(call, TRANSPORT_DEADLINE);
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

fn banner_footer(state: &AppState) -> Option<&str> {
    state
        .surface("banner")
        .and_then(|banner| banner.get("footer"))
        .map(String::as_str)
}

fn write_bindings(harness: &Harness, bindings: &str) {
    let file = harness.directory().join("config/hypr/bindings.lua");
    let written = file
        .parent()
        .map(std::fs::create_dir_all)
        .and_then(Result::ok)
        .and_then(|()| std::fs::write(&file, bindings).ok());
    assert!(written.is_some(), "write {}", file.display());
}

/// The Banner's footer names the keybinding actually set, and follows the
/// Hyprland config as it changes while a Call rings: from none, with no
/// `hypr` folder yet, to the Desktop Calls one, to one taken by another app.
#[test]
fn the_banner_names_the_keybinding_set_and_follows_the_config_as_it_changes() {
    let (harness, _client) = connected_with_a_focused_window();
    ring_until_banner(&harness);
    harness.wait_for_app("the footer says no keybinding is set", DEADLINE, |state| {
        banner_footer(state) == Some(NO_KEYBINDING)
    });

    write_bindings(&harness, DESKTOP_CALLS_BINDING);
    harness.wait_for_app("the footer names the keybinding", DEADLINE, |state| {
        banner_footer(state) == Some("Super Alt C then A or D")
    });

    write_bindings(
        &harness,
        &format!("{DESKTOP_CALLS_BINDING}o.bind(\"SUPER + ALT + C\", \"Calendar\", \"calendar\")\n"),
    );
    harness.wait_for_app(
        "the footer says no keybinding is set once another app takes the keys",
        DEADLINE,
        |state| banner_footer(state) == Some(NO_KEYBINDING),
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
    let state = harness.wait_for_app(
        "the focused Banner shows its keys and how to give the keyboard back",
        DEADLINE,
        |state| {
            state.surface("banner").is_some_and(|banner| {
                banner.get("keycaps").map(String::as_str) == Some("A, D")
                    && banner.get("footer").map(String::as_str)
                        == Some("Esc gives the keyboard back; the Call keeps ringing")
            })
        },
    );
    assert_eq!(
        state
            .surface("banner")
            .and_then(|banner| banner.get("heading"))
            .map(String::as_str),
        Some("Incoming call"),
        "the eyebrow stays put; the keys are on the buttons"
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
fn during_a_live_call_the_keybinding_focuses_the_call_window_and_escape_collapses_it_leaving_the_call_live()
{
    let (harness, _client) = connected_with_a_focused_window();
    let (_, call) = ring_until_banner(&harness);
    harness.answer(call);
    harness.wait_for_app("the Call Window shows the live Call", DEADLINE, |state| {
        is_live(state, call)
            && state
                .surface("call-window")
                .is_some_and(|window| window.contains_key("handoff"))
    });

    press_the_keybinding(&harness);
    harness.wait_for_app("the Call Window has keyboard focus", DEADLINE, |state| {
        focused(state, "call-window")
    });
    harness.keyboard().press(Key::Escape);

    harness.wait_for_app(
        "the Call Window collapses to the Call Pill, letting go of the keyboard",
        DEADLINE,
        |state| state.surface("call-pill").is_some() && state.surface("call-window").is_none(),
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
