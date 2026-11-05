//! Scenarios: the control interface publishes a Call's new state before the
//! surfaces that follow from it (ticket #107). A client following
//! `PropertiesChanged`, such as the Real-Phone Checklist runner, never sees
//! a surface for a Call the control interface doesn't list yet, nor the
//! Incoming Call Banner closed beside a Call still listed as ringing when
//! the Desktop wasn't asked to answer or decline it.

use std::time::Duration;

use test_harness::{Builder, Harness, Published};

/// How long the app may take to notice a change on the Phone.
const DEADLINE: Duration = Duration::from_secs(5);
/// How long the tests' own round trip may take; the targets are asserted on
/// the app's log elsewhere (ticket #83).
const ROUND_TRIP: Duration = Duration::from_secs(3);

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
