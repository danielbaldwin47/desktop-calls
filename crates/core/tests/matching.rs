//! Matching's `search`: the Dialer's ranked rows for what the user typed,
//! from Contacts and the Call History.
//!
//! Foner's search tests (`AminBlg/kaller@fc25b13`, `tests/test_contacts.cpp`)
//! are the oracle for the name cases; their Algerian numbers are replaced by
//! numbers from Ofcom's drama ranges, and their keypad (T9) queries by the
//! letters they spell, because the Dialer is typed on a keyboard.

use desktop_calls_core::matching::{DialerRow, DialerRowKind, search};
use desktop_calls_core::{
    CallHistory, CallHistoryEntry, CallHistoryKind, Caller, Contact, ContactNumber, Contacts,
    Directory, NumberLabel, Timestamp,
};

fn contact(name: &str, numbers: &[(NumberLabel, &str)]) -> Contact {
    Contact {
        name: name.to_owned(),
        numbers: numbers
            .iter()
            .map(|(label, number)| ContactNumber {
                number: (*number).to_owned(),
                label: label.clone(),
            })
            .collect(),
    }
}

fn mobile(name: &str, number: &str) -> Contact {
    contact(name, &[(NumberLabel::Mobile, number)])
}

fn call(kind: CallHistoryKind, number: Option<&str>, at: u64) -> CallHistoryEntry {
    CallHistoryEntry {
        kind,
        caller: number.map_or(Caller::Withheld, |number| Caller::Number(number.to_owned())),
        at: Some(Timestamp(at)),
        name: String::new(),
    }
}

/// Contacts and Call History as the Desktop holds them, in the UK.
fn snapshot(contacts: Vec<Contact>, calls: Vec<CallHistoryEntry>) -> (Contacts, CallHistory) {
    let directory = Directory::new(contacts, Some("GB"));
    let call_history = CallHistory::new(calls, &directory);
    (Contacts::new(directory), call_history)
}

fn rows(query: &str, (contacts, call_history): &(Contacts, CallHistory)) -> Vec<DialerRow> {
    search(query, contacts, call_history)
}

fn names(rows: &[DialerRow]) -> Vec<&str> {
    rows.iter().map(|row| row.name.as_str()).collect()
}

/// The names on the Contact rows for `query`, in order.
fn contact_names(query: &str, held: &(Contacts, CallHistory)) -> Vec<String> {
    rows(query, held)
        .into_iter()
        .filter(|row| row.kind == DialerRowKind::Contact)
        .map(|row| row.name)
        .collect()
}

fn first_name(query: &str, held: &(Contacts, CallHistory)) -> Option<String> {
    rows(query, held).first().map(|row| row.name.clone())
}

fn people() -> (Contacts, CallHistory) {
    snapshot(
        vec![
            mobile("Joanna Smithers", "+447700900002"),
            mobile("John Smallwood", "+447700900003"),
            mobile("Joseph Bloggs", "+447700900004"),
            mobile("Sam Jones", "+447700900005"),
            mobile("José Smith", "+447700900001"),
        ],
        Vec::new(),
    )
}

#[test]
fn word_prefixes_find_jose_smith_first() {
    assert_eq!(
        first_name("jo sm", &people()).as_deref(),
        Some("José Smith")
    );
}

#[test]
fn an_unaccented_first_name_finds_jose_smith_first() {
    assert_eq!(first_name("jose", &people()).as_deref(), Some("José Smith"));
}

#[test]
fn initials_find_jose_smith() {
    let held = snapshot(
        vec![
            mobile("Joseph Bloggs", "+447700900004"),
            mobile("Sam Jones", "+447700900005"),
            mobile("José Smith", "+447700900001"),
        ],
        Vec::new(),
    );
    assert_eq!(names(&rows("js", &held)), ["José Smith"]);
}

#[test]
fn a_fuzzy_query_finds_jose_smith() {
    assert_eq!(
        first_name("jsmth", &people()).as_deref(),
        Some("José Smith")
    );
}

