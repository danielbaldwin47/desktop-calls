//! Scenarios: a Phone that refuses to share Contacts, and sharing turned on
//! later in the Phone's Bluetooth settings (ticket #394, from the owner's
//! sitting in #392). The Pixel 9a with sharing off for this Desktop never
//! asks: obexd fails `CreateSession` at once with `Transport got
//! disconnected`. Turning sharing on changes nothing on the link, so the app
//! has to ask again by itself.

use std::path::Path;
use std::time::{Duration, Instant};

use test_harness::{AppState, Harness, vcards};

/// How long the app may take to notice a change on the Phone.
const DEADLINE: Duration = Duration::from_secs(5);
/// How long a whole sync may take.
const SYNC_DEADLINE: Duration = Duration::from_secs(15);
/// How long the app may take to pick up Contacts sharing turned on later,
/// with the Phone connected all along: it asks again every 10 s, and a
/// sync of a few Contacts is quick. Three asks' worth, for a loaded machine.
const SHARED_LATER_DEADLINE: Duration = Duration::from_secs(30);
const CONTACTS: usize = 6;

fn is_ready(state: &AppState) -> bool {
    state.contacts.state == "ready"
}

fn is_denied(state: &AppState) -> bool {
    state.contacts.state == "denied"
}

fn problem_kinds(state: &AppState) -> Vec<&str> {
    state
        .problems
        .iter()
        .map(|problem| problem.kind.as_str())
        .collect()
}

fn create_sessions(harness: &Harness) -> usize {
    harness
        .simulated_phone()
        .wait_for_recorded("the record", DEADLINE, |_| true)
        .iter()
        .filter(|call| call.member == "CreateSession")
        .count()
}

fn files_in(directory: &Path) -> usize {
    std::fs::read_dir(directory).map_or(0, Iterator::count)
}

/// A Phone that closes the PBAP connection at once has refused, as one
/// answering `Forbidden` has: not a sync that failed. So the "Contacts not
/// shared" Problem shows, and the Contacts cached from before are deleted.
#[test]
fn a_phone_that_refuses_at_once_like_the_pixel_has_not_shared_contacts() {
    let harness = Harness::start();
    let phone = harness.simulated_phone();
    phone.set_pbap_folder("pb", &vcards::contacts(CONTACTS, 0));
    phone.connect();
    harness.wait_for_app("ContactsState is ready", SYNC_DEADLINE, is_ready);
    let cache = harness.directory().join("cache/desktop-calls");
    assert_eq!(files_in(&cache), 1);
    phone.disconnect();
    harness.wait_for_app("the Phone is gone", DEADLINE, |state| {
        state.connection_state == "disconnected"
    });
    phone.set_contacts_prompt("refuse");

    phone.connect();

    let state = harness.wait_for_app("ContactsState is denied", SYNC_DEADLINE, is_denied);
    assert_eq!(
        (state.contacts.contacts, state.contacts.call_history),
        (0, 0),
        "nothing of a Phone that refuses is kept"
    );
    assert_eq!(problem_kinds(&state), ["contacts-denied"]);
    let started = Instant::now();
    while files_in(&cache) > 0 {
        assert!(
            started.elapsed() < DEADLINE,
            "the cache is still in {}",
            cache.display()
        );
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// The "Contacts not shared" Problem tells the user to turn on Contacts and
/// call history in the Phone's Bluetooth settings, and nothing more. The
/// Phone stays connected, so the app notices by itself, without pairing
/// again or a reconnect. While the Phone still refuses, it asks quietly:
/// one warning in the log, not one each time.
#[test]
fn contacts_shared_later_are_synced_without_pairing_again_or_reconnecting() {
    let harness = Harness::start();
    let phone = harness.simulated_phone();
    phone.set_pbap_folder("pb", &vcards::contacts(CONTACTS, 0));
    phone.set_contacts_prompt("refuse");
    phone.connect();
    harness.wait_for_app("ContactsState is denied", SYNC_DEADLINE, is_denied);
    let asked = create_sessions(&harness);
    let started = Instant::now();
    while create_sessions(&harness) <= asked {
        assert!(
            started.elapsed() < SHARED_LATER_DEADLINE,
            "the app didn't ask the refusing Phone again within {SHARED_LATER_DEADLINE:?}"
        );
        std::thread::sleep(Duration::from_millis(200));
    }
    let state = harness.app_state();
    assert!(is_denied(&state), "{:?}", state.contacts);
    assert_eq!(problem_kinds(&state), ["contacts-denied"]);
    let warnings = harness
        .app_log()
        .lines()
        .filter(|line| {
            line.contains(r#""event_kind":"contacts-sync-failed""#)
                && line.contains(r#""level":"WARN""#)
        })
        .count();
    assert_eq!(warnings, 1, "asking again stays out of the log's warnings");

    // On the Phone: Bluetooth settings, this computer, Contacts and call
    // history on. Nothing else happens on the link.
    phone.set_contacts_prompt("grant");

    let state = harness.wait_for_app(
        "the Contacts shared later are synced",
        SHARED_LATER_DEADLINE,
        |state| is_ready(state) && usize::try_from(state.contacts.contacts) == Ok(CONTACTS),
    );
    assert!(state.problems.is_empty(), "{:?}", state.problems);
    assert_eq!(state.connection_state, "connected");
}

/// A Phone that asks, told no on its prompt: Android remembers the answer,
/// so asking again shows no prompt, and Contacts turned on later in its
/// Bluetooth settings are picked up as for a Phone that never asked.
#[test]
fn contacts_shared_later_after_a_no_on_the_prompt_are_synced_without_reconnecting() {
    let harness = Harness::start();
    let phone = harness.simulated_phone();
    phone.set_pbap_folder("pb", &vcards::contacts(CONTACTS, 0));
    phone.set_contacts_prompt("ask");
    phone.connect();
    harness.wait_for_app("the Phone asking", SYNC_DEADLINE, |state| {
        state.contacts.state == "permission-needed"
    });
    phone.answer_contacts_prompt("deny");
    phone.set_contacts_prompt("deny");
    harness.wait_for_app("ContactsState is denied", SYNC_DEADLINE, is_denied);

    phone.set_contacts_prompt("grant");

    let state = harness.wait_for_app(
        "the Contacts shared later are synced",
        SHARED_LATER_DEADLINE,
        |state| is_ready(state) && usize::try_from(state.contacts.contacts) == Ok(CONTACTS),
    );
    assert!(state.problems.is_empty(), "{:?}", state.problems);
    assert_eq!(state.connection_state, "connected");
}
