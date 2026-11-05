//! Scenarios: Contacts and Call History sync from the Phone over PBAP, so
//! the Banner and Call Window name the caller.

use std::collections::BTreeMap;
use std::os::unix::fs::PermissionsExt as _;
use std::path::Path;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use test_harness::vcards::{self, CallKind};
use test_harness::{AppState, Harness, RecordedMethodCall, Screenshot};

/// How long the app may take to notice a change on the Phone.
const DEADLINE: Duration = Duration::from_secs(5);
/// How long a whole sync may take.
const SYNC_DEADLINE: Duration = Duration::from_secs(15);
/// How long the Banner may take to show after the Phone rings: from the
/// Call appearing to its first frame, by the app's log (scorecard item 1).
const BANNER_DEADLINE: Duration = Duration::from_secs(1);
/// How long the tests' own round trip may take: the Phone ringing, and the
/// change coming back on the control interface. Looser than the scorecard
/// target because a loaded CI runner stretches it even when the app is fast
/// (ticket #83); the target is asserted on the app's log.
const ROUND_TRIP: Duration = Duration::from_secs(3);
/// How long a pull of a few hundred Contacts takes on the Pixel.
const REAL_PULL: Duration = Duration::from_secs(5);

fn is_ready(state: &AppState) -> bool {
    state.contacts.state == "ready"
}

#[test]
fn contacts_and_call_history_sync_when_the_phone_connects() {
    let harness = Harness::start();
    let phone = harness.simulated_phone();
    phone.set_pbap_folder("pb", &vcards::contacts(312, 0));
    phone.set_pbap_folder("cch", &vcards::call_history(200));
    // Slow enough to see the sync under way.
    phone.set_pull_delay(Duration::from_millis(500));

    phone.connect();

    harness.wait_for_app("ContactsState is syncing", DEADLINE, |state| {
        state.contacts.state == "syncing"
    });
    harness.wait_for_app("ContactsState is ready", SYNC_DEADLINE, is_ready);
    // The Call History arrives 50 at a time, as the Pixel gives it.
    let state = harness.wait_for_app("the whole Call History is read", SYNC_DEADLINE, |state| {
        state.contacts.call_history == 200
    });
    assert_eq!(state.contacts.contacts, 312);
    assert!(
        state.problems.is_empty(),
        "no Problems: {:?}",
        state.problems
    );
}

#[test]
fn before_any_sync_contacts_are_unavailable() {
    let harness = Harness::start();

    let state = harness.app_state();

    assert_eq!(state.contacts.state, "unavailable");
    assert_eq!(state.contacts.reason, "not-synced");
}

#[test]
fn pulls_ask_for_vcard_3() {
    let harness = Harness::start();
    let phone = harness.simulated_phone();
    phone.set_pbap_folder("pb", &vcards::contacts(3, 0));
    phone.set_pbap_folder(
        "cch",
        &vcards::call(CallKind::Missed, None, "20261104T090000"),
    );

    phone.connect();
    harness.wait_for_app("ContactsState is ready", SYNC_DEADLINE, is_ready);

    let recorded = phone.wait_for_recorded("the app pulls", DEADLINE, |recorded| {
        recorded.iter().any(|call| call.member == "PullAll")
    });
    for pull in recorded.iter().filter(|call| call.member == "PullAll") {
        assert!(
            pull.arguments.contains(r#""Format": <"vcard30">"#),
            "the app asked for another format: {}",
            pull.arguments
        );
    }
}

/// Alice's number as she is stored in Contacts: written at home, with its
/// leading 0.
const ALICE_IN_CONTACTS: &str = "07700 900123";
/// The same number as the Phone reports her Calls.
const ALICE_CALLING: &str = "+447700900123";

/// Alice, among other Contacts with international numbers, so the app can
/// tell the numbers are British.
fn contacts_with_alice(others: usize) -> String {
    let mut all = vcards::contact("Alice Smith", &[("CELL", ALICE_IN_CONTACTS)]);
    // Clear of Alice's number.
    all.push_str(&vcards::contacts(others, 1_000));
    all
}

/// The Banner's Answer button's green.
const GREEN: [u8; 3] = [0x26, 0xa2, 0x69];
/// How long the screen may take to show what the app reports.
const SCREEN_DEADLINE: Duration = Duration::from_secs(10);

fn shows_answer(screenshot: &Screenshot) -> bool {
    screenshot
        .region_where(|pixel| {
            pixel
                .iter()
                .zip(GREEN)
                .all(|(channel, wanted)| channel.abs_diff(wanted) <= 16)
        })
        .is_some()
}

fn synced(harness: &Harness) -> AppState {
    harness.simulated_phone().connect();
    harness.wait_for_app("ContactsState is ready", SYNC_DEADLINE, is_ready)
}

fn shown<'a>(content: Option<&'a BTreeMap<String, String>>, key: &str) -> Option<&'a str> {
    content
        .and_then(|content| content.get(key))
        .map(String::as_str)
}

