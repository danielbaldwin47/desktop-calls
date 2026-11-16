//! The Call Window's states that show how long a Call lasted: ended, and
//! contact lost. Each Call runs a few seconds first, so the screenshot
//! shows a real duration, not 0:00 (#262, item 3).

use test_harness::{AppState, Harness};

use super::call_pill::{expanded, hold_call_window_expanded};
use super::{DEADLINE, State, as_is, connect, ring, shows, without_controls};

/// How long, in seconds, each Call runs before it ends or contact is lost,
/// at least, by the Call Window's timer. The Call Window would collapse 5 s
/// after the Call connected; the pointer holds it open meanwhile.
const LASTED: u64 = 2;

pub(super) const DURATION_STATES: &[State] = &[
    State {
        name: "call-window-ended",
        title: "The Call Window, just after the Call ended: who and how long it lasted, and no controls",
        still: |_| true,
        prepare: as_is,
        moment: None,
        reach: |harness| {
            let call = live_for(harness);
            harness.hang_up(call);
            harness.wait_for_app(
                "the Call Window says Call ended, and for how long",
                DEADLINE,
                |state| {
                    call_window(state, "state").is_some_and(|line| {
                        line.strip_prefix("Call ended · ")
                            .and_then(seconds)
                            .is_some_and(|lasted| lasted >= LASTED)
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
        moment: None,
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
                        && call_window(state, "timer")
                            .and_then(|timer| seconds(timer))
                            .is_some_and(|lasted| lasted >= LASTED)
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

/// The seconds in a duration as the Call Window shows it, `m:ss`.
fn seconds(shown: &str) -> Option<u64> {
    let (minutes, seconds) = shown.split_once(':')?;
    Some(minutes.parse::<u64>().ok()? * 60 + seconds.parse::<u64>().ok()?)
}

/// Answers a Call here, holds the Call Window open, and waits until the
/// audio is on the Desktop and the timer reads [`LASTED`] seconds or more.
/// Returns the Call's id.
fn live_for(harness: &Harness) -> u32 {
    connect(harness);
    let call = ring(harness);
    harness.answer(call);
    harness.wait_for_app(
        "the Call Window shows a timer, expanded, the Banner gone",
        DEADLINE,
        |state| {
            expanded(state)
                && state.surface("banner").is_none()
                && call_window(state, "timer").is_some()
        },
    );
    hold_call_window_expanded(harness);
    harness.wait_for_app("the Call has lasted a few seconds", DEADLINE, |state| {
        state.audio_location == "desktop"
            && call_window(state, "timer")
                .and_then(|timer| seconds(timer))
                .is_some_and(|lasted| lasted >= LASTED)
    });
    call
}
