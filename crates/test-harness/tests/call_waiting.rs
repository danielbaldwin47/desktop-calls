//! Scenarios 14 and 15: call waiting. A second Call rings during a live
//! one; Hold & answer, Swap, End & answer and Decline act on it, by the
//! control interface and by keys alone, against the Simulated Phone, which
//! carries them out as PipeWire's `AT+CHLD` does.

use std::collections::BTreeMap;
use std::time::Duration;

use test_harness::vcards::{self, CallKind};
use test_harness::{AppState, BAR_COLOUR, Harness, RecordedMethodCall, Region, Screenshot};

/// How long the app may take to notice a change on the Phone.
const DEADLINE: Duration = Duration::from_secs(5);
/// How long a whole sync, or a Call History refresh, may take.
const SYNC_DEADLINE: Duration = Duration::from_secs(15);
/// How long the screen may take to show a change. One capture takes up to a
/// second on a slow runner.
const SCREEN_DEADLINE: Duration = Duration::from_secs(10);
/// How long something that must not happen is watched for.
const HOLD: Duration = Duration::from_millis(500);

/// Bob, the first caller.
const BOB: &str = "+447700900123";
/// Alice, who calls while the user is on with Bob.
const ALICE: &str = "+447700900456";

/// The Answer and Hold & answer buttons' green, and the Decline and Hang up
/// buttons' red.
const GREEN: [u8; 3] = [0x26, 0xa2, 0x69];
const RED: [u8; 3] = [0xc0, 0x1c, 0x28];

/// Connected, on a screen of its own: the surfaces show only on a screen.
fn connected() -> Harness {
    connect(Harness::builder().headless_screen().start())
}

fn connected_on_screen() -> Harness {
    connect(Harness::builder().headless_screen().omarchy_bar().start())
}

fn connect(harness: Harness) -> Harness {
    harness.simulated_phone().connect();
    harness.wait_for_app("ConnectionState is connected", DEADLINE, |state| {
        state.connection_state == "connected"
    });
    harness
}

/// Bob rings and the user answers on the Desktop. Returns the Call's object
/// path and id.
fn on_a_call(harness: &Harness) -> (String, u32) {
    let path = harness.simulated_phone().ring(BOB);
    let state = harness.wait_for_app("Bob's Call rings", DEADLINE, |state| {
        state.ringing && state.calls.len() == 1
    });
    let call = state.calls[0].id;
    harness.answer(call);
    harness.wait_for_app("Bob's Call is live here", DEADLINE, |state| {
        state.audio_location == "desktop" && state_of(state, call) == Some("active")
    });
    (path, call)
}

/// Alice rings during the Call: waits until the Banner offers the
/// call-waiting options. Returns her Call's object path and id.
fn alice_waits(harness: &Harness) -> (String, u32) {
    let path = harness.simulated_phone().ring(ALICE);
    let state = harness.wait_for_app("Alice's Call waits", DEADLINE, |state| {
        state.calls.len() == 2
            && state.calls[1].state == "waiting"
            && banner(state).is_some_and(|banner| banner.contains_key("options"))
    });
    (path, state.calls[1].id)
}

fn state_of(state: &AppState, call: u32) -> Option<&str> {
    state
        .calls
        .iter()
        .find(|known| known.id == call)
        .map(|known| known.state.as_str())
}

fn banner(state: &AppState) -> Option<&BTreeMap<String, String>> {
    state.surface("banner")
}

fn call_window(state: &AppState, key: &str) -> Option<String> {
    state
        .surface("call-window")
        .and_then(|window| window.get(key).cloned())
}

/// Waits until the Phone has been asked `member` on the Telephony service,
/// and returns every Telephony method call so far.
fn wait_for_phone_asked(harness: &Harness, member: &str) -> Vec<RecordedMethodCall> {
    let telephony = |recorded: &[RecordedMethodCall]| -> Vec<RecordedMethodCall> {
        recorded
            .iter()
            .filter(|call| {
                call.bus == "session" && call.path.starts_with("/org/pipewire/Telephony")
            })
            .cloned()
            .collect()
    };
    let recorded = harness.simulated_phone().wait_for_recorded(
        &format!("the Phone is asked {member}"),
        DEADLINE,
        |recorded| telephony(recorded).iter().any(|call| call.member == member),
    );
    telephony(&recorded)
}