/// Foner's `anAccentedNameIsFoundWithoutTheAccent`.
#[test]
fn an_accented_name_is_found_with_or_without_the_accent_in_any_case() {
    let held = snapshot(
        vec![
            mobile("Ali Example", "+447700900011"),
            mobile("Béchir Bouzid", "+447700900013"),
        ],
        Vec::new(),
    );
    for query in ["Bechir", "Béchir", "bechir", "BECHIR", "Bouzid"] {
        assert_eq!(
            first_name(query, &held).as_deref(),
            Some("Béchir Bouzid"),
            "{query}"
        );
    }
    assert!(rows("Zzz", &held).is_empty());
}

/// Foner's `ranksNameStartAboveWordStartAboveMidWord`, typed as letters.
#[test]
fn a_name_start_ranks_above_a_word_start_above_mid_word() {
    let held = snapshot(
        vec![
            mobile("Khalil Other", "+447700900013"),
            mobile("Sami Ali", "+447700900012"),
            mobile("Ali Example", "+447700900011"),
        ],
        Vec::new(),
    );
    assert_eq!(
        names(&rows("ali", &held)),
        ["Ali Example", "Sami Ali", "Khalil Other"]
    );
}

/// Foner's `searchMatchesNamesAndNumbers`.
#[test]
fn names_match_letters_and_numbers_match_digits() {
    let held = snapshot(
        vec![
            mobile("Alice Example", "07700 900001"),
            mobile("Bob Other", "07700 900042"),
        ],
        Vec::new(),
    );
    assert_eq!(names(&rows("alice", &held)), ["Alice Example"]);
    assert_eq!(contact_names("0042", &held), ["Bob Other"]);
}

/// Foner's `reportsNoMatchRatherThanGuessing`.
#[test]
fn digits_that_match_no_number_give_only_the_call_row() {
    let held = snapshot(vec![mobile("Ali Example", "07700 900011")], Vec::new());
    let found = rows("999888", &held);
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(found[0].kind, DialerRowKind::CallNumber);
}

#[test]
fn digits_match_a_number_whatever_its_formatting() {
    let held = snapshot(
        vec![
            mobile("Alice Smith", "+44 7700 900123"),
            mobile("Bob Jones", "07700 900456"),
        ],
        Vec::new(),
    );
    for (query, name) in [
        ("900 123", "Alice Smith"),
        ("900-123", "Alice Smith"),
        ("07700900123", "Alice Smith"),
        ("+447700900456", "Bob Jones"),
        ("(07700) 900 456", "Bob Jones"),
    ] {
        assert_eq!(contact_names(query, &held), [name], "{query}");
    }
}

#[test]
fn every_contact_number_is_a_row_of_its_own_with_its_label() {
    let held = snapshot(
        vec![contact(
            "Alice Smith",
            &[
                (NumberLabel::Mobile, "+447700900123"),
                (NumberLabel::Work, "+442079460123"),
            ],
        )],
        Vec::new(),
    );
    let found = rows("alice", &held);
    let labelled: Vec<(Option<NumberLabel>, Caller)> = found
        .iter()
        .map(|row| (row.label.clone(), row.caller.clone()))
        .collect();
    assert_eq!(
        labelled,
        [
            (
                Some(NumberLabel::Mobile),
                Caller::Number("+447700900123".to_owned())
            ),
            (
                Some(NumberLabel::Work),
                Caller::Number("+442079460123".to_owned())
            ),
        ]
    );
}

#[test]
fn the_most_called_of_two_equal_matches_ranks_first() {
    let held = snapshot(
        vec![
            mobile("Sam Brown", "+447700900021"),
            mobile("Sam Jones", "07700 900022"),
        ],
        vec![
            call(CallHistoryKind::Outgoing, Some("+447700900021"), 9_000),
            call(CallHistoryKind::Outgoing, Some("+447700900022"), 3_000),
            call(CallHistoryKind::Incoming, Some("+447700900022"), 2_000),
        ],
    );
    assert_eq!(names(&rows("sam", &held)), ["Sam Jones", "Sam Brown"]);
}

