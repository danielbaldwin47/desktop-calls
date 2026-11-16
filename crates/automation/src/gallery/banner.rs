//! The Incoming Call Banner's states in the gallery (#226): ringing, with
//! the keyboard, identifying, not in Contacts, withheld and with no
//! keybinding, and a Waiting Call with and without hold. A third Call
//! waiting, beside the Call Window with two Calls, is in the main list.

use std::time::{Duration, Instant};

use desktop_calls_core::NUMBER_LATE_MS;
use test_harness::{AppState, Builder, DESKTOP_CALLS_BINDING, Harness};

use super::call_pill::{expanded, hold_call_window_expanded};
use super::{DEADLINE, State, as_is, bordered, connect, dial, on_a_call, ring, shows, sync};

/// José Smith, in the gallery's Contacts: the caller of a Waiting Call.
const JOSE: &str = "+447700900001";

pub(super) const BANNER_STATES: &[State] = &[
    State {
        name: "incoming-call-banner",
        title: "The Incoming Call Banner, ringing: a Contact, Decline left and Answer right, and the keybinding in the footer",
        still: names_the_keybinding,
        prepare: with_keybinding,
        moment: None,
        reach: |harness| {
            sync(harness);
            ring(harness);
            wait_for_footer(harness);
        },
        shown: |screenshot, look| shows(screenshot, look.green()) && shows(screenshot, look.red()),
    },
    State {
        name: "incoming-call-banner-keyboard",
        title: "The Incoming Call Banner given the keyboard by the keybinding: an accent border, keycaps on the buttons, and Escape in the footer",
        still: has_the_keyboard,
        prepare: with_keybinding,
        moment: None,
        reach: |harness| {
            sync(harness);
            ring(harness);
            take_the_keyboard(harness);
        },
        shown: |screenshot, look| {
            shows(screenshot, look.green())
                && shows(screenshot, look.red())
                && bordered(screenshot, look.accent())
        },
    },
    State {
        name: "incoming-call-banner-not-in-contacts",
        title: "The Incoming Call Banner for a number not in Contacts: a # tile and the number in monospace",
        still: names_the_keybinding,
        prepare: with_keybinding,
        moment: None,
        reach: |harness| {
            connect(harness);
            ring(harness);
            wait_for_footer(harness);
        },
        shown: |screenshot, look| shows(screenshot, look.green()) && shows(screenshot, look.red()),
    },
    State {
        name: "incoming-call-banner-no-keybinding",
        title: "The Incoming Call Banner on a Desktop with no keybinding set: the footer says where to add one",
        still: |state| footer(state).is_some_and(|footer| footer.starts_with("No keybinding")),
        prepare: as_is,
        moment: None,
        reach: |harness| {
            sync(harness);
            ring(harness);
            wait_for_footer(harness);
        },
        shown: |screenshot, look| shows(screenshot, look.green()) && shows(screenshot, look.red()),
    },
    State {
        name: "incoming-call-banner-keybinding-unreadable",
        title: "The Incoming Call Banner with bindings.lua unreadable: the footer says so, and where to find what to do",
        still: |state| footer(state).is_some_and(|footer| footer.starts_with("Couldn't read")),
        prepare: |builder| builder.hyprland_bindings_unreadable(),
        moment: None,
        reach: |harness| {
            sync(harness);
            ring(harness);
            wait_for_footer(harness);
        },
        shown: |screenshot, look| shows(screenshot, look.green()) && shows(screenshot, look.red()),
    },
    State {
        name: "incoming-call-banner-identifying",
        title: "The Incoming Call Banner, ringing with no number yet, which may still come",
        // The Banner identifies the caller for only 2 s: the app is stopped
        // within them, and `render` doesn't ask it again.
        still: identifying,
        prepare: with_keybinding,
        reach: |harness| {
            connect(harness);
            // The app's first frame of a surface held its main loop up for
            // 1.6 s of these 2 s on a loaded runner (run 43678330490), and
            // the app was stopped too late. A Banner drawn and declined
            // first leaves the identifying Banner's frame quick.
            let warm_up = ring(harness);
            harness.wait_for_app_log("Drew the banner surface's first frame", DEADLINE);
            harness.decline(warm_up);
            harness.wait_for_app("the first Banner has gone", DEADLINE, |state| {
                state.calls.is_empty() && state.surface("banner").is_none()
            });
        },
        // A withheld number: the Banner identifies the caller for as long as
        // the Phone may still report one, longer than the
        // `late-line-identification` quirk's number takes.
        moment: Some(|harness| {
            let rang = Instant::now();
            let _path = harness.simulated_phone().ring("");
            harness.wait_for_app(
                "the Banner is identifying the caller",
                DEADLINE,
                identifying,
            );
            rang + Duration::from_millis(NUMBER_LATE_MS.into())
        }),
        shown: |screenshot, look| shows(screenshot, look.green()) && shows(screenshot, look.red()),
    },
    State {
        name: "incoming-call-banner-withheld",
        title: "The Incoming Call Banner for a withheld number, once no number came: Withheld number, and nothing to call back",
        still: |state| {
            caller(state).is_some_and(|caller| caller == "Withheld number")
                && names_the_keybinding(state)
        },
        prepare: with_keybinding,
        moment: None,
        reach: |harness| {
            connect(harness);
            let _path = harness.simulated_phone().ring("");
            // The Banner identifies the caller for 2 s first.
            harness.wait_for_app(
                "the Banner says Withheld number, with the keybinding",
                DEADLINE,
                |state| {
                    caller(state).is_some_and(|caller| caller == "Withheld number")
                        && names_the_keybinding(state)
                },
            );
        },
        shown: |screenshot, look| shows(screenshot, look.green()) && shows(screenshot, look.red()),
    },
    State {
        name: "call-waiting-banner",
        title: "A Waiting Call that can be held: Hold & answer says who waits on hold, End & answer whose Call ends; the Banner top right, the Call Window bottom right",
        still: expanded,
        prepare: with_keybinding,
        moment: None,
        reach: |harness| {
            sync(harness);
            on_a_call(harness);
            hold_call_window_expanded(harness);
            waiting_from(harness, JOSE);
        },
        shown: |screenshot, look| shows(screenshot, look.green()) && shows(screenshot, look.red()),
    },
    State {
        name: "call-waiting-banner-keyboard",
        title: "A Waiting Call's Banner given the keyboard: H, E and D on its buttons",
        still: has_the_keyboard,
        prepare: with_keybinding,
        moment: None,
        reach: |harness| {
            sync(harness);
            on_a_call(harness);
            waiting_from(harness, JOSE);
            take_the_keyboard(harness);
        },
        // The Call Window shares the screen, so the accent can't edge the
        // whole of what is on it.
        shown: |screenshot, look| {
            shows(screenshot, look.green()) && shows(screenshot, look.accent())
        },
    },
    State {
        name: "call-waiting-banner-without-hold",
        title: "A Waiting Call while a Call is still being placed, which can't be held: only Decline and End & answer, naming the Call it ends",
        still: |state| {
            state
                .surface("banner")
                .and_then(|banner| banner.get("options"))
                .is_some_and(|options| options == "End & answer, Decline")
        },
        prepare: with_keybinding,
        moment: None,
        reach: |harness| {
            sync(harness);
            dial(harness);
            let _path = harness.simulated_phone().alert();
            harness.wait_for_app("the Call rings at the other end", DEADLINE, |state| {
                state.calls.len() == 1 && state.calls[0].state == "alerting"
            });
            waiting_from(harness, JOSE);
        },
        shown: |screenshot, look| shows(screenshot, look.red()),
    },
];

