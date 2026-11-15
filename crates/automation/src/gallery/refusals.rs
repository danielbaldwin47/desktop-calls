//! The gallery's states that say why not (#26): the Dialer refusing a
//! withheld Missed Call, a second Call and a number no phone can dial, and
//! the Call Window after a Handoff to the Phone didn't happen.

use test_harness::{AppState, Harness, Key};

use super::{
    DEADLINE, State, as_is, dialer_shows, live_here, on_a_call, open_dialer, shows, shows_dialer,
    sync,
};

pub(super) const REFUSAL_STATES: &[State] = &[
    State {
        name: "dialer-withheld-missed-call",
        title: "The Dialer, after Enter on a Missed Call from a withheld number: why it can't be called back, as a strip under the search row",
        still: |_| true,
        prepare: as_is,
        reach: |harness| {
            sync(harness);
            open_dialer(harness, "");
            harness.keyboard().press(Key::Down);
            harness.wait_for_app("the withheld Missed Call is selected", DEADLINE, |state| {
                dialer_shows(state, "selected", "1")
            });
            harness.keyboard().press(Key::Return);
            wait_for_notice(harness, "That caller withheld their number");
        },
        shown: |screenshot, _| shows_dialer(screenshot),
    },
    State {
        name: "dialer-call-in-progress",
        title: "The Dialer, after Enter during a Call: hang up the current Call first, and the Call Window still there",
        still: |_| true,
        prepare: as_is,
        reach: |harness| {
            sync(harness);
            on_a_call(harness);
            open_dialer(harness, "jose");
            harness.keyboard().press(Key::Return);
            wait_for_notice(harness, "Hang up the current Call first.");
        },
        shown: |screenshot, _| shows_dialer(screenshot),
    },
    State {
        name: "dialer-undialable",
        title: "The Dialer with a number no phone can dial typed: no rows, and why, before Enter",
        still: |_| true,
        prepare: as_is,
        reach: |harness| {
            sync(harness);
            open_dialer(harness, "0770 09!00");
            wait_for_notice(harness, "That isn't a number your phone can dial");
        },
        shown: |screenshot, _| shows_dialer(screenshot),
    },
    State {
        name: "call-window-audio-not-moved",
        title: "The Call Window after the Phone kept the audio here: a notice to use the phone's audio button, and the Call still live",
        still: |_| true,
        prepare: as_is,
        reach: |harness| {
            live_here(harness);
            harness
                .simulated_phone()
                .enable_quirk("disconnect-profile-refused");
            harness
                .move_audio("phone")
                .unwrap_or_else(|error| panic!("move the audio to the phone: {error:?}"));
            harness.wait_for_app(
                "the Call Window says the audio stayed here",
                DEADLINE,
                |state| {
                    call_window_notice(state)
                        .is_some_and(|notice| notice.starts_with("The audio stayed here"))
                },
            );
        },
        shown: |screenshot, look| shows(screenshot, look.red()),
    },
];

fn call_window_notice(state: &AppState) -> Option<&String> {
    state
        .surface("call-window")
        .and_then(|window| window.get("notice"))
}

/// Waits until the Dialer's notice starts with `words`.
fn wait_for_notice(harness: &Harness, words: &str) {
    harness.wait_for_app(&format!("the Dialer says {words:?}"), DEADLINE, |state| {
        state
            .surface("dialer")
            .and_then(|dialer| dialer.get("notice"))
            .is_some_and(|notice| notice.starts_with(words))
    });
}
