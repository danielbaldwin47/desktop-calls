//! How the Dialer shows its rows (#232): the letters of a name that matched
//! what was typed, a glyph, the detail line, the time column and the labels
//! over its sections.

use desktop_calls_core::matching::{DialerRow, DialerRowKind, search};
use desktop_calls_core::messages::{self, DialerGlyph};
use desktop_calls_core::{
    CallHistory, CallHistoryEntry, CallHistoryKind, Caller, Contact, ContactNumber, Contacts,
    Directory, NumberLabel, Timestamp,
};

/// Sunday 2026-11-15, 12:00 UTC.
const NOW: Timestamp = Timestamp(1_794_744_000_000);

fn mobile(name: &str, number: &str) -> Contact {
    Contact {
        name: name.to_owned(),
        numbers: vec![ContactNumber {
            number: number.to_owned(),
            label: NumberLabel::Mobile,
        }],
    }
}

fn held() -> (Contacts, CallHistory) {
    let directory = Directory::new(
        vec![
            mobile("Alice Smith", "+447700900123"),
            mobile("José Smith", "+447700900001"),
            mobile("Joanna Smithers", "+447700900002"),
            mobile("Zoe\u{301} Ball", "+447700900003"),
        ],
        Some("GB"),
    );
    let calls = vec![CallHistoryEntry {
        kind: CallHistoryKind::Missed,
        caller: Caller::Number("+447700900999".to_owned()),
        at: Some(Timestamp(1_794_492_300_000)),
        name: "Bob Jones".to_owned(),
    }];
    let call_history = CallHistory::new(calls, &directory);
    (Contacts::new(directory), call_history)
}

/// The matched letters of the row named `name` for `query`.
fn underlined(query: &str, name: &str) -> Vec<String> {
    let (contacts, call_history) = held();
    let rows = search(query, &contacts, &call_history);
    let row = rows
        .iter()
        .find(|row| row.name == name)
        .unwrap_or_else(|| panic!("{name} matches {query:?}: {rows:?}"));
    row.matched
        .iter()
        .map(|range| row.name[range.clone()].to_owned())
        .collect()
}

#[test]
fn the_letters_that_matched_are_reported_however_the_name_matched() {
    // The start of a word, and of each word in order.
    assert_eq!(underlined("sm", "Alice Smith"), ["Sm"]);
    assert_eq!(underlined("al sm", "Alice Smith"), ["Al", "Sm"]);
    // Words in another order.
    assert_eq!(underlined("smith al", "Alice Smith"), ["Al", "Smith"]);
    // Initials.
    assert_eq!(underlined("as", "Alice Smith"), ["A", "S"]);
    // Inside a word.
    assert_eq!(underlined("mith", "Joanna Smithers"), ["mith"]);
    // Letters in order, anywhere.
    assert_eq!(underlined("jsr", "Joanna Smithers"), ["J", "S", "r"]);
    // Folded: the accent is part of the letter typed without it.
    assert_eq!(underlined("jose", "José Smith"), ["José"]);
    // An accent written as a combining mark goes with its letter.
    assert_eq!(underlined("zoe", "Zoe\u{301} Ball"), ["Zoe\u{301}"]);
    // A Call History caller found by name.
    assert_eq!(underlined("bob", "Bob Jones"), ["Bob"]);
}

#[test]
fn nothing_is_underlined_with_nothing_typed_or_when_the_number_matched() {
    let (contacts, call_history) = held();
    for row in search("", &contacts, &call_history) {
        assert!(row.matched.is_empty(), "{row:?}");
    }
    for row in search("07700 900123", &contacts, &call_history) {
        assert!(row.matched.is_empty(), "{row:?}");
    }
}

fn history(kind: CallHistoryKind, caller: Caller, name: &str, at: u64) -> DialerRow {
    DialerRow {
        kind: DialerRowKind::CallHistory(kind),
        name: name.to_owned(),
        label: (!name.is_empty()).then_some(NumberLabel::Mobile),
        shown_number: match &caller {
            Caller::Number(_) => "+44 7700 900123".to_owned(),
            Caller::Withheld => String::new(),
        },
        caller,
        at: Some(Timestamp(at)),
        matched: Vec::new(),
    }
}

#[test]
fn a_call_history_row_shows_its_direction_and_at_the_right_when() {
    let alice = || Caller::Number("+447700900123".to_owned());
    let row = messages::dialer_row(
        &history(
            CallHistoryKind::Missed,
            alice(),
            "Alice Smith",
            1_794_731_400_000,
        ),
        NOW,
        0,
    );
    assert_eq!(row.glyph, DialerGlyph::Missed);
    assert_eq!(row.title, "Alice Smith");
    assert!(row.missed);
    assert_eq!(row.detail, "mobile · +44 7700 900123");
    assert_eq!(row.time, "08:30");
    assert_eq!(
        row.line(),
        "Alice Smith · Missed · mobile · +44 7700 900123 · 08:30"
    );

    let row = messages::dialer_row(
        &history(CallHistoryKind::Incoming, alice(), "", 1_794_492_300_000),
        NOW,
        0,
    );
    assert_eq!(row.glyph, DialerGlyph::Incoming);
    assert!(!row.missed, "only a Missed Call says so in words");
    assert_eq!(row.detail, "Not in Contacts");
    assert_eq!(row.time, "Thu 14:05");

    let row = messages::dialer_row(
        &history(
            CallHistoryKind::Outgoing,
            Caller::Withheld,
            "",
            1_793_869_200_000,
        ),
        NOW,
        0,
    );
    assert_eq!(row.glyph, DialerGlyph::Outgoing);
    assert_eq!(row.title, "Withheld number");
    assert_eq!(row.detail, "can't be called back");
    assert_eq!(row.time, "5 Nov");
}

#[test]
fn a_contact_shows_their_initial_and_the_number_typed_a_hash() {
    let (contacts, call_history) = held();
    let rows = search("jo", &contacts, &call_history);
    let jose = rows
        .iter()
        .find(|row| row.name == "José Smith")
        .unwrap_or_else(|| panic!("{rows:?}"));
    let text = messages::dialer_row(jose, NOW, 0);
    assert_eq!(text.glyph, DialerGlyph::Initial("J".to_owned()));
    assert_eq!(text.time, "", "a Contact has no time");
    assert_eq!(
        messages::dialer_section(jose.kind),
        Some("Contacts"),
        "{rows:?}"
    );

    let rows = search("07700 900888", &contacts, &call_history);
    let text = messages::dialer_row(&rows[0], NOW, 0);
    assert_eq!(text.glyph, DialerGlyph::Initial("#".to_owned()));
    assert_eq!(text.title, "Call 07700 900888");
    assert_eq!(messages::dialer_section(rows[0].kind), None);
}

#[test]
fn the_time_column_says_the_weekday_within_a_week_and_the_year_if_another() {
    let at = |ms| messages::dialer_time(Timestamp(ms), NOW, 0);
    assert_eq!(at(1_794_731_400_000), "08:30");
    assert_eq!(at(1_794_492_300_000), "Thu 14:05");
    assert_eq!(at(1_793_869_200_000), "5 Nov");
    assert_eq!(at(1_767_085_200_000), "30 Dec 2025");
    // In the Desktop's time zone: 08:30 UTC is 09:30 an hour east.
    assert_eq!(
        messages::dialer_time(Timestamp(1_794_731_400_000), NOW, 3_600),
        "09:30"
    );
}

#[test]
fn the_call_history_is_under_recent() {
    assert_eq!(
        messages::dialer_section(DialerRowKind::CallHistory(CallHistoryKind::Incoming)),
        Some("Recent")
    );
}
