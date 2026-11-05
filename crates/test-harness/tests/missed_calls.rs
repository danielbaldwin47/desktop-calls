//! Scenarios: Missed Calls and withheld numbers. A Call that rings out and
//! that the Phone lists as missed raises one notification per caller;
//! clicking it opens the Dialer with that Call selected, and its Call back
//! places the Call. Observed through the notification server stand-in, the
//! control interface's `MissedCall` signal and `Surfaces`, and what the
//! Simulated Phone was asked to do.

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use test_harness::vcards::{self, CallKind};
use test_harness::{AppState, Harness, Key, Notification, RecordedMethodCall};

/// How long the app may take to show a change.
const DEADLINE: Duration = Duration::from_secs(5);
/// How long a whole sync, or a Call History refresh, may take.
const SYNC_DEADLINE: Duration = Duration::from_secs(15);
/// How long something that must not happen is watched for.
const HOLD: Duration = Duration::from_millis(1_000);

/// Alice's number as the Phone reports her Calls.
const ALICE: &str = "+447700900123";

fn contacts() -> String {
    let mut all = vcards::contact("Alice", &[("CELL", "+44 7700 900123")]);
    // Clear of Alice's number, so the app can tell the numbers are British.
    all.push_str(&vcards::contacts(5, 1_000));
    all
}

/// An older Call, so the Call History isn't empty before the Missed Calls.
fn older_call() -> String {
    vcards::call(CallKind::Dialed, Some("+447700900001"), "20261104T090000")
}

/// Puts `newest` on top of the Phone's Call History, newest first, as the
/// Phone does once a Call ends.
fn call_history(harness: &Harness, newest: &[String]) {
    let mut all = newest.concat();
    all.push_str(&older_call());
    harness.simulated_phone().set_pbap_folder("cch", &all);
}

fn missed(number: Option<&str>, at: &str) -> String {
    vcards::call(CallKind::Missed, number, at)
}

/// A Phone with Contacts and a Call History, connected and synced, on a
/// screen of its own.
fn synced() -> Harness {
    synced_with(&contacts())
}

/// [`synced`], with `contacts` as the Phone's Contacts.
fn synced_with(contacts: &str) -> Harness {
    let harness = Harness::builder().headless_screen().start();
    let phone = harness.simulated_phone();
    phone.set_pbap_folder("pb", contacts);
    call_history(&harness, &[]);
    phone.connect();
    harness.wait_for_app(
        "Contacts and Call History are synced",
        SYNC_DEADLINE,
        |state| state.contacts.state == "ready" && state.contacts.call_history == 1,
    );
    harness
}

/// Rings the Phone from `number` and waits until the app lists the Call
/// ringing: the Desktop rings too, unless Do Not Disturb is on. Returns the
/// Call's object path and id.
fn ring(harness: &Harness, number: &str) -> (String, u32) {
    let path = harness.simulated_phone().ring(number);
    let state = harness.wait_for_app("the Call rings", DEADLINE, |state| {
        state.calls.len() == 1
            && state.calls[0].state == "ringing"
            && state.ringing != state.do_not_disturb
    });
    (path, state.calls[0].id)
}

/// The caller gives up before anyone answers.
fn rings_out(harness: &Harness, path: &str) {
    harness.simulated_phone().caller_hangs_up(path);
    harness.wait_for_app("the Call is over", DEADLINE, |state| state.calls.is_empty());
}

fn missed_call_notifications(harness: &Harness) -> Vec<Notification> {
    harness
        .notifications()
        .received()
        .into_iter()
        .filter(|notification| notification.app_name == "Desktop Calls")
        .collect()
}

