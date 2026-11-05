//! Scenarios 11 and 12: a Call placed on the Phone keeps its audio there,
//! and a Handoff brings it to the Desktop, by the Call Window's button, the
//! control interface and the CLI; a Phone that ignores `Activate` once gets
//! one more try, and one that keeps ignoring it gets a plain message and
//! keeps the Call. Checked through the control interface, the Call Window's
//! `Surfaces`, screenshots, and what the Simulated Phone was asked.

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use test_harness::{AppState, Harness, RecordedMethodCall, Region, Screenshot};

/// How long the app may take to notice a change on the Phone.
const DEADLINE: Duration = Duration::from_secs(5);
/// How long the screen may take to show a change. One capture takes up to a
/// second on a slow runner.
const SCREEN_DEADLINE: Duration = Duration::from_secs(10);
/// How long the audio may take to reach the Desktop once asked (ticket #13).
const HANDOFF_DEADLINE: Duration = Duration::from_millis(1_500);
/// How long something the Phone must not be asked is watched for.
const QUIET: Duration = Duration::from_secs(1);
/// How long the Desktop waits for each attempt, twice, plus slack.
const GIVE_UP: Duration = Duration::from_secs(10);

const NUMBER: &str = "+447700900123";
const TRANSPORT: &str = "org.pipewire.Telephony.AudioGatewayTransport1";

/// The Hang up button's red.
const RED: [u8; 3] = [0xc0, 0x1c, 0x28];
/// The space between the Call Window's rows.
const ROW_GAP: f64 = 8.0;

/// The app on a screen of its own, so its surfaces show, with the Phone
/// connected.
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

/// The method calls among `calls` made on the Telephony service.
fn telephony(calls: &[RecordedMethodCall]) -> impl Iterator<Item = &RecordedMethodCall> {
    calls
        .iter()
        .filter(|call| call.bus == "session" && call.path.starts_with("/org/pipewire/Telephony"))
}

fn is_reject_sco(call: &RecordedMethodCall, value: bool) -> bool {
    call.member == "Set"
        && call.arguments.contains(TRANSPORT)
        && call.arguments.contains("RejectSCO")
        && call
            .arguments
            .contains(if value { "true" } else { "false" })
}

fn is_activate(call: &RecordedMethodCall) -> bool {
    call.member == "Activate"
}

/// What the Desktop asked of the audio, in order: `false` and `true` for
/// each `RejectSCO` write, `"Activate"` for each `Activate`.
fn audio_requests(calls: &[RecordedMethodCall]) -> Vec<&'static str> {
    telephony(calls)
        .filter_map(|call| {
            if is_reject_sco(call, false) {
                Some("RejectSCO=false")
            } else if is_reject_sco(call, true) {
                Some("RejectSCO=true")
            } else if is_activate(call) {
                Some("Activate")
            } else {
                None
            }
        })
        .collect()
}

/// What the Desktop asked of the audio from its first Handoff on: nothing
/// before it clears `RejectSCO`, which it never does for a Call placed on
/// the Phone until asked to move the audio.
fn handoff_requests(calls: &[RecordedMethodCall]) -> Vec<&'static str> {
    audio_requests(calls)
        .into_iter()
        .skip_while(|request| *request != "RejectSCO=false")
        .collect()
}

/// The person holding the Phone places a Call on it, and the app shows it
/// with the audio on the Phone.
fn placed_on_phone(harness: &Harness) -> AppState {
    let _call = harness.simulated_phone().dial_on_phone(NUMBER);
    harness.wait_for_app(
        "the Call Window shows the Call placed on the Phone",
        DEADLINE,
        |state| {
            state.calls.len() == 1
                && shown(state, "audio-location").as_deref() == Some("Audio on phone")
        },
    )
}

/// A Call placed on the Phone, connected, with its audio on the Phone.
fn live_on_phone(harness: &Harness) {
    placed_on_phone(harness);
    harness.simulated_phone().callee_answers();
    harness.wait_for_app("the Call is live on the Phone", DEADLINE, |state| {
        state
            .calls
            .first()
            .is_some_and(|call| call.state == "active")
            && state.audio_location == "phone"
            && shown(state, "handoff").as_deref() == Some("Move audio here")
    });
}

/// Waits until the audio is on the Desktop and the Call Window offers to
/// move it back, and checks it took no longer than [`HANDOFF_DEADLINE`]
/// from `asked`.
fn wait_for_audio_here(harness: &Harness, asked: Instant) -> AppState {
    let state = harness.wait_for_app(
        "the audio is on the Desktop and the Call Window offers to move it back",
        DEADLINE,
        |state| {
            state.audio_location == "desktop"
                && shown(state, "audio-location").as_deref() == Some("Audio on this computer")
                && shown(state, "handoff").as_deref() == Some("Move audio to phone")
        },
    );
    let took = asked.elapsed();
    assert!(
        took <= HANDOFF_DEADLINE,
        "the audio took {took:?} to reach the Desktop"
    );
    state
}

