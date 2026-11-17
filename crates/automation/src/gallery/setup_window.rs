//! The Setup Window's Problems in the gallery, each one card with its Fix
//! or its command: Bluetooth off (the step is dimmed, and Enter presses
//! Turn on), Bluetooth blocked, Contacts support missing, audio not set up
//! and BlueZ's own hands-free on (these three leave the step as it is,
//! with a Phone listed). With them, calls being off: a neutral card in the
//! same shape, with Turn calls on and its key. And the finish page that
//! "Phone setup…" opens on with a Phone set up (#389): its keybinding
//! card, and Pair another phone to choose a different one.

use std::time::Duration;

use test_harness::{AppState, Builder, Harness};

use super::{
    DEADLINE, PHONE_NAME, State, app_region, as_is, connect, fresh_install,
    pair_through_setup_window, setup_window_shows, sync, with_hyprland_config,
};

/// Audio settings that leave out hands-free, so Calls' audio isn't set up.
const ROLES_WITHOUT_HANDS_FREE: &str = "\
# Only music, please.
monitor.bluez.properties = {
  bluez5.roles = [ a2dp_sink a2dp_source ]
  bluez5.enable-msbc = true
}
";

pub(super) const SETUP_PROBLEM_STATES: &[State] = &[
    State {
        name: "setup-window-calls-off",
        title: "The Setup Window while calls are off until 07:00: a neutral card with Turn calls on and its key, the steps dimmed, and no Phone to list",
        still: |_| true,
        prepare: as_is,
        moment: None,
        reach: |harness| {
            sync(harness);
            let _output = harness.cli(&["calls", "off", "--until-morning"]);
            harness.wait_for_app("calls are off", DEADLINE, |state| {
                state.calls_off == "off" && state.connection_state == "disconnected"
            });
            let _output = harness.cli(&["connect"]);
            harness.pair_another_phone();
            harness.wait_for_app("the Setup Window says calls are off", DEADLINE, |state| {
                setup_window_shows(state, "calls-off", "Calls are off")
                    && setup_window_shows(state, "focused", "true")
            });
        },
        shown: |screenshot, _| app_region(screenshot).is_some(),
    },
    State {
        name: "setup-window-calls-off-bluetooth-off",
        title: "The Setup Window while calls are off and Bluetooth is off: both cards, Turn calls on with its key, and a phone to choose once both are on (#369)",
        still: |_| true,
        prepare: as_is,
        moment: None,
        reach: |harness| {
            sync(harness);
            let _output = harness.cli(&["calls", "off", "--until-morning"]);
            harness.wait_for_app("calls are off", DEADLINE, |state| {
                state.calls_off == "off" && state.connection_state == "disconnected"
            });
            harness.set_bluetooth_power("off");
            let _output = harness.cli(&["connect"]);
            harness.pair_another_phone();
            harness.wait_for_app(
                "the Setup Window says calls are off and Bluetooth is off",
                DEADLINE,
                |state| {
                    setup_window_shows(state, "calls-off", "Calls are off")
                        && setup_window_shows(state, "focused", "true")
                        && shows_problem(state, "bluetooth-off")
                },
            );
        },
        shown: |screenshot, _| app_region(screenshot).is_some(),
    },
    State {
        name: "setup-problem-bluetooth-off",
        title: "The Setup Window with Bluetooth off: one card with Turn on and its key, the steps dimmed, and no Phone to list",
        still: |_| true,
        prepare: |builder| fresh_install(builder).bluetooth_off(),
        moment: None,
        reach: |harness| wait_for_problem(harness, "bluetooth-off", false),
        shown: |screenshot, _| app_region(screenshot).is_some(),
    },
    State {
        name: "setup-problem-bluetooth-blocked",
        title: "The Setup Window with Bluetooth blocked: one card with the command to copy",
        still: |_| true,
        prepare: |builder| fresh_install(builder).bluetooth_blocked(),
        moment: None,
        reach: |harness| wait_for_problem(harness, "bluetooth-blocked", false),
        shown: |screenshot, _| app_region(screenshot).is_some(),
    },
    State {
        name: "setup-problem-contacts-support-missing",
        title: "The Setup Window without Contacts support: the card with the install command, the Phone listed and Enter still pairing",
        still: |_| true,
        prepare: |builder| fresh_install(builder).without_contacts_support(),
        moment: None,
        reach: |harness| wait_for_problem(harness, "contacts-support-missing", true),
        shown: |screenshot, _| app_region(screenshot).is_some(),
    },
    State {
        name: "setup-problem-audio-not-set-up",
        title: "The Setup Window with audio not set up for Calls: the card with Set up audio, the Phone listed and Enter still pairing",
        still: |_| true,
        prepare: audio_not_set_up,
        moment: None,
        reach: |harness| wait_for_problem(harness, "audio-not-set-up", true),
        shown: |screenshot, _| app_region(screenshot).is_some(),
    },
    State {
        name: "setup-problem-bluez-hands-free",
        title: "The Setup Window with BlueZ's own calling on: the card with its Fix, the Phone listed and Enter still pairing",
        still: |_| true,
        prepare: |builder| fresh_install(builder).bluez_hands_free_on(),
        moment: None,
        reach: |harness| wait_for_problem(harness, "bluez-hands-free", true),
        shown: |screenshot, _| app_region(screenshot).is_some(),
    },
];

