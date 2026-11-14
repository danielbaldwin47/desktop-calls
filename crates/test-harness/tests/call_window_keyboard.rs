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
/// A second caller, who rings during the Call.
const OTHER_NUMBER: &str = "+447700900456";
const AUDIO_GATEWAY: &str = "org.pipewire.Telephony.AudioGateway1";

/// The Hang up button's red, and stock Adwaita's warning colour, which
/// marks the microphone muted.
const RED: [u8; 3] = [0xc0, 0x1c, 0x28];
const WARNING: [u8; 3] = [0xe5, 0xa5, 0x0a];

/// The space between the Call Window's rows, and the tiles' height.
const ROW_GAP: f64 = 12.0;
const TILE_HEIGHT: f64 = 58.0;

/// What the state line says while the audio is on the Phone.
const ON_PHONE: &str = "On your phone. Mute there, or move the audio here";

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

/// Where any amber pixel is, such as the warning colour's text, in the
/// Call Window's rows above the tiles and inside its border.
fn amber_above_tiles(screenshot: &Screenshot, hang_up: Region) -> Option<Region> {
    let area = Region {
        left: hang_up.left,
        top: 0.0,
        right: hang_up.right,
        bottom: hang_up.top - ROW_GAP - TILE_HEIGHT - 2.0,
    };
    screenshot.region_within(area, |[red, green, blue]| {
        red >= 0x90 && red.saturating_sub(blue) >= 0x60 && green > blue && green < red
    })
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

/// The Mute tile: the first of the three just above Hang up.
fn mute_button_centre(hang_up: Region) -> (f64, f64) {
    (
        hang_up.left + (hang_up.right - hang_up.left) / 6.0,
        hang_up.top - ROW_GAP - TILE_HEIGHT / 2.0,
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
fn clicking_mute_mutes_the_microphone_recolours_the_window_and_moves_nothing() {
    let harness = connected();
    harness.simulated_phone().set_microphone_volume(9);
    live_on_desktop(&harness);
    let hang_up = hang_up_button(&harness);
    let before_shot = harness.screenshot("call-window-unmuted-first");
    let before = app_region(&before_shot).expect("the Call Window shows");
    let (x, y) = mute_button_centre(hang_up);

    harness.pointer().click(x, y);

    // The Call Window reports what it shows on its own, after the state.
    let state = harness.wait_for_app(
        "Muted is true and the Call Window shows it",
        DEADLINE,
        |state| state.muted && shown(state, "mute").as_deref() == Some("Unmute"),
    );
    assert_eq!(
        shown(&state, "state").as_deref(),
        Some("Muted. They can't hear you")
    );
    // The warning colour on the Mute tile and on the window's border.
    let screenshot = harness.screenshot_when("call-window-muted", SCREEN_DEADLINE, |screenshot| {
        find(screenshot, WARNING).is_some()
    });
    let marked = find(&screenshot, WARNING).expect("the warning colour is on screen");
    // Hang up sits in the window's 16 px padding; the border is beyond it.
    assert!(
        marked.left < hang_up.left - 12.0
            && marked.right > hang_up.right + 12.0
            && marked.bottom > hang_up.bottom,
        "the Call Window's border isn't in the warning colour: {marked:?}; see {}",
        screenshot.path.display()
    );
    // The Mute tile is filled with it, just inside its corner.
    let tile_top = hang_up.top - ROW_GAP - TILE_HEIGHT;
    assert!(
        close_to(screenshot.colour_at(hang_up.left + 8.0, tile_top + 8.0), WARNING),
        "the Mute tile isn't in the warning colour; see {}",
        screenshot.path.display()
    );
    // The state line, above the tiles and inside the border, turns amber;
    // unmuted, nothing there is.
    assert!(
        amber_above_tiles(&screenshot, hang_up).is_some(),
        "the state line isn't in the warning colour; see {}",
        screenshot.path.display()
    );
    assert_eq!(amber_above_tiles(&before_shot, hang_up), None, "{}", before_shot.path.display());
    // Nothing moved: the window is the size it was, and Hang up is where it
    // was.
    assert_eq!(app_region(&screenshot), Some(before), "{}", screenshot.path.display());
    assert_eq!(find(&screenshot, RED), Some(hang_up), "{}", screenshot.path.display());
    harness.simulated_phone().wait_for_recorded(
        "MicrophoneVolume is set to 0",
        DEADLINE,
        |recorded| microphone_volumes_set(recorded) == [set_to(0)],
    );

    harness.pointer().click(x, y);

    harness.wait_for_app("Muted is false", DEADLINE, |state| {
        !state.muted && shown(state, "state").as_deref() == Some("On this computer")
    });
    let unmuted = harness.screenshot("call-window-unmuted");
    assert_eq!(find(&unmuted, WARNING), None, "{}", unmuted.path.display());
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

    assert_eq!(shown(&state, "handoff-shortcut").as_deref(), Some("A"));
    harness.keyboard().type_text("a");
    harness.wait_for_app("A moves the audio to the Phone", DEADLINE, |state| {
        state.audio_location == "phone" && shown(state, "handoff").as_deref() == Some("Move here")
    });
    harness.keyboard().type_text("a");
    harness.wait_for_app("A brings the audio back here", DEADLINE, |state| {
        state.audio_location == "desktop" && shown(state, "handoff").as_deref() == Some("To phone")
    });
    // And by a click on the tile, the last of the three.
    let hang_up = hang_up_button(&harness);
    let tile = (
        hang_up.right - (hang_up.right - hang_up.left) / 6.0,
        hang_up.top - ROW_GAP - TILE_HEIGHT / 2.0,
    );
    harness.pointer().click(tile.0, tile.1);
    harness.wait_for_app("clicking To phone moves the audio", DEADLINE, |state| {
        state.audio_location == "phone" && shown(state, "handoff").as_deref() == Some("Move here")
    });
    harness.pointer().click(tile.0, tile.1);
    harness.wait_for_app("clicking Move here brings it back", DEADLINE, |state| {
        state.audio_location == "desktop" && shown(state, "handoff").as_deref() == Some("To phone")
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
    // The tone entry is there only while the keypad is open; tones pasted
    // or typed with it closed open it.
    let state = harness.app_state();
    assert_eq!(shown(&state, "tone-entry"), None);
    assert_eq!(shown(&state, "keypad"), None);

    let pasted = harness.set_clipboard("1*#");
    harness.keyboard().press_with_control('v');
    harness.wait_for_app("tones pasted open the keypad", DEADLINE, |state| {
        shown(state, "tone-entry").is_some() && shown(state, "tones").as_deref() == Some("1*#")
    });
    harness.keyboard().type_text("k");
    harness.wait_for_app("K closes the keypad", DEADLINE, |state| {
        shown(state, "tone-entry").is_none()
    });
    harness.keyboard().type_text("1234#");

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
        shown(state, "tone-entry").is_some() && shown(state, "tones").as_deref() == Some("1*#1234#")
    });
    // Closed, the keypad keeps the tones sent for when it opens again.
    harness.keyboard().type_text("k");
    harness.wait_for_app("K closes the keypad", DEADLINE, |state| {
        shown(state, "tone-entry").is_none() && shown(state, "tones").is_none()
    });
    harness.keyboard().type_text("k");
    harness.wait_for_app("K opens it with the tones sent", DEADLINE, |state| {
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
    let hang_up = hang_up_button(&harness);

    harness.keyboard().type_text("k");

    harness.wait_for_app("K shows the keypad", DEADLINE, |state| {
        shown(state, "keypad").as_deref() == Some("shown")
    });
    // The screenshot waits for the Call Window to finish growing (#53).
    let after = app_region(&harness.screenshot("keypad-shown")).expect("the Call Window shows");
    assert!(after.top < before.top, "the keypad isn't drawn");
    // Anchored at the bottom, the window grew upwards. The keypad sits
    // just above the tiles, which sit just above Hang up: 3 buttons across,
    // 4 down, * 0 # last.
    let bottom = hang_up.top - ROW_GAP - TILE_HEIGHT - ROW_GAP;
    harness
        .pointer()
        .click(f64::midpoint(hang_up.left, hang_up.right), bottom - 10.0);

    let recorded =
        harness
            .simulated_phone()
            .wait_for_recorded("a tone is sent", DEADLINE, |recorded| {
                !tones_sent(recorded).is_empty()
            });
    assert_eq!(tones_sent(&recorded), ["0"]);
}

#[test]
fn with_the_audio_on_the_phone_mute_is_disabled_and_the_state_line_says_why() {
    let harness = connected();
    let path = harness.simulated_phone().ring(NUMBER);
    harness.wait_for_app("the Call rings", DEADLINE, |state| state.ringing);
    harness.simulated_phone().answer_on_phone(&path);
    let state = harness.wait_for_app(
        "the Call Window says the audio is on the Phone",
        DEADLINE,
        |state| shown(state, "state").as_deref() == Some(ON_PHONE),
    );
    assert_eq!(shown(&state, "handoff").as_deref(), Some("Move here"));
    // Mute shows, disabled, so its shortcut isn't offered.
    assert_eq!(shown(&state, "mute").as_deref(), Some("Mute"));
    assert_eq!(shown(&state, "mute-shortcut"), None);
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
    assert_eq!(find(&screenshot, WARNING), None);
}

/// A Call left alone on hold, as the other caller hung up, offers Resume
/// in place of the tiles, and S resumes it.
#[test]
fn a_lone_call_on_hold_offers_resume_and_s_resumes_it() {
    let harness = connected();
    live_on_desktop(&harness);
    let other = harness.simulated_phone().ring(OTHER_NUMBER);
    harness.wait_for_app("the second Call waits", DEADLINE, |state| {
        state.calls.len() == 2 && state.calls[1].state == "waiting"
    });
    harness.hold_and_answer().expect("Hold & Answer is passed on");
    harness.wait_for_app("the first Call is on hold", DEADLINE, |state| {
        state.calls.len() == 2 && state.calls[0].state == "held"
    });
    harness.simulated_phone().caller_hangs_up(&other);
    let state = harness.wait_for_app("the first Call is alone on hold, offered Resume", DEADLINE, |state| {
        state.calls.len() == 1
            && state.calls[0].state == "held"
            && shown(state, "swap").as_deref() == Some("Resume")
    });
    assert_eq!(shown(&state, "swap-shortcut").as_deref(), Some("S"));
    assert_eq!(shown(&state, "mute"), None, "the tiles give way to Resume");
    focus_call_window(&harness);

    harness.keyboard().type_text("s");

    harness.wait_for_app("S resumes the Call", DEADLINE, |state| {
        state.calls.len() == 1 && state.calls[0].state == "active"
    });
}
