//! Scenarios: a Phone that reports the caller's number after the Call
//! appears, as PipeWire does for a Phone without `+CLCC`, whose number
//! comes with the `+CLIP` after the first `RING`. The Simulated Phone's
//! `late-line-identification` quirk does that. Every surface showing the
//! Call follows the number once it arrives: the Incoming Call Banner, the
//! Missed Call notification, and the Dialer it opens. Until the number
//! arrives, or the Phone has had long enough to send it, the Banner says it
//! is identifying the caller rather than "Withheld number". Observed through the
//! control interface's `Calls`, `Surfaces` and `MissedCall`, and the
//! notification server stand-in.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use test_harness::vcards::{self, CallKind};
use test_harness::{AppState, Harness, Notification};

/// How long the app may take to show a change.
const DEADLINE: Duration = Duration::from_secs(5);
/// How long a whole sync, or a Call History refresh, may take.
const SYNC_DEADLINE: Duration = Duration::from_secs(15);

/// Alice's number as the Phone reports her Calls.
const ALICE: &str = "+447700900123";

/// A Phone with Alice in its Contacts, connected and synced, that reports
/// each caller's number late, on a screen of its own.
fn synced_late() -> Harness {
    let harness = Harness::builder().headless_screen().start();
    let phone = harness.simulated_phone();
    let mut contacts = vcards::contact("Alice", &[("CELL", "+44 7700 900123")]);
    // Clear of Alice's number, so the app can tell the numbers are British.
    contacts.push_str(&vcards::contacts(5, 1_000));
    phone.set_pbap_folder("pb", &contacts);
    phone.set_pbap_folder("cch", &older_call());
    phone.enable_quirk("late-line-identification");
    phone.connect();
    harness.wait_for_app(
        "Contacts and Call History are synced",
        SYNC_DEADLINE,
        |state| state.contacts.state == "ready" && state.contacts.call_history == 1,
    );
    harness
}

/// An older Call, so the Call History isn't empty before the Missed Call.
fn older_call() -> String {
    vcards::call(CallKind::Dialed, Some("+447700900001"), "20261104T090000")
}

fn banner(state: &AppState) -> Option<&BTreeMap<String, String>> {
    state.surface("banner")
}

/// Rings the Phone from Alice, and waits until the app has her number,
/// which the Phone reports after the Call. Returns the Call's object path
/// and the state then.
fn ring_from_alice(harness: &Harness) -> (String, AppState) {
    let path = harness.simulated_phone().ring(ALICE);
    let state = harness.wait_for_app("the Call shows Alice's number", DEADLINE, |state| {
        state.calls.len() == 1 && state.calls[0].number == ALICE
    });
    (path, state)
}

/// Waits for a Missed Call notification reading `summary`, and for the
/// `MissedCall` signal the app sends once it is shown.
fn wait_for_notification(harness: &Harness, summary: &str) -> Notification {
    let notification = harness.notifications().wait_for(
        &format!("a notification {summary:?}"),
        SYNC_DEADLINE,
        |notification| notification.summary == summary,
    );
    let started = Instant::now();
    while harness.missed_calls().is_empty() {
        assert!(started.elapsed() < DEADLINE, "no MissedCall signal");
        std::thread::sleep(Duration::from_millis(20));
    }
    notification
}

/// The Banner names Alice once the Phone reports her number, though the
/// Call appeared with none.
#[test]
fn the_banner_shows_the_contact_once_the_phone_reports_the_number_late() {
    let harness = synced_late();
    let (_, state) = ring_from_alice(&harness);
    assert_eq!(state.calls[0].display_name, "Alice", "{:?}", state.calls);

    let state = harness.wait_for_app("the Banner names Alice", DEADLINE, |state| {
        banner(state)
            .and_then(|banner| banner.get("caller"))
            .map(String::as_str)
            == Some("Alice")
    });
    let banner = banner(&state).expect("the Banner is shown");
    assert_eq!(
        banner.get("number-label").map(String::as_str),
        Some("mobile"),
        "{banner:?}"
    );
}

/// What the Banner's caller line says while the Phone may still report
/// the number.
const IDENTIFYING: &str = "Identifying caller…";

/// The Banner's caller line, while the Banner shows.
fn banner_caller(state: &AppState) -> Option<String> {
    banner(state)?.get("caller").cloned()
}

/// While the number is on its way, the Banner doesn't claim it is
/// withheld: from the moment it shows until it names Alice, it never reads
/// "Withheld number".
#[test]
fn the_banner_never_says_withheld_number_while_the_number_is_on_its_way() {
    let harness = synced_late();
    let seen = RefCell::new(Vec::new());

    let _path = harness.simulated_phone().ring(ALICE);
    harness.wait_for_app("the Banner names Alice", DEADLINE, |state| {
        let caller = banner_caller(state);
        if let Some(caller) = &caller {
            seen.borrow_mut().push(caller.clone());
        }
        caller.as_deref() == Some("Alice")
    });

    let seen = seen.into_inner();
    assert!(
        !seen.iter().any(|caller| caller == "Withheld number"),
        "the Banner's caller line read, in turn: {seen:?}"
    );
}

/// A Call whose number is truly withheld: the Banner says it is
/// identifying the caller, then, once the Phone has had long enough to
/// report a number, "Withheld number".
#[test]
fn a_withheld_number_shows_as_withheld_once_the_wait_is_over() {
    let harness = synced_late();

    let _path = harness.simulated_phone().ring("");
    harness.wait_for_app("the Banner is identifying the caller", DEADLINE, |state| {
        banner_caller(state).as_deref() == Some(IDENTIFYING)
    });
    let state = harness.wait_for_app(
        "the Banner says the number is withheld",
        DEADLINE,
        |state| banner_caller(state).as_deref() != Some(IDENTIFYING),
    );

    assert_eq!(
        banner_caller(&state).as_deref(),
        Some("Withheld number"),
        "{state:?}"
    );
}

/// A Call whose number came late rings out: the Missed Call names Alice,
/// and the Dialer its click opens selects her.
#[test]
fn a_missed_call_whose_number_came_late_shows_the_contact() {
    let harness = synced_late();
    let (path, state) = ring_from_alice(&harness);
    let call = state.calls[0].id;
    let mut history = vcards::call(CallKind::Missed, Some(ALICE), "20261105T100000");
    history.push_str(&older_call());
    harness.simulated_phone().set_pbap_folder("cch", &history);

    harness.simulated_phone().caller_hangs_up(&path);

    let notification = wait_for_notification(&harness, "Missed call from Alice (mobile)");
    let signalled = harness.missed_calls();
    assert_eq!(
        signalled
            .iter()
            .map(|signal| (
                signal.call,
                signal.number.as_str(),
                signal.display_name.as_str()
            ))
            .collect::<Vec<_>>(),
        vec![(call, ALICE, "Alice")]
    );

    harness
        .notifications()
        .invoke_default_action(notification.id);
    let state = harness.wait_for_app("the Dialer selects Alice", DEADLINE, |state| {
        state
            .surface("dialer")
            .and_then(|dialer| dialer.get(&format!("row-{}", dialer.get("selected")?)))
            .is_some_and(|row| row.contains("Alice") && row.contains("Missed"))
    });
    assert!(state.surface("dialer").is_some());
}
