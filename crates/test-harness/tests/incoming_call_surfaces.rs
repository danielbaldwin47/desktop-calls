//! Scenarios: the Incoming Call Banner and the Call Window on a screen of
//! their own, driven by synthetic keyboard and pointer, and checked on
//! screenshots and through the control interface's `Surfaces`.

use std::time::Duration;

use test_harness::{
    AppEvent, AppState, BAR_COLOUR, BAR_HEIGHT, Builder, Harness, Key, LAYOUT_SIZE, Region,
    Screenshot,
};

/// How long the app may take to notice a change on the Phone.
const DEADLINE: Duration = Duration::from_secs(5);
/// How long the Banner may take to show after the Phone rings: from the
/// Call appearing to the Banner's first frame, by the app's log.
const BANNER_DEADLINE: Duration = Duration::from_secs(1);
/// How long the Call audio may take to reach the Desktop after Answer: from
/// the request reaching the app to the audio on the Desktop, by its log.
const TRANSPORT_DEADLINE: Duration = Duration::from_millis(1_500);
/// How long the tests' own round trip may take: the Phone or the test's
/// request reaching the app, and the change coming back on the control
/// interface. Looser than the targets above because a loaded CI runner
/// stretches it even when the app is fast (ticket #83); the targets are
/// asserted on the app's log instead.
const ROUND_TRIP: Duration = Duration::from_secs(3);
/// How long the app may take to do its first-frame work after it starts.
const FIRST_FRAMES_DEADLINE: Duration = Duration::from_secs(15);

const NUMBER: &str = "+447700900123";
const FORMATTED: &str = "+44 7700 900123";

/// The standard gap from the screen's edges when Hyprland can't say.
const FALLBACK_GAP: f64 = 12.0;

/// The Answer button's green and the Decline and Hang up buttons' red.
const GREEN: [u8; 3] = [0x20, 0x86, 0x57];
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
    let state = wait_for_banner(harness);
    (path, state.calls[0].id)
}

/// Waits until the Banner shows for the ringing Call, and checks the app
/// drew it within [`BANNER_DEADLINE`] of the Call appearing.
fn wait_for_banner(harness: &Harness) -> AppState {
    let state = harness.wait_for_app("the Banner shows", ROUND_TRIP, |state| {
        state.ringing && state.surface("banner").is_some()
    });
    harness.assert_banner_within(state.calls[0].id, BANNER_DEADLINE);
    state
}

/// Waits until the Call is live with its audio on the Desktop, and checks
/// the app took no longer than [`TRANSPORT_DEADLINE`] from Answer.
fn wait_for_audio_here(harness: &Harness, call: u32) -> AppState {
    let state = harness.wait_for_app(
        "the Call is active with its audio on the Desktop",
        ROUND_TRIP,
        |state| is_live_on_desktop(state, call),
    );
    harness.assert_answered_audio_within(call, TRANSPORT_DEADLINE);
    state
}

fn close_to(colour: [u8; 3], target: [u8; 3]) -> bool {
    colour
        .iter()
        .zip(target)
        .all(|(channel, wanted)| channel.abs_diff(wanted) <= 16)
}

/// Takes screenshots until one shows `colour`, and returns where its
/// lowest button is: the bottom 20 px of what is in that colour, since the
/// Banner's ringing dot, above Answer, is the same green.
fn find_on_screen(harness: &Harness, name: &str, colour: [u8; 3]) -> Region {
    let found = |screenshot: &test_harness::Screenshot| {
        let all = screenshot.region_where(|pixel| close_to(pixel, colour))?;
        screenshot.region_within(
            Region {
                top: (all.bottom - 20.0).max(all.top),
                ..all
            },
            |pixel| close_to(pixel, colour),
        )
    };
    let screenshot =
        harness.screenshot_when(name, DEADLINE, |screenshot| found(screenshot).is_some());
    found(&screenshot).unwrap_or_else(|| panic!("{colour:02x?} left the screen"))
}

fn click(harness: &Harness, region: Region) {
    let (x, y) = region.centre();
    harness.pointer().click(x, y);
}

/// Where the app's surfaces are: everything that is neither the black
/// background nor the bar stand-in.
fn app_region(screenshot: &Screenshot) -> Option<Region> {
    screenshot.region_where(|pixel| pixel != [0, 0, 0] && pixel != BAR_COLOUR)
}

/// Asserts that `region` is on the output, below the bar.
fn assert_clear_of_bar(region: Region) {
    assert!(
        region.left >= 0.0 && region.right <= LAYOUT_SIZE.0,
        "{region:?}"
    );
    assert!(region.bottom <= LAYOUT_SIZE.1, "{region:?}");
    assert!(region.top >= BAR_HEIGHT, "under the bar: {region:?}");
}