fn hung_up(recorded: &[RecordedMethodCall], path: &str) -> bool {
    recorded
        .iter()
        .any(|call| call.member == "Hangup" && call.path == path)
}

/// Waits until Bob is on hold and Alice live, and the Call Window lists
/// both.
fn wait_for_alice_live_bob_held(harness: &Harness, bob: u32, alice: u32) -> AppState {
    harness.wait_for_app(
        "the Call Window lists Alice live and Bob on hold",
        DEADLINE,
        |state| {
            state_of(state, bob) == Some("held")
                && state_of(state, alice) == Some("active")
                && call_window(state, "call") == Some(alice.to_string())
                && call_window(state, "held-call") == Some(bob.to_string())
        },
    )
}

#[test]
fn a_call_during_a_live_one_waits_with_three_options_and_the_desktop_stays_silent() {
    let harness = connected();
    on_a_call(&harness);

    let path = harness.simulated_phone().ring(ALICE);

    let state = harness.wait_for_app("the Banner shows Alice waiting", DEADLINE, |state| {
        banner(state).is_some()
    });
    let shown = banner(&state).expect("the Banner shows");
    assert_eq!(
        shown.get("options").map(String::as_str),
        Some("Hold & answer, End & answer, Decline")
    );
    assert_eq!(
        shown.get("heading").map(String::as_str),
        Some("Call waiting")
    );
    assert_eq!(
        shown.get("caller").map(String::as_str),
        Some("+44 7700 900456")
    );
    assert!(state.surface("call-window").is_some());
    harness.app_holds("the Desktop doesn't ring", HOLD, |state| !state.ringing);
    assert!(
        harness
            .simulated_phone()
            .calls_on_phone()
            .contains(&(path, "waiting".to_owned()))
    );
}

/// Scenario 14: Hold & answer, Swap, and hanging up one Call leaves the
/// other live.
#[test]
fn scenario_14_hold_and_answer_then_swap_then_hanging_up_one_leaves_the_other_live() {
    let harness = connected();
    let (bob_path, bob) = on_a_call(&harness);
    let (_, alice) = alice_waits(&harness);

    harness
        .hold_and_answer()
        .expect("Hold & answer is passed on");
    wait_for_phone_asked(&harness, "HoldAndAnswer");
    let state = wait_for_alice_live_bob_held(&harness, bob, alice);
    assert_eq!(
        call_window(&state, "held-caller").as_deref(),
        Some("+44 7700 900123")
    );
    assert_eq!(call_window(&state, "swap").as_deref(), Some("Swap"));
    assert!(banner(&state).is_none());

    harness.swap().expect("Swap is passed on");
    wait_for_phone_asked(&harness, "SwapCalls");
    harness.wait_for_app("the marks swap", DEADLINE, |state| {
        state_of(state, bob) == Some("active")
            && state_of(state, alice) == Some("held")
            && call_window(state, "call") == Some(bob.to_string())
            && call_window(state, "held-call") == Some(alice.to_string())
    });

    harness.hang_up(bob);
    let recorded = wait_for_phone_asked(&harness, "Hangup");
    assert!(hung_up(&recorded, &bob_path), "{recorded:#?}");
    let state = harness.wait_for_app("Alice is live again, alone", DEADLINE, |state| {
        state.calls.len() == 1
            && state_of(state, alice) == Some("active")
            && call_window(state, "call") == Some(alice.to_string())
            && call_window(state, "held-call").is_none()
    });
    assert_eq!(state.audio_location, "desktop");
}

/// Scenario 15, first half: End & answer ends the live Call and connects
/// the waiting one.
#[test]
fn scenario_15_end_and_answer_ends_the_live_call_and_connects_the_waiting_one() {
    let harness = connected();
    on_a_call(&harness);
    let (_, alice) = alice_waits(&harness);

    harness.end_and_answer().expect("End & answer is passed on");

    wait_for_phone_asked(&harness, "ReleaseAndAnswer");
    let state = harness.wait_for_app("only Alice's Call is left, live", DEADLINE, |state| {
        state.calls.len() == 1
            && state_of(state, alice) == Some("active")
            && call_window(state, "call") == Some(alice.to_string())
    });
    assert_eq!(state.audio_location, "desktop");
    assert!(banner(&state).is_none());
}

