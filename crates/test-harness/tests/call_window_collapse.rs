//! Spec #197, ticket #200: a few seconds after a Call connects, the Call
//! Window collapses to the Call Pill in the bottom-right corner. A click on
//! the Call Pill, the keybinding or a click on the Indicator expands it;
//! Escape collapses it. Observed through `Surfaces`, the app's log and
//! screenshots, with synthetic pointer and keyboard input.

use std::collections::BTreeMap;
use std::time::Duration;

use test_harness::{
    AppEvent, AppState, BAR_COLOUR, BAR_HEIGHT, Harness, Key, LAYOUT_SIZE, RecordedMethodCall,
    Region, Screenshot,
};

/// How long the app may take to notice a change on the Phone, or a key.
const DEADLINE: Duration = Duration::from_secs(5);
/// How long the Call Window may take to collapse by itself: the 5 s, and a
/// margin for a loaded runner.
const COLLAPSE_DEADLINE: Duration = Duration::from_secs(9);
/// How long the screen may take to show a change. One capture takes up to a
/// second on a slow runner.
const SCREEN_DEADLINE: Duration = Duration::from_secs(10);
/// How long something that keeps the Call Window expanded is watched for:
/// past the 5 s it would otherwise take to collapse.
const KEPT_EXPANDED: Duration = Duration::from_secs(6);

/// The gap the Call Pill keeps from the screen's edges.
const GAP: f64 = 12.0;

const NUMBER: &str = "+447700900123";
const SECOND_NUMBER: &str = "+447700900456";
const AUDIO_GATEWAY: &str = "org.pipewire.Telephony.AudioGateway1";

/// The Hang up button's red, the Answer button's green, and stock
/// Adwaita's warning colour, which marks the microphone muted.
const RED: [u8; 3] = [0xc0, 0x1c, 0x28];
const GREEN: [u8; 3] = [0x20, 0x86, 0x57];
const WARNING: [u8; 3] = [0xe5, 0xa5, 0x0a];

fn connected() -> Harness {
    let harness = Harness::builder().headless_screen().omarchy_bar().start();
    harness.simulated_phone().connect();
    harness.wait_for_app("ConnectionState is connected", DEADLINE, |state| {
        state.connection_state == "connected"
    });
    harness
}

fn pill(state: &AppState) -> Option<&BTreeMap<String, String>> {
    state.surface("call-pill")
}

/// The Call Window shows collapsed: the Call Pill is on screen, and the
/// window isn't.
fn collapsed(state: &AppState) -> bool {
    pill(state).is_some_and(|pill| pill.get("call-window").map(String::as_str) == Some("collapsed"))
        && state.surface("call-window").is_none()
}

/// The Call Window shows expanded, with the keyboard.
fn expanded_with_keyboard(state: &AppState) -> bool {
    pill(state).is_none()
        && state
            .surface("call-window")
            .is_some_and(|window| window.get("focused").map(String::as_str) == Some("true"))
}

/// Answers on the Desktop and waits until the Call is live, with the Call
/// Window expanded. Returns the Call's id.
fn live_on_desktop(harness: &Harness) -> u32 {
    let _path = harness.simulated_phone().ring(NUMBER);
    let state = harness.wait_for_app("the Call rings", DEADLINE, |state| {
        state.ringing && state.calls.len() == 1
    });
    let call = state.calls[0].id;
    harness.answer(call);
    harness.wait_for_app("the Call Window shows the live Call", DEADLINE, |state| {
        state.audio_location == "desktop"
            && state
                .surface("call-window")
                .is_some_and(|window| window.contains_key("audio-location"))
    });
    call
}

