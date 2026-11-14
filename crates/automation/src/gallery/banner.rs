//! The Incoming Call Banner's states in the gallery (#226): ringing, with
//! the keyboard, identifying, not in Contacts and with no keybinding, and a
//! Waiting Call with and without hold. A third Call waiting, beside the
//! Call Window with two Calls, is in the main list.

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
        // The Banner identifies the caller for only 2 s, so a capture on a
        // slow runner could show "Withheld number" under this caption.
        still: identifying,
        prepare: with_keybinding,
        reach: |harness| {
            connect(harness);
            // A withheld number: the Banner identifies the caller for as long
            // as the Phone may still report one, longer than the
            // `late-line-identification` quirk's number takes.
            let _path = harness.simulated_phone().ring("");
            harness.wait_for_app(
                "the Banner is identifying the caller",
                DEADLINE,
                identifying,
            );
        },
        shown: |screenshot, look| shows(screenshot, look.green()) && shows(screenshot, look.red()),
    },
    State {
        name: "call-waiting-banner",
        title: "A Waiting Call that can be held: Hold & Answer says who waits on hold, End & Answer whose Call ends; the Banner top right, the Call Window bottom right",
        still: expanded,
        prepare: with_keybinding,
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
        title: "A Waiting Call while a Call is still being placed, which can't be held: only Decline and End & Answer, naming the Call it ends",
        still: |state| {
            state
                .surface("banner")
                .and_then(|banner| banner.get("options"))
                .is_some_and(|options| options == "End & Answer, Decline")
        },
        prepare: with_keybinding,
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

fn with_keybinding(builder: Builder) -> Builder {
    builder.hyprland_bindings(DESKTOP_CALLS_BINDING)
}

fn footer(state: &AppState) -> Option<&String> {
    state
        .surface("banner")
        .and_then(|banner| banner.get("footer"))
}

fn names_the_keybinding(state: &AppState) -> bool {
    footer(state).is_some_and(|footer| footer.starts_with("Super Alt C then"))
}

fn has_the_keyboard(state: &AppState) -> bool {
    state
        .surface("banner")
        .is_some_and(|banner| banner.contains_key("keycaps"))
}

/// Whether the Banner's caller line says the caller is being identified.
fn identifying(state: &AppState) -> bool {
    state
        .surface("banner")
        .and_then(|banner| banner.get("caller"))
        .is_some_and(|caller| caller == "Identifying caller…")
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
