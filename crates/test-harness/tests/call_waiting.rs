//! Scenarios 14 and 15: call waiting. A second Call rings during a live
//! one; Hold & Answer, Swap, End & Answer and Decline act on it, by the
//! control interface and by keys alone, against the Simulated Phone, which
//! carries them out as PipeWire's `AT+CHLD` does.

use std::collections::BTreeMap;
use std::time::Duration;

use test_harness::vcards::{self, CallKind};
use test_harness::{AppState, BAR_COLOUR, Bus, Harness, RecordedMethodCall, Region, Screenshot};

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

/// The Answer and Hold & Answer buttons' green, and the Decline and Hang up
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
/// Waiting Call's options. Returns her Call's object path and id.
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
        Some("Hold & Answer, End & Answer, Decline")
    );
    assert_eq!(
        shown.get("heading").map(String::as_str),
        Some("Waiting Call")
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

/// Scenario 14: Hold & Answer, Swap, and hanging up one Call leaves the
/// other live.
#[test]
fn scenario_14_hold_and_answer_then_swap_then_hanging_up_one_leaves_the_other_live() {
    let harness = connected();
    let (bob_path, bob) = on_a_call(&harness);
    let (_, alice) = alice_waits(&harness);

    harness
        .hold_and_answer()
        .expect("Hold & Answer is passed on");
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

/// Scenario 15, first half: End & Answer ends the live Call and connects
/// the waiting one.
#[test]
fn scenario_15_end_and_answer_ends_the_live_call_and_connects_the_waiting_one() {
    let harness = connected();
    on_a_call(&harness);
    let (_, alice) = alice_waits(&harness);

    harness.end_and_answer().expect("End & Answer is passed on");

    wait_for_phone_asked(&harness, "ReleaseAndAnswer");
    let state = harness.wait_for_app("only Alice's Call is left, live", DEADLINE, |state| {
        state.calls.len() == 1
            && state_of(state, alice) == Some("active")
            && call_window(state, "call") == Some(alice.to_string())
    });
    assert_eq!(state.audio_location, "desktop");
    assert!(banner(&state).is_none());
}

/// Scenario 15, second half: Decline on the Waiting Call hangs up only
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

/// The quirk scenario: the Phone refuses to hold, as one that didn't list
/// `AT+CHLD` support does. The user sees a plain message, and both Calls
/// stay as they were.
#[test]
fn when_the_phone_refuses_to_hold_the_user_is_told_and_both_calls_stay_as_they_were() {
    let harness = connected();
    harness.simulated_phone().enable_quirk("chld-unsupported");
    let (_, bob) = on_a_call(&harness);
    let (_, alice) = alice_waits(&harness);

    harness
        .hold_and_answer()
        .expect("Hold & Answer is passed on");

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
    focus_banner_saying(
        harness,
        "Press H to Hold & Answer, E to End & Answer, or D to decline",
    );
}

/// [`focus_banner`], for a Banner whose heading then says `heading`.
fn focus_banner_saying(harness: &Harness, heading: &str) {
    harness.activate();
    harness.wait_for_app("the Banner has the keyboard", DEADLINE, |state| {
        banner(state).is_some_and(|banner| {
            banner.get("focused").map(String::as_str) == Some("true")
                && banner.get("heading").map(String::as_str) == Some(heading)
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
    let hold_and_answer = find(&both, GREEN).expect("Hold & Answer is on screen");
    assert!(
        hold_and_answer.top > window.bottom,
        "the Banner's Hold & Answer {hold_and_answer:?} overlaps the Call Window {window:?}"
    );
    let red = find(&both, RED).expect("Hang up is on screen");
    assert!(
        (red.top - hang_up.top).abs() < 1.0,
        "Hang up moved from {hang_up:?} to {red:?}"
    );
}

/// Polish scorecard item 7: Hold & Answer, Swap and Hang up for the held
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

/// Polish scorecard item 7: End & Answer and Decline on a Waiting Call by
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

/// The quirk scenario for End & Answer: the Phone refuses to end the live
/// Call. The user is told, both Calls stay as they were, and the Banner
/// offers the Waiting Call again.
#[test]
fn when_the_phone_refuses_end_and_answer_the_user_is_told_and_the_banner_comes_back() {
    let harness = connected();
    harness.simulated_phone().enable_quirk("chld-unsupported");
    let (_, bob) = on_a_call(&harness);
    let (_, alice) = alice_waits(&harness);

    harness.end_and_answer().expect("End & Answer is passed on");

    let error = harness.wait_for_app_error(DEADLINE);
    assert_eq!(error.kind, "end-and-answer-failed");
    let sentence = "Your phone didn't end your Call, so both Calls are as they were; end it on your phone instead.";
    assert_eq!(error.message, sentence);
    let state = harness.wait_for_app(
        "the Call Window says why and the Banner is back",
        DEADLINE,
        |state| {
            call_window(state, "notice").as_deref() == Some(sentence)
                && banner(state).is_some_and(|banner| banner.contains_key("options"))
        },
    );
    assert_eq!(state_of(&state, bob), Some("active"));
    assert_eq!(state_of(&state, alice), Some("waiting"));
    assert!(!state.ringing);
}

/// Carol, who rings while Alice is live and Bob on hold.
const CAROL: &str = "+447700900789";

/// What the Call Window says, and a refused Hang up or Swap, while a Call is
/// on hold and another waits.
const WAITING_CALL_FIRST: &str = "Answer or decline the Waiting Call first: until then, your phone would act on it instead of these Calls.";

/// Alice live, Bob on hold, and Carol waiting, the Banner showing her
/// options. Returns the Calls' object paths and ids: Bob's, Alice's, then
/// Carol's.
fn three_calls(harness: &Harness) -> [(String, u32); 3] {
    let bob = on_a_call(harness);
    let alice = alice_waits(harness);
    let held = harness.hold_and_answer().map_err(|error| error.kind);
    assert_eq!(held, Ok(()), "Hold & Answer is passed on");
    wait_for_alice_live_bob_held(harness, bob.1, alice.1);
    let carol_path = harness.simulated_phone().ring(CAROL);
    let state = harness.wait_for_app("Carol's Call waits", DEADLINE, |state| {
        state.calls.len() == 3 && banner(state).is_some_and(|banner| banner.contains_key("options"))
    });
    [bob, alice, (carol_path, state.calls[2].id)]
}

/// Three Calls: Alice live, Bob on hold, and Carol waiting. With a Call
/// waiting, the Phone's `AT+CHLD=0` (Hang up on the Held Call) would decline
/// Carol and its `AT+CHLD=2` (Swap) answer her, so the Call Window offers
/// neither, and says why.
#[test]
fn while_a_third_call_waits_the_held_call_offers_neither_swap_nor_hang_up() {
    let harness = connected();
    let (bob_path, bob) = on_a_call(&harness);
    let (_, alice) = alice_waits(&harness);
    harness
        .hold_and_answer()
        .expect("Hold & Answer is passed on");
    wait_for_alice_live_bob_held(&harness, bob, alice);

    let carol_path = harness.simulated_phone().ring(CAROL);
    let sentence = WAITING_CALL_FIRST;
    let state = harness.wait_for_app(
        "Carol waits and the Call Window says why Bob can't be hung up",
        DEADLINE,
        |state| {
            state.calls.len() == 3
                && banner(state).is_some_and(|banner| banner.contains_key("options"))
                && call_window(state, "held-waiting-first").as_deref() == Some(sentence)
        },
    );
    assert_eq!(call_window(&state, "held-call"), Some(bob.to_string()));
    assert_eq!(call_window(&state, "hang-up-held-shortcut"), None);
    assert_eq!(call_window(&state, "swap"), None);

    harness.hang_up(bob);
    let error = harness.wait_for_app_error(DEADLINE);
    assert_eq!(error.kind, "waiting-call-first");
    assert_eq!(error.message, sentence);
    let error = harness.swap().expect_err("Swap is refused");
    assert!(error.kind.ends_with("WaitingCallFirst"), "{error:?}");
    let state = harness.app_state();
    assert_eq!(state.calls.len(), 3, "{:?}", state.calls);
    assert_eq!(state_of(&state, bob), Some("held"));
    let on_phone = harness.simulated_phone().calls_on_phone();
    assert!(
        on_phone.contains(&(carol_path.clone(), "waiting".to_owned())),
        "Carol still waits: {on_phone:?}"
    );
    harness.simulated_phone().recorded_holds(
        "the Phone is never asked to hang up Bob",
        HOLD,
        |recorded| !hung_up(recorded, &bob_path),
    );

    // Once Carol is declined, Bob can be swapped to or hung up again.
    harness.decline(state.calls[2].id);
    harness.wait_for_app("the Call Window offers Swap again", DEADLINE, |state| {
        state.calls.len() == 2
            && call_window(state, "swap").as_deref() == Some("Swap")
            && call_window(state, "held-waiting-first").is_none()
    });
}

/// The Simulated Phone carries out `Hangup` on a Held Call as PipeWire's
/// `AT+CHLD=0` does in 3GPP TS 27.007: with a Call waiting, it is the
/// Waiting Call that ends, and the held one stays.
#[test]
fn on_the_simulated_phone_hanging_up_a_held_call_while_one_waits_declines_the_waiting_one() {
    let harness = connected();
    let (bob_path, bob) = on_a_call(&harness);
    let (_, alice) = alice_waits(&harness);
    harness
        .hold_and_answer()
        .expect("Hold & Answer is passed on");
    wait_for_alice_live_bob_held(&harness, bob, alice);
    let carol_path = harness.simulated_phone().ring(CAROL);
    harness.wait_for_app("Carol waits", DEADLINE, |state| state.calls.len() == 3);

    harness.call_phone_method(
        Bus::Session,
        "org.pipewire.Telephony",
        &bob_path,
        "org.pipewire.Telephony.Call1",
        "Hangup",
        &(),
    );

    let on_phone = harness.simulated_phone().calls_on_phone();
    assert!(
        on_phone.contains(&(bob_path, "held".to_owned())),
        "Bob stays on hold: {on_phone:?}"
    );
    assert!(
        !on_phone.iter().any(|(path, _)| *path == carol_path),
        "Carol's Call ended: {on_phone:?}"
    );
}

/// A Call that rings while the user's own Call is still ringing at the
/// other end is `waiting`, and the Phone won't `Answer` it. The Banner
/// offers End & Answer and Decline; End & Answer hangs up the Call being
/// placed, and answers the new one once the Phone presents it as ringing.
#[test]
fn a_call_ringing_while_one_is_being_placed_is_answered_by_end_and_answer() {
    let harness = connected();
    let dial = harness.start_dial("07700900123");
    wait_for_phone_asked(&harness, "Dial");
    let placing_path = harness.simulated_phone().alert();
    dial.outcome(DEADLINE).expect("the Call is placed");
    let placing = harness
        .wait_for_app("the Call rings at the other end", DEADLINE, |state| {
            state.calls.len() == 1 && state.calls[0].state == "alerting"
        })
        .calls[0]
        .id;

    let path = harness.simulated_phone().ring(ALICE);
    let state = harness.wait_for_app("Alice's Call waits", DEADLINE, |state| {
        state.calls.len() == 2 && banner(state).is_some_and(|banner| banner.contains_key("options"))
    });
    let alice = state
        .calls
        .iter()
        .find(|call| call.id != placing)
        .expect("Alice's Call")
        .clone();
    assert_eq!(alice.state, "waiting");
    assert_eq!(
        banner(&state)
            .and_then(|banner| banner.get("options"))
            .map(String::as_str),
        Some("End & Answer, Decline")
    );
    assert!(!state.ringing);
    let error = harness
        .hold_and_answer()
        .expect_err("a Call not yet connected can't be held");
    assert!(error.kind.ends_with("NotConnectedYet"), "{error:?}");

    harness.end_and_answer().expect("End & Answer is passed on");

    let recorded = wait_for_phone_asked(&harness, "Answer");
    assert!(hung_up(&recorded, &placing_path), "{recorded:#?}");
    assert!(
        recorded
            .iter()
            .any(|call| call.member == "Answer" && call.path == path),
        "{recorded:#?}"
    );
    assert!(
        !recorded
            .iter()
            .any(|call| call.member == "ReleaseAndAnswer"),
        "{recorded:#?}"
    );
    harness.wait_for_app("only Alice's Call is left, live here", DEADLINE, |state| {
        state.calls.len() == 1
            && state_of(state, alice.id) == Some("active")
            && state.audio_location == "desktop"
            && banner(state).is_none()
    });
}

/// With Bob on hold, the Phone holds no second Call, so for Carol the Banner
/// offers End & Answer and Decline alone, and H, or A, holds nobody; E ends
/// only Alice, live, and answers Carol, Bob staying on hold.
#[test]
fn with_a_call_on_hold_a_third_call_offers_only_end_and_answer_which_ends_only_the_live_call() {
    let harness = connected_on_screen();
    let [(bob_path, bob), (alice_path, alice), (carol_path, carol)] = three_calls(&harness);

    let state = harness.app_state();
    let options = banner(&state).and_then(|banner| banner.get("options").cloned());
    assert_eq!(options.as_deref(), Some("End & Answer, Decline"));
    let error = harness
        .hold_and_answer()
        .expect_err("Hold & Answer is refused");
    assert!(error.kind.ends_with("AlreadyHolding"), "{error:?}");

    let asked = |recorded: &[RecordedMethodCall]| {
        recorded
            .iter()
            .filter(|call| {
                matches!(
                    call.member.as_str(),
                    "HoldAndAnswer" | "ReleaseAndAnswer" | "SwapCalls" | "Hangup"
                )
            })
            .count()
    };
    let before = asked(&harness.simulated_phone().wait_for_recorded(
        "what the Phone was asked so far",
        DEADLINE,
        |_| true,
    ));
    focus_banner_saying(&harness, "Press E to End & Answer, or D to decline");
    harness.keyboard().type_text("ha");
    harness.simulated_phone().recorded_holds(
        "neither H nor A asks the Phone anything",
        HOLD,
        |recorded| asked(recorded) == before,
    );
    harness.keyboard().type_text("e");
    let recorded = wait_for_phone_asked(&harness, "ReleaseAndAnswer");
    assert!(!hung_up(&recorded, &bob_path), "{recorded:#?}");
    harness.wait_for_app(
        "Alice's Call is gone, Carol live and Bob still on hold",
        DEADLINE,
        |state| {
            state_of(state, alice).is_none()
                && state_of(state, carol) == Some("active")
                && state_of(state, bob) == Some("held")
                && call_window(state, "held-call") == Some(bob.to_string())
        },
    );
    let on_phone = harness.simulated_phone().calls_on_phone();
    assert!(
        on_phone.contains(&(bob_path, "held".to_owned()))
            && on_phone.contains(&(carol_path, "active".to_owned()))
            && !on_phone.iter().any(|(path, _)| *path == alice_path),
        "{on_phone:?}"
    );
}

/// Hang up on Alice, live beside Bob on hold, is PipeWire's `AT+CHLD=1`,
/// which would answer Carol unasked: it is refused, and the Phone asked
/// nothing.
#[test]
fn hanging_up_the_live_call_while_one_is_held_and_one_waits_is_refused() {
    let harness = connected();
    let [(_, bob), (alice_path, alice), (carol_path, _)] = three_calls(&harness);

    harness.hang_up(alice);
    let error = harness.wait_for_app_error(DEADLINE);
    assert_eq!(error.kind, "waiting-call-first");
    assert_eq!(error.message, WAITING_CALL_FIRST);
    harness.simulated_phone().recorded_holds(
        "the Phone is never asked to hang up Alice",
        HOLD,
        |recorded| !hung_up(recorded, &alice_path),
    );
    let state = harness.app_state();
    assert_eq!(state_of(&state, alice), Some("active"));
    assert_eq!(state_of(&state, bob), Some("held"));
    let on_phone = harness.simulated_phone().calls_on_phone();
    assert!(
        on_phone.contains(&(carol_path, "waiting".to_owned())),
        "Carol still waits: {on_phone:?}"
    );
}

/// Bob on hold alone, as Alice hung up, when Carol calls. Nothing is live to
/// hold or end, so the Banner offers Answer, which PipeWire carries out as
/// `SwapCalls` (`AT+CHLD=2`): Carol is answered and Bob stays on hold.
#[test]
fn with_only_a_held_call_answer_answers_the_waiting_call_and_keeps_the_held_one() {
    let harness = connected();
    let (bob_path, bob) = on_a_call(&harness);
    let (alice_path, alice) = alice_waits(&harness);
    harness
        .hold_and_answer()
        .expect("Hold & Answer is passed on");
    wait_for_alice_live_bob_held(&harness, bob, alice);
    harness.simulated_phone().caller_hangs_up(&alice_path);
    harness.wait_for_app("only Bob's Call is left, on hold", DEADLINE, |state| {
        state.calls.len() == 1 && state_of(state, bob) == Some("held")
    });

    let carol_path = harness.simulated_phone().ring(CAROL);
    let state = harness.wait_for_app(
        "Carol waits, offered Answer, and the Call Window says why Bob can't be resumed",
        DEADLINE,
        |state| {
            state.calls.len() == 2
                && banner(state)
                    .and_then(|banner| banner.get("options"))
                    .is_some_and(|options| options == "Answer, Decline")
                && call_window(state, "held-waiting-first").as_deref() == Some(WAITING_CALL_FIRST)
        },
    );
    let carol = state.calls[1].id;
    assert_eq!(call_window(&state, "swap"), None);
    for refused in [harness.hold_and_answer(), harness.end_and_answer()] {
        let error = refused.expect_err("nothing is live to hold or end");
        assert!(error.kind.ends_with("NothingToHoldOrEnd"), "{error:?}");
    }

    harness.answer(carol);
    let recorded = wait_for_phone_asked(&harness, "SwapCalls");
    assert!(!hung_up(&recorded, &bob_path), "{recorded:#?}");
    harness.wait_for_app("Carol is live and Bob still on hold", DEADLINE, |state| {
        state_of(state, carol) == Some("active")
            && state_of(state, bob) == Some("held")
            && call_window(state, "call") == Some(carol.to_string())
            && call_window(state, "held-call") == Some(bob.to_string())
    });
    let on_phone = harness.simulated_phone().calls_on_phone();
    assert!(
        on_phone.contains(&(bob_path, "held".to_owned()))
            && on_phone.contains(&(carol_path, "active".to_owned())),
        "{on_phone:?}"
    );
}