fn wait_until_collapsed(harness: &Harness) -> AppState {
    harness.wait_for_app(
        "the Call Window collapses to the Call Pill",
        COLLAPSE_DEADLINE,
        collapsed,
    )
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

/// Where the Call Pill is on screen, once it is drawn with its red Hang up,
/// with nothing else of the app shown.
fn pill_on_screen(harness: &Harness, name: &str) -> (Region, Screenshot) {
    let screenshot = harness.screenshot_when(name, SCREEN_DEADLINE, |screenshot| {
        find(screenshot, RED).is_some()
    });
    let region = app_region(&screenshot).unwrap_or_else(|| panic!("the Call Pill left the screen"));
    (region, screenshot)
}

/// Clicks the Call Pill's body: the caller's name, at its left end.
fn click_pill_body(harness: &Harness, pill: Region) {
    harness
        .pointer()
        .click(pill.left + 30.0, f64::midpoint(pill.top, pill.bottom));
}

/// Where the Call Window is on screen, once it is drawn.
fn call_window_on_screen(harness: &Harness, name: &str) -> Region {
    let screenshot = harness.screenshot_when(name, SCREEN_DEADLINE, |screenshot| {
        find(screenshot, RED).is_some()
    });
    app_region(&screenshot).unwrap_or_else(|| panic!("the Call Window left the screen"))
}

fn microphone_volumes_set(recorded: &[RecordedMethodCall]) -> usize {
    recorded
        .iter()
        .filter(|call| {
            call.member == "Set"
                && call.arguments.contains(AUDIO_GATEWAY)
                && call.arguments.contains("MicrophoneVolume")
        })
        .count()
}

#[test]
fn a_connected_call_collapses_to_the_call_pill_in_the_bottom_right_corner_after_five_seconds() {
    let harness = connected();
    let call = live_on_desktop(&harness);

    let state = wait_until_collapsed(&harness);

    // By the app's own log: still expanded 4 s after the Call connected,
    // and collapsed within the 5 s and a margin.
    let took = harness.app_took(
        &AppEvent::kind("call-changed")
            .with("call_id", call)
            .with("call_state", "active"),
        &AppEvent::kind("surface-shown").with("surface", "call-pill"),
        DEADLINE,
    );
    assert!(
        took >= Duration::from_secs(4) && took < COLLAPSE_DEADLINE,
        "the Call Pill showed {took:?} after the Call connected"
    );
    let shown = pill(&state).cloned().unwrap_or_default();
    assert_eq!(
        shown.get("caller").map(String::as_str),
        Some("+44 7700 900123")
    );
    assert!(
        shown.get("status").is_some_and(|status| status.contains(':')),
        "the Call Pill shows the timer: {shown:?}"
    );
    assert_eq!(shown.get("mute").map(String::as_str), Some("Mute"));
    assert_eq!(shown.get("hang-up").map(String::as_str), Some("Hang up"));
    assert_eq!(shown.get("anchor").map(String::as_str), Some("bottom-right"));
    assert!(!shown.contains_key("focused"), "the Call Pill took the keyboard");

    // On screen: one strip in the bottom-right corner, 12 px from both edges.
    let (region, screenshot) = pill_on_screen(&harness, "call-pill");
    let (width, height) = LAYOUT_SIZE;
    assert!(
        (width - GAP - region.right).abs() <= 2.0 && (height - GAP - region.bottom).abs() <= 2.0,
        "the Call Pill isn't 12 px from the bottom-right corner: {region:?} in {}",
        screenshot.path.display()
    );
    assert!(
        region.right - region.left <= 340.0 && region.bottom - region.top <= 80.0,
        "the Call Pill is no small strip: {region:?} in {}",
        screenshot.path.display()
    );
}

#[test]
fn clicking_the_call_pill_or_pressing_the_keybinding_expands_it_with_the_keyboard_and_escape_collapses_it()
 {
    let harness = connected();
    live_on_desktop(&harness);
    wait_until_collapsed(&harness);
    let (pill_region, _) = pill_on_screen(&harness, "call-pill");

    click_pill_body(&harness, pill_region);

    harness.wait_for_app(
        "the click expands the Call Window with the keyboard",
        DEADLINE,
        expanded_with_keyboard,
    );
    // In place: it shares the Call Pill's bottom-right corner, so the click
    // brought the controls up under the pointer.
    let window = call_window_on_screen(&harness, "call-window-expanded");
    assert!(
        (window.right - pill_region.right).abs() <= 2.0
            && (window.bottom - pill_region.bottom).abs() <= 2.0
            && window.top > BAR_HEIGHT
            && window.left < pill_region.left + 30.0,
        "the Call Window didn't open where the Call Pill was ({pill_region:?}): {window:?}"
    );
    let state = harness.app_state();
    let reported = state.surface("call-window").cloned().unwrap_or_default();
    assert_eq!(
        (
            reported.get("anchor").map(String::as_str),
            reported.get("margin").map(String::as_str)
        ),
        (Some("bottom-right"), Some("12")),
        "{reported:?}"
    );
    // Its shortcuts work: K opens the keypad.
    harness.keyboard().type_text("k");
    harness.wait_for_app("K shows the keypad", DEADLINE, |state| {
        state
            .surface("call-window")
            .is_some_and(|window| window.get("keypad").map(String::as_str) == Some("shown"))
    });

    harness.keyboard().press(Key::Escape);

    let state = harness.wait_for_app(
        "Escape collapses the Call Window, even with the keypad open",
        DEADLINE,
        collapsed,
    );
    assert!(!pill(&state).is_some_and(|pill| pill.contains_key("focused")));

    harness.activate();

    let state = harness.wait_for_app(
        "the keybinding expands the Call Window with the keyboard",
        DEADLINE,
        expanded_with_keyboard,
    );
    assert_eq!(
        state
            .surface("call-window")
            .and_then(|window| window.get("keypad"))
            .map(String::as_str),
        Some("shown"),
        "the keypad stays open for the same Call"
    );
    assert!(state.calls.iter().all(|call| call.state == "active"));
}

#[test]
fn clicking_the_indicator_expands_the_collapsed_call_window() {
    let harness = connected();
    live_on_desktop(&harness);
    wait_until_collapsed(&harness);

    harness.tray().activate();

    let state = harness.wait_for_app(
        "the Indicator expands the Call Window with the keyboard",
        DEADLINE,
        expanded_with_keyboard,
    );
    assert_eq!(state.surface("dialer"), None, "the Dialer stays shut");
}

#[test]
fn the_pointer_keeps_it_expanded_until_five_seconds_after_it_leaves_and_so_does_the_keyboard_until_escape()
 {
    let harness = connected();
    live_on_desktop(&harness);
    let window = call_window_on_screen(&harness, "call-window");

    // The pointer rests on the Call Window's padding, beside its caller.
    harness.pointer().move_to(window.left + 6.0, window.top + 6.0);
    harness.app_holds(
        "the Call Window stays expanded while the pointer is over it",
        KEPT_EXPANDED,
        |state| state.surface("call-window").is_some() && pill(state).is_none(),
    );

    // Once the pointer leaves, it collapses 5 s later.
    harness.pointer().move_to(LAYOUT_SIZE.0 / 2.0, LAYOUT_SIZE.1 / 2.0);
    harness.app_holds(
        "the Call Window stays expanded for a moment after the pointer leaves",
        Duration::from_secs(4),
        |state| state.surface("call-window").is_some() && pill(state).is_none(),
    );
    wait_until_collapsed(&harness);
    harness.activate();
    harness.wait_for_app("the keybinding expands it", DEADLINE, expanded_with_keyboard);

    // A click on the window keeps the keyboard there, which keeps it
    // expanded with the pointer gone.
    harness.pointer().click(window.left + 6.0, window.top + 6.0);
    harness.wait_for_app("the Call Window has the keyboard", DEADLINE, |state| {
        state
            .surface("call-window")
            .is_some_and(|window| window.contains_key("focused"))
    });
    harness.pointer().move_to(LAYOUT_SIZE.0 / 2.0, LAYOUT_SIZE.1 / 2.0);
    harness.app_holds(
        "the Call Window stays expanded while it has the keyboard",
        KEPT_EXPANDED,
        |state| state.surface("call-window").is_some() && pill(state).is_none(),
    );

    // Escape gives the keyboard back, and collapses it at once.
    harness.keyboard().press(Key::Escape);
    harness.wait_for_app("Escape collapses the Call Window", DEADLINE, collapsed);

    // Expanded by the keybinding and let go of with Escape, it collapses.
    harness.activate();
    harness.wait_for_app("the keybinding expands it", DEADLINE, expanded_with_keyboard);
    harness.keyboard().press(Key::Escape);
    harness.wait_for_app("Escape collapses it again", DEADLINE, collapsed);
}

#[test]
fn mute_and_hang_up_on_the_call_pill_work_and_it_shows_muted() {
    let harness = connected();
    harness.simulated_phone().set_microphone_volume(9);
    live_on_desktop(&harness);
    wait_until_collapsed(&harness);
    let (_, screenshot) = pill_on_screen(&harness, "call-pill");
    let hang_up = find(&screenshot, RED).unwrap_or_else(|| panic!("Hang up is on screen"));
    let middle = f64::midpoint(hang_up.top, hang_up.bottom);

    // Mute sits just left of Hang up.
    harness.pointer().click(hang_up.left - 20.0, middle);

    let state = harness.wait_for_app(
        "Muted is true and the Call Pill shows it",
        DEADLINE,
        |state| state.muted && pill(state).is_some_and(|pill| pill.contains_key("muted")),
    );
    assert_eq!(
        pill(&state).and_then(|pill| pill.get("mute")).map(String::as_str),
        Some("Unmute")
    );
    assert!(collapsed(&state), "a click on Mute doesn't expand it");
    // Mute, just left of Hang up, takes the warning colour, and so does
    // the Call Pill's border.
    let on_mute = (hang_up.left - 11.0, middle);
    let muted = harness.screenshot_when("call-pill-muted", SCREEN_DEADLINE, |screenshot| {
        close_to(screenshot.colour_at(on_mute.0, on_mute.1), WARNING)
    });
    let marked = find(&muted, WARNING).unwrap_or_else(|| panic!("the warning colour is on screen"));
    assert!(
        marked.right > hang_up.right && marked.bottom > hang_up.bottom,
        "the Call Pill's border isn't in the warning colour: {marked:?}; see {}",
        muted.path.display()
    );
    harness.simulated_phone().wait_for_recorded(
        "MicrophoneVolume is set to 0",
        DEADLINE,
        |recorded| microphone_volumes_set(recorded) == 1,
    );

    harness
        .pointer()
        .click(f64::midpoint(hang_up.left, hang_up.right), middle);

    harness.wait_for_app("Hang up on the Call Pill ends the Call", DEADLINE, |state| {
        state.calls.is_empty()
    });
    harness
        .simulated_phone()
        .wait_for_recorded("the Call is hung up", DEADLINE, |recorded| {
            recorded.iter().any(|recorded| recorded.member == "Hangup")
        });
}

#[test]
fn a_waiting_calls_banner_takes_its_usual_place_while_the_call_pill_shows() {
    let harness = connected();
    live_on_desktop(&harness);
    wait_until_collapsed(&harness);

    let _path = harness.simulated_phone().ring(SECOND_NUMBER);

    let state = harness.wait_for_app("the Waiting Call's Banner shows", DEADLINE, |state| {
        state
            .surface("banner")
            .is_some_and(|banner| banner.contains_key("options"))
    });
    assert!(collapsed(&state), "a ringing Waiting Call doesn't expand it");
    let screenshot = harness.screenshot_when("call-pill-and-banner", SCREEN_DEADLINE, |shot| {
        find(shot, GREEN).is_some()
    });
    let hold_and_answer = find(&screenshot, GREEN).unwrap_or_else(|| panic!("no Hold & Answer"));
    // Flush top right, as always. The Banner's heading and caller sit above
    // Hold & Answer.
    assert!(
        hold_and_answer.top < BAR_HEIGHT + GAP + 140.0,
        "the Banner isn't at the top: {hold_and_answer:?} in {}",
        screenshot.path.display()
    );
    let red = find(&screenshot, RED).unwrap_or_else(|| panic!("no Hang up on the Call Pill"));
    assert!(
        red.bottom > LAYOUT_SIZE.1 / 2.0,
        "the Call Pill moved: {red:?}"
    );
}

#[test]
fn once_a_call_ended_while_collapsed_has_gone_the_indicator_opens_the_dialer() {
    let harness = connected();
    let dial = harness.start_dial(NUMBER);
    harness
        .simulated_phone()
        .wait_for_recorded("the Phone is asked to dial", DEADLINE, |calls| {
            calls.iter().any(|call| call.member == "Dial")
        });
    let _path = harness.simulated_phone().alert();
    dial.outcome(DEADLINE)
        .unwrap_or_else(|error| panic!("dial: {error}"));
    harness.simulated_phone().callee_answers();
    let state = wait_until_collapsed(&harness);
    let call = state.calls[0].id;
    harness.hang_up(call);
    harness.wait_for_app("the Call Pill and the Call Window go", DEADLINE, |state| {
        state.calls.is_empty() && pill(state).is_none() && state.surface("call-window").is_none()
    });

    harness.tray().activate();

    harness.wait_for_app("the Indicator opens the Dialer", DEADLINE, |state| {
        state.surface("dialer").is_some()
    });
}

/// Ticket #201: a notice expands the collapsed Call Window by itself, so a
/// failure is never hidden in a strip.
#[test]
fn a_notice_expands_the_collapsed_call_window() {
    let harness = connected();
    live_on_desktop(&harness);
    wait_until_collapsed(&harness);
    harness
        .simulated_phone()
        .enable_quirk("disconnect-profile-refused");

    harness
        .move_audio("phone")
        .unwrap_or_else(|error| panic!("MoveAudio(\"phone\") was refused: {error:?}"));

    let state = harness.wait_for_app("the notice expands the Call Window", DEADLINE, |state| {
        state
            .surface("call-window")
            .is_some_and(|window| window.contains_key("notice"))
    });
    assert_eq!(pill(&state), None);
    // Whether the window, mapped again, gets the keyboard is the
    // compositor's choice: sway hands it over when nothing else has it. The
    // core doesn't ask for it (core tests).
}

/// Ticket #201: answered on the Phone, the Call Pill says the audio is on the
/// Phone, and Mute is unavailable.
#[test]
fn with_the_audio_on_the_phone_the_call_pill_says_so_and_mute_is_unavailable() {
    let harness = connected();
    let path = harness.simulated_phone().ring(NUMBER);
    harness.wait_for_app("the Banner shows", DEADLINE, |state| {
        state.surface("banner").is_some()
    });
    harness.simulated_phone().answer_on_phone(&path);

    let state = wait_until_collapsed(&harness);

    let Some(pill) = pill(&state) else {
        panic!("the Call Pill shows");
    };
    assert_eq!(
        pill.get("audio-location").map(String::as_str),
        Some("Audio on phone")
    );
    assert!(pill.contains_key("mute-unavailable"), "{pill:?}");
}

/// Ticket #201: a ringing Waiting Call leaves the Call Window collapsed, as
/// the Banner shows it; Hold & Answer expands it, by itself, with the Call
/// on hold in view. Collapsed again by hand, the Call Pill says 1 on hold.
#[test]
fn hold_and_answer_expands_the_call_pill_and_collapsed_again_it_says_one_on_hold() {
    let harness = connected();
    live_on_desktop(&harness);
    wait_until_collapsed(&harness);
    let _path = harness.simulated_phone().ring(SECOND_NUMBER);
    harness.wait_for_app("the Banner offers Hold & Answer", DEADLINE, |state| {
        state
            .surface("banner")
            .is_some_and(|banner| banner.contains_key("options"))
    });
    harness.app_holds(
        "the Call Window stays collapsed while the Waiting Call rings",
        Duration::from_millis(500),
        collapsed,
    );

    harness
        .hold_and_answer()
        .unwrap_or_else(|error| panic!("Hold & Answer: {error:?}"));

    harness.wait_for_app(
        "Hold & Answer expands the Call Window with the Call on hold",
        DEADLINE,
        |state| {
            pill(state).is_none()
                && state
                    .surface("call-window")
                    .is_some_and(|window| window.keys().any(|key| key.starts_with("held")))
        },
    );
    // Whether the window, mapped again, gets the keyboard is the
    // compositor's choice: sway hands it over when nothing else has it. The
    // core doesn't ask for it (core tests).
    harness.activate();
    harness.wait_for_app("the keybinding gives it the keyboard", DEADLINE, expanded_with_keyboard);
    harness.keyboard().press(Key::Escape);
    let state = harness.wait_for_app("Escape collapses it", DEADLINE, collapsed);
    assert_eq!(
        pill(&state).and_then(|pill| pill.get("held")).map(String::as_str),
        Some("1 on hold")
    );
}

/// Ticket #201: End & Answer expands the collapsed Call Window for the new
/// Call, which collapses 5 s later under the usual rule.
#[test]
fn end_and_answer_expands_the_call_pill_for_the_new_call_which_collapses_again() {
    let harness = connected();
    live_on_desktop(&harness);
    wait_until_collapsed(&harness);
    let _path = harness.simulated_phone().ring(SECOND_NUMBER);
    harness.wait_for_app("the Banner offers End & Answer", DEADLINE, |state| {
        state
            .surface("banner")
            .is_some_and(|banner| banner.contains_key("options"))
    });

    harness
        .end_and_answer()
        .unwrap_or_else(|error| panic!("End & Answer: {error:?}"));

    harness.wait_for_app(
        "End & Answer expands the Call Window for the new Call",
        DEADLINE,
        |state| {
            pill(state).is_none()
                && state.calls.len() == 1
                && state.calls[0].state == "active"
                && state.surface("call-window").is_some()
        },
    );
    // Whether the window, mapped again, gets the keyboard is the
    // compositor's choice: sway hands it over when nothing else has it. The
    // core doesn't ask for it (core tests).
    let state = wait_until_collapsed(&harness);
    assert_eq!(
        pill(&state).and_then(|pill| pill.get("caller")).map(String::as_str),
        Some("+44 7700 900456")
    );
}

/// Ticket #201: a Call ended while collapsed shows Call ended on the Call
/// Pill, for as long as the Call Window would, and then goes.
#[test]
fn a_call_ended_while_collapsed_says_call_ended_on_the_call_pill_then_goes() {
    let harness = connected();
    let call = live_on_desktop(&harness);
    wait_until_collapsed(&harness);

    harness.hang_up(call);

    harness.wait_for_app("the Call Pill says Call ended", DEADLINE, |state| {
        pill(state)
            .and_then(|pill| pill.get("status"))
            .is_some_and(|status| status == "Call ended")
    });
    harness.wait_for_app("the Call Pill goes", DEADLINE, |state| {
        pill(state).is_none() && state.surface("call-window").is_none()
    });
}

