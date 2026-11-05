//! Scenarios: the app reads Omarchy 4.5's Do Not Disturb, from
//! `~/.local/state/omarchy/notifications/quiet.json`, at startup and live,
//! and reads it as off wherever Omarchy would, or without Omarchy.

use std::time::Duration;

use test_harness::Harness;

/// How long the app may take to see a Do Not Disturb change.
const DEADLINE: Duration = Duration::from_secs(2);

/// How long the app may take to see Omarchy's state directory appear after
/// it started: GLib looks for a directory that isn't there every 4 s.
const APPEAR_DEADLINE: Duration = Duration::from_secs(10);

fn wait_for_do_not_disturb(harness: &Harness, on: bool, deadline: Duration) {
    harness.wait_for_app(&format!("DoNotDisturb is {on}"), deadline, |state| {
        state.do_not_disturb == on
    });
}

#[test]
fn the_app_follows_do_not_disturb_as_omarchy_45_toggles_it() {
    // Omarchy's shell has prepared its state: an empty `quiet.json`, off.
    let harness = Harness::start();
    wait_for_do_not_disturb(&harness, false, DEADLINE);

    harness.set_do_not_disturb(true);
    wait_for_do_not_disturb(&harness, true, DEADLINE);

    harness.set_do_not_disturb(false);
    wait_for_do_not_disturb(&harness, false, DEADLINE);
}

#[test]
fn a_desktop_left_in_do_not_disturb_starts_in_it() {
    let harness = Harness::builder().do_not_disturb().start();
    wait_for_do_not_disturb(&harness, true, DEADLINE);
}

/// What Omarchy 4.5 makes of each `quiet.json`: whether its shell starts
/// with Do Not Disturb on. Worked out by running Omarchy's own `Store.js`
/// `parseQuiet` on these texts; see
/// `docs/research/2026-11-05-omarchy-45-do-not-disturb.md`. Each verdict
/// differs from the one before, so the app's change shows.
const OMARCHY_VERDICTS: &[(&str, bool)] = &[
    // What Omarchy's migration from 4.0 writes for a Desktop left in Do Not
    // Disturb.
    (
        "{\"snoozes\":{},\"dnd\":true,\"silencedSince\":0,\"codesBypassQuiet\":true}\n",
        true,
    ),
    ("not json", false),
    ("{ \"dnd\" : true }", true),
    ("{\"dnd\":\"true\"}", false),
    ("{\"version\":3,\"dnd\":true}", true),
    ("{\"dnd\":1}", false),
    ("{\"dnd\":true}\n", true),
    ("[true]", false),
    ("{\"dnd\":true}\n", true),
    ("{\"dnd\":true", false),
    ("{\"dnd\":true}\n", true),
    ("\n", false),
];

#[test]
fn the_app_reads_quiet_json_as_omarchy_does() {
    let harness = Harness::start();
    wait_for_do_not_disturb(&harness, false, DEADLINE);

    for (contents, on) in OMARCHY_VERDICTS {
        harness.replace_omarchy_quiet_file(contents);
        harness.wait_for_app(
            &format!("DoNotDisturb is {on} for {contents:?}"),
            DEADLINE,
            |state| state.do_not_disturb == *on,
        );
    }
}

#[test]
fn without_omarchy_do_not_disturb_is_off_until_omarchy_turns_it_on() {
    let harness = Harness::builder().no_omarchy_notification_state().start();
    wait_for_do_not_disturb(&harness, false, DEADLINE);

    // Omarchy installed after Desktop Calls started, and Do Not Disturb
    // turned on.
    harness.set_do_not_disturb(true);
    wait_for_do_not_disturb(&harness, true, APPEAR_DEADLINE);
}