#[test]
fn of_two_equally_called_matches_the_most_recent_ranks_first() {
    let held = snapshot(
        vec![
            mobile("Sam Brown", "+447700900021"),
            mobile("Sam Jones", "+447700900022"),
        ],
        vec![
            call(CallHistoryKind::Outgoing, Some("+447700900021"), 1_000),
            call(CallHistoryKind::Outgoing, Some("+447700900022"), 5_000),
        ],
    );
    assert_eq!(names(&rows("sam", &held)), ["Sam Jones", "Sam Brown"]);
}

#[test]
fn a_dialable_number_gets_a_call_row_that_dials_it() {
    let held = snapshot(vec![mobile("Alice Smith", "+447700900123")], Vec::new());
    let found = rows("07700900999", &held);
    let first = &found[0];
    assert_eq!(first.kind, DialerRowKind::CallNumber);
    assert_eq!(first.dial().as_deref(), Some("07700900999"));
    assert_eq!(first.shown_number, "07700 900999");
}

#[test]
fn a_pasted_tel_link_finds_the_contact_it_belongs_to() {
    let held = snapshot(vec![mobile("Alice Smith", "07700 900123")], Vec::new());
    let found = rows("tel:+447700900123", &held);
    assert_eq!(names(&found), ["Alice Smith"], "{found:?}");
}

#[test]
fn a_pasted_tel_link_to_a_stranger_dials_its_number() {
    let held = snapshot(vec![mobile("Alice Smith", "07700 900123")], Vec::new());
    let found = rows("tel:+44-7700-900999", &held);
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(found[0].kind, DialerRowKind::CallNumber);
    assert_eq!(found[0].dial().as_deref(), Some("+447700900999"));
}

#[test]
fn a_partial_number_lists_the_contacts_it_matches_before_the_call_row() {
    let held = snapshot(vec![mobile("Alice Smith", "+447700900123")], Vec::new());
    let kinds: Vec<DialerRowKind> = rows("900 123", &held).iter().map(|row| row.kind).collect();
    assert_eq!(kinds, [DialerRowKind::Contact, DialerRowKind::CallNumber]);
}

#[test]
fn before_typing_the_call_history_shows_newest_first_with_missed_calls_marked() {
    let held = snapshot(
        vec![mobile("Alice Smith", "07700 900123")],
        vec![
            call(CallHistoryKind::Outgoing, Some("+447700900123"), 1_000),
            call(CallHistoryKind::Missed, None, 3_000),
            call(CallHistoryKind::Missed, Some("+447700900999"), 2_000),
        ],
    );
    let found = rows("", &held);
    let shown: Vec<(DialerRowKind, &str, Option<String>)> = found
        .iter()
        .map(|row| (row.kind, row.name.as_str(), row.dial()))
        .collect();
    assert_eq!(
        shown,
        [
            (
                DialerRowKind::CallHistory(CallHistoryKind::Missed),
                "",
                None
            ),
            (
                DialerRowKind::CallHistory(CallHistoryKind::Missed),
                "",
                Some("+447700900999".to_owned())
            ),
            (
                DialerRowKind::CallHistory(CallHistoryKind::Outgoing),
                "Alice Smith",
                Some("+447700900123".to_owned())
            ),
        ]
    );
    assert_eq!(found[2].label, Some(NumberLabel::Mobile));
}

#[test]
fn a_stranger_in_the_call_history_is_found_by_number() {
    let held = snapshot(
        vec![mobile("Alice Smith", "07700 900123")],
        vec![call(CallHistoryKind::Missed, Some("+447700900999"), 2_000)],
    );
    let found = rows("900 999", &held);
    assert_eq!(
        found[0].kind,
        DialerRowKind::CallHistory(CallHistoryKind::Missed),
        "{found:?}"
    );
    assert_eq!(found[0].dial().as_deref(), Some("+447700900999"));
}
