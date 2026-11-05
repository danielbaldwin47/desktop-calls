//! Scenarios: the control interface publishes a Call's new state before the
//! surfaces that follow from it (ticket #107). A client following
//! `PropertiesChanged`, such as the Real-Phone Checklist runner, never sees
//! a surface for a Call the control interface doesn't list yet, nor the
//! Incoming Call Banner closed beside a Call still listed as ringing when
//! the Desktop wasn't asked to answer or decline it. The `Error` and
//! `MissedCall` signals follow the state they describe too (ticket #116):
//! `MissedCall` once `Calls` has dropped the Call that rang out, and
//! `Error` once the state it follows from, such as `ConnectionState`, is
//! published.

use std::time::Duration;

use test_harness::{Builder, Harness, Published};

/// How long the app may take to notice a change on the Phone.
const DEADLINE: Duration = Duration::from_secs(5);
/// How long the tests' own round trip may take; the targets are asserted on
/// the app's log elsewhere (ticket #83).
const ROUND_TRIP: Duration = Duration::from_secs(3);
/// How long a Contacts sync may take.
const SYNC: Duration = Duration::from_secs(15);

const NUMBER: &str = "+447700900123";

/// Starts `builder` on a screen of its own, with the Phone connected.
fn connected(builder: Builder) -> Harness {
    let harness = builder.headless_screen().start();
    harness.simulated_phone().connect();
    harness.wait_for_app("ConnectionState is connected", DEADLINE, |state| {
        state.connection_state == "connected"
    });
    harness
}

/// Rings the Phone and waits for the Banner. Returns the Call's object path
/// on the Phone.
fn ring_until_banner(harness: &Harness) -> String {
    let path = harness.simulated_phone().ring(NUMBER);
    harness.wait_for_app("the Banner shows", ROUND_TRIP, |state| {
        state.surface("banner").is_some()
    });
    path
}

/// The id of the Call the surface `name` names, if it shows one.
fn named_call<'a>(published: &'a Published, name: &str) -> Option<&'a str> {
    published
        .surfaces
        .get(name)
        .and_then(|content| content.get("call"))
        .map(String::as_str)
}

/// The published state of the Call with id `call`, if `Calls` lists it.
fn call_state<'a>(published: &'a Published, call: &str) -> Option<&'a str> {
    published
        .calls
        .iter()
        .find(|listed| listed.id.to_string() == call)
        .map(|listed| listed.state.as_str())
}

/// Checks every step the app published, in the order it sent them, for a
/// scenario in which only the Phone acts: a surface starts naming a Call
/// only once `Calls` lists it (the Banner, as ringing), and the Banner
/// closes only once `Calls` says why (its Call answered or gone).
fn assert_calls_published_before_surfaces(harness: &Harness) {
    let published = harness.published();
    let mut before = Published::default();
    for (step, now) in published.iter().enumerate() {
        for name in now.surfaces.keys() {
            let Some(call) = named_call(now, name) else {
                continue;
            };
            if named_call(&before, name) == Some(call) {
                continue;
            }
            let listed = call_state(now, call);
            assert!(
                listed.is_some_and(|state| name != "banner" || state == "ringing"),
                "step {step}: the {name} named Call {call} while Calls listed it as {listed:?}; \
                 every step: {published:#?}"
            );
        }
        if let Some(call) = named_call(&before, "banner")
            && !now.surfaces.contains_key("banner")
        {
            assert_ne!(
                call_state(now, call),
                Some("ringing"),
                "step {step}: the Banner closed while Calls still listed Call {call} as ringing; \
                 every step: {published:#?}"
            );
        }
        before = now.clone();
    }
}

/// Checks every step the app published, in the order it sent them, for a
/// Call that rang out: the `MissedCall` signal that tells of it comes only
/// once `Calls` has dropped the Call, and the Call was listed before.
fn assert_missed_calls_published_after_their_call_is_over(harness: &Harness, expected: usize) {
    let published = harness.published();
    let mut before = Published::default();
    for (step, now) in published.iter().enumerate() {
        for missed in &now.missed_calls[before.missed_calls.len()..] {
            let call = missed.call.to_string();
            assert_eq!(
                call_state(now, &call),
                None,
                "step {step}: MissedCall told of Call {call} while Calls still listed it; \
                 every step: {published:#?}"
            );
            assert!(
                published[..step]
                    .iter()
                    .any(|earlier| call_state(earlier, &call).is_some()),
                "step {step}: MissedCall told of Call {call}, which Calls never listed; \
                 every step: {published:#?}"
            );
        }
        before = now.clone();
    }
    let signalled = published.last().map_or(0, |last| last.missed_calls.len());
    assert_eq!(
        signalled, expected,
        "MissedCall signals; every step: {published:#?}"
    );
}

