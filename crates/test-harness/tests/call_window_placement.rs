//! Spec #221, ticket #223: a live Call has one home, the bottom-right
//! corner. The Call Pill and the expanded Call Window share its bottom-right
//! corner at the standard gap, and the Call Window grows upwards, so Hang up
//! keeps its place whatever comes and goes. Observed through `Surfaces` and
//! screenshots, with synthetic pointer and keyboard input.

use std::collections::BTreeMap;
use std::time::Duration;

use test_harness::{AppState, BAR_COLOUR, BAR_HEIGHT, Builder, Harness, LAYOUT_SIZE, Region, Screenshot};

/// How long the app may take to notice a change on the Phone, or a key.
const DEADLINE: Duration = Duration::from_secs(5);
/// How long the Call Window may take to collapse by itself: the 5 s, and a
/// margin for a loaded runner.
const COLLAPSE_DEADLINE: Duration = Duration::from_secs(9);
/// How long the screen may take to show a change. One capture takes up to a
/// second on a slow runner.
const SCREEN_DEADLINE: Duration = Duration::from_secs(10);
/// How long the Desktop waits for the audio before it says it didn't come
/// (ticket #70), and how long it may take after that to say so.
const HANDOFF_LIMIT: Duration = Duration::from_secs(3);
const GIVE_UP_SLACK: Duration = Duration::from_millis(1_500);

const BOB: &str = "+447700900123";
const ALICE: &str = "+447700900456";

/// The Hang up button's red, and stock Adwaita's warning colour, which
/// marks the microphone muted.
const RED: [u8; 3] = [0xc0, 0x1c, 0x28];
const WARNING: [u8; 3] = [0xe5, 0xa5, 0x0a];