#[test]
fn a_contact_rings_by_name_however_the_number_is_written() {
    let harness = Harness::builder().headless_screen().start();
    let phone = harness.simulated_phone();
    phone.set_pbap_folder("pb", &contacts_with_alice(20));
    synced(&harness);

    let _call = phone.ring(ALICE_CALLING);

    let state = harness.wait_for_app("the Banner shows", DEADLINE, |state| {
        state.surface("banner").is_some()
    });
    assert_eq!(
        shown(state.surface("banner"), "caller"),
        Some("Alice Smith")
    );
    assert_eq!(
        shown(state.surface("banner"), "number-label"),
        Some("mobile")
    );
    assert_eq!(state.calls[0].display_name, "Alice Smith");
    // For review: the name, then "mobile · +44 7700 900123".
    let _ = harness.screenshot_when("banner-contact", SCREEN_DEADLINE, shows_answer);

    harness.answer(state.calls[0].id);
    let state = harness.wait_for_app("the Call Window shows", DEADLINE, |state| {
        state.surface("call-window").is_some()
    });
    assert_eq!(
        shown(state.surface("call-window"), "caller"),
        Some("Alice Smith")
    );
    assert_eq!(
        shown(state.surface("call-window"), "number-label"),
        Some("mobile")
    );
}

#[test]
fn denied_contacts_leave_a_problem_and_calls_still_work_by_number() {
    let harness = Harness::builder().headless_screen().start();
    let phone = harness.simulated_phone();
    phone.set_pbap_folder("pb", &contacts_with_alice(5));
    phone.set_contacts_prompt("deny");

    phone.connect();

    let state = harness.wait_for_app("ContactsState is denied", SYNC_DEADLINE, |state| {
        state.contacts.state == "denied"
    });
    assert_eq!(
        (state.contacts.contacts, state.contacts.call_history),
        (0, 0)
    );
    assert_eq!(state.problems.len(), 1, "{:?}", state.problems);
    let problem = &state.problems[0];
    assert_eq!(problem.kind, "contacts-denied");
    assert!(
        !problem.message.contains('\n') && problem.message.contains("Bluetooth settings"),
        "one line saying how to allow it: {}",
        problem.message
    );

    let _call = phone.ring(ALICE_CALLING);
    let state = harness.wait_for_app("the Banner shows", DEADLINE, |state| {
        state.surface("banner").is_some()
    });
    assert_eq!(
        shown(state.surface("banner"), "caller"),
        Some("+44 7700 900123")
    );
    harness.answer(state.calls[0].id);
    harness.wait_for_app("the Call is answered", DEADLINE, |state| {
        state
            .calls
            .first()
            .is_some_and(|call| call.state == "active")
    });
}

