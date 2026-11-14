//! The Call Pill's states in the gallery: the Call Window collapsed during a
//! Call, in the bottom-right corner, with a check of where it sits on every
//! capture. Also what keeps the expanded Call Window's states expanded long
//! enough to capture them.

use std::fs;
use std::sync::atomic::{AtomicUsize, Ordering};

use test_harness::{AppState, Harness, Key, LAYOUT_SIZE, Screenshot};

use super::{
    DEADLINE, NUMBER, State, app_region, as_is, connect, held, on_a_call, ring, shows,
    without_controls,
};

/// How many temporary captures [`hold_call_window_expanded`] and
/// [`point_at`] have taken.
static HOVER_CAPTURES: AtomicUsize = AtomicUsize::new(0);

/// The gap the Call Pill keeps from the screen's edges, in layout pixels.
const GAP: f64 = 12.0;

pub(super) const CALL_PILL_STATES: &[State] = &[
    State {
        name: "call-pill",
        title: "The Call Pill: the Call Window collapsed by itself 5 s after the Call connected, bottom right",
        still: collapsed,
        prepare: as_is,
        reach: |harness| {
            connect(harness);
            on_a_call(harness);
            wait_until_collapsed(harness);
        },
        // At rest, nothing is red: Hang up is neutral until pointed at.
        shown: |screenshot, look| {
            !shows(screenshot, look.red())
                && shows(screenshot, look.green())
                && in_corner(screenshot)
        },
    },
    State {
        name: "call-pill-hover",
        title: "The Call Pill with the pointer over Hang up: red, with its tooltip",
        still: collapsed,
        prepare: as_is,
        reach: |harness| {
            connect(harness);
            on_a_call(harness);
            wait_until_collapsed(harness);
            point_at(harness, HANG_UP_FROM_RIGHT);
        },
        // The tooltip shows above Hang up.
        shown: |screenshot, look| {
            shows(screenshot, look.red())
                && app_region(screenshot).is_some_and(|region| region.bottom - region.top > 60.0)
        },
    },
    State {
        name: "call-pill-hover-body",
        title: "The Call Pill with the pointer over its body: the chevron, a lighter strip, and Show Call",
        still: collapsed,
        prepare: as_is,
        reach: |harness| {
            connect(harness);
            on_a_call(harness);
            wait_until_collapsed(harness);
            point_at(harness, BODY_FROM_RIGHT);
        },
        // The tooltip shows above the body; Hang up stays neutral.
        shown: |screenshot, look| {
            !shows(screenshot, look.red())
                && app_region(screenshot).is_some_and(|region| region.bottom - region.top > 60.0)
        },
    },
    State {
        name: "call-pill-muted",
        title: "The Call Pill, muted: the strip in the warning tint, the crossed microphone, and Muted",
        still: |state| collapsed(state) && state.muted,
        prepare: as_is,
        reach: |harness| {
            connect(harness);
            on_a_call(harness);
            wait_until_collapsed(harness);
            harness
                .set_muted(true)
                .unwrap_or_else(|error| panic!("mute: {error:?}"));
            harness.wait_for_app("the Call Pill shows muted", DEADLINE, |state| {
                state
                    .surface("call-pill")
                    .is_some_and(|pill| pill.contains_key("muted"))
            });
        },
        shown: |screenshot, look| {
            !shows(screenshot, look.red())
                && shows(screenshot, look.warning())
                && in_corner(screenshot)
        },
    },
    State {
        name: "call-window-calls-off-after",
        title: "The Call Window while a turn-off waits for the Call to end: the Calls off after tag on the top line, between who and the timer",
        still: |state| expanded(state) && window_has(state, "calls-off-after"),
        prepare: as_is,
        reach: |harness| {
            connect(harness);
            on_a_call(harness);
            turn_calls_off(harness);
            harness.wait_for_app("the Call Window is tagged", DEADLINE, |state| {
                expanded(state) && window_has(state, "calls-off-after")
            });
            hold_call_window_expanded(harness);
        },
        shown: |screenshot, look| shows(screenshot, look.red()) && !shows(screenshot, look.green()),
    },
    State {
        name: "call-pill-calls-off-after",
        title: "The Call Pill while a turn-off waits for the Call to end: the Calls off after tag beside the caller, in the Muted tag's style",
        still: |state| collapsed(state) && pill_has(state, "calls-off-after"),
        prepare: as_is,
        reach: |harness| {
            connect(harness);
            on_a_call(harness);
            turn_calls_off(harness);
            harness.wait_for_app("the Call Pill is tagged", DEADLINE, |state| {
                collapsed(state) && pill_has(state, "calls-off-after")
            });
        },
        shown: |screenshot, look| {
            !shows(screenshot, look.red())
                && shows(screenshot, look.green())
                && in_corner(screenshot)
        },
    },
    State {
        name: "call-pill-muted-calls-off-after",
        title: "The Call Pill, muted while a turn-off waits for the Call to end: Muted and Calls off after, the caller's name still readable",
        still: |state| collapsed(state) && state.muted && pill_has(state, "calls-off-after"),
        prepare: as_is,
        reach: |harness| {
            connect(harness);
            on_a_call(harness);
            turn_calls_off(harness);
            wait_until_collapsed(harness);
            harness
                .set_muted(true)
                .unwrap_or_else(|error| panic!("mute: {error:?}"));
            harness.wait_for_app("the Call Pill is muted and tagged", DEADLINE, |state| {
                pill_has(state, "muted") && pill_has(state, "calls-off-after")
            });
        },
        shown: |screenshot, look| {
            !shows(screenshot, look.red())
                && shows(screenshot, look.warning())
                && in_corner(screenshot)
        },
    },
    State {
        name: "call-pill-audio-on-phone",
        title: "The Call Pill, answered on the phone: On phone, and Mute unavailable",
        still: |state| collapsed(state) && pill_has(state, "audio-location"),
        prepare: as_is,
        reach: |harness| {
            connect(harness);
            let path = harness.simulated_phone().ring(NUMBER);
            harness.wait_for_app("the Banner shows", DEADLINE, |state| {
                state.surface("banner").is_some()
            });
            harness.simulated_phone().answer_on_phone(&path);
            harness.wait_for_app(
                "the Call Pill says the audio is on the phone",
                DEADLINE,
                |state| collapsed(state) && pill_has(state, "mute-unavailable"),
            );
        },
        shown: |screenshot, look| {
            !shows(screenshot, look.red())
                && !shows(screenshot, look.warning())
                && in_corner(screenshot)
        },
    },
    State {
        name: "call-pill-on-your-phone",
        title: "The Call Pill, collapsed by hand after contact with the Call was lost: On your phone, in place of the timer",
        still: |state| collapsed(state) && !pill_has(state, "hang-up"),
        prepare: as_is,
        reach: |harness| {
            connect(harness);
            on_a_call(harness);
            wait_until_collapsed(harness);
            harness
                .simulated_phone()
                .enable_quirk("connect-profile-fails");
            harness
                .move_audio("phone")
                .unwrap_or_else(|error| panic!("move the audio to the phone: {error:?}"));
            harness.wait_for_app("lost contact expands the Call Window", DEADLINE, |state| {
                state
                    .surface("call-window")
                    .is_some_and(|window| window.contains_key("notice"))
            });
            collapse_by_hand(harness);
            harness.wait_for_app("the Call Pill says On your phone", DEADLINE, |state| {
                state
                    .surface("call-pill")
                    .and_then(|pill| pill.get("status"))
                    .is_some_and(|status| status == "On your phone")
            });
        },
        // Mute and Hang up go: they would do nothing. The screen can't tell
        // a neutral Hang up apart, so the app's report says so.
        shown: |screenshot, look| without_controls(screenshot, look) && in_corner(screenshot),
    },
    State {
        name: "call-pill-held",
        title: "The Call Pill, collapsed by hand with a Call on hold: 1 on hold",
        still: |state| collapsed(state) && pill_has(state, "held"),
        prepare: as_is,
        reach: |harness| {
            held(harness);
            collapse_by_hand(harness);
            harness.wait_for_app("the Call Pill says 1 on hold", DEADLINE, |state| {
                collapsed(state) && pill_has(state, "held")
            });
        },
        shown: |screenshot, look| !shows(screenshot, look.red()) && in_corner(screenshot),
    },
    State {
        name: "call-pill-ended",
        title: "The Call Pill, just after the Call ended: Call ended and how long it lasted",
        still: |state| {
            state
                .surface("call-pill")
                .and_then(|pill| pill.get("status"))
                .is_some_and(|status| status.starts_with("Call ended · "))
                && !pill_has(state, "hang-up")
        },
        prepare: as_is,
        reach: |harness| {
            connect(harness);
            let call = ring(harness);
            harness.answer(call);
            wait_until_collapsed(harness);
            harness.hang_up(call);
            harness.wait_for_app("the Call Pill says Call ended", DEADLINE, |state| {
                state
                    .surface("call-pill")
                    .and_then(|pill| pill.get("status"))
                    .is_some_and(|status| status.starts_with("Call ended · "))
            });
        },
        // Mute and Hang up go once the Call has ended, as the app reports.
        shown: |screenshot, look| without_controls(screenshot, look) && in_corner(screenshot),
    },
];

