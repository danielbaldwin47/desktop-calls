//! Scenarios: the app on a screen of its own, captured as PNG, and the
//! harness's synthetic keyboard and pointer.

use std::sync::Barrier;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::Duration;

use test_harness::{Harness, Key, Screenshot};

/// How long the app, or a client, may take to notice a change.
const DEADLINE: Duration = Duration::from_secs(5);

#[test]
fn the_app_runs_in_the_headless_compositor_and_its_screen_is_captured() {
    let harness = Harness::builder().headless_screen().start();
    harness.simulated_phone().connect();
    harness.wait_for_app("ConnectionState is connected", DEADLINE, |state| {
        state.connection_state == "connected"
    });

    let screenshot = harness.screenshot("connected");

    // The headless output matches the user's monitor: 3840 by 2160 pixels.
    assert_eq!((screenshot.width, screenshot.height), (3840, 2160));
    let png = std::fs::read(&screenshot.path).expect("the screenshot is kept");
    assert!(
        png.starts_with(b"\x89PNG\r\n\x1a\n"),
        "{} is not a PNG",
        screenshot.path.display()
    );
}

#[test]
fn synthetic_keys_reach_the_focused_client() {
    let harness = Harness::builder().headless_screen().start();
    let client = harness.start_test_client();
    client.wait_for("the test client has keyboard focus", DEADLINE, |line| {
        line == "keyboard-enter"
    });

    harness.keyboard().type_text("Hi");
    harness.keyboard().press(Key::Return);

    client.wait_for("Return is released", DEADLINE, |line| {
        line == "key 28 released"
    });
    let events: Vec<String> = client
        .lines()
        .into_iter()
        .filter(|line| line.starts_with("key ") || line.starts_with("modifiers "))
        .collect();
    // Key codes from linux/input-event-codes.h: Left Shift 42, H 35, I 23,
    // Enter 28. Modifier mask 1 is Shift.
    assert_eq!(
        events,
        [
            "key 42 pressed",
            "modifiers 1",
            "key 35 pressed",
            "key 35 released",
            "key 42 released",
            "modifiers 0",
            "key 23 pressed",
            "key 23 released",
            "key 28 pressed",
            "key 28 released",
        ]
    );
}

#[test]
fn a_synthetic_click_reaches_the_client_under_the_pointer() {
    let harness = Harness::builder().headless_screen().start();
    let client = harness.start_test_client();
    client.wait_for("the test client fills the screen", DEADLINE, |line| {
        line.starts_with("configured ")
    });

    // The middle of the screen, in the 2400 by 1350 layout a 3840 by 2160
    // output has at scale 1.6. The only window fills the screen.
    harness.pointer().click(1200.0, 675.0);

    client.wait_for("the click is released", DEADLINE, |line| {
        line.starts_with("button 272 released")
    });
    // Button code 272 is BTN_LEFT in linux/input-event-codes.h.
    assert!(
        client
            .lines()
            .iter()
            .any(|line| line.starts_with("button 272 pressed")),
        "the client saw no press: {:#?}",
        client.lines()
    );
}

/// How long the brief state stays on screen.
const BRIEFLY: Duration = Duration::from_millis(200);

/// How long the compositor may take to take a closed window off screen.
const CLOSING: Duration = Duration::from_millis(500);

/// The test client's window colour, RGB 0x80 0x80 0x80.
const CLIENT_GREY: [u8; 3] = [0x80; 3];

/// A state on screen for only 200 ms, from start to end while `shown` is
/// still judging the first frame, is captured: frames are recorded as the
/// screen changes, not taken once the last judgement ends. "Call ended"
/// shows for 2 s, which the gallery missed while it sampled the screen (#42).
#[test]
fn screenshot_when_catches_a_state_shown_for_200_ms() {
    let harness = Harness::builder().headless_screen().start();
    let shows_client = |screenshot: &Screenshot| {
        screenshot
            .region_where(|pixel| pixel == CLIENT_GREY)
            .is_some()
    };
    // Met twice: when the first judgement starts, and once the window has
    // come and gone.
    let judging = Barrier::new(2);
    let first = AtomicBool::new(true);

    thread::scope(|scope| {
        scope.spawn(|| {
            judging.wait();
            let client = harness.start_test_client();
            client.wait_for("the test client has drawn", DEADLINE, |line| {
                line.starts_with("configured")
            });
            thread::sleep(BRIEFLY);
            // Dropping the client closes its window.
            drop(client);
            thread::sleep(CLOSING);
            judging.wait();
        });
        // Panics unless a frame shows the window.
        let screenshot = harness.screenshot_when("brief-window", DEADLINE * 2, |screenshot| {
            if first.swap(false, Ordering::Relaxed) {
                judging.wait();
                judging.wait();
            }
            shows_client(screenshot)
        });
        assert!(shows_client(&screenshot), "{}", screenshot.path.display());
    });
}
