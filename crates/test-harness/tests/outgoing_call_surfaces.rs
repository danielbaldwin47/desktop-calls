//! Scenarios: the Call Window while the user places a Call, on a screen of
//! its own, checked through the control interface's `Surfaces` and on
//! screenshots, and clicked with a synthetic pointer.
//!
//! Each step waits for what the app reports before the Simulated Phone moves
//! on, so a slow CI runner only makes a scenario slower, never a race.

use std::collections::BTreeMap;
use std::time::Duration;

use test_harness::{AppState, Harness, Region, RunningCli};

/// How long the app may take to show a change.
const DEADLINE: Duration = Duration::from_secs(5);

/// The number as the user types it, and as the Call Window shows it.
const TYPED: &str = "+447700900123";
const SHOWN: &str = "+44 7700 900123";

/// The Hang up button's red.
const RED: [u8; 3] = [0xc0, 0x1c, 0x28];

fn connected_on_screen() -> Harness {
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

fn shown<'a>(state: &'a AppState, key: &str) -> Option<&'a str> {
    call_window(state)
        .and_then(|window| window.get(key))
        .map(String::as_str)
}

/// The Call Window's state line.
fn status_of(state: &AppState) -> Option<&str> {
    shown(state, "state")
}

fn wait_for_status(harness: &Harness, status: &str) -> AppState {
    harness.wait_for_app(
        &format!("the Call Window says {status}"),
        DEADLINE,
        |state| status_of(state) == Some(status),
    )
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

/// Every Call Window state line the app reported since the scenario started.
fn statuses_seen(harness: &Harness) -> Vec<String> {
    harness
        .surfaces_seen()
        .iter()
        .filter_map(|surfaces| surfaces.get("call-window")?.get("state").cloned())
        .collect()
}

/// Starts the CLI dialling `number`, and waits until the Phone is asked to
/// dial, so the Simulated Phone can `Alert`. The CLI runs until the Phone
/// reports the Call or refuses it.
fn dial(harness: &Harness, number: &str) -> RunningCli {
    let cli = harness.start_cli(&["dial", number]);
    harness
        .simulated_phone()
        .wait_for_recorded("the Phone is asked to dial", DEADLINE, |calls| {
            calls.iter().any(|call| call.member == "Dial")
        });
    cli
}

/// The controls the Call Window can show besides Hang up.
const CONTROLS: [&str; 6] = ["mute", "keypad-toggle", "handoff", "tone-entry", "keypad", "swap"];

/// Placing a Call, the Call Window offers Hang up and nothing that wouldn't
/// work yet.
fn assert_offers_only_hang_up(window: &BTreeMap<String, String>) {
    assert_eq!(window.get("hang-up").map(String::as_str), Some("Hang up"), "{window:?}");
    for key in CONTROLS {
        assert!(!window.contains_key(key), "{key}: {window:?}");
    }
}

/// Once there is no Call, the Call Window offers nothing at all.
fn assert_offers_nothing(window: &BTreeMap<String, String>) {
    for key in CONTROLS.iter().chain(&["hang-up"]) {
        assert!(!window.contains_key(*key), "{key}: {window:?}");
    }
}

fn stderr(output: &std::process::Output) -> String {
    String::from_utf8_lossy(&output.stderr).trim().to_owned()
}

#[test]
fn a_placed_call_shows_calling_then_ringing_then_a_timer_with_the_audio_here() {
    let harness = connected_on_screen();

    let cli = dial(&harness, TYPED);

    let state = wait_for_status(&harness, "Calling…");
    let window = call_window(&state).expect("the Call Window shows");
    assert_eq!(window.get("caller").map(String::as_str), Some(SHOWN));
    assert_eq!(window.get("notice"), None);
    assert_offers_only_hang_up(window);
    find_on_screen(&harness, "calling", RED);

    let _call = harness.simulated_phone().alert();
    let state = wait_for_status(&harness, "Ringing…");
    assert_offers_only_hang_up(call_window(&state).expect("the Call Window shows"));
    let output = cli.finish(DEADLINE);
    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(state.calls[0].direction, "outgoing");
    assert_eq!(
        call_window(&state)
            .and_then(|window| window.get("caller"))
            .map(String::as_str),
        Some(SHOWN)
    );

    harness.simulated_phone().callee_answers();
    let state = harness.wait_for_app("the Call Window shows a timer", DEADLINE, |state| {
        shown(state, "timer").is_some_and(is_timer)
    });
    assert_eq!(state.audio_location, "desktop");
    assert_eq!(status_of(&state), Some("On this computer"));
    assert_eq!(shown(&state, "handoff"), Some("To phone"));
    assert_eq!(
        call_window(&state).and_then(|window| window.get("call")),
        Some(&state.calls[0].id.to_string())
    );
}

#[test]
fn hanging_up_while_calling_says_call_ended_and_hangs_the_call_up_when_it_appears() {
    let harness = connected_on_screen();
    let _cli = dial(&harness, TYPED);
    wait_for_status(&harness, "Calling…");
    let hang_up = find_on_screen(&harness, "calling", RED);

    let (x, y) = hang_up.centre();
    harness.pointer().click(x, y);

    wait_for_status(&harness, "Call ended");
    let path = harness.simulated_phone().alert();
    harness.wait_for_app("the dialled Call is hung up", DEADLINE, |state| {
        state.calls.is_empty()
    });
    harness.wait_for_app("the Call Window closes", DEADLINE, |state| {
        call_window(state).is_none()
    });
    harness.simulated_phone().wait_for_recorded(
        "the Phone is asked to hang up the dialled Call",
        DEADLINE,
        |calls| {
            calls
                .iter()
                .any(|recorded| recorded.path == path && recorded.member == "Hangup")
        },
    );
    let seen = statuses_seen(&harness);
    assert!(!seen.iter().any(|status| status == "Ringing…"), "{seen:?}");
}

#[test]
fn a_refused_call_shows_why_in_the_call_window() {
    let harness = connected_on_screen();
    harness.simulated_phone().disconnect();
    harness.wait_for_app("ConnectionState is disconnected", DEADLINE, |state| {
        state.connection_state == "disconnected"
    });

    let output = harness.cli(&["dial", TYPED]);

    assert!(!output.status.success());
    let state = harness.wait_for_app("the Call Window says Call not placed", DEADLINE, |state| {
        shown(state, "notice-heading") == Some("Call not placed")
    });
    let window = call_window(&state).expect("the Call Window shows");
    assert_eq!(window.get("caller").map(String::as_str), Some(SHOWN));
    assert_offers_nothing(window);
    assert_eq!(
        window.get("notice").map(String::as_str),
        Some("Phone not connected: it's out of range or has Bluetooth off.")
    );
    assert_eq!(window.get("call"), None);
}

#[test]
fn each_phone_failure_shows_one_plain_sentence_in_the_call_window() {
    // Each differs from the one before, so each wait sees its own.
    let failures = [
        (
            "invalid-format",
            "Your phone can't dial that number; check it and try again.",
        ),
        (
            "invalid-state",
            "Your phone is busy with another Call; try again in a moment.",
        ),
        (
            "not-supported",
            "Your phone doesn't let this computer place Calls; call from your phone instead.",
        ),
        (
            "failed",
            "Your phone couldn't place the Call; check it has signal and isn't in airplane mode.",
        ),
        (
            "cme:30",
            "Your phone has no mobile signal, or is in airplane mode.",
        ),
        (
            "cme:31",
            "The mobile network didn't answer your phone; try again in a moment.",
        ),
    ];
    let harness = connected_on_screen();

    for (error, sentence) in failures {
        harness.simulated_phone().refuse_next_dial(error);

        let output = harness.cli(&["dial", TYPED]);

        assert!(
            !output.status.success(),
            "{error}: the CLI reported success"
        );
        assert_eq!(stderr(&output), sentence, "{error}");
        harness.wait_for_app(
            &format!("the Call Window explains {error}"),
            DEADLINE,
            |state| {
                shown(state, "notice-heading") == Some("Call not placed")
                    && call_window(state)
                        .and_then(|window| window.get("notice"))
                        .is_some_and(|notice| notice == sentence)
            },
        );
    }
}

#[test]
fn while_get_calls_is_empty_during_dialling_the_call_window_never_says_call_ended() {
    let harness = connected_on_screen();
    harness
        .simulated_phone()
        .enable_quirk("get-calls-empty-while-dialling");

    let _cli = dial(&harness, TYPED);
    wait_for_status(&harness, "Calling…");
    harness.app_holds(
        "the Call Window says Calling…",
        Duration::from_secs(1),
        |state| status_of(state) == Some("Calling…"),
    );
    let _call = harness.simulated_phone().alert();
    wait_for_status(&harness, "Ringing…");
    harness.app_holds(
        "the Call Window says Ringing…",
        Duration::from_secs(1),
        |state| status_of(state) == Some("Ringing…"),
    );
    harness.simulated_phone().callee_answers();
    harness.wait_for_app("the Call Window shows a timer", DEADLINE, |state| {
        shown(state, "timer").is_some_and(is_timer)
    });

    let seen = statuses_seen(&harness);
    assert!(
        !seen.iter().any(|status| status.starts_with("Call ended")),
        "{seen:?}"
    );
    assert_eq!(
        seen.first().map(String::as_str),
        Some("Calling…"),
        "{seen:?}"
    );
}

#[test]
fn a_call_that_ends_says_how_long_it_lasted_and_offers_nothing() {
    let harness = connected_on_screen();
    let cli = dial(&harness, TYPED);
    let call = harness.simulated_phone().alert();
    wait_for_status(&harness, "Ringing…");
    let _ = cli.finish(DEADLINE);
    harness.simulated_phone().callee_answers();
    harness.wait_for_app("the Call Window shows a timer", DEADLINE, |state| {
        shown(state, "timer").is_some_and(is_timer)
    });

    harness.simulated_phone().caller_hangs_up(&call);

    let state = harness.wait_for_app("the Call Window says Call ended", DEADLINE, |state| {
        status_of(state).is_some_and(|line| line.starts_with("Call ended"))
    });
    let line = status_of(&state).unwrap_or_default();
    let lasted = line.strip_prefix("Call ended · ");
    assert!(lasted.is_some_and(is_timer), "{line}");
    assert_offers_nothing(call_window(&state).expect("the Call Window shows"));
    assert_eq!(shown(&state, "timer"), None);
}