/// Scenario 15, second half: Decline on the waiting Call hangs up only
/// that Call.
#[test]
fn scenario_15_declining_the_waiting_call_hangs_up_only_that_call() {
    let harness = connected();
    let (bob_path, bob) = on_a_call(&harness);
    let (alice_path, alice) = alice_waits(&harness);

    harness.decline(alice);

    let recorded = wait_for_phone_asked(&harness, "Hangup");
    assert!(hung_up(&recorded, &alice_path), "{recorded:#?}");
    assert!(!hung_up(&recorded, &bob_path), "{recorded:#?}");
    let state = harness.wait_for_app("only Bob's Call is left, live", DEADLINE, |state| {
        state.calls.len() == 1 && banner(state).is_none()
    });
    assert_eq!(state_of(&state, bob), Some("active"));
    assert_eq!(call_window(&state, "call"), Some(bob.to_string()));
}

#[test]
fn a_waiting_call_left_to_ring_out_raises_a_missed_call_notification() {
    let harness = Harness::builder().headless_screen().start();
    let phone = harness.simulated_phone();
    phone.set_pbap_folder(
        "pb",
        &vcards::contact("Alice", &[("CELL", "+44 7700 900456")]),
    );
    phone.set_pbap_folder(
        "cch",
        &vcards::call(CallKind::Dialed, Some("+447700900001"), "20261104T090000"),
    );
    let harness = connect(harness);
    harness.wait_for_app("Contacts are synced", SYNC_DEADLINE, |state| {
        state.contacts.state == "ready"
    });
    on_a_call(&harness);
    let (alice_path, _) = alice_waits(&harness);
    let mut history = vcards::call(CallKind::Missed, Some(ALICE), "20261105T100000");
    history.push_str(&vcards::call(
        CallKind::Received,
        Some(BOB),
        "20261105T095900",
    ));
    harness.simulated_phone().set_pbap_folder("cch", &history);

    harness.simulated_phone().caller_hangs_up(&alice_path);

    harness.notifications().wait_for(
        "a Missed Call notification for Alice",
        SYNC_DEADLINE,
        |notification| notification.summary == "Missed call from Alice (mobile)",
    );
    let state = harness.app_state();
    assert_eq!(state.calls.len(), 1);
    assert!(state.surface("call-window").is_some());
}

/// The quirk scenario: the Phone refuses to hold. The user sees a plain
/// message, and both Calls stay as they were.
#[test]
fn when_the_phone_refuses_to_hold_the_user_is_told_and_both_calls_stay_as_they_were() {
    let harness = connected();
    harness.simulated_phone().enable_quirk("hold-refused");
    let (_, bob) = on_a_call(&harness);
    let (_, alice) = alice_waits(&harness);

    harness
        .hold_and_answer()
        .expect("Hold & answer is passed on");

    let error = harness.wait_for_app_error(DEADLINE);
    assert_eq!(error.kind, "hold-and-answer-failed");
    let sentence = "Your phone didn't put your Call on hold, so both Calls are as they were; answer on your phone instead.";
    assert_eq!(error.message, sentence);
    let state = harness.wait_for_app("the Call Window says why", DEADLINE, |state| {
        call_window(state, "notice").as_deref() == Some(sentence)
    });
    assert_eq!(state_of(&state, bob), Some("active"));
    assert_eq!(state_of(&state, alice), Some("waiting"));
    assert!(banner(&state).is_some_and(|banner| banner.contains_key("options")));
    assert!(!state.ringing);
}