/// Asserts that `region` sits flush in the top-right corner, `gap` from the
/// screen's right edge and from the bar.
fn assert_flush_top_right(region: Region, gap: f64, screenshot: &Screenshot) {
    assert_clear_of_bar(region);
    assert!(
        (LAYOUT_SIZE.0 - gap - region.right).abs() <= 2.0
            && (BAR_HEIGHT + gap - region.top).abs() <= 2.0,
        "not {gap} px from the top-right corner: {region:?} in {}",
        screenshot.path.display()
    );
}

/// Whether the Banner reports itself anchored top right, `margin` from both
/// edges.
fn banner_placed(state: &AppState, margin: &str) -> bool {
    state.surface("banner").is_some_and(|banner| {
        banner.get("anchor").map(String::as_str) == Some("top-right")
            && banner.get("margin").map(String::as_str) == Some(margin)
    })
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
    // Including while the app prepares the surfaces' first frames.
    harness.wait_for_first_frames_prepared(FIRST_FRAMES_DEADLINE);

    let state = harness.app_state();

    assert!(state.surfaces.is_empty(), "{:?}", state.surfaces);
    assert_eq!(harness.surfaces_seen(), Vec::new());
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

    let state = wait_for_banner(&harness);
    let banner = state.surface("banner").expect("the Banner shows");
    assert_eq!(banner.get("caller").map(String::as_str), Some(FORMATTED));
    // A number not in Contacts: a # tile, and the meta line says so.
    assert_eq!(banner.get("initial").map(String::as_str), Some("#"));
    assert_eq!(
        banner.get("meta").map(String::as_str),
        Some("Not in Contacts")
    );
    assert!(state.ringing);
    assert_eq!(call_window(&state), None);
}

/// A timing judged one scenario at a time, as on the Desktop, which runs one
/// app (issue #47). CI runs it alone, after the whole run. With four
/// scenarios at once on one CPU quota, as in a
/// whole run, the Banner window's own first paint held the main loop up for
/// 175 ms in 2 of 4 runs; one scenario at a time under `just test-ci-like`,
/// the whole first frame took 63 to 64 ms in 5 of 5.
#[test]
#[ignore = "one scenario at a time: just test-file incoming_call_surfaces the_first_banner --ignored --ci-like"]
fn the_first_banner_shows_without_holding_up_the_main_loop() {
    let harness = connected_on_screen();
    // As for a person's first Call, which comes long after login.
    harness.wait_for_first_frames_prepared(FIRST_FRAMES_DEADLINE);

    let (_, call) = ring_until_banner(&harness);

    let (appeared, shown) = (
        AppEvent::kind("call-added").with("call_id", call),
        AppEvent::kind("surface-shown").with("surface", "banner"),
    );
    let stalls = harness.app_stalls_between(&appeared, &shown);
    eprintln!(
        "first Banner drawn {:?} after the Call appeared; main loop held up for {stalls:?} ms",
        harness.app_took(&appeared, &shown, DEADLINE)
    );
    assert!(
        stalls.is_empty(),
        "the main loop was held up for {stalls:?} ms while the first Banner appeared"
    );
}

/// Spec #221: the Banner sits flush in the top-right corner, below the bar,
/// at the standard gap: 12 px without Hyprland to say.
#[test]
fn the_banner_sits_flush_in_the_top_right_corner_below_the_bar() {
    let harness = connected_on_screen();
    ring_until_banner(&harness);

    find_on_screen(&harness, "banner", GREEN);
    let screenshot = harness.screenshot("banner");
    let banner = app_region(&screenshot).expect("the Banner is on screen");

    assert_flush_top_right(banner, FALLBACK_GAP, &screenshot);
    assert!(banner_placed(&harness.app_state(), "12"));
}

/// The standard gap is Omarchy's edge gap, half of Hyprland's
/// `general:gaps_out`, read again when Hyprland reloads its configuration.
#[test]
fn the_banner_follows_hyprlands_gap_from_the_edge_and_its_reloads() {
    let harness = connected(
        Harness::builder()
            .headless_screen()
            .omarchy_bar()
            .hyprland_gaps_out(30),
    );
    ring_until_banner(&harness);
    harness.wait_for_app("the Banner sits 15 px in", DEADLINE, |state| {
        banner_placed(state, "15")
    });
    let screenshot = harness.screenshot_when("banner-gap-15", DEADLINE, |screenshot| {
        app_region(screenshot)
            .is_some_and(|banner| (LAYOUT_SIZE.0 - 15.0 - banner.right).abs() <= 2.0)
    });
    let banner = app_region(&screenshot).expect("the Banner is on screen");
    assert_flush_top_right(banner, 15.0, &screenshot);

    // An odd gap: half of it rounds up, as in Omarchy's shell.
    harness.set_hyprland_gaps_out(49);

    harness.wait_for_app("the Banner moves to 25 px in", DEADLINE, |state| {
        banner_placed(state, "25")
    });
    let screenshot = harness.screenshot_when("banner-gap-25", DEADLINE, |screenshot| {
        app_region(screenshot)
            .is_some_and(|banner| (LAYOUT_SIZE.0 - 25.0 - banner.right).abs() <= 2.0)
    });
    let banner = app_region(&screenshot).expect("the Banner is on screen");
    assert_flush_top_right(banner, 25.0, &screenshot);

    // A reload that drops the gap: the fallback applies again.
    harness.unset_hyprland_gaps_out();

    harness.wait_for_app("the Banner moves back to 12 px in", DEADLINE, |state| {
        banner_placed(state, "12")
    });
}

