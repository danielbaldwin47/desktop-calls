//! Scenarios: the app reads Omarchy 4.5's Do Not Disturb, from
//! `~/.local/state/omarchy/notifications/quiet.json`, at startup and live,
//! and reads it as off wherever Omarchy would, or without Omarchy. Under Do
//! Not Disturb the Incoming Call Banner still shows but the Desktop doesn't
//! ring, and a Missed Call is still notified, for Omarchy to file away.

use std::time::{Duration, SystemTime};

use test_harness::{AppEvent, AppState, Builder, Harness};

/// How long the app may take to see a Do Not Disturb change.
const DEADLINE: Duration = Duration::from_secs(2);

/// How long the app may take to see Omarchy's state directory appear after
/// it started: GLib looks for a directory that isn't there every 4 s.
const APPEAR_DEADLINE: Duration = Duration::from_secs(10);

/// How long the app may take to notice a change on the Phone.
const PHONE_DEADLINE: Duration = Duration::from_secs(5);
/// How long the Banner may take to show after the Phone rings: from the
/// Call appearing to the Banner's first frame, by the app's log (polish
/// scorecard item 1).
const BANNER_DEADLINE: Duration = Duration::from_secs(1);
/// How long the tests' own round trip may take; the targets are asserted on
/// the app's log (ticket #83).
const ROUND_TRIP: Duration = Duration::from_secs(3);
/// How long the ringtone may take to stop or start once Do Not Disturb is
/// toggled mid-ring: from Omarchy's file being replaced to the app's ringer
/// acting, by the app's log.
const TOGGLE_DEADLINE: Duration = Duration::from_millis(250);
/// How long the Desktop is watched for a ring that must not come.
const HOLD: Duration = Duration::from_millis(1_000);
/// How long a Missed Call may take to be notified once the Call is over.
const NOTIFY_DEADLINE: Duration = Duration::from_secs(15);

const NUMBER: &str = "+447700900123";

fn wait_for_do_not_disturb(harness: &Harness, on: bool, deadline: Duration) {
    harness.wait_for_app(&format!("DoNotDisturb is {on}"), deadline, |state| {
        state.do_not_disturb == on
    });
}

/// Starts `builder` on a screen of its own, with the Phone connected.
fn connected(builder: Builder) -> Harness {
    let harness = builder.headless_screen().start();
    harness.simulated_phone().connect();
    harness.wait_for_app("ConnectionState is connected", PHONE_DEADLINE, |state| {
        state.connection_state == "connected"
    });
    harness
}

/// Whether the Call is ringing on the Phone and its Banner shows.
fn banner_shows(state: &AppState) -> bool {
    state.calls.len() == 1 && state.calls[0].state == "ringing" && state.surface("banner").is_some()
}

/// Rings the Phone and waits for the Banner, checking the app drew it within
/// [`BANNER_DEADLINE`] of the Call appearing. Returns the Call's object path
/// on the Phone, and the state once the Banner shows.
fn ring_until_banner(harness: &Harness) -> (String, AppState) {
    let path = harness.simulated_phone().ring(NUMBER);
    let state = harness.wait_for_app("the Banner shows", ROUND_TRIP, banner_shows);
    harness.assert_banner_within(state.calls[0].id, BANNER_DEADLINE);
    (path, state)
}

/// Toggles Do Not Disturb mid-ring and checks the app's ringer acted, by
/// its log, within [`TOGGLE_DEADLINE`] of Omarchy's file being replaced.
fn toggle_mid_ring(harness: &Harness, on: bool) {
    let since = SystemTime::now();
    harness.set_do_not_disturb(on);
    let (ringer, what) = if on {
        (
            "ringing-stopped",
            "the ringtone to stop under Do Not Disturb",
        )
    } else {
        (
            "ringing-started",
            "the ringtone to start without Do Not Disturb",
        )
    };
    harness.assert_app_within_since(what, since, &AppEvent::kind(ringer), TOGGLE_DEADLINE);
    harness.wait_for_app(
        &format!("Ringing is {} with the Banner still shown", !on),
        DEADLINE,
        |state| state.ringing != on && state.do_not_disturb == on && banner_shows(state),
    );
}

/// Scenario 16 (polish scorecard items 1 and 5).
#[test]
fn scenario_16_under_do_not_disturb_the_banner_shows_and_the_desktop_stays_silent() {
    let harness = connected(Harness::builder().do_not_disturb());
    wait_for_do_not_disturb(&harness, true, DEADLINE);

    let (_, state) = ring_until_banner(&harness);
    assert!(!state.ringing, "{state:?}");

    harness.app_holds("the Desktop stays silent", HOLD, |state| {
        !state.ringing && state.do_not_disturb
    });
    assert!(
        !harness.app_logged(&AppEvent::kind("ringing-started")),
        "the ringer started under Do Not Disturb"
    );
}

#[test]
fn turning_do_not_disturb_on_mid_ring_silences_the_desktop_and_off_rings_it_again() {
    let harness = connected(Harness::builder());
    let (_, state) = ring_until_banner(&harness);
    assert!(state.ringing && !state.do_not_disturb, "{state:?}");

    toggle_mid_ring(&harness, true);
    toggle_mid_ring(&harness, false);
}

#[test]
fn without_omarchys_state_file_do_not_disturb_is_off_and_the_desktop_rings() {
    let harness = connected(Harness::builder().no_omarchy_notification_state());

    let (_, state) = ring_until_banner(&harness);

    assert!(state.ringing && !state.do_not_disturb, "{state:?}");
}

#[test]
fn a_call_missed_under_do_not_disturb_is_still_notified_for_omarchy_to_file_away() {
    let harness = Harness::builder()
        .do_not_disturb()
        .headless_screen()
        .start();
    // Without Contacts, a Call that rings out is notified without waiting
    // for the Phone's Call History.
    let phone = harness.simulated_phone();
    phone.set_contacts_prompt("deny");
    phone.connect();
    harness.wait_for_app("ContactsState is denied", NOTIFY_DEADLINE, |state| {
        state.contacts.state == "denied"
    });
    let (path, _) = ring_until_banner(&harness);

    harness.simulated_phone().caller_hangs_up(&path);

    let notification = harness.notifications().wait_for(
        "a Missed Call notification",
        NOTIFY_DEADLINE,
        |notification| notification.app_name == "Desktop Calls",
    );
    // Omarchy keeps it off the screen and files it to notification history:
    // it isn't transient, and it asks for no sound of its own.
    assert!(notification.silenced, "{notification:?}");
    assert!(
        !notification
            .hints
            .keys()
            .any(|hint| hint == "transient" || hint.starts_with("sound")),
        "{notification:?}"
    );
    assert!(
        !harness.app_logged(&AppEvent::kind("ringing-started")),
        "the ringer started under Do Not Disturb"
    );
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
