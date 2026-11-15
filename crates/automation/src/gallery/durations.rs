//! The Call Window's states that show how long a Call lasted: ended, and
//! contact lost. Each Call runs a few seconds first, so the screenshot
//! shows a real duration, not 0:00 (#262, item 3).

use test_harness::{AppState, Harness};

use super::{DEADLINE, State, as_is, connect, ring, shows, without_controls};

/// How long each Call runs before it ends or contact is lost, as the
/// Call Window's timer shows it.
const LASTED: &str = "0:03";

/// What the Call Window may show once the Call has lasted [`LASTED`]: the
/// app may take up to a second more to see it end.
const SHOWN: [&str; 2] = ["0:03", "0:04"];

pub(super) const DURATION_STATES: &[State] = &[
    State {
        name: "call-window-ended",
        title: "The Call Window, just after the Call ended: who and how long it lasted, and no controls",
        still: |_| true,
        prepare: as_is,
        reach: |harness| {
            let call = live_for(harness);
            harness.hang_up(call);
            harness.wait_for_app(
                "the Call Window says Call ended, and for how long",
                DEADLINE,
                |state| {
                    call_window(state, "state").is_some_and(|line| {
                        line.strip_prefix("Call ended · ")
                            .is_some_and(|lasted| SHOWN.contains(&lasted))
                    })
                },
            );
        },
        // No Hang up once the Call has ended.
        shown: without_controls,
    },
    State {
        name: "call-window-connection-lost",
        title: "The Call Window, after the audio went to the phone and hands-free didn't come back: the timer struck through, a notice to hang up on the phone, and no controls",
        still: |_| true,
        prepare: as_is,
        reach: |harness| {
            let _call = live_for(harness);
            harness
                .simulated_phone()
                .enable_quirk("connect-profile-fails");
            harness
                .move_audio("phone")
                .unwrap_or_else(|error| panic!("move the audio to the phone: {error:?}"));
            harness.wait_for_app(
                "the Call Window says the connection is lost, and for how long the Call lasted",
                DEADLINE,
                |state| {
                    call_window(state, "notice-heading").is_some()
                        && call_window(state, "timer").is_some_and(|timer| timer != "0:00")
                },
            );
        },
        shown: |screenshot, look| {
            without_controls(screenshot, look)
                && !shows(screenshot, look.green())
                && shows(screenshot, look.warning())
        },
    },
];

fn call_window<'state>(state: &'state AppState, key: &str) -> Option<&'state String> {
    state
        .surface("call-window")
        .and_then(|window| window.get(key))
}

/// Answers a Call here and waits until its audio is on the Desktop and
/// its timer reads [`LASTED`]. Returns the Call's id.
fn live_for(harness: &Harness) -> u32 {
    connect(harness);
    let call = ring(harness);
    harness.answer(call);
    harness.wait_for_app("the Call has lasted a few seconds", DEADLINE, |state| {
        state.audio_location == "desktop"
            && call_window(state, "timer").is_some_and(|timer| timer == LASTED)
    });
    call
}
