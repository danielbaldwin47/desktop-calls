//! Scenarios: the Dialer finds anyone in Contacts and the Call History,
//! synced from the Simulated Phone, observed through the control
//! interface's `Search`.

use std::time::Duration;

use test_harness::vcards::{self, CallKind};
use test_harness::{AppDialerRow, AppState, Harness};

/// How long a whole sync may take.
const SYNC_DEADLINE: Duration = Duration::from_secs(15);

/// Contacts with a few awkward cases among others: accents, two numbers
/// with labels, and two equal matches that differ in how often they're
/// called. Every number is from Ofcom's drama ranges.
fn contacts() -> String {
    let mut all = String::new();
    all.push_str(&vcards::contact(
        "Joanna Smithers",
        &[("CELL", "+447700900002")],
    ));
    all.push_str(&vcards::contact("José Smith", &[("CELL", "+447700900001")]));
    all.push_str(&vcards::contact(
        "Alice Smith",
        &[("CELL", "+44 7700 900123"), ("WORK", "020 7946 0123")],
    ));
    all.push_str(&vcards::contact("Sam Brown", &[("CELL", "07700 900021")]));
    all.push_str(&vcards::contact("Sam Jones", &[("CELL", "07700 900022")]));
    all.push_str(&vcards::contacts(20, 1_000));
    all
}

/// Sam Jones has been called twice, Sam Brown once; and a stranger missed.
fn call_history() -> String {
    [
        vcards::call(CallKind::Dialed, Some("+447700900022"), "20261104T090000"),
        vcards::call(CallKind::Received, Some("+447700900022"), "20261104T080000"),
        vcards::call(CallKind::Dialed, Some("+447700900021"), "20261104T100000"),
        vcards::call(CallKind::Missed, Some("+447700900999"), "20261104T110000"),
    ]
    .concat()
}

fn synced() -> Harness {
    let harness = Harness::start();
    let phone = harness.simulated_phone();
    phone.set_pbap_folder("pb", &contacts());
    phone.set_pbap_folder("cch", &call_history());
    phone.connect();
    harness.wait_for_app(
        "Contacts and Call History are synced",
        SYNC_DEADLINE,
        |state: &AppState| state.contacts.state == "ready" && state.contacts.call_history == 4,
    );
    harness
}

fn names(rows: &[AppDialerRow]) -> Vec<&str> {
    rows.iter().map(|row| row.name.as_str()).collect()
}

#[test]
fn search_ranks_synced_contacts_and_call_history() {
    let harness = synced();

    assert_eq!(harness.search("jo sm")[0].name, "José Smith");
    assert_eq!(harness.search("jose")[0].name, "José Smith");

    let rows = harness.search("alice");
    assert_eq!(
        rows.iter()
            .map(|row| (row.number_label.as_str(), row.number.as_str()))
            .collect::<Vec<_>>(),
        [("mobile", "+447700900123"), ("work", "02079460123")]
    );

    let by_number = harness.search("900 123");
    assert_eq!(by_number[0].name, "Alice Smith");
    assert_eq!(by_number[0].shown_number, "+44 7700 900123");

    assert_eq!(names(&harness.search("sam")), ["Sam Jones", "Sam Brown"]);
}

#[test]
fn before_typing_search_lists_the_call_history_newest_first_with_missed_calls_marked() {
    let harness = synced();

    let rows = harness.search("");

    let listed: Vec<(&str, &str, &str)> = rows
        .iter()
        .map(|row| {
            (
                row.call_kind.as_str(),
                row.name.as_str(),
                row.number.as_str(),
            )
        })
        .collect();
    assert_eq!(
        listed,
        [
            ("missed", "", "+447700900999"),
            ("outgoing", "Sam Brown", "+447700900021"),
            ("outgoing", "Sam Jones", "+447700900022"),
            ("incoming", "Sam Jones", "+447700900022"),
        ]
    );
    assert!(rows.iter().all(|row| row.kind == "call-history"));
}

#[test]
fn a_dialable_number_or_a_pasted_tel_link_gets_a_call_row() {
    let harness = synced();

    let typed = harness.search("07700 900888");
    assert_eq!(typed[0].kind, "call-number");
    assert_eq!(typed[0].number, "07700900888");
    assert_eq!(typed[0].shown_number, "07700 900888");

    let pasted = harness.search("tel:+447700900555");
    assert_eq!(pasted.len(), 1, "{pasted:?}");
    assert_eq!(pasted[0].kind, "call-number");
    assert_eq!(pasted[0].number, "+447700900555");
}