/// Turns calls off from the command line during the Call, so the turn-off
/// waits for it to end.
pub(super) fn turn_calls_off(harness: &Harness) {
    let output = harness.cli(&["calls", "off"]);
    assert!(output.status.success(), "calls off failed: {output:?}");
    harness.wait_for_app("a turn-off waits for the Call", DEADLINE, |state| {
        state.calls_off_pending
    });
}

fn window_has(state: &AppState, key: &str) -> bool {
    state
        .surface("call-window")
        .is_some_and(|window| window.contains_key(key))
}

fn pill_has(state: &AppState, key: &str) -> bool {
    state
        .surface("call-pill")
        .is_some_and(|pill| pill.contains_key(key))
}

/// The keybinding gives the expanded Call Window the keyboard, and Escape
/// collapses it, as the user does by hand.
fn collapse_by_hand(harness: &Harness) {
    harness.activate();
    harness.wait_for_app("the Call Window has the keyboard", DEADLINE, |state| {
        state.surface("call-window").is_some_and(|window| {
            window
                .get("focused")
                .is_some_and(|focused| focused == "true")
        })
    });
    harness.keyboard().press(Key::Escape);
}

/// The Call Window shows collapsed: the Call Pill, anchored bottom right,
/// and no Call Window.
pub(super) fn collapsed(state: &AppState) -> bool {
    state.surface("call-window").is_none()
        && state.surface("call-pill").is_some_and(|pill| {
            pill.get("anchor")
                .is_some_and(|anchor| anchor == "bottom-right")
        })
}