/// Checks every step the app published, in the order it sent them, for a
/// dial the Phone's going out of range cancels: the `Error` that says so
/// comes only once `ConnectionState` says the Phone is disconnected.
fn assert_out_of_range_published_after_the_connection_is_lost(harness: &Harness) {
    let published = harness.published();
    let mut before = Published::default();
    let mut told = 0;
    for (step, now) in published.iter().enumerate() {
        for error in &now.errors[before.errors.len()..] {
            told += 1;
            assert_eq!(error.kind, "phone-out-of-range", "{published:#?}");
            assert_eq!(
                now.connection_state, "disconnected",
                "step {step}: the Error {:?} came while ConnectionState was {:?}; \
                 every step: {published:#?}",
                error.kind, now.connection_state
            );
        }
        before = now.clone();
    }
    assert_eq!(told, 1, "Error signals; every step: {published:#?}");
}

#[test]
fn a_call_that_rings_out_with_contacts_denied_calls_drops_it_before_missed_call_is_signalled() {
    let harness = Harness::builder().headless_screen().start();
    harness.simulated_phone().set_contacts_prompt("deny");
    harness.simulated_phone().connect();
    harness.wait_for_app("ContactsState is denied", SYNC, |state| {
        state.contacts.state == "denied"
    });
    let path = ring_until_banner(&harness);

    harness.simulated_phone().caller_hangs_up(&path);

    harness.wait_for_app("the Call is over", SYNC, |state| {
        state.surface("banner").is_none() && state.calls.is_empty()
    });
    let started = std::time::Instant::now();
    while harness.missed_calls().is_empty() {
        assert!(
            started.elapsed() < DEADLINE,
            "no MissedCall signal within {DEADLINE:?}"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    assert_missed_calls_published_after_their_call_is_over(&harness, 1);
}

#[test]
fn a_dial_the_phone_going_out_of_range_cancels_says_disconnected_before_the_error() {
    let harness = connected(Harness::builder());
    let dial = harness.start_dial(NUMBER);
    harness.simulated_phone().wait_for_recorded(
        "the Phone is asked to dial",
        DEADLINE,
        |calls| calls.iter().any(|call| call.member == "Dial"),
    );

    harness.simulated_phone().disconnect();

    assert_eq!(
        harness.wait_for_app_error(DEADLINE).kind,
        "phone-out-of-range"
    );
    dial.outcome(DEADLINE)
        .expect_err("the dial is not placed once the Phone is out of range");
    assert_out_of_range_published_after_the_connection_is_lost(&harness);
}

#[test]
fn answered_on_the_phone_under_do_not_disturb_calls_says_answered_before_the_banner_closes() {
    let harness = connected(Harness::builder().do_not_disturb());
    let path = ring_until_banner(&harness);

    harness.simulated_phone().answer_on_phone(&path);

    harness.wait_for_app(
        "the Call Window shows the answered Call",
        DEADLINE,
        |state| {
            state.surface("banner").is_none()
                && state.surface("call-window").is_some()
                && state
                    .calls
                    .first()
                    .is_some_and(|call| call.state == "active")
        },
    );
    assert_calls_published_before_surfaces(&harness);
}

#[test]
fn answered_on_the_phone_calls_says_answered_before_the_banner_closes() {
    let harness = connected(Harness::builder());
    let path = ring_until_banner(&harness);

    harness.simulated_phone().answer_on_phone(&path);

    harness.wait_for_app(
        "the Call Window shows the answered Call",
        DEADLINE,
        |state| {
            state.surface("banner").is_none()
                && state.surface("call-window").is_some()
                && state
                    .calls
                    .first()
                    .is_some_and(|call| call.state == "active")
        },
    );
    assert_calls_published_before_surfaces(&harness);
}

#[test]
fn declined_on_the_phone_calls_drops_the_call_before_the_banner_closes() {
    let harness = connected(Harness::builder());
    let path = ring_until_banner(&harness);

    harness.simulated_phone().decline_on_phone(&path);

    harness.wait_for_app("the Banner closes", DEADLINE, |state| {
        state.surface("banner").is_none() && state.calls.is_empty()
    });
    assert_calls_published_before_surfaces(&harness);
}

#[test]
fn the_caller_hanging_up_calls_drops_the_call_before_the_banner_closes() {
    let harness = connected(Harness::builder());
    let path = ring_until_banner(&harness);

    harness.simulated_phone().caller_hangs_up(&path);

    harness.wait_for_app("the Banner closes", DEADLINE, |state| {
        state.surface("banner").is_none() && state.calls.is_empty()
    });
    assert_calls_published_before_surfaces(&harness);
}
