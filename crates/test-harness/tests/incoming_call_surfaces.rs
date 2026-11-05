//! Scenarios: the Incoming Call Banner and the Call Window on a screen of
//! their own, driven by synthetic keyboard and pointer, and checked on
//! screenshots and through the control interface's `Surfaces`.

use std::time::Duration;

use test_harness::{
    AppState, BAR_COLOUR, BAR_HEIGHT, Builder, Harness, Key, LAYOUT_SIZE, Region, Screenshot,
};

/// How long the app may take to notice a change on the Phone.
const DEADLINE: Duration = Duration::from_secs(5);
/// How long the Banner may take to show after the Phone rings.
const BANNER_DEADLINE: Duration = Duration::from_secs(1);
/// How long the Call audio may take to reach the Desktop after Answer.
const TRANSPORT_DEADLINE: Duration = Duration::from_millis(1_500);

const NUMBER: &str = "+447700900123";
const FORMATTED: &str = "+44 7700 900123";

/// How wide the app takes Omarchy's notification-toast column to be when
/// Hyprland can't say.
const FALLBACK_TOAST_COLUMN_WIDTH: f64 = 400.0;
/// How far a surface may sit from what it is placed beside.
const NEAR: f64 = 24.0;

/// The Answer button's green and the Decline and Hang up buttons' red.
const GREEN: [u8; 3] = [0x26, 0xa2, 0x69];
const RED: [u8; 3] = [0xc0, 0x1c, 0x28];

fn connected_on_screen() -> Harness {
    connected(Harness::builder().headless_screen().omarchy_bar())
}

fn connected(builder: Builder) -> Harness {
    let harness = builder.start();
    harness.simulated_phone().connect();
    harness.wait_for_app("ConnectionState is connected", DEADLINE, |state| {
        state.connection_state == "connected"
    });
    harness
}

/// Rings the Phone and waits for the Banner. Returns the Call's object path
/// and id.
fn ring_until_banner(harness: &Harness) -> (String, u32) {
    let path = harness.simulated_phone().ring(NUMBER);
    let state = harness.wait_for_app("the Banner shows", BANNER_DEADLINE, |state| {
        state.ringing && state.surface("banner").is_some()
    });
    (path, state.calls[0].id)
}

fn close_to(colour: [u8; 3], target: [u8; 3]) -> bool {
    colour
        .iter()
        .zip(target)
        .all(|(channel, wanted)| channel.abs_diff(wanted) <= 16)
}

/// Takes screenshots until one shows `colour`, and returns where.
fn find_on_screen(harness: &Harness, name: &str, colour: [u8; 3]) -> Region {
    let found = |screenshot: &test_harness::Screenshot| {
        screenshot.region_where(|pixel| close_to(pixel, colour))
    };
    let screenshot =
        harness.screenshot_when(name, DEADLINE, |screenshot| found(screenshot).is_some());
    found(&screenshot).unwrap_or_else(|| panic!("{colour:02x?} left the screen"))
}

fn click(harness: &Harness, region: Region) {
    let (x, y) = region.centre();
    harness.pointer().click(x, y);
}

fn toast_column(width: f64) -> Region {
    Region {
        left: LAYOUT_SIZE.0 - width,
        top: 0.0,
        right: LAYOUT_SIZE.0,
        bottom: LAYOUT_SIZE.1,
    }
}

/// Where the app's surfaces are: everything that is neither the black
/// background nor the bar stand-in.
fn app_region(screenshot: &Screenshot) -> Option<Region> {
    screenshot.region_where(|pixel| pixel != [0, 0, 0] && pixel != BAR_COLOUR)
}

fn assert_clear_of_bar_and_toasts(region: Region) {
    assert_clear_of_bar_and(region, FALLBACK_TOAST_COLUMN_WIDTH);
}

fn assert_clear_of_bar_and(region: Region, toast_column_width: f64) {
    assert!(
        region.left >= 0.0 && region.right <= LAYOUT_SIZE.0,
        "{region:?}"
    );
    assert!(region.bottom <= LAYOUT_SIZE.1, "{region:?}");
    assert!(region.top >= BAR_HEIGHT, "under the bar: {region:?}");
    assert!(
        !region.intersects(&toast_column(toast_column_width)),
        "over the toast column: {region:?}"
    );
}

fn is_live_on_desktop(state: &AppState, call: u32) -> bool {
    state.audio_location == "desktop"
        && state
            .calls
            .iter()
            .any(|known| known.id == call && known.state == "active")
}