#[test]
fn denying_contacts_later_deletes_the_cache() {
    let harness = Harness::start();
    let phone = harness.simulated_phone();
    phone.set_pbap_folder("pb", &vcards::contacts(10, 0));
    synced(&harness);
    let cache = harness.directory().join("cache/desktop-calls");
    assert_eq!(files_in(&cache), 1);
    phone.disconnect();
    harness.wait_for_app("the Phone is gone", DEADLINE, |state| {
        state.connection_state == "disconnected"
    });
    phone.set_contacts_prompt("deny");

    phone.connect();

    let state = harness.wait_for_app("ContactsState is denied", SYNC_DEADLINE, |state| {
        state.contacts.state == "denied"
    });
    assert_eq!(state.contacts.contacts, 0);
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

fn files_in(directory: &Path) -> usize {
    std::fs::read_dir(directory).map_or(0, Iterator::count)
}

#[test]
fn an_unanswered_prompt_needs_permission_until_the_phone_gives_up() {
    let harness = Harness::start();
    let phone = harness.simulated_phone();
    phone.set_pbap_folder("pb", &vcards::contacts(3, 0));
    phone.set_contacts_prompt("ignore");

    phone.connect();

    let state = harness.wait_for_app("ContactsState is permission-needed", DEADLINE, |state| {
        state.contacts.state == "permission-needed"
    });
    let now = now_ms();
    assert!(
        (now + 25_000..=now + 30_000).contains(&state.contacts.deadline_ms),
        "the Phone's prompt lasts 30 s: the deadline is {} ms from now",
        state.contacts.deadline_ms.saturating_sub(now)
    );
    let state = harness.wait_for_app(
        "ContactsState says the prompt timed out",
        Duration::from_secs(40),
        |state| state.contacts.state != "permission-needed",
    );
    assert_eq!(
        (
            state.contacts.state.as_str(),
            state.contacts.reason.as_str()
        ),
        ("unavailable", "prompt-timed-out")
    );
    let kinds: Vec<&str> = state
        .problems
        .iter()
        .map(|problem| problem.kind.as_str())
        .collect();
    assert_eq!(kinds, ["contacts-prompt-unanswered"]);
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| u64::try_from(elapsed.as_millis()).unwrap_or(0))
}

#[test]
fn pbap_sessions_never_overlap() {
    let harness = Harness::start();
    let phone = harness.simulated_phone();
    phone.set_pbap_folder("pb", &vcards::contacts(50, 0));
    phone.set_pbap_folder("cch", &vcards::call_history(20));
    phone.set_pull_delay(Duration::from_millis(300));

    phone.connect();
    // Two Calls end while the sync is under way, each asking for the Call
    // History again.
    for _ in 0..2 {
        let _call = phone.ring(ALICE_CALLING);
        let state =
            harness.wait_for_app("the Phone rings", DEADLINE, |state| !state.calls.is_empty());
        harness.decline(state.calls[0].id);
        harness.wait_for_app("the Call ends", DEADLINE, |state| state.calls.is_empty());
    }
    harness.wait_for_app("ContactsState is ready", SYNC_DEADLINE, is_ready);
    wait_for_folder_pulls(&harness, "cch", 3);

    assert_eq!(phone.most_pbap_sessions_at_once(), 1);
}

