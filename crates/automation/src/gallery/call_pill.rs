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
/// [`point_at_hang_up`] have taken.
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
            point_at_hang_up(harness);
        },
        // The tooltip shows above Hang up.
        shown: |screenshot, look| {
            shows(screenshot, look.red())
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
        still: collapsed,
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
        // Mute and Hang up go: they would do nothing.
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
        // Mute and Hang up go once the Call has ended.
        shown: |screenshot, look| without_controls(screenshot, look) && in_corner(screenshot),
    },
];

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

/// Rests the pointer on the Call Pill's Hang up: 6 px of padding and a
/// 1 px border in from its right edge, and 32 px wide.
fn point_at_hang_up(harness: &Harness) {
    let unique = HOVER_CAPTURES.fetch_add(1, Ordering::Relaxed);
    let screenshot = harness.screenshot(&format!("call-pill-before-hover-{unique}"));
    let pill = app_region(&screenshot);
    let _ = fs::remove_file(&screenshot.path);
    let Some(pill) = pill else {
        panic!("the Call Pill isn't on screen");
    };
    harness
        .pointer()
        .move_to(pill.right - 23.0, f64::midpoint(pill.top, pill.bottom));
}