/// Waits for a Missed Call notification reading `summary`, and for the
/// `MissedCall` signal the app sends once it is shown, so the app knows the
/// notification's id before the test acts on it.
fn wait_for_notification(harness: &Harness, summary: &str) -> Notification {
    let notification = harness.notifications().wait_for(
        &format!("a notification {summary:?}"),
        SYNC_DEADLINE,
        |notification| notification.summary == summary,
    );
    let shown = missed_call_notifications(harness).len();
    let started = Instant::now();
    while harness.missed_calls().len() < shown {
        assert!(
            started.elapsed() < DEADLINE,
            "{shown} notifications but only {:?} MissedCall signals",
            harness.missed_calls()
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    notification
}

/// Waits until the Dialer has the keyboard.
fn wait_for_dialer(harness: &Harness) -> AppState {
    harness.wait_for_app("the Dialer has keyboard focus", DEADLINE, |state| {
        dialer(state)
            .is_some_and(|dialer| dialer.get("focused").map(String::as_str) == Some("true"))
    })
}

/// Whether `text` is a time of day, `HH:MM`.
fn is_time(text: &str) -> bool {
    let bytes = text.as_bytes();
    bytes.len() == 5
        && bytes[2] == b':'
        && [0, 1, 3, 4]
            .iter()
            .all(|index| bytes[*index].is_ascii_digit())
}

fn action<'a>(notification: &'a Notification, key: &str) -> Option<&'a str> {
    notification
        .actions
        .iter()
        .find(|(offered, _)| offered == key)
        .map(|(_, label)| label.as_str())
}

fn dialer(state: &AppState) -> Option<&BTreeMap<String, String>> {
    state.surface("dialer")
}

/// The Dialer's selected row, or empty if the Dialer has none.
fn selected_row(state: &AppState) -> String {
    dialer(state)
        .and_then(|dialer| dialer.get(&format!("row-{}", dialer.get("selected")?)))
        .cloned()
        .unwrap_or_default()
}

/// Rings the Simulated Phone from a withheld number, one way a Phone may
/// send it. Returns the Call's object path.
type RingWithheld = fn(&Harness) -> String;

fn dials(recorded: &[RecordedMethodCall], number: &str) -> bool {
    let expected = format!("(\"{number}\",)");
    recorded
        .iter()
        .any(|call| call.member == "Dial" && call.arguments == expected)
}

fn dialled(harness: &Harness, number: &str) {
    harness.simulated_phone().wait_for_recorded(
        &format!("the Phone is asked to dial {number}"),
        DEADLINE,
        |recorded| dials(recorded, number),
    );
}

/// No Missed Call notification arrives while `HOLD` passes.
fn never_notified(harness: &Harness) {
    std::thread::sleep(HOLD);
    assert_eq!(missed_call_notifications(harness), Vec::new());
    assert_eq!(harness.missed_calls(), Vec::new());
}

/// Scenario 8: the caller hangs up while it rings, so a Missed Call
/// notification appears; its click opens the Dialer with the Call
/// selected, and Enter calls back.
#[test]
fn scenario_8_a_caller_who_hangs_up_leaves_a_missed_call_notification_whose_click_calls_back() {
    let harness = synced();
    let (path, call) = ring(&harness, ALICE);
    call_history(&harness, &[missed(Some(ALICE), "20261105T100000")]);

    rings_out(&harness, &path);

    let notification = wait_for_notification(&harness, "Missed call from Alice (mobile)");
    assert_eq!(notification.app_name, "Desktop Calls");
    assert!(is_time(&notification.body), "{notification:?}");
    assert!(
        action(&notification, "default").is_some(),
        "{notification:?}"
    );
    assert_eq!(action(&notification, "call-back"), Some("Call back"));
    assert_eq!(action(&notification, "dismiss"), Some("Dismiss"));
    assert_eq!(
        notification.hints.get("category").map(String::as_str),
        Some("\"call.unanswered\"")
    );
    let signalled = harness.missed_calls();
    assert_eq!(signalled.len(), 1, "{signalled:?}");
    assert_eq!(
        (
            signalled[0].call,
            signalled[0].number.as_str(),
            signalled[0].display_name.as_str(),
            signalled[0].count
        ),
        (call, ALICE, "Alice", 1)
    );
    assert_eq!(missed_call_notifications(&harness).len(), 1);

    harness
        .notifications()
        .invoke_default_action(notification.id);

    let state = wait_for_dialer(&harness);
    let row = selected_row(&state);
    assert!(row.contains("Alice") && row.contains("Missed"), "{row:?}");
    harness.keyboard().press(Key::Return);
    dialled(&harness, ALICE);
}

#[test]
fn call_back_on_the_notification_places_the_call() {
    let harness = synced();
    let (path, _) = ring(&harness, ALICE);
    call_history(&harness, &[missed(Some(ALICE), "20261105T100000")]);
    rings_out(&harness, &path);
    let notification = wait_for_notification(&harness, "Missed call from Alice (mobile)");

    harness
        .notifications()
        .invoke_action(notification.id, "call-back");

    dialled(&harness, ALICE);
}