fn connected(builder: Builder) -> Harness {
    let harness = builder.headless_screen().omarchy_bar().start();
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

/// Whether `surface` reports itself anchored bottom right, `margin` from
/// both edges.
fn placed_bottom_right(surface: Option<&BTreeMap<String, String>>, margin: &str) -> bool {
    surface.is_some_and(|surface| {
        surface.get("anchor").map(String::as_str) == Some("bottom-right")
            && surface.get("margin").map(String::as_str) == Some(margin)
    })
}

/// Bob rings and the user answers on the Desktop: waits until the Call is
/// live here.
fn bob_live_here(harness: &Harness) {
    let _path = harness.simulated_phone().ring(BOB);
    let state = harness.wait_for_app("Bob's Call rings", DEADLINE, |state| {
        state.ringing && state.calls.len() == 1
    });
    harness.answer(state.calls[0].id);
    harness.wait_for_app("Bob's Call is live here", DEADLINE, |state| {
        state.audio_location == "desktop" && shown(state, "mute").as_deref() == Some("Mute")
    });
}

fn close_to(colour: [u8; 3], target: [u8; 3]) -> bool {
    colour
        .iter()
        .zip(target)
        .all(|(channel, wanted)| channel.abs_diff(wanted) <= 16)
}

/// Where the app's surfaces are in `area`: everything that is neither the
/// black background nor the bar stand-in.
fn app_region_within(screenshot: &Screenshot, area: Region) -> Option<Region> {
    screenshot.region_within(area, |pixel| pixel != [0, 0, 0] && pixel != BAR_COLOUR)
}

/// The bottom half of the screen, the live Call's.
fn bottom_half() -> Region {
    Region {
        left: 0.0,
        top: LAYOUT_SIZE.1 / 2.0,
        right: LAYOUT_SIZE.0,
        bottom: LAYOUT_SIZE.1,
    }
}

/// Where the red is in the bottom half of `screenshot`: Hang up.
fn red_below(screenshot: &Screenshot, area: Region) -> Option<Region> {
    screenshot.region_within(area, |pixel| close_to(pixel, RED))
}

/// Where Hang up is, once the screen shows the Call Window with it.
fn hang_up_on_screen(harness: &Harness, name: &str) -> Region {
    let screenshot = harness.screenshot_when(name, SCREEN_DEADLINE, |screenshot| {
        red_below(screenshot, bottom_half()).is_some()
    });
    red_below(&screenshot, bottom_half()).unwrap_or_else(|| panic!("Hang up left the screen"))
}

/// Rests the pointer in the Call Window's padding beside Hang up, so it
/// stays expanded. The window grows upwards, so the pointer stays in it.
fn hover_beside(harness: &Harness, hang_up: Region) {
    harness
        .pointer()
        .move_to(hang_up.left - 8.0, f64::midpoint(hang_up.top, hang_up.bottom));
}

/// Checks a capture taken once `changed` is on screen still shows Hang up
/// exactly at `hang_up`: the red near it is exactly Hang up's.
fn assert_hang_up_stays(
    harness: &Harness,
    name: &str,
    hang_up: Region,
    changed: impl Fn(&Screenshot) -> bool,
) {
    let screenshot = harness.screenshot_when(name, SCREEN_DEADLINE, changed);
    let near = Region {
        left: 0.0,
        top: hang_up.top - 6.0,
        right: LAYOUT_SIZE.0,
        bottom: LAYOUT_SIZE.1,
    };
    let moved = red_below(&screenshot, near).unwrap_or_else(|| panic!("Hang up left the screen"));
    // Within a screen pixel: at the scale of 1.6, a window of a new height
    // can round Hang up's edge to the next pixel.
    let within = |a: f64, b: f64| (a - b).abs() <= 1.0;
    assert!(
        within(moved.left, hang_up.left)
            && within(moved.top, hang_up.top)
            && within(moved.right, hang_up.right)
            && within(moved.bottom, hang_up.bottom),
        "Hang up moved with {name}, from {hang_up:?} to {moved:?}: see {}",
        screenshot.path.display()
    );
}

/// The Call Window's height in `screenshot`'s bottom half.
fn window_height(screenshot: &Screenshot) -> f64 {
    app_region_within(screenshot, bottom_half()).map_or(0.0, |window| window.bottom - window.top)
}

#[test]
fn the_call_pill_and_the_expanded_call_window_share_the_bottom_right_corner_at_hyprlands_gap() {
    // Half of Hyprland's 40, as Omarchy's own panels sit.
    let harness = connected(Harness::builder().hyprland_gaps_out(40));
    bob_live_here(&harness);
    let state = harness.wait_for_app("the Call Window sits 20 px in", DEADLINE, |state| {
        placed_bottom_right(call_window(state), "20")
    });
    assert!(state.surface("call-pill").is_none());
    let (width, height) = LAYOUT_SIZE;
    let in_corner = |region: Region| {
        (width - 20.0 - region.right).abs() <= 2.0 && (height - 20.0 - region.bottom).abs() <= 2.0
    };
    let expanded = harness.screenshot_when("call-window", SCREEN_DEADLINE, |screenshot| {
        app_region_within(screenshot, bottom_half()).is_some_and(in_corner)
    });
    let window = app_region_within(&expanded, bottom_half()).expect("the Call Window shows");
    assert!(
        window.right - window.left > 300.0 && window.top > BAR_HEIGHT,
        "{window:?} in {}",
        expanded.path.display()
    );

    let state = harness.wait_for_app(
        "the Call Window collapses to the Call Pill",
        COLLAPSE_DEADLINE,
        |state| call_window(state).is_none() && state.surface("call-pill").is_some(),
    );

    assert!(
        placed_bottom_right(state.surface("call-pill"), "20"),
        "{:?}",
        state.surface("call-pill")
    );
    let collapsed = harness.screenshot_when("call-pill", SCREEN_DEADLINE, |screenshot| {
        app_region_within(screenshot, bottom_half())
            .is_some_and(|pill| in_corner(pill) && pill.bottom - pill.top < 80.0)
    });
    let pill = app_region_within(&collapsed, bottom_half()).expect("the Call Pill shows");
    assert!(
        (pill.right - window.right).abs() <= 1.0 && (pill.bottom - window.bottom).abs() <= 1.0,
        "the Call Pill {pill:?} and the Call Window {window:?} don't share their corner"
    );
}

#[test]
fn hang_up_stays_put_when_the_microphone_is_muted_the_keypad_opens_or_a_call_goes_on_hold() {
    let harness = connected(Harness::builder());
    bob_live_here(&harness);
    let hang_up = hang_up_on_screen(&harness, "call-window");
    hover_beside(&harness, hang_up);
    let height = window_height(&harness.screenshot("call-window-plain"));

    harness
        .set_muted(true)
        .unwrap_or_else(|error| panic!("mute: {error:?}"));
    assert_hang_up_stays(&harness, "call-window-muted", hang_up, |screenshot| {
        screenshot
            .region_within(bottom_half(), |pixel| close_to(pixel, WARNING))
            .is_some()
            && window_height(screenshot) > height
    });
    let height = window_height(&harness.screenshot("call-window-muted-settled"));

    // A click in the padding gives the window the keyboard; K opens the
    // keypad.
    harness
        .pointer()
        .click(hang_up.left - 8.0, f64::midpoint(hang_up.top, hang_up.bottom));
    harness.keyboard().type_text("k");
    harness.wait_for_app("K shows the keypad", DEADLINE, |state| {
        shown(state, "keypad").as_deref() == Some("shown")
    });
    assert_hang_up_stays(&harness, "call-window-keypad", hang_up, |screenshot| {
        window_height(screenshot) > height + 100.0
    });
    let height = window_height(&harness.screenshot("call-window-keypad-settled"));

    let _path = harness.simulated_phone().ring(ALICE);
    harness.wait_for_app("Alice's Call waits", DEADLINE, |state| {
        state
            .surface("banner")
            .is_some_and(|banner| banner.contains_key("options"))
    });
    harness
        .hold_and_answer()
        .unwrap_or_else(|error| panic!("Hold & Answer: {error:?}"));
    harness.wait_for_app("Bob's Call is on hold", DEADLINE, |state| {
        shown(state, "held-caller").is_some() && state.surface("banner").is_none()
    });
    // The keypad closes for the newly live Call, and the Held Call's row
    // shows above it: the window changes height either way.
    assert_hang_up_stays(&harness, "call-window-held", hang_up, |screenshot| {
        (window_height(screenshot) - height).abs() > 20.0
    });
}

#[test]
fn hang_up_stays_put_when_a_notice_shows() {
    let harness = connected(Harness::builder());
    let path = harness.simulated_phone().ring(BOB);
    harness.wait_for_app("Bob's Call rings", DEADLINE, |state| state.ringing);
    harness.simulated_phone().answer_on_phone(&path);
    harness.wait_for_app("the Call is live on the Phone", DEADLINE, |state| {
        state.audio_location == "phone" && shown(state, "handoff").as_deref() == Some("Move audio here")
    });
    let hang_up = hang_up_on_screen(&harness, "call-window");
    hover_beside(&harness, hang_up);
    let height = window_height(&harness.screenshot("call-window-plain"));
    harness.simulated_phone().enable_quirk("activate-ignored");

    harness
        .move_audio("desktop")
        .expect("MoveAudio(\"desktop\") is accepted");

    harness.wait_for_app(
        "the Call Window says the Phone kept the audio",
        HANDOFF_LIMIT + GIVE_UP_SLACK + DEADLINE,
        |state| shown(state, "notice").is_some(),
    );
    assert_hang_up_stays(&harness, "call-window-notice", hang_up, |screenshot| {
        window_height(screenshot) > height
    });
}
