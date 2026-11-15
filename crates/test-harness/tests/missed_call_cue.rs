//! Scenarios: the lasting Missed Call cue (#236). After a Missed Call the
//! Indicator's icon carries a badge, its tooltip and menu count the Missed
//! Calls, and the menu's line opens the Dialer, which marks them at the top
//! of its Call History. Opening the Dialer, or Call back, clears the cue. It
//! counts under Do Not Disturb, and survives a restart. Observed through the
//! tray watcher stand-in, the notification server stand-in and `Surfaces`.

use std::time::Duration;

use test_harness::vcards::{self, CallKind};
use test_harness::{Harness, MenuEntry, TrayItem};

/// How long the app may take to show a change.
const DEADLINE: Duration = Duration::from_secs(5);
/// How long a whole sync, or a Call History refresh, may take.
const SYNC_DEADLINE: Duration = Duration::from_secs(15);

const PHONE_NAME: &str = "My Pixel";
/// Alice's number as the Phone reports her Calls.
const ALICE: &str = "+447700900123";

const CONNECTED_ICON: &str = "desktop-calls-connected-symbolic";
const BADGED_ICON: &str = "desktop-calls-connected-missed-symbolic";

fn contacts() -> String {
    let mut all = vcards::contact("Alice", &[("CELL", "+44 7700 900123")]);
    all.push_str(&vcards::contacts(5, 1_000));
    all
}

/// An older Call, so the Call History isn't empty before the Missed Calls.
fn older_call() -> String {
    vcards::call(CallKind::Dialed, Some("+447700900001"), "20261104T090000")
}

/// A Phone with Contacts and a Call History, connected and synced.
fn synced() -> Harness {
    let harness = Harness::builder()
        .headless_screen()
        .phone_name(PHONE_NAME)
        .start();
    let phone = harness.simulated_phone();
    phone.set_pbap_folder("pb", &contacts());
    phone.set_pbap_folder("cch", &older_call());
    phone.connect();
    harness.wait_for_app(
        "Contacts and Call History are synced",
        SYNC_DEADLINE,
        |state| state.contacts.state == "ready" && state.contacts.call_history == 1,
    );
    harness
}

/// Alice rings and gives up; the Phone lists a Missed Call from her, the
/// newest of `missed` there, at `at`. Waits for the notification.
fn alice_rings_out(harness: &Harness, missed: &[&str], summary: &str) {
    let path = harness.simulated_phone().ring(ALICE);
    harness.wait_for_app("the Call rings", DEADLINE, |state| state.calls.len() == 1);
    let mut history: String = missed
        .iter()
        .map(|at| vcards::call(CallKind::Missed, Some(ALICE), at))
        .collect();
    history.push_str(&older_call());
    harness.simulated_phone().set_pbap_folder("cch", &history);
    harness.simulated_phone().caller_hangs_up(&path);
    harness.notifications().wait_for(
        &format!("a notification {summary:?}"),
        SYNC_DEADLINE,
        |notification| notification.summary == summary,
    );
}

/// Waits until the Indicator's tooltip is `tooltip`, and returns it.
fn indicator_says(harness: &Harness, tooltip: &str) -> TrayItem {
    let items = harness.tray().wait_for(
        &format!("the Indicator says {tooltip:?}"),
        DEADLINE,
        |items| {
            items
                .iter()
                .any(|item| item.tooltip.as_deref() == Some(tooltip))
        },
    );
    items.into_iter().next().unwrap_or_else(|| unreachable!())
}

/// Waits until the menu's labels are `labels`.
fn menu_reads(harness: &Harness, labels: &[&str]) {
    harness
        .tray()
        .wait_for_menu(&format!("the menu reads {labels:?}"), DEADLINE, |menu| {
            MenuEntry::labels(menu) == labels
        });
}

const CUE_MENU: [&str; 7] = [
    "My Pixel connected",
    "1 missed call…",
    "New call…",
    "Turn calls off",
    "Quiet hours…",
    "---",
    "Phone setup…",
];
const PLAIN_MENU: [&str; 6] = [
    "My Pixel connected",
    "New call…",
    "Turn calls off",
    "Quiet hours…",
    "---",
    "Phone setup…",
];

#[test]
fn a_missed_call_badges_the_indicator_and_its_menu_line_opens_the_dialer_which_clears_it() {
    let harness = synced();
    let plain = indicator_says(&harness, "My Pixel connected");
    assert_eq!(plain.icon_name.as_deref(), Some(CONNECTED_ICON));

    alice_rings_out(&harness, &["20261115T090000"], "Missed call from Alice");

    let badged = indicator_says(&harness, "My Pixel connected · 1 missed call");
    assert_eq!(badged.icon_name.as_deref(), Some(BADGED_ICON));
    menu_reads(&harness, &CUE_MENU);

    harness.tray().click("1 missed call…");
    let state = harness.wait_for_app("the Dialer opens", DEADLINE, |state| {
        state.surface("dialer").is_some()
    });
    let dialer = state.surface("dialer").cloned().unwrap_or_default();
    assert_eq!(
        dialer.get("new-missed").map(String::as_str),
        Some("1"),
        "the Dialer marks the Missed Call: {dialer:?}"
    );
    assert!(
        dialer
            .get("row-0")
            .is_some_and(|row| row.starts_with("Alice · Missed")),
        "the Missed Call is at the top: {dialer:?}"
    );

    let cleared = indicator_says(&harness, "My Pixel connected");
    assert_eq!(cleared.icon_name.as_deref(), Some(CONNECTED_ICON));
    menu_reads(&harness, &PLAIN_MENU);
}

#[test]
fn a_missed_call_under_do_not_disturb_sets_the_cue_too() {
    let harness = synced();
    harness.set_do_not_disturb(true);
    harness.wait_for_app("DoNotDisturb is on", DEADLINE, |state| state.do_not_disturb);

    alice_rings_out(&harness, &["20261115T090000"], "Missed call from Alice");

    let badged = indicator_says(&harness, "My Pixel connected · 1 missed call");
    assert_eq!(badged.icon_name.as_deref(), Some(BADGED_ICON));
    menu_reads(&harness, &CUE_MENU);
}

#[test]
fn the_cue_counts_each_missed_call_and_survives_a_restart() {
    let mut harness = synced();
    alice_rings_out(&harness, &["20261115T090000"], "Missed call from Alice");
    alice_rings_out(
        &harness,
        &["20261115T090500", "20261115T090000"],
        "2 missed calls from Alice",
    );
    indicator_says(&harness, "My Pixel connected · 2 missed calls");

    harness.restart_app();

    harness
        .tray()
        .wait_for_menu("the menu still counts 2", SYNC_DEADLINE, |menu| {
            MenuEntry::find(menu, "2 missed calls…").is_some()
        });
}