fn is_timer(text: &str) -> bool {
    let mut parts = text.split(':');
    let (Some(minutes), Some(seconds), None) = (parts.next(), parts.next(), parts.next()) else {
        return false;
    };
    !minutes.is_empty()
        && minutes.chars().all(|digit| digit.is_ascii_digit())
        && seconds.len() == 2
        && seconds.chars().all(|digit| digit.is_ascii_digit())
}

fn call_window(state: &AppState) -> Option<&std::collections::BTreeMap<String, String>> {
    state.surface("call-window")
}

#[test]
fn nothing_is_on_screen_while_nothing_rings() {
    let harness = connected_on_screen();

    let state = harness.app_state();

    assert!(state.surfaces.is_empty(), "{:?}", state.surfaces);
    let screenshot = harness.screenshot("idle");
    assert_eq!(
        app_region(&screenshot),
        None,
        "{}",
        screenshot.path.display()
    );
}

#[test]
fn the_banner_shows_the_formatted_number_within_a_second() {
    let harness = connected_on_screen();

    let _call = harness.simulated_phone().ring(NUMBER);

    let state = harness.wait_for_app("the Banner shows", BANNER_DEADLINE, |state| {
        state.surface("banner").is_some()
    });
    let banner = state.surface("banner").expect("the Banner shows");
    assert_eq!(banner.get("caller").map(String::as_str), Some(FORMATTED));
    assert!(state.ringing);
    assert_eq!(call_window(&state), None);
}

#[test]
fn the_banner_sits_inside_the_output_below_the_bar_and_left_of_the_toasts() {
    let harness = connected_on_screen();
    ring_until_banner(&harness);

    find_on_screen(&harness, "banner", GREEN);
    let screenshot = harness.screenshot("banner");
    let banner = app_region(&screenshot).expect("the Banner is on screen");

    assert_clear_of_bar_and_toasts(banner);
}

#[test]
fn the_banner_clears_the_toast_column_hyprland_reports() {
    let harness = connected(
        Harness::builder()
            .headless_screen()
            .omarchy_bar()
            .hyprland_toast_column(300),
    );
    ring_until_banner(&harness);

    find_on_screen(&harness, "banner", GREEN);
    let screenshot = harness.screenshot("banner");
    let banner = app_region(&screenshot).expect("the Banner is on screen");

    assert_clear_of_bar_and(banner, 300.0);
    // It sits close beside the column Hyprland reports, not the wider
    // fallback, and close under the bar, not a fixed margin below it.
    assert!(banner.right >= LAYOUT_SIZE.0 - 300.0 - NEAR, "{banner:?}");
    assert!(banner.top <= BAR_HEIGHT + NEAR, "{banner:?}");
}

#[test]
fn the_banner_shows_over_a_fullscreen_window() {
    let harness = connected_on_screen();
    let client = harness.start_fullscreen_test_client();
    client.wait_for("the test client is fullscreen", DEADLINE, |line| {
        line == "configured 2400 1350"
    });

    ring_until_banner(&harness);

    let answer = find_on_screen(&harness, "banner-over-fullscreen", GREEN);
    let decline = find_on_screen(&harness, "banner-over-fullscreen", RED);
    assert_clear_of_bar_and_toasts(answer);
    assert_clear_of_bar_and_toasts(decline);
}

#[test]
fn typing_while_the_banner_shows_reaches_the_focused_window_and_it_keeps_ringing() {
    let harness = connected_on_screen();
    let client = harness.start_test_client();
    client.wait_for("the test client has keyboard focus", DEADLINE, |line| {
        line == "keyboard-enter"
    });
    ring_until_banner(&harness);
    find_on_screen(&harness, "banner-over-client", GREEN);

    harness.keyboard().type_text("a");
    harness.keyboard().press(Key::Return);

    client.wait_for("Return reaches the test client", DEADLINE, |line| {
        line == "key 28 released"
    });
    assert!(
        !client.lines().iter().any(|line| line == "keyboard-leave"),
        "the Banner took keyboard focus: {:?}",
        client.lines()
    );
    let state = harness.app_state();
    assert!(state.ringing);
    assert!(state.surface("banner").is_some());
}