#[test]
fn three_missed_calls_from_alice_leave_one_notification_counting_them() {
    let harness = synced();
    let mut newest = Vec::new();
    for (count, at) in [
        (1, "20261105T100000"),
        (2, "20261105T100500"),
        (3, "20261105T101000"),
    ] {
        let (path, _) = ring(&harness, ALICE);
        newest.insert(0, missed(Some(ALICE), at));
        call_history(&harness, &newest);
        rings_out(&harness, &path);
        harness.notifications().wait_for(
            &format!("the notification counts {count}"),
            SYNC_DEADLINE,
            |notification| {
                if count == 1 {
                    notification.summary == "Missed call from Alice (mobile)"
                } else {
                    notification.summary == format!("{count} missed calls from Alice")
                }
            },
        );
    }

    let notifications = missed_call_notifications(&harness);
    assert_eq!(notifications.len(), 3, "{notifications:#?}");
    let first = notifications[0].id;
    assert!(
        notifications
            .iter()
            .all(|notification| notification.id == first),
        "each replaces the first: {notifications:#?}"
    );
    assert_eq!(notifications[1].replaces_id, first);
    assert_eq!(notifications[2].replaces_id, first);
    assert!(
        notifications[2].body.starts_with("Latest "),
        "{:?}",
        notifications[2]
    );
}

#[test]
fn a_call_declined_on_the_desktop_is_not_notified() {
    let harness = synced();
    let (_, call) = ring(&harness, ALICE);
    // Some Phones list a Call declined from a hands-free device as missed.
    call_history(&harness, &[missed(Some(ALICE), "20261105T100000")]);

    harness.decline(call);

    harness.wait_for_app("the Call is over", DEADLINE, |state| state.calls.is_empty());
    harness.wait_for_app("the Call History is refreshed", SYNC_DEADLINE, |state| {
        state.contacts.call_history == 2
    });
    never_notified(&harness);
}

#[test]
fn a_call_declined_on_the_phone_is_not_in_its_missed_calls_and_is_not_notified() {
    let harness = synced();
    let (path, _) = ring(&harness, ALICE);
    call_history(
        &harness,
        &[vcards::call(
            CallKind::Received,
            Some(ALICE),
            "20261105T100000",
        )],
    );

    harness.simulated_phone().decline_on_phone(&path);

    harness.wait_for_app("the Call is over", DEADLINE, |state| state.calls.is_empty());
    harness.wait_for_app("the Call History is refreshed", SYNC_DEADLINE, |state| {
        state.contacts.call_history == 2
    });
    never_notified(&harness);
}

#[test]
fn with_contacts_denied_a_call_that_rings_out_is_still_notified() {
    let harness = Harness::start();
    let phone = harness.simulated_phone();
    phone.set_pbap_folder("pb", &contacts());
    phone.set_contacts_prompt("deny");
    phone.connect();
    harness.wait_for_app("ContactsState is denied", SYNC_DEADLINE, |state| {
        state.contacts.state == "denied"
    });
    let (path, _) = ring(&harness, ALICE);

    rings_out(&harness, &path);

    let notification = wait_for_notification(&harness, "Missed call from +44 7700 900123");
    assert!(is_time(&notification.body), "{notification:?}");
    assert_eq!(harness.missed_calls().len(), 1);
}

#[test]
fn a_call_that_rings_out_is_notified_if_the_call_history_refresh_fails() {
    let harness = synced();
    let (path, _) = ring(&harness, ALICE);
    // The Phone stops offering any Call History.
    for folder in ["cch", "mch", "ich", "och"] {
        harness.simulated_phone().remove_pbap_folder(folder);
    }

    rings_out(&harness, &path);

    wait_for_notification(&harness, "Missed call from Alice (mobile)");
}

