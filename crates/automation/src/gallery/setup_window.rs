//! The Setup Window's Problems in the gallery, each one card with its Fix
//! or its command: Bluetooth off (the step is dimmed, and Enter presses
//! Turn on), Bluetooth blocked, Contacts support missing, audio not set up
//! and BlueZ's own hands-free on (these three leave the step as it is,
//! with a Phone listed).

use test_harness::{AppState, Builder, Harness};

use super::{DEADLINE, PHONE_NAME, State, app_region, fresh_install, setup_window_shows};

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
        name: "setup-problem-bluetooth-off",
        title: "The Setup Window with Bluetooth off: one card with Turn on and its key, the steps dimmed, and no Phone to list",
        still: |_| true,
        prepare: |builder| fresh_install(builder).bluetooth_off(),
        reach: |harness| wait_for_problem(harness, "bluetooth-off", false),
        shown: |screenshot, _| app_region(screenshot).is_some(),
    },
    State {
        name: "setup-problem-bluetooth-blocked",
        title: "The Setup Window with Bluetooth blocked: one card with the command to copy",
        still: |_| true,
        prepare: |builder| fresh_install(builder).bluetooth_blocked(),
        reach: |harness| wait_for_problem(harness, "bluetooth-blocked", false),
        shown: |screenshot, _| app_region(screenshot).is_some(),
    },
    State {
        name: "setup-problem-contacts-support-missing",
        title: "The Setup Window without Contacts support: the card with the install command, the Phone listed and Enter still pairing",
        still: |_| true,
        prepare: |builder| fresh_install(builder).without_contacts_support(),
        reach: |harness| wait_for_problem(harness, "contacts-support-missing", true),
        shown: |screenshot, _| app_region(screenshot).is_some(),
    },
    State {
        name: "setup-problem-audio-not-set-up",
        title: "The Setup Window with audio not set up for Calls: the card with Set up audio, the Phone listed and Enter still pairing",
        still: |_| true,
        prepare: audio_not_set_up,
        reach: |harness| wait_for_problem(harness, "audio-not-set-up", true),
        shown: |screenshot, _| app_region(screenshot).is_some(),
    },
    State {
        name: "setup-problem-bluez-hands-free",
        title: "The Setup Window with BlueZ's own calling on: the card with its Fix, the Phone listed and Enter still pairing",
        still: |_| true,
        prepare: |builder| fresh_install(builder).bluez_hands_free_on(),
        reach: |harness| wait_for_problem(harness, "bluez-hands-free", true),
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