/// Checks the Phone was asked to clear `RejectSCO`, to `Activate`, and,
/// once the audio was here, to set `RejectSCO` again, and nothing more.
fn assert_handed_off(harness: &Harness) {
    let calls = harness.simulated_phone().wait_for_recorded(
        "RejectSCO is set again once the audio is here",
        DEADLINE,
        |calls| handoff_requests(calls).len() >= 3,
    );
    assert_eq!(
        handoff_requests(&calls),
        ["RejectSCO=false", "Activate", "RejectSCO=true"],
        "{calls:#?}"
    );
}

#[test]
fn a_call_placed_on_the_phone_shows_calling_and_leaves_the_audio_there() {
    let harness = connected();

    let state = placed_on_phone(&harness);

    assert_eq!(shown(&state, "status").as_deref(), Some("Calling…"));
    assert_eq!(state.calls[0].direction, "outgoing");
    assert_eq!(state.calls[0].number, NUMBER);
    assert_eq!(state.audio_location, "phone");
    let calls = harness.simulated_phone().recorded_holds(
        "the Desktop never clears RejectSCO or activates the audio",
        QUIET,
        |calls| handoff_requests(calls).is_empty() && !telephony(calls).any(is_activate),
    );
    assert_eq!(handoff_requests(&calls), Vec::<&str>::new());
}

#[test]
fn move_audio_desktop_brings_the_audio_here_and_sets_reject_sco_again() {
    let harness = connected();
    live_on_phone(&harness);

    let asked = Instant::now();
    harness
        .move_audio("desktop")
        .expect("MoveAudio(\"desktop\") is accepted");

    wait_for_audio_here(&harness, asked);
    assert_handed_off(&harness);
    let state = harness.app_holds("the audio stays on the Desktop", QUIET, |state| {
        state.audio_location == "desktop"
    });
    assert_eq!(state.calls.len(), 1);
}

#[test]
fn desktop_calls_audio_here_brings_the_audio_here() {
    let harness = connected();
    live_on_phone(&harness);

    let asked = Instant::now();
    let output = harness.cli(&["audio", "here"]);

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&output.stdout).trim(),
        "Moving the audio to this computer."
    );
    wait_for_audio_here(&harness, asked);
    assert_handed_off(&harness);
}

#[test]
fn a_handoff_works_while_the_call_placed_on_the_phone_still_dials() {
    let harness = connected();
    placed_on_phone(&harness);

    let asked = Instant::now();
    harness
        .move_audio("desktop")
        .expect("MoveAudio(\"desktop\") is accepted");

    let state = wait_for_audio_here(&harness, asked);
    assert_eq!(shown(&state, "status").as_deref(), Some("Calling…"));
}

#[test]
fn an_incoming_call_answered_on_the_phone_can_move_its_audio_here() {
    let harness = connected();
    let call = harness.simulated_phone().ring(NUMBER);
    harness.wait_for_app("the Call rings", DEADLINE, |state| state.ringing);
    harness.simulated_phone().answer_on_phone(&call);
    harness.wait_for_app(
        "the Call Window shows the audio on the Phone",
        DEADLINE,
        |state| {
            state.audio_location == "phone"
                && shown(state, "handoff").as_deref() == Some("Move audio here")
        },
    );

    let asked = Instant::now();
    harness
        .move_audio("desktop")
        .expect("MoveAudio(\"desktop\") is accepted");

    wait_for_audio_here(&harness, asked);
    assert_handed_off(&harness);
}

#[test]
fn a_phone_that_ignores_activate_once_is_asked_once_more() {
    let harness = connected();
    live_on_phone(&harness);
    harness
        .simulated_phone()
        .enable_quirk("activate-ignored-once");

    harness
        .move_audio("desktop")
        .expect("MoveAudio(\"desktop\") is accepted");

    let state = harness.wait_for_app("the audio reaches the Desktop", GIVE_UP, |state| {
        state.audio_location == "desktop"
    });
    assert_eq!(shown(&state, "notice"), None);
    let calls = harness.simulated_phone().wait_for_recorded(
        "RejectSCO is set again once the audio is here",
        DEADLINE,
        |calls| handoff_requests(calls).len() >= 5,
    );
    assert_eq!(
        handoff_requests(&calls),
        [
            "RejectSCO=false",
            "Activate",
            "RejectSCO=false",
            "Activate",
            "RejectSCO=true"
        ],
        "{calls:#?}"
    );
}