#[test]
fn clicking_answer_answers_and_the_call_window_shows_the_audio_here_and_a_timer() {
    let harness = connected_on_screen();
    let (path, call) = ring_until_banner(&harness);
    let answer = find_on_screen(&harness, "banner", GREEN);

    click(&harness, answer);

    let state = harness.wait_for_app(
        "the Call is active with its audio on the Desktop",
        TRANSPORT_DEADLINE,
        |state| is_live_on_desktop(state, call),
    );
    assert!(!state.ringing);
    let state = harness.wait_for_app("the Call Window shows a timer", DEADLINE, |state| {
        state.surface("banner").is_none()
            && call_window(state).is_some_and(|window| {
                window.get("audio-location").map(String::as_str) == Some("Audio on this computer")
                    && window.get("status").is_some_and(|status| is_timer(status))
            })
    });
    let started = call_window(&state)
        .and_then(|window| window.get("status"))
        .cloned();
    harness.wait_for_app("the timer runs", DEADLINE, |state| {
        call_window(state)
            .and_then(|window| window.get("status"))
            .is_some_and(|status| is_timer(status) && Some(status) != started.as_ref())
    });

    let calls: Vec<_> = harness
        .simulated_phone()
        .recorded_method_calls()
        .into_iter()
        .filter(|recorded| recorded.bus == "session")
        .collect();
    let answer = calls
        .iter()
        .position(|recorded| recorded.path == path && recorded.member == "Answer")
        .expect("the Phone was asked to answer");
    let reject = calls
        .iter()
        .position(|recorded| {
            recorded.member == "Set"
                && recorded.arguments.contains("RejectSCO")
                && recorded.arguments.contains("false")
        })
        .expect("the Phone was asked to set RejectSCO=false");
    let activate = calls
        .iter()
        .position(|recorded| recorded.member == "Activate")
        .expect("the Phone was asked to Activate");
    assert!(answer < reject && reject < activate, "{calls:#?}");
}

#[test]
fn clicking_decline_declines_and_closes_the_banner() {
    let harness = connected_on_screen();
    let (path, _) = ring_until_banner(&harness);
    let decline = find_on_screen(&harness, "banner", RED);

    click(&harness, decline);

    let state = harness.wait_for_app("the Banner closes", DEADLINE, |state| {
        state.calls.is_empty() && state.surface("banner").is_none()
    });
    assert!(!state.ringing);
    assert_eq!(call_window(&state), None);
    assert!(
        harness
            .simulated_phone()
            .recorded_method_calls()
            .iter()
            .any(|recorded| recorded.path == path && recorded.member == "Hangup")
    );
}

#[test]
fn hanging_up_shows_call_ended_then_closes_the_call_window() {
    let harness = connected_on_screen();
    let (path, call) = ring_until_banner(&harness);
    harness.answer(call);
    harness.wait_for_app("the Call Window shows a timer", DEADLINE, |state| {
        call_window(state)
            .and_then(|window| window.get("status"))
            .is_some_and(|status| is_timer(status))
    });
    let hang_up = find_on_screen(&harness, "call-window", RED);

    click(&harness, hang_up);

    harness.wait_for_app("the Call Window says Call ended", DEADLINE, |state| {
        state.calls.is_empty()
            && call_window(state)
                .and_then(|window| window.get("status"))
                .is_some_and(|status| status == "Call ended")
    });
    harness.wait_for_app("the Call Window closes", DEADLINE, |state| {
        call_window(state).is_none()
    });
    let calls = harness.simulated_phone().recorded_method_calls();
    let asked_at = calls
        .iter()
        .position(|recorded| recorded.path == path && recorded.member == "Hangup")
        .expect("the Phone was asked to hang up");
    assert!(
        calls[asked_at..]
            .iter()
            .any(|recorded| recorded.member == "Set"
                && recorded.arguments.contains("RejectSCO")
                && recorded.arguments.contains("true")),
        "RejectSCO was not set back to true: {calls:#?}"
    );
}

#[test]
fn a_wideband_call_shows_hd() {
    let harness = connected_on_screen();
    harness.simulated_phone().set_codec(2);
    let (_, call) = ring_until_banner(&harness);

    harness.answer(call);

    let state = harness.wait_for_app("the Call Window shows HD", DEADLINE, |state| {
        call_window(state).is_some_and(|window| window.get("hd").is_some())
    });
    assert_eq!(state.codec, "msbc");
    assert!(state.wideband);
}

#[test]
fn a_narrowband_call_does_not_show_hd() {
    let harness = connected_on_screen();
    harness.simulated_phone().set_codec(1);
    let (_, call) = ring_until_banner(&harness);

    harness.answer(call);

    let state = harness.wait_for_app("the Call Window shows a timer", DEADLINE, |state| {
        state.codec == "cvsd"
            && call_window(state)
                .and_then(|window| window.get("status"))
                .is_some_and(|status| is_timer(status))
    });
    assert_eq!(
        call_window(&state).and_then(|window| window.get("hd")),
        None
    );
}
