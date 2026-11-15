//! Call waiting with three Calls: a Call on hold and a live one when a
//! third rings, and a Call ringing while one is being placed. The rest of
//! call waiting is in `call_waiting.rs`, whose helpers these use.

use test_harness::{Bus, Harness, Key, RecordedMethodCall};

use super::call_waiting::*;

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
            && call_window(state, "hang-up-shortcut").is_some()
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

/// With Bob on hold, the Phone holds no other Call, so for Carol the Banner
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
    focus_banner_showing(&harness, "E, D");
    harness.keyboard().type_text("ha");
    // Enter has no Answer to press here, and must not end or decline.
    harness.keyboard().press(Key::Return);
    harness.simulated_phone().recorded_holds(
        "neither H, A nor Enter asks the Phone anything",
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

    let state = harness.app_state();
    assert_eq!(
        call_window(&state, "hang-up-shortcut"),
        None,
        "the main Hang up is greyed out"
    );
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
    assert_eq!(
        banner(&state)
            .and_then(|banner| banner.get("answer-consequence"))
            .map(String::as_str),
        Some("+44\u{a0}7700\u{a0}900123 stays on hold")
    );
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