/// The Setup Window opened from "Phone setup…" with a Phone set up: the
/// finish page for it, never choosing a Phone (#389).
pub(super) const SETUP_OPENED_STATES: &[State] = &[
    State {
        name: "setup-window-opened-set-up",
        title: "\"Phone setup…\" with a Phone set up: its finish page, with Add keybinding, Done, and Pair another phone last (#389)",
        still: |_| true,
        prepare: |builder| builder.config_under_home(),
        reach: |harness| {
            with_hyprland_config(harness);
            sync(harness);
            let _output = harness.cli(&["connect"]);
            harness.wait_for_app("the finish page offers the keybinding", DEADLINE, |state| {
                setup_window_shows(state, "step", "done")
                    && setup_window_shows(
                        state,
                        "buttons",
                        "Add keybinding\nDone\nPair another phone",
                    )
            });
        },
        moment: None,
        shown: |screenshot, _| app_region(screenshot).is_some(),
    },
    State {
        name: "setup-window-opened-keybinding-unreadable",
        title: "\"Phone setup…\" with a Phone set up and bindings.lua unreadable: the finish page says so and what to bind instead (#389)",
        still: |_| true,
        prepare: |builder| builder.config_under_home().hyprland_bindings_unreadable(),
        reach: |harness| {
            sync(harness);
            let _output = harness.cli(&["connect"]);
            harness.wait_for_app("the keybinding card says why", DEADLINE, |state| {
                setup_window_shows(state, "step", "done")
                    && state
                        .surface("setup-window")
                        .and_then(|window| window.get("keybinding"))
                        .is_some_and(|card| card.contains("Couldn't read"))
            });
        },
        moment: None,
        shown: |screenshot, _| app_region(screenshot).is_some(),
    },
    State {
        name: "setup-window-opened-contacts-denied",
        title: "\"Phone setup…\" with a Phone set up that refused Contacts: the finish page points at this computer's name on the phone, which it hasn't read (#393)",
        still: |_| true,
        prepare: |builder| builder.config_under_home(),
        reach: |harness| {
            with_hyprland_config(harness);
            harness.simulated_phone().set_contacts_prompt("deny");
            connect(harness);
            harness.wait_for_app("ContactsState is denied", DEADLINE, |state| {
                state.contacts.state == "denied"
            });
            let _output = harness.cli(&["connect"]);
            harness.wait_for_app(
                "the finish page says where to allow Contacts",
                DEADLINE,
                |state| {
                    setup_window_shows(state, "step", "done")
                        && state
                            .surface("setup-window")
                            .and_then(|window| window.get("summary"))
                            .is_some_and(|summary| summary.contains("next to this computer's name"))
                },
            );
        },
        moment: None,
        shown: |screenshot, _| app_region(screenshot).is_some(),
    },
    State {
        name: "setup-window-opened-calls-off-bluetooth-off",
        title: "\"Phone setup…\" with a Phone set up, calls off and Bluetooth off: the finish page under both cards, Done keeping Enter since calls are off by choice (#389)",
        still: |_| true,
        prepare: as_is,
        reach: |harness| {
            sync(harness);
            let _output = harness.cli(&["calls", "off", "--until-morning"]);
            harness.wait_for_app("calls are off", DEADLINE, |state| {
                state.calls_off == "off" && state.connection_state == "disconnected"
            });
            harness.set_bluetooth_power("off");
            let _output = harness.cli(&["connect"]);
            harness.wait_for_app(
                "the finish page says calls are off and Bluetooth is off",
                DEADLINE,
                |state| {
                    setup_window_shows(state, "step", "done")
                        && setup_window_shows(state, "calls-off", "Calls are off")
                        && shows_problem(state, "bluetooth-off")
                },
            );
        },
        moment: None,
        shown: |screenshot, _| app_region(screenshot).is_some(),
    },
];