/// A Contact's name can't change how Omarchy treats the notification. Omarchy
/// 4.5 lets through Do Not Disturb any notification in which it finds a
/// verification code: 4 to 8 digits near a word such as "code" or "pin". A
/// Missed Call from a Contact named "Gate code 4821" reads the same as ever,
/// yet is silenced like Alice's and offers no code to copy.
#[test]
fn a_missed_call_from_a_contact_named_like_a_verification_code_is_silenced_under_do_not_disturb() {
    // The Phone reports the gate's Calls from this number.
    const GATE: &str = "+447700900456";
    let mut all = contacts();
    all.push_str(&vcards::contact(
        "Gate code 4821",
        &[("CELL", "+44 7700 900456")],
    ));
    let harness = synced_with(&all);
    harness.set_do_not_disturb(true);

    let mut newest = Vec::new();
    for (number, reads, at) in [
        (ALICE, "Missed call from Alice (mobile)", "20261105T100000"),
        (
            GATE,
            "Missed call from Gate code 4821 (mobile)",
            "20261105T100500",
        ),
        (
            GATE,
            "2 missed calls from Gate code 4821",
            "20261105T101000",
        ),
    ] {
        let (path, _) = ring(&harness, number);
        newest.insert(0, missed(Some(number), at));
        call_history(&harness, &newest);

        rings_out(&harness, &path);

        let notification = harness.notifications().wait_for(
            &format!("a notification reading {reads:?}"),
            SYNC_DEADLINE,
            |notification| as_read(&notification.summary) == reads,
        );
        assert_eq!(
            (notification.silenced, notification.verification_codes.len()),
            (true, 0),
            "{notification:?}"
        );
    }
}

/// `text` as a reader sees it: without the invisible format characters
/// (zero-width spaces and joiners, word joiners, byte order marks).
fn as_read(text: &str) -> String {
    text.chars()
        .filter(|character| {
            !matches!(
                character,
                '\u{200b}'..='\u{200f}' | '\u{2060}'..='\u{2064}' | '\u{feff}'
            )
        })
        .collect()
}

/// Scenario 9, and the withheld quirk scenario: a withheld number arrives
/// as an empty `LineIdentification`, as `"withheld"`, and as the `"",129`
/// caller ID PipeWire can't parse. Each shows "Withheld number" on the
/// Banner and, rung out, is notified; the Dialer lists it as one that
/// can't be called back, and Enter on it sends nothing to the Phone.
#[test]
fn scenario_9_a_withheld_number_rings_then_is_missed_and_cant_be_called_back() {
    let harness = synced();
    let phone = harness.simulated_phone();
    let mut newest = Vec::new();
    let forms: [(&str, RingWithheld); 3] = [
        ("empty", |harness| harness.simulated_phone().ring("")),
        ("\"withheld\"", |harness| {
            harness.simulated_phone().ring("withheld")
        }),
        ("\"\",129", |harness| {
            harness.simulated_phone().ring_with_caller_id("\"\",129")
        }),
    ];
    let mut last = None;
    for (count, (form, ring)) in (1_u32..).zip(forms) {
        let path = ring(&harness);
        let state = harness.wait_for_app(
            &format!("the Banner shows for a number withheld as {form}"),
            DEADLINE,
            |state| state.surface("banner").is_some(),
        );
        let banner = state.surface("banner").expect("the Banner is shown");
        assert_eq!(
            banner.get("caller").map(String::as_str),
            Some("Withheld number"),
            "withheld as {form}: {banner:?}"
        );
        newest.insert(0, missed(None, &format!("20261105T10{:02}00", count * 5)));
        call_history(&harness, &newest);

        rings_out(&harness, &path);

        let summary = if count == 1 {
            "Missed call from a withheld number".to_owned()
        } else {
            format!("{count} missed calls from withheld numbers")
        };
        let notification = wait_for_notification(&harness, &summary);
        assert!(
            notification.body.ends_with("can't be called back"),
            "{notification:?}"
        );
        assert_eq!(action(&notification, "call-back"), None, "{notification:?}");
        last = Some(notification);
    }
    let signalled = harness.missed_calls();
    assert_eq!(signalled.len(), 3, "{signalled:?}");
    assert!(signalled.iter().all(|signal| signal.number.is_empty()));

    let notification = last.expect("notified");
    harness
        .notifications()
        .invoke_default_action(notification.id);
    let state = wait_for_dialer(&harness);
    let row = selected_row(&state);
    assert!(
        row.contains("Withheld number") && row.contains("can't be called back"),
        "{row:?}"
    );
    harness.keyboard().press(Key::Return);
    let state = harness.wait_for_app("Enter explains", DEADLINE, |state| {
        dialer(state).is_some_and(|dialer| dialer.contains_key("notice"))
    });
    assert_eq!(
        dialer(&state)
            .and_then(|dialer| dialer.get("notice"))
            .map(String::as_str),
        Some("That caller withheld their number, so it can't be called back.")
    );
    phone.recorded_holds("the Phone is never asked to dial", HOLD, |recorded| {
        !recorded.iter().any(|call| call.member == "Dial")
    });
}