#[test]
fn swap_with_no_call_on_hold_is_refused_with_the_reason() {
    let harness = connected();
    on_a_call(&harness);

    let error = harness.swap().expect_err("Swap is refused");

    assert!(error.kind.ends_with("NoHeldCall"), "{error:?}");
    assert_eq!(error.message, "There's no Call on hold to swap to.");
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

/// The keybinding gives the Banner the keyboard; waits until it has it.
fn focus_banner(harness: &Harness) {
    harness.activate();
    harness.wait_for_app("the Banner has the keyboard", DEADLINE, |state| {
        banner(state).is_some_and(|banner| {
            banner.get("focused").map(String::as_str) == Some("true")
                && banner.get("heading").map(String::as_str)
                    == Some("Press H to hold and answer, E to end and answer, or D to decline")
        })
    });
}

/// The ticket's stacking decision: the Banner shows below the Call Window,
/// which stays where it was.
#[test]
fn the_waiting_calls_banner_shows_below_the_call_window_which_stays_put() {
    let harness = connected_on_screen();
    on_a_call(&harness);
    let alone = harness.screenshot_when("call-window", SCREEN_DEADLINE, |screenshot| {
        find(screenshot, RED).is_some()
    });
    let window = app_region(&alone).expect("the Call Window is on screen");
    let hang_up = find(&alone, RED).expect("Hang up is on screen");

    alice_waits(&harness);

    let both = harness.screenshot_when("call-waiting-banner", SCREEN_DEADLINE, |screenshot| {
        find(screenshot, GREEN).is_some()
    });
    let hold_and_answer = find(&both, GREEN).expect("Hold & answer is on screen");
    assert!(
        hold_and_answer.top > window.bottom,
        "the Banner's Hold & answer {hold_and_answer:?} overlaps the Call Window {window:?}"
    );
    let red = find(&both, RED).expect("Hang up is on screen");
    assert!(
        (red.top - hang_up.top).abs() < 1.0,
        "Hang up moved from {hang_up:?} to {red:?}"
    );
}

/// Polish scorecard item 7: Hold & answer, Swap and Hang up for the held
/// Call by keys alone, with the keybinding to reach each surface.
#[test]
fn hold_and_answer_swap_and_hanging_up_the_held_call_work_by_keys_alone() {
    let harness = connected_on_screen();
    let (_, bob) = on_a_call(&harness);
    let (alice_path, alice) = alice_waits(&harness);

    focus_banner(&harness);
    harness.keyboard().type_text("h");
    wait_for_phone_asked(&harness, "HoldAndAnswer");
    wait_for_alice_live_bob_held(&harness, bob, alice);
    // The Banner is gone, and the Call Window shows both Calls.
    let _two_calls = harness.screenshot_when("two-calls", SCREEN_DEADLINE, |screenshot| {
        find(screenshot, RED).is_some() && find(screenshot, GREEN).is_none()
    });

    harness.activate();
    harness.wait_for_app("the Call Window has the keyboard", DEADLINE, |state| {
        call_window(state, "focused").as_deref() == Some("true")
    });
    harness.keyboard().type_text("s");
    wait_for_phone_asked(&harness, "SwapCalls");
    harness.wait_for_app("Bob is live and Alice on hold", DEADLINE, |state| {
        state_of(state, bob) == Some("active") && state_of(state, alice) == Some("held")
    });

    harness.keyboard().press_with_control('H');
    let recorded = wait_for_phone_asked(&harness, "Hangup");
    assert!(hung_up(&recorded, &alice_path), "{recorded:#?}");
    harness.wait_for_app("only Bob's Call is left", DEADLINE, |state| {
        state.calls.len() == 1 && state_of(state, bob) == Some("active")
    });
}

/// Polish scorecard item 7: End & answer and Decline on a waiting Call by
/// keys alone.
#[test]
fn end_and_answer_and_decline_work_by_keys_alone() {
    let harness = connected_on_screen();
    on_a_call(&harness);
    let (_, alice) = alice_waits(&harness);

    focus_banner(&harness);
    harness.keyboard().type_text("e");
    wait_for_phone_asked(&harness, "ReleaseAndAnswer");
    harness.wait_for_app("only Alice's Call is left, live", DEADLINE, |state| {
        state.calls.len() == 1 && state_of(state, alice) == Some("active")
    });

    let carol = harness.simulated_phone().ring("+447700900789");
    harness.wait_for_app("Carol's Call waits", DEADLINE, |state| {
        state.calls.len() == 2 && banner(state).is_some_and(|banner| banner.contains_key("options"))
    });
    focus_banner(&harness);
    harness.keyboard().type_text("d");
    let recorded = wait_for_phone_asked(&harness, "Hangup");
    assert!(hung_up(&recorded, &carol), "{recorded:#?}");
    harness.wait_for_app("only Alice's Call is left", DEADLINE, |state| {
        state.calls.len() == 1 && banner(state).is_none()
    });
}