/// Longer than the Phone's 30 s contacts prompt, which starts only once
/// the Phone has paired and connected.
const PROMPT_TIMEOUT: Duration = Duration::from_secs(50);

/// The Setup Window when the Phone didn't share Contacts (#26).
pub(super) const SETUP_CONTACTS_STATES: &[State] = &[
    State {
        name: "setup-window-contacts-prompt-timed-out",
        title: "The Setup Window after nobody answered the Phone's contacts prompt: Try again, with its key, and Skip",
        still: |_| true,
        prepare: fresh_install,
        moment: None,
        reach: |harness| {
            harness.simulated_phone().set_contacts_prompt("ask");
            pair_through_setup_window(harness);
            harness.wait_for_app(
                "the Setup Window says the Phone stopped asking",
                PROMPT_TIMEOUT,
                |state| setup_window_shows(state, "step", "contacts-prompt-timed-out"),
            );
        },
        shown: |screenshot, _| app_region(screenshot).is_some(),
    },
    State {
        name: "setup-window-done-contacts-denied",
        title: "The Setup Window's finish page after the Phone refused Contacts: calls show numbers, and where to allow it",
        still: |_| true,
        prepare: |builder| fresh_install(builder).config_under_home(),
        moment: None,
        reach: |harness| {
            with_hyprland_config(harness);
            harness.simulated_phone().set_contacts_prompt("deny");
            pair_through_setup_window(harness);
            harness.wait_for_app(
                "the finish page says Contacts aren't shared, and offers the keybinding",
                DEADLINE,
                |state| {
                    setup_window_shows(state, "buttons", "Add keybinding\nDone")
                        && setup_window_shows(state, "step", "done")
                        && state
                            .surface("setup-window")
                            .and_then(|window| window.get("summary"))
                            .is_some_and(|summary| {
                                summary.starts_with("Your phone isn't sharing contacts,")
                            })
                },
            );
        },
        shown: |screenshot, _| app_region(screenshot).is_some(),
    },
];

fn audio_not_set_up(builder: Builder) -> Builder {
    fresh_install(builder).wireplumber_config("51-music.conf", ROLES_WITHOUT_HANDS_FREE)
}

fn shows_problem(state: &AppState, kind: &str) -> bool {
    state
        .surface("setup-problems")
        .and_then(|problems| problems.get("problems"))
        .is_some_and(|shown| shown == kind)
}

/// Waits until the Setup Window shows the Problem `kind`, and the Phone
/// listed when `phone_listed` (with Bluetooth off or blocked there is none).
fn wait_for_problem(harness: &Harness, kind: &str, phone_listed: bool) {
    harness.wait_for_app(
        &format!("the Setup Window shows the Problem {kind:?}"),
        DEADLINE,
        |state| {
            shows_problem(state, kind)
                && setup_window_shows(state, "step", "choose")
                && (!phone_listed || setup_window_shows(state, "phones", PHONE_NAME))
        },
    );
}