#[test]
fn a_phone_that_keeps_the_audio_gets_a_plain_message_and_the_call_stays_live() {
    let harness = connected();
    live_on_phone(&harness);
    harness.simulated_phone().enable_quirk("activate-ignored");

    harness
        .move_audio("desktop")
        .expect("MoveAudio(\"desktop\") is accepted");

    let state = harness.wait_for_app(
        "the Call Window says the Phone kept the audio",
        GIVE_UP,
        |state| shown(state, "notice").is_some(),
    );
    let notice = shown(&state, "notice").unwrap_or_default();
    assert!(
        notice.contains("tap the audio button on your phone's call screen"),
        "{notice}"
    );
    assert_eq!(state.audio_location, "phone");
    assert_eq!(shown(&state, "handoff").as_deref(), Some("Move audio here"));
    let error = harness.wait_for_app_error(DEADLINE);
    assert_eq!(error.kind, "audio-not-moved");
    // The Call is still live, and the Phone was never asked to hang up.
    let state = harness.app_holds("the Call stays live", QUIET, |state| {
        state
            .calls
            .first()
            .is_some_and(|call| call.state == "active")
    });
    assert_eq!(state.calls.len(), 1);
    harness.simulated_phone().recorded_holds(
        "the Phone is never asked to hang up",
        QUIET,
        |calls| !telephony(calls).any(|call| call.member == "Hangup"),
    );
}

#[test]
fn moving_the_audio_with_no_call_is_refused_plainly() {
    let harness = connected();

    let error = harness
        .move_audio("desktop")
        .expect_err("MoveAudio is refused with no Call");

    assert!(error.kind.ends_with("NoLiveCall"), "{error:?}");
    let output = harness.cli(&["audio", "here"]);
    assert!(!output.status.success());
    assert_eq!(
        String::from_utf8_lossy(&output.stderr).trim(),
        error.message,
        "the CLI prints the same sentence"
    );
}

#[test]
fn moving_the_audio_to_the_phone_gives_the_instruction_for_the_phone() {
    let harness = connected();
    live_on_phone(&harness);
    harness
        .move_audio("desktop")
        .expect("MoveAudio(\"desktop\") is accepted");
    harness.wait_for_app("the audio is on the Desktop", DEADLINE, |state| {
        state.audio_location == "desktop"
    });

    let output = harness.cli(&["audio", "phone"]);

    assert!(!output.status.success());
    assert_eq!(
        String::from_utf8_lossy(&output.stderr).trim(),
        "Tap the audio button on your phone's call screen to move the audio to your phone."
    );
    harness.app_holds("the audio stays on the Desktop", QUIET, |state| {
        state.audio_location == "desktop"
    });
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

/// Where the Hang up button is, once the screen shows it.
fn hang_up_button(harness: &Harness, name: &str) -> Region {
    let screenshot = harness.screenshot_when(name, SCREEN_DEADLINE, |screenshot| {
        find(screenshot, RED).is_some()
    });
    find(&screenshot, RED).unwrap_or_else(|| panic!("Hang up left the screen"))
}

/// The Handoff button: at the right of the audio row, two rows above Hang
/// up, as tall as it.
fn handoff_button_centre(hang_up: Region) -> (f64, f64) {
    let height = hang_up.bottom - hang_up.top;
    (
        hang_up.right - 24.0,
        hang_up.top - 2.0 * (ROW_GAP + height) + height / 2.0,
    )
}

#[test]
fn clicking_move_audio_here_brings_the_audio_here_and_the_button_offers_the_phone() {
    let harness = connected();
    live_on_phone(&harness);
    let hang_up = hang_up_button(&harness, "call-window-audio-on-phone");
    let (x, y) = handoff_button_centre(hang_up);

    let asked = Instant::now();
    harness.pointer().click(x, y);

    wait_for_audio_here(&harness, asked);
    assert_handed_off(&harness);
    // Hang up stayed on its row, and the window on the right edge, so the
    // Handoff button is still under the pointer. The window narrows as the
    // note on why Mute is disabled goes.
    let moved = hang_up_button(&harness, "call-window-audio-here");
    assert!(
        (moved.top - hang_up.top).abs() < 1.0 && (moved.right - hang_up.right).abs() < 1.0,
        "Hang up moved from {hang_up:?} to {moved:?}"
    );

    harness.pointer().click(x, y);

    let state = harness.wait_for_app(
        "the Call Window says how to move the audio on the Phone",
        DEADLINE,
        |state| shown(state, "notice").is_some(),
    );
    assert_eq!(
        shown(&state, "notice").as_deref(),
        Some("Tap the audio button on your phone's call screen to move the audio to your phone.")
    );
    assert_eq!(state.audio_location, "desktop");
}
