//! The Dialer in the gallery: its Call History, a search, one that
//! matches nothing, its notice and Calls Off strips, and the Missed Calls
//! it marks as new.

use test_harness::vcards::{self, CallKind};
use test_harness::{AppState, Harness, Key, Screenshot};

use super::{DEADLINE, NUMBER, State, app_region, as_is, sync};

/// The Dialer's states.
pub(super) const DIALER_STATES: &[State] = &[
    State {
        name: "dialer-call-history",
        title: "The Dialer, before typing: the Call History, newest first, the selected Missed Call in red at 4.5:1, and the footer of keys with the Phone connected",
        still: |_| true,
        prepare: as_is,
        moment: None,
        reach: |harness| {
            sync(harness);
            open_dialer(harness, "");
        },
        shown: |screenshot, _| shows_dialer(screenshot),
    },
    State {
        name: "dialer-new-missed-calls",
        title: "The Dialer opened after a Missed Call: the Missed Calls since it was last opened at the top, each with a red dot at its right",
        still: |_| true,
        prepare: as_is,
        moment: None,
        reach: |harness| {
            sync(harness);
            missed_call(harness);
            open_dialer(harness, "");
            harness.wait_for_app("the Dialer marks the Missed Call", DEADLINE, |state| {
                dialer_shows(state, "new-missed", "1")
            });
        },
        shown: |screenshot, _| shows_dialer(screenshot),
    },
    State {
        name: "dialer-results",
        title: "The Dialer, searching for smith, the second match selected as Omarchy's cursor row",
        still: |_| true,
        prepare: as_is,
        moment: None,
        reach: |harness| {
            sync(harness);
            open_dialer(harness, "smith");
            harness.keyboard().press(Key::Down);
            harness.wait_for_app("the second row is selected", DEADLINE, |state| {
                dialer_shows(state, "selected", "1")
            });
        },
        shown: |screenshot, _| shows_dialer(screenshot),
    },
    State {
        name: "dialer-no-matches",
        title: "The Dialer after typing what matches nothing: the rows the Call History showed go, and it says so",
        still: |_| true,
        prepare: as_is,
        moment: None,
        reach: |harness| {
            sync(harness);
            open_dialer(harness, "");
            harness.keyboard().type_text("zzq");
            harness.wait_for_app("the Dialer shows no rows", DEADLINE, |state| {
                dialer_shows(state, "query", "zzq") && dialer_shows(state, "rows", "0")
            });
        },
        shown: |screenshot, _| shows_dialer(screenshot),
    },
    State {
        name: "dialer-not-connected",
        title: "The Dialer, after Enter with the Phone out of range: the notice as a strip under the search row, and the footer says Out of range",
        still: |_| true,
        prepare: as_is,
        moment: None,
        reach: |harness| {
            sync(harness);
            harness.simulated_phone().disconnect();
            harness.wait_for_app("the Phone is disconnected", DEADLINE, |state| {
                state.connection_state == "disconnected"
            });
            open_dialer(harness, "jose");
            harness.keyboard().press(Key::Return);
            harness.wait_for_app("the Dialer says why not", DEADLINE, |state| {
                state
                    .surface("dialer")
                    .is_some_and(|dialer| dialer.contains_key("notice"))
            });
        },
        shown: |screenshot, _| shows_dialer(screenshot),
    },
    State {
        name: "dialer-calls-off",
        title: "The Dialer while calls are off until 07:00: the strip says Enter calls anyway, and the footer's Enter reads Call anyway",
        still: |_| true,
        prepare: as_is,
        moment: None,
        reach: |harness| {
            sync(harness);
            let _output = harness.cli(&["calls", "off", "--until-morning"]);
            harness.wait_for_app("calls are off", DEADLINE, |state| {
                state.calls_off == "off" && state.connection_state == "disconnected"
            });
            open_dialer(harness, "smith");
            harness.wait_for_app("the Dialer says calls are off", DEADLINE, |state| {
                state
                    .surface("dialer")
                    .is_some_and(|dialer| dialer.contains_key("calls-off"))
            });
        },
        shown: |screenshot, _| shows_dialer(screenshot),
    },
];

/// After [`sync`]: a Call from [`NUMBER`] rings out, and the Phone lists it
/// as missed, on top of the Call History `sync` set.
fn missed_call(harness: &Harness) {
    let phone = harness.simulated_phone();
    let path = phone.ring(NUMBER);
    harness.wait_for_app("the Call rings", DEADLINE, |state| state.ringing);
    phone.set_pbap_folder(
        "cch",
        &[
            vcards::call(
                CallKind::Missed,
                Some(NUMBER),
                &format!("{}T095500", vcards::days_ago(0)),
            ),
            super::recent_calls().concat(),
        ]
        .concat(),
    );
    phone.caller_hangs_up(&path);
    harness.wait_for_app(
        "the Missed Call is in the Call History",
        DEADLINE,
        |state| state.calls.is_empty() && state.contacts.call_history == 5,
    );
}

/// Opens the Dialer with `query` typed, and waits until it has the keyboard.
pub(super) fn open_dialer(harness: &Harness, query: &str) {
    harness.open_dialer(query, 0);
    harness.wait_for_app("the Dialer has keyboard focus", DEADLINE, |state| {
        dialer_shows(state, "focused", "true") && dialer_shows(state, "query", query)
    });
}

pub(super) fn dialer_shows(state: &AppState, key: &str, value: &str) -> bool {
    state
        .surface("dialer")
        .and_then(|dialer| dialer.get(key))
        .is_some_and(|shown| shown == value)
}

/// Whether the Dialer, wider than the other surfaces, is on screen.
pub(super) fn shows_dialer(screenshot: &Screenshot) -> bool {
    app_region(screenshot).is_some_and(|region| region.right - region.left > 500.0)
}