pub(super) fn with_keybinding(builder: Builder) -> Builder {
    builder.hyprland_bindings(DESKTOP_CALLS_BINDING)
}

fn footer(state: &AppState) -> Option<&String> {
    state
        .surface("banner")
        .and_then(|banner| banner.get("footer"))
}

pub(super) fn names_the_keybinding(state: &AppState) -> bool {
    footer(state).is_some_and(|footer| footer.starts_with("Super Alt C then"))
}

fn has_the_keyboard(state: &AppState) -> bool {
    state
        .surface("banner")
        .is_some_and(|banner| banner.contains_key("keycaps"))
}

fn caller(state: &AppState) -> Option<&String> {
    state
        .surface("banner")
        .and_then(|banner| banner.get("caller"))
}

/// Whether the Banner's caller line says the caller is being identified.
fn identifying(state: &AppState) -> bool {
    caller(state).is_some_and(|caller| caller == "Identifying caller…")
}

fn wait_for_footer(harness: &Harness) {
    harness.wait_for_app("the Banner's footer shows", DEADLINE, |state| {
        footer(state).is_some()
    });
}

/// Presses the keybinding and waits until the Banner has the keyboard.
fn take_the_keyboard(harness: &Harness) {
    harness.activate();
    harness.wait_for_app("the Banner has the keyboard", DEADLINE, |state| {
        state
            .surface("banner")
            .and_then(|banner| banner.get("focused"))
            .is_some_and(|focused| focused == "true")
            && has_the_keyboard(state)
    });
}

/// `number` calls during the Call: waits for its Banner.
fn waiting_from(harness: &Harness, number: &str) {
    let _path = harness.simulated_phone().ring(number);
    harness.wait_for_app("the Waiting Call's Banner shows", DEADLINE, |state| {
        state
            .surface("banner")
            .is_some_and(|banner| banner.contains_key("options"))
    });
}