/// A Hyprland whose answers have no `general:gaps_out` leaves the Banner at
/// the fallback gap, as no Hyprland at all does.
#[test]
fn the_banner_sits_at_the_fallback_gap_when_hyprland_has_none() {
    let harness = connected(
        Harness::builder()
            .headless_screen()
            .omarchy_bar()
            .hyprland_rounding(0),
    );
    ring_until_banner(&harness);
    let state = harness.wait_for_app("the Banner shows", DEADLINE, |state| {
        state.surface("banner").is_some()
    });
    assert!(banner_placed(&state, "12"), "{:?}", state.surface("banner"));
    let screenshot = harness.screenshot_when("banner-fallback-gap", DEADLINE, |screenshot| {
        app_region(screenshot)
            .is_some_and(|banner| (LAYOUT_SIZE.0 - FALLBACK_GAP - banner.right).abs() <= 2.0)
    });
    let banner = app_region(&screenshot).expect("the Banner is on screen");
    assert_flush_top_right(banner, FALLBACK_GAP, &screenshot);
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
    assert_clear_of_bar(answer);
    assert_clear_of_bar(decline);
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

    let state = wait_for_audio_here(&harness, call);
    assert!(!state.ringing);
    let state = harness.wait_for_app("the Call Window shows a timer", DEADLINE, |state| {
        state.surface("banner").is_none()
            && call_window(state).is_some_and(|window| {
                window.get("state").map(String::as_str) == Some("On this computer")
                    && window.get("timer").is_some_and(|status| is_timer(status))
            })
    });
    let started = call_window(&state)
        .and_then(|window| window.get("timer"))
        .cloned();
    harness.wait_for_app("the timer runs", DEADLINE, |state| {
        call_window(state)
            .and_then(|window| window.get("timer"))
            .is_some_and(|status| is_timer(status) && Some(status) != started.as_ref())
    });

    let calls: Vec<_> = harness
        .simulated_phone()
        .wait_for_recorded("the transport is activated", DEADLINE, |recorded| {
            recorded
                .iter()
                .any(|recorded| recorded.bus == "session" && recorded.member == "Activate")
        })
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
    harness
        .simulated_phone()
        .wait_for_recorded("the Call is hung up", DEADLINE, |recorded| {
            recorded
                .iter()
                .any(|recorded| recorded.path == path && recorded.member == "Hangup")
        });
}

#[test]
fn hanging_up_shows_call_ended_then_closes_the_call_window() {
    let harness = connected_on_screen();
    let (path, call) = ring_until_banner(&harness);
    harness.answer(call);
    harness.wait_for_app("the Call Window shows a timer", DEADLINE, |state| {
        call_window(state)
            .and_then(|window| window.get("timer"))
            .is_some_and(|status| is_timer(status))
    });
    let hang_up = find_on_screen(&harness, "call-window", RED);

    click(&harness, hang_up);

    harness.wait_for_app("the Call Window says Call ended", DEADLINE, |state| {
        state.calls.is_empty()
            && call_window(state)
                .and_then(|window| window.get("state"))
                .is_some_and(|line| line.starts_with("Call ended"))
    });
    harness.wait_for_app("the Call Window closes", DEADLINE, |state| {
        call_window(state).is_none()
    });
    harness.simulated_phone().wait_for_recorded(
        "the Call is hung up, then RejectSCO is set back to true",
        DEADLINE,
        |recorded| {
            recorded
                .iter()
                .position(|recorded| recorded.path == path && recorded.member == "Hangup")
                .is_some_and(|asked_at| {
                    recorded[asked_at..].iter().any(|recorded| {
                        recorded.member == "Set"
                            && recorded.arguments.contains("RejectSCO")
                            && recorded.arguments.contains("true")
                    })
                })
        },
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
                .and_then(|window| window.get("timer"))
                .is_some_and(|status| is_timer(status))
    });
    assert_eq!(
        call_window(&state).and_then(|window| window.get("hd")),
        None
    );
}