/// How many times among `recorded` the app selected `folder` on the Phone.
fn folder_selections(recorded: &[RecordedMethodCall], folder: &str) -> usize {
    let selected = format!(r#"("int", "{folder}")"#);
    recorded
        .iter()
        .filter(|call| call.member == "Select" && call.arguments == selected)
        .count()
}

/// Waits until the app has selected `folder` at least `count` times, and
/// returns how many times it has.
fn wait_for_folder_pulls(harness: &Harness, folder: &str, count: usize) -> usize {
    let recorded = harness.simulated_phone().wait_for_recorded(
        &format!("the app selects {folder} {count} times"),
        SYNC_DEADLINE,
        |recorded| folder_selections(recorded, folder) >= count,
    );
    folder_selections(&recorded, folder)
}

#[test]
fn call_history_is_pulled_again_after_a_call_ends() {
    let harness = Harness::start();
    let phone = harness.simulated_phone();
    phone.set_pbap_folder("cch", &vcards::call_history(5));
    synced(&harness);
    assert_eq!(wait_for_folder_pulls(&harness, "cch", 1), 1);
    phone.set_pbap_folder("cch", &vcards::call_history(6));

    let _call = phone.ring(ALICE_CALLING);
    let state = harness.wait_for_app("the Phone rings", DEADLINE, |state| !state.calls.is_empty());
    harness.decline(state.calls[0].id);

    wait_for_folder_pulls(&harness, "cch", 2);
    harness.wait_for_app("the Call History has the new Call", DEADLINE, |state| {
        state.contacts.call_history == 6
    });
}

#[test]
fn quoted_printable_and_folded_vcard_2_1_names_decode() {
    let harness = Harness::start();
    let phone = harness.simulated_phone();
    // As Phones that ignore the request for 3.0 send them: quoted-printable
    // UTF-8 with a soft line break mid-character, and a folded line.
    phone.set_pbap_folder(
        "pb",
        "BEGIN:VCARD\r\nVERSION:2.1\r\n\
         N;CHARSET=UTF-8;ENCODING=QUOTED-PRINTABLE:Garc=C3=ADa;Jos=C3=\r\n=A9;;;\r\n\
         FN;CHARSET=UTF-8;ENCODING=QUOTED-PRINTABLE:Jos=C3=\r\n=A9\r\n\
         TEL;CELL:07700900456\r\nEND:VCARD\r\n\
         BEGIN:VCARD\r\nVERSION:2.1\r\nN:Okafor;Chidi;;;\r\nFN:Chidi\r\n  Okafor\r\n\
         TEL;HOME:+44 1632 960789\r\nEND:VCARD\r\n",
    );
    phone.set_pbap_folder(
        "cch",
        &[
            vcards::call(CallKind::Received, Some("+447700900456"), "20261104T090000"),
            vcards::call(CallKind::Missed, Some("01632 960789"), "20261104T080000"),
        ]
        .concat(),
    );

    synced(&harness);

    let names: Vec<(String, String)> = harness
        .call_history()
        .into_iter()
        .map(|entry| (entry.display_name, entry.number_label))
        .collect();
    assert_eq!(
        names,
        [
            ("José".to_owned(), "mobile".to_owned()),
            ("Chidi Okafor".to_owned(), "home".to_owned())
        ]
    );
}

#[test]
fn without_cch_the_call_history_merges_incoming_outgoing_and_missed() {
    let harness = Harness::start();
    let phone = harness.simulated_phone();
    phone.remove_pbap_folder("cch");
    // This Phone lists its Missed Call among incoming Calls too.
    phone.set_pbap_folder(
        "mch",
        &vcards::call(CallKind::Missed, Some("+447700900001"), "20261104T100000"),
    );
    phone.set_pbap_folder(
        "ich",
        &[
            vcards::call(CallKind::Received, Some("+447700900001"), "20261104T100000"),
            vcards::call(CallKind::Received, Some("+447700900002"), "20261104T090000"),
        ]
        .concat(),
    );
    phone.set_pbap_folder(
        "och",
        &vcards::call(CallKind::Dialed, Some("+447700900003"), "20261104T080000"),
    );

    let state = synced(&harness);

    assert_eq!(state.contacts.call_history, 3);
    let calls: Vec<(String, String)> = harness
        .call_history()
        .into_iter()
        .map(|entry| (entry.kind, entry.number))
        .collect();
    assert_eq!(
        calls,
        [
            ("missed".to_owned(), "+447700900001".to_owned()),
            ("incoming".to_owned(), "+447700900002".to_owned()),
            ("outgoing".to_owned(), "+447700900003".to_owned()),
        ]
    );
}

#[test]
fn a_folder_the_phone_lacks_does_not_stop_the_others() {
    let harness = Harness::start();
    let phone = harness.simulated_phone();
    phone.remove_pbap_folder("pb");
    phone.remove_pbap_folder("cch");
    phone.remove_pbap_folder("ich");
    phone.set_pbap_folder(
        "mch",
        &vcards::call(CallKind::Missed, None, "20261104T100000"),
    );
    phone.set_pbap_folder(
        "och",
        &vcards::call(CallKind::Dialed, Some("+447700900003"), "20261104T080000"),
    );

    let state = synced(&harness);

    assert_eq!(
        (state.contacts.contacts, state.contacts.call_history),
        (0, 2),
        "the withheld Missed Call is kept"
    );
}

#[test]
fn call_history_times_without_a_zone_are_desktop_local_time() {
    // India's offset, which no daylight saving moves.
    let harness = Harness::builder().time_zone("<+0530>-5:30").start();
    let phone = harness.simulated_phone();
    phone.set_pbap_folder(
        "cch",
        &[
            vcards::call(CallKind::Received, Some("+447700900001"), "20261104T091500"),
            vcards::call(
                CallKind::Received,
                Some("+447700900002"),
                "20261104T091500Z",
            ),
            vcards::call(
                CallKind::Received,
                Some("+447700900003"),
                "20261104T091500+0200",
            ),
        ]
        .concat(),
    );

    synced(&harness);

    let mut times: Vec<(String, u64)> = harness
        .call_history()
        .into_iter()
        .map(|entry| (entry.number, entry.at_ms))
        .collect();
    times.sort();
    assert_eq!(
        times,
        [
            // 09:15 at +05:30 is 03:45 UTC.
            ("+447700900001".to_owned(), 1_793_763_900_000),
            // 09:15 UTC.
            ("+447700900002".to_owned(), 1_793_783_700_000),
            // 09:15 at +02:00 is 07:15 UTC.
            ("+447700900003".to_owned(), 1_793_776_500_000),
        ]
    );
}

#[test]
fn restarted_with_the_phone_away_the_cached_contacts_still_count() {
    let mut harness = Harness::start();
    let phone = harness.simulated_phone();
    phone.set_pbap_folder("pb", &vcards::contacts(312, 0));
    phone.set_pbap_folder("cch", &vcards::call_history(200));
    synced(&harness);
    harness.wait_for_app("the whole Call History is read", SYNC_DEADLINE, |state| {
        state.contacts.call_history == 200
    });
    harness.simulated_phone().disconnect();

    harness.restart_app();

    let state = harness.wait_for_app("the cached counts are back", DEADLINE, |state| {
        state.contacts.contacts == 312
    });
    // The app may be trying to reach the Phone, but hasn't.
    assert_ne!(state.connection_state, "connected");
    assert_eq!(state.contacts.call_history, 200);
    let cache = harness.directory().join("cache/desktop-calls");
    assert_eq!(
        mode(&cache),
        0o700,
        "{} is the user's alone",
        cache.display()
    );
    let files: Vec<_> = std::fs::read_dir(&cache)
        .expect("list the cache directory")
        .map(|entry| entry.expect("read the cache directory").path())
        .collect();
    assert!(!files.is_empty(), "the cache directory is empty");
    for file in files {
        assert_eq!(mode(&file), 0o600, "{} is the user's alone", file.display());
    }
    assert_eq!(
        files_in(&harness.directory().join("runtime/desktop-calls")),
        0,
        "pulled vCards are deleted once read"
    );
}

fn mode(path: &Path) -> u32 {
    std::fs::metadata(path)
        .unwrap_or_else(|error| panic!("{}: {error}", path.display()))
        .permissions()
        .mode()
        & 0o777
}

#[test]
fn with_5000_contacts_the_banner_still_shows_within_a_second() {
    let harness = Harness::builder().headless_screen().omarchy_bar().start();
    let phone = harness.simulated_phone();
    phone.set_pbap_folder("pb", &contacts_with_alice(5_000));
    phone.set_pbap_folder("cch", &vcards::call_history(50));
    // The Pixel took 5.35 s to give 319 Contacts; each pull here takes 5 s.
    phone.set_pull_delay(REAL_PULL);

    // The Phone rings while the 5,000 Contacts are being read.
    phone.connect();
    harness.wait_for_app("ContactsState is syncing", DEADLINE, |state| {
        state.contacts.state == "syncing"
    });
    let _call = phone.ring(ALICE_CALLING);
    let state = harness.wait_for_app("the Banner shows", ROUND_TRIP, |state| {
        state.surface("banner").is_some()
    });
    harness.assert_banner_within(state.calls[0].id, BANNER_DEADLINE);
    assert_eq!(
        state.contacts.state, "syncing",
        "the Banner showed while the Contacts were still being pulled"
    );
    harness.decline(state.calls[0].id);
    harness.wait_for_app(
        "ContactsState is ready",
        REAL_PULL * 2 + SYNC_DEADLINE,
        is_ready,
    );
    harness.wait_for_app("the Banner is gone", DEADLINE, |state| {
        state.surface("banner").is_none()
    });

    // And again once they are indexed: the Banner names her.
    let _call = phone.ring(ALICE_CALLING);
    let state = harness.wait_for_app("the Banner names Alice", ROUND_TRIP, |state| {
        shown(state.surface("banner"), "caller") == Some("Alice Smith")
    });
    harness.assert_banner_within(state.calls[0].id, BANNER_DEADLINE);
    assert_eq!(state.contacts.contacts, 5_001);
}

/// The `PullAll`s among `recorded` that the app made of `folder`, by their
/// filters, in order.
fn folder_pulls(recorded: &[RecordedMethodCall], folder: &str) -> Vec<String> {
    let mut selected = "";
    let mut pulls = Vec::new();
    for call in recorded {
        match call.member.as_str() {
            "Select" => selected = &call.arguments,
            "PullAll" if selected == format!(r#"("int", "{folder}")"#) => {
                pulls.push(call.arguments.clone());
            }
            _ => {}
        }
    }
    pulls
}

/// Waits until the app has pulled the Call History at least `count` times,
/// and returns its pulls.
fn wait_for_call_history_pulls(harness: &Harness, count: usize) -> Vec<String> {
    let recorded = harness.simulated_phone().wait_for_recorded(
        &format!("the app pulls cch {count} times"),
        SYNC_DEADLINE,
        |recorded| folder_pulls(recorded, "cch").len() >= count,
    );
    folder_pulls(&recorded, "cch")
}

#[test]
fn call_history_past_the_phones_50_entry_pulls_is_read_page_by_page() {
    let harness = Harness::start();
    let phone = harness.simulated_phone();
    phone.set_pbap_folder("pb", &vcards::contacts(10, 0));
    // The Pixel gives 50 a pull however many it holds (922 on 2026-11-05).
    phone.set_pbap_folder("cch", &vcards::call_history(120));
    // Slow enough to see the newest page reported before the rest.
    phone.set_pull_delay(Duration::from_secs(2));

    harness.simulated_phone().connect();

    // The newest 50 Calls come first, while the sync goes on.
    harness.wait_for_app("the newest 50 Calls are read", SYNC_DEADLINE, |state| {
        state.contacts.state == "syncing" && state.contacts.call_history == 50
    });
    let state = harness.wait_for_app("ContactsState is ready", SYNC_DEADLINE, is_ready);
    assert_eq!(
        (state.contacts.contacts, state.contacts.call_history),
        (10, 120),
        "Contacts come with the whole Call History"
    );
    let pulls = wait_for_call_history_pulls(&harness, 3);
    assert_eq!(pulls.len(), 3, "three pages: {pulls:?}");
    assert!(!pulls[0].contains("Offset"), "{}", pulls[0]);
    assert!(
        pulls[1].contains(r#""Offset": <uint16 50>"#),
        "{}",
        pulls[1]
    );
    assert!(
        pulls[2].contains(r#""Offset": <uint16 100>"#),
        "{}",
        pulls[2]
    );
    let history = harness.call_history();
    assert_eq!(
        history.last().map(|entry| entry.number.as_str()),
        Some(vcards::number(119).as_str()),
        "the oldest Call is there"
    );
}

#[test]
fn after_a_call_only_the_newest_page_is_pulled_and_older_calls_are_kept() {
    let harness = Harness::start();
    let phone = harness.simulated_phone();
    phone.set_pbap_folder("cch", &vcards::call_history(120));
    synced(&harness);
    harness.wait_for_app("the whole Call History is read", SYNC_DEADLINE, |state| {
        state.contacts.call_history == 120
    });
    let pulled_at_sync = wait_for_call_history_pulls(&harness, 3).len();
    // The Call that ends goes on top.
    phone.set_pbap_folder(
        "cch",
        &[
            vcards::call(CallKind::Missed, Some(ALICE_CALLING), "20261104T120100"),
            vcards::call_history(120),
        ]
        .concat(),
    );

    let _call = phone.ring(ALICE_CALLING);
    let state = harness.wait_for_app("the Phone rings", DEADLINE, |state| !state.calls.is_empty());
    harness.decline(state.calls[0].id);

    harness.wait_for_app("the new Call is on top", SYNC_DEADLINE, |state| {
        state.contacts.call_history == 121
    });
    let pulls = wait_for_call_history_pulls(&harness, pulled_at_sync + 1);
    assert_eq!(pulls.len(), pulled_at_sync + 1, "one page: {pulls:?}");
    assert!(!pulls[pulled_at_sync].contains("Offset"));
    let history = harness.call_history();
    assert_eq!(history[0].number, ALICE_CALLING);
    assert_eq!(history[0].kind, "missed");
    assert_eq!(
        history.last().map(|entry| entry.number.as_str()),
        Some(vcards::number(119).as_str()),
        "Calls older than the newest page are kept"
    );
}

#[test]
fn pixel_shaped_contacts_name_the_caller() {
    let harness = Harness::builder().headless_screen().start();
    let phone = harness.simulated_phone();
    // As the Pixel sends them (2026-11-05): CHARSET on the name, PREF on
    // a number, photos folded over many lines, and a Contact with no number.
    phone.set_pbap_folder(
        "pb",
        &[
            vcards::contact_with_photo("Alice Smith", &[("VOICE;PREF", ALICE_IN_CONTACTS)]),
            vcards::contact_with_photo("Bob Jones", &[]),
            vcards::contact("Carol White", &[("HOME;PREF", "+12025550123")]),
            vcards::contacts(20, 1_000),
        ]
        .concat(),
    );
    synced(&harness);
    let state = harness.app_state();
    assert_eq!(
        state.contacts.contacts, 23,
        "the Contact with no number counts"
    );

    let _call = phone.ring(ALICE_CALLING);
    let state = harness.wait_for_app("the Banner names Alice", DEADLINE, |state| {
        shown(state.surface("banner"), "caller") == Some("Alice Smith")
    });
    assert_eq!(
        shown(state.surface("banner"), "number-label"),
        Some("other")
    );
    harness.decline(state.calls[0].id);
    harness.wait_for_app("the Call ends", DEADLINE, |state| state.calls.is_empty());

    let _call = phone.ring("+12025550123");
    let state = harness.wait_for_app("the Banner names Carol", DEADLINE, |state| {
        shown(state.surface("banner"), "caller") == Some("Carol White")
    });
    assert_eq!(shown(state.surface("banner"), "number-label"), Some("home"));
}

#[test]
fn pixel_shaped_call_history_keeps_every_name_and_number_shape() {
    let harness = Harness::start();
    let phone = harness.simulated_phone();
    // The Pixel's shapes (2026-11-05): a named entry with CHARSET, entries
    // with no name, and numbers of 11 digits after +, 10, 7 and 17.
    let numbers = ["+12025550123", "2025550124", "5550125", "12345678901234567"];
    phone.set_pbap_folder(
        "cch",
        &[
            vcards::named_call(
                CallKind::Received,
                "José Núñez",
                numbers[0],
                "20261104T110000",
            ),
            vcards::call(CallKind::Dialed, Some(numbers[1]), "20261104T100000"),
            vcards::call(CallKind::Missed, Some(numbers[2]), "20261104T090000"),
            vcards::call(CallKind::Missed, Some(numbers[3]), "20261104T080000"),
            vcards::call(CallKind::Missed, None, "20261104T070000"),
        ]
        .concat(),
    );

    synced(&harness);

    let history: Vec<(String, String, String)> = harness
        .call_history()
        .into_iter()
        .map(|entry| (entry.kind, entry.number, entry.display_name))
        .collect();
    let expected: Vec<(String, String, String)> = [
        ("incoming", numbers[0], "José Núñez"),
        ("outgoing", numbers[1], ""),
        ("missed", numbers[2], ""),
        ("missed", numbers[3], ""),
        ("missed", "", ""),
    ]
    .into_iter()
    .map(|(kind, number, name)| (kind.to_owned(), number.to_owned(), name.to_owned()))
    .collect();
    assert_eq!(history, expected);
}