/// The Call Window shows expanded.
pub(super) fn expanded(state: &AppState) -> bool {
    state.surface("call-window").is_some() && state.surface("call-pill").is_none()
}

fn wait_until_collapsed(harness: &Harness) {
    harness.wait_for_app(
        "the Call Window collapses to the Call Pill",
        DEADLINE,
        collapsed,
    );
}

/// Whether the app's only surface on screen sits 12 px from the bottom and
/// right edges, as the Call Pill does, and is no bigger than a strip.
fn in_corner(screenshot: &Screenshot) -> bool {
    let (width, height) = LAYOUT_SIZE;
    app_region(screenshot).is_some_and(|pill| {
        (width - GAP - pill.right).abs() <= 2.0
            && (height - GAP - pill.bottom).abs() <= 2.0
            && pill.right - pill.left <= 360.0
            && pill.bottom - pill.top <= 80.0
    })
}

/// Rests the pointer on the expanded Call Window's padding, so it stays
/// expanded for its capture however long that takes: it would otherwise
/// collapse 5 s after the Call connected. The pointer isn't drawn in a
/// capture, and the padding has nothing to highlight.
pub(super) fn hold_call_window_expanded(harness: &Harness) {
    // Named apart from the other looks' captures, which render at the same
    // time into the same directory.
    let unique = HOVER_CAPTURES.fetch_add(1, Ordering::Relaxed);
    let screenshot = harness.screenshot(&format!("call-window-before-hover-{unique}"));
    let window = app_region(&screenshot);
    let _ = fs::remove_file(&screenshot.path);
    let Some(window) = window else {
        panic!("the Call Window isn't on screen");
    };
    harness
        .pointer()
        .move_to(window.left + 6.0, window.top + 6.0);
}

/// From the Call Pill's right edge to the middle of Hang up: 6 px of
/// padding and a 1 px border, and half its 32 px; and to its body, on the
/// caller.
const HANG_UP_FROM_RIGHT: f64 = 23.0;
const BODY_FROM_RIGHT: f64 = 220.0;

/// Rests the pointer `from_right` in from the Call Pill's right edge,
/// halfway down.
fn point_at(harness: &Harness, from_right: f64) {
    let unique = HOVER_CAPTURES.fetch_add(1, Ordering::Relaxed);
    let screenshot = harness.screenshot(&format!("call-pill-before-hover-{unique}"));
    let pill = app_region(&screenshot);
    let _ = fs::remove_file(&screenshot.path);
    let Some(pill) = pill else {
        panic!("the Call Pill isn't on screen");
    };
    harness.pointer().move_to(
        pill.right - from_right,
        f64::midpoint(pill.top, pill.bottom),
    );
}
