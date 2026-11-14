//! The Call Pill's states in the gallery: the Call Window collapsed during a
//! Call, in the bottom-right corner, with a check of where it sits on every
//! capture. Also what keeps the expanded Call Window's states expanded long
//! enough to capture them.

use std::fs;
use std::sync::atomic::{AtomicUsize, Ordering};

use test_harness::{AppState, Harness, LAYOUT_SIZE, Screenshot};

use super::{DEADLINE, ORANGE, State, app_region, as_is, connect, on_a_call, shows};

/// How many temporary captures [`hold_call_window_expanded`] has taken.
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
        shown: |screenshot, look| shows(screenshot, look.red()) && in_corner(screenshot),
    },
    State {
        name: "call-pill-muted",
        title: "The Call Pill, muted: Unmute in orange",
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
            shows(screenshot, look.red()) && shows(screenshot, ORANGE) && in_corner(screenshot)
        },
    },
];

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
