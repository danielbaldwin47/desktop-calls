//! Scenarios: the Phone goes out of range and comes back, the Desktop
//! suspends and resumes, a Call loses the Bluetooth connection, and the
//! Phone forgets the pairing (ticket #19; polish scorecard items 5 and 6,
//! scenarios 17 and 18).

use std::time::{Duration, Instant};

use test_harness::{AppState, Bus, Harness, RecordedMethodCall, TrayItem};

/// How long the app may take to notice a change on the Phone.
const DEADLINE: Duration = Duration::from_secs(5);
/// How soon the Phone is connected again once it is back in range, or once
/// the Desktop has resumed with it in range (spec stories 31 and 32).
const RECONNECT_DEADLINE: Duration = Duration::from_secs(10);
/// Long enough away for the app to have failed several connects.
const AWAY: Duration = Duration::from_secs(12);
const SYNC_DEADLINE: Duration = Duration::from_secs(15);
const PHONE_NAME: &str = "My Pixel";
/// The Phone's side of hands-free.
const HANDS_FREE_AUDIO_GATEWAY_UUID: &str = "0000111f-0000-1000-8000-00805f9b34fb";
const NUMBER: &str = "07700900123";

fn connected() -> Harness {
    let harness = Harness::builder().phone_name(PHONE_NAME).start();
    harness.simulated_phone().connect();
    wait_connected(&harness, DEADLINE);
    harness
}

fn wait_connected(harness: &Harness, deadline: Duration) -> AppState {
    harness.wait_for_app("ConnectionState is connected", deadline, |state| {
        state.connection_state == "connected"
    })
}

fn out_of_range(harness: &Harness) -> AppState {
    harness.simulated_phone().disconnect();
    harness.wait_for_app("ConnectionState is disconnected", DEADLINE, |state| {
        state.connection_state == "disconnected"
    })
}

/// Waits until the Indicator, the only tray item, reports `status` and
/// `tooltip`, and returns it.
fn indicator_shows(harness: &Harness, status: &str, tooltip: &str) -> TrayItem {
    let items = harness.tray().wait_for(
        &format!("the Indicator is {status} and says {tooltip:?}"),
        DEADLINE,
        |items| {
            items.iter().any(|item| {
                item.status.as_deref() == Some(status) && item.tooltip.as_deref() == Some(tooltip)
            })
        },
    );
    items.into_iter().next().unwrap_or_else(|| unreachable!())
}

/// Every method call the Simulated Phone has received so far.
fn recorded(harness: &Harness) -> Vec<RecordedMethodCall> {
    harness
        .simulated_phone()
        .wait_for_recorded("the record", DEADLINE, |_| true)
}

fn connects(recorded: &[RecordedMethodCall]) -> usize {
    recorded
        .iter()
        .filter(|call| call.interface == "org.bluez.Device1" && call.member == "Connect")
        .count()
}

/// The `PullAll`s among `recorded` of `folder`.
fn pulls_of(recorded: &[RecordedMethodCall], folder: &str) -> usize {
    let mut selected = "";
    let mut pulls = 0;
    for call in recorded {
        match call.member.as_str() {
            "Select" => selected = &call.arguments,
            "PullAll" if selected == format!(r#"("int", "{folder}")"#) => pulls += 1,
            _ => {}
        }
    }
    pulls
}

#[test]
fn out_of_range_only_changes_the_indicator() {
    let harness = connected();
    indicator_shows(&harness, "Active", "My Pixel connected");

    out_of_range(&harness);

    indicator_shows(&harness, "Active", "My Pixel out of range");
    // No notification, now or as the app tries to reach the Phone.
    std::thread::sleep(Duration::from_secs(1));
    assert_eq!(harness.notifications().received(), []);
}

/// Scenario 17.
#[test]
fn back_in_range_the_phone_is_connected_within_10_s_and_contacts_and_call_history_refresh() {
    let harness = connected();
    harness.wait_for_app("Contacts are synced", SYNC_DEADLINE, |state| {
        state.contacts.state == "ready"
    });
    let before = recorded(&harness);
    out_of_range(&harness);
    std::thread::sleep(AWAY);
    let tried = recorded(&harness);
    assert!(
        connects(&tried) >= 2,
        "the app keeps trying while the Phone is away: {} connects",
        connects(&tried)
    );

    let back = Instant::now();
    harness.simulated_phone().return_to_range();

    wait_connected(&harness, RECONNECT_DEADLINE);
    assert!(back.elapsed() < RECONNECT_DEADLINE, "{:?}", back.elapsed());
    let recorded = harness.simulated_phone().wait_for_recorded(
        "the app pulls Contacts and the Call History again",
        SYNC_DEADLINE,
        |recorded| {
            pulls_of(recorded, "pb") > pulls_of(&before, "pb")
                && pulls_of(recorded, "cch") > pulls_of(&before, "cch")
        },
    );
    assert!(pulls_of(&recorded, "pb") > pulls_of(&before, "pb"));
    assert_eq!(harness.notifications().received(), []);
}

/// A connect under way when the Phone leaves must not bring it back (#196).
/// The app pages the Phone as it starts; once the Phone is in range that
/// connect takes another 600 ms to finish, and finishing it after the Phone
/// had left brought the Phone back, so GitHub's run of
/// `restarted_with_the_phone_away_the_cached_contacts_still_count` saw the
/// app connected.
#[test]
fn a_connect_under_way_when_the_phone_leaves_does_not_bring_it_back() {
    let harness = Harness::builder().phone_name(PHONE_NAME).start();
    harness.simulated_phone().wait_for_recorded(
        "the app pages the Phone before it is in range",
        DEADLINE,
        |recorded| connects(recorded) >= 1,
    );
    harness.simulated_phone().connect();
    wait_connected(&harness, DEADLINE);

    // Long enough for the Simulated Phone to have seen the Phone in range and
    // begun the 600 ms the app's connect takes to finish (it looks every
    // 20 ms), well inside them.
    std::thread::sleep(Duration::from_millis(150));
    out_of_range(&harness);

    harness.app_holds("the Phone stays away", Duration::from_secs(2), |state| {
        state.connection_state != "connected"
    });
}

/// The Phone connects by itself while the Desktop is paging it, and BlueZ
/// then fails that page as connected already: the Desktop must not page a
/// connected Phone again (ticket #212, where the real Desktop paged one about
/// every 45 s through a Call).
#[test]
fn a_phone_that_connects_by_itself_during_a_page_is_not_paged_again() {
    let harness = Harness::builder().phone_name(PHONE_NAME).start();
    let phone = harness.simulated_phone();
    phone.wait_for_recorded(
        "the app pages the Phone before it is in range",
        DEADLINE,
        |recorded| connects(recorded) >= 1,
    );
    phone.connect();
    wait_connected(&harness, DEADLINE);

    // The page under way is recorded already; it ends 600 ms after the
    // Phone connected, with BlueZ's `br-connection-already-connected`.
    let paged = connects(&recorded(&harness));
    let held = phone.recorded_holds(
        "no Connect to a connected Phone",
        RECONNECT_DEADLINE,
        |recorded| connects(recorded) == paged,
    );
    assert_eq!(connects(&held), paged);
}

/// The Phone connects by itself between two pages, and the app never hears
/// BlueZ mark it connected, so it still holds the Phone as away: when the
/// next page is due, the app sees BlueZ has the Phone connected and doesn't
/// page it (ticket #212).
#[test]
fn a_phone_connected_unheard_between_pages_is_not_paged() {
    let harness = Harness::builder().phone_name(PHONE_NAME).start();
    let phone = harness.simulated_phone();
    phone.enable_quirk("connected-unannounced");
    // The first page fails; the next is due 1 s later.
    harness.wait_for_app_log("br-connection-page-timeout", RECONNECT_DEADLINE);
    let paged = connects(&recorded(&harness));
    // Had the next page begun, it would end as connected already, and this
    // would test the other path.
    assert_eq!(paged, 1, "the second page began before the Phone connected");

    phone.connect();
    wait_connected(&harness, DEADLINE);

    let held = phone.recorded_holds(
        "no Connect to a connected Phone",
        RECONNECT_DEADLINE,
        |recorded| connects(recorded) == paged,
    );
    assert_eq!(connects(&held), paged);
    harness.wait_for_app_log("phone-present", DEADLINE);
}

/// The Phone connects by itself while the app pages it, unheard, and BlueZ
/// fails the page `br-connection-already-connected`: the app reads that as
/// the Phone present, not as a failure to try again (ticket #212).
#[test]
fn a_page_answered_already_connected_counts_as_the_phone_present() {
    let harness = Harness::builder().phone_name(PHONE_NAME).start();
    let phone = harness.simulated_phone();
    phone.enable_quirk("connected-unannounced");
    phone.wait_for_recorded(
        "the app pages the Phone before it is in range",
        DEADLINE,
        |recorded| connects(recorded) >= 1,
    );

    phone.connect();
    wait_connected(&harness, DEADLINE);

    // Only the page's answer can say the Phone is present: BlueZ never
    // announced it.
    harness.wait_for_app_log("phone-present", DEADLINE);
    let failures: Vec<_> = harness
        .app_log()
        .lines()
        .filter(|line| line.contains("phone-connect-failed") && line.contains("already-connected"))
        .map(str::to_owned)
        .collect();
    assert_eq!(failures, Vec::<String>::new());
}

/// The Desktop's Bluetooth goes off, which drops the Phone: nothing can be
/// paged until it is back on, so the app asks BlueZ at most once rather
/// than every few seconds (ticket #213).
#[test]
fn with_the_desktops_bluetooth_off_the_phone_is_paged_at_most_once() {
    let harness = connected();
    let before = connects(&recorded(&harness));

    harness.set_bluetooth_power("off");
    harness.wait_for_app("ConnectionState is disconnected", DEADLINE, |state| {
        state.connection_state == "disconnected"
    });

    let held = harness.simulated_phone().recorded_holds(
        "at most one Connect with Bluetooth off",
        RECONNECT_DEADLINE,
        |recorded| connects(recorded) <= before + 1,
    );
    assert!(connects(&held) <= before + 1);
}

/// The Desktop's Bluetooth comes back on: the Phone, which waits to be
/// paged, is connected again within 10 s of it coming on (ticket #213).
/// That the waits start short again is the core's tests' to show.
#[test]
fn when_the_desktops_bluetooth_comes_back_on_the_phone_is_connected_within_10_s() {
    let harness = connected();
    harness.set_bluetooth_power("off");
    harness.wait_for_app("ConnectionState is disconnected", DEADLINE, |state| {
        state.connection_state == "disconnected"
    });

    harness.set_bluetooth_power("on");
    wait_connected(&harness, RECONNECT_DEADLINE);
}

/// Another app drops hands-free alone, and the Phone stays connected with
/// its media: the app connects hands-free again within 10 s (ticket #216).
#[test]
fn hands_free_dropped_with_the_phone_still_connected_comes_back_within_10_s() {
    let harness = connected();
    drop_hands_free_from_elsewhere(&harness);
    harness.wait_for_app("ConnectionState is disconnected", DEADLINE, |state| {
        state.connection_state == "disconnected"
    });

    wait_connected(&harness, RECONNECT_DEADLINE);
}

/// Hands-free stays down with the Phone still connected: neither the
/// Indicator nor `desktop-calls status` says the Phone is out of range
/// (ticket #216).
#[test]
fn a_connected_phone_without_hands_free_is_not_called_out_of_range() {
    let harness = connected();
    harness.simulated_phone().enable_quirk("connect-profile-fails");
    drop_hands_free_from_elsewhere(&harness);
    harness.wait_for_app("ConnectionState is disconnected", DEADLINE, |state| {
        state.connection_state == "disconnected"
    });

    indicator_shows(&harness, "Active", "Connection to My Pixel lost");
    let printed = harness.cli(&["status"]);
    let status = String::from_utf8_lossy(&printed.stdout);
    assert!(
        status.contains("is in range, but not connected for Calls"),
        "{printed:?}"
    );
}

/// BlueZ never answers the connect the app sends as the Phone leaves: the
/// Phone back in range is still connected within 10 s (ticket #216).
#[test]
fn after_a_connect_that_is_never_answered_the_phone_back_in_range_is_connected_within_10_s() {
    let harness = connected();
    let phone = harness.simulated_phone();
    phone.enable_quirk("connect-never-answers-once");
    let before = connects(&recorded(&harness));
    out_of_range(&harness);
    phone.wait_for_recorded("the app pages the Phone", DEADLINE, |recorded| {
        connects(recorded) > before
    });

    phone.return_to_range();

    wait_connected(&harness, RECONNECT_DEADLINE);
}

/// BlueZ never answers a connect the app sent while the Phone was away, and
/// the Phone came back by itself meanwhile: hands-free dropped later, with
/// the Phone still connected, still comes back within 10 s (ticket #216).
#[test]
fn after_a_connect_that_is_never_answered_hands_free_dropped_comes_back_within_10_s() {
    let harness = connected();
    let phone = harness.simulated_phone();
    phone.enable_quirk("connect-never-answers-once");
    let before = connects(&recorded(&harness));
    out_of_range(&harness);
    phone.wait_for_recorded("the app pages the Phone", DEADLINE, |recorded| {
        connects(recorded) > before
    });
    phone.connect();
    wait_connected(&harness, DEADLINE);

    drop_hands_free_from_elsewhere(&harness);
    harness.wait_for_app("ConnectionState is disconnected", DEADLINE, |state| {
        state.connection_state == "disconnected"
    });

    wait_connected(&harness, RECONNECT_DEADLINE);
}

/// `DisconnectProfile` for the Phone's hands-free, as any BlueZ client can
/// send it: the Phone stays connected.
fn drop_hands_free_from_elsewhere(harness: &Harness) {
    let device = format!(
        "/org/bluez/hci0/dev_{}",
        harness.phone().address.replace(':', "_")
    );
    harness.call_phone_method(
        Bus::System,
        "org.bluez",
        &device,
        "org.bluez.Device1",
        "DisconnectProfile",
        &(HANDS_FREE_AUDIO_GATEWAY_UUID,),
    );
}

/// Scenario 18.
#[test]
fn after_resume_the_phone_is_connected_within_10_s() {
    suspended_out_of_range_then_resumed(&connected());
}

/// The app pages a Phone that is not in range as soon as it starts, so
/// whether a Connect has already gone out when the Phone appears depends on
/// timing. Here it has, and the Desktop still must not page while it sleeps.
#[test]
fn after_resume_the_phone_is_connected_within_10_s_when_the_app_had_already_paged_it() {
    let harness = Harness::builder().phone_name(PHONE_NAME).start();
    harness.simulated_phone().wait_for_recorded(
        "the app pages the Phone before it is in range",
        DEADLINE,
        |recorded| connects(recorded) >= 1,
    );
    harness.simulated_phone().connect();
    wait_connected(&harness, DEADLINE);
    suspended_out_of_range_then_resumed(&harness);
}

fn suspended_out_of_range_then_resumed(harness: &Harness) {
    let phone = harness.simulated_phone();

    phone.prepare_for_sleep(true);
    // The app logs this just before it acts on the notice.
    harness.wait_for_app_log("desktop-suspending", DEADLINE);
    let before = connects(&recorded(harness));
    out_of_range(harness);
    // Asleep, the Desktop doesn't page the Phone.
    let asleep = phone.recorded_holds(
        "no connect while suspended",
        Duration::from_secs(2),
        |recorded| connects(recorded) == before,
    );
    assert_eq!(connects(&asleep), before);
    phone.prepare_for_sleep(false);
    let resumed = Instant::now();
    phone.return_to_range();

    wait_connected(harness, RECONNECT_DEADLINE);
    assert!(
        resumed.elapsed() < RECONNECT_DEADLINE,
        "{:?}",
        resumed.elapsed()
    );
}

#[test]
fn a_call_that_loses_the_connection_carries_on_on_the_phone_and_the_window_says_so() {
    let harness = Harness::builder()
        .phone_name(PHONE_NAME)
        .headless_screen()
        .start();
    let phone = harness.simulated_phone();
    phone.connect();
    wait_connected(&harness, DEADLINE);
    let call = phone.dial_on_phone(NUMBER);
    phone.callee_answers();
    harness.wait_for_app("the Call is active", DEADLINE, |state| {
        state.calls.iter().any(|call| call.state == "active")
    });

    phone.disconnect();

    let state = harness.wait_for_app(
        "the Call Window says the connection is lost",
        DEADLINE,
        |state| {
            state
                .surface("call-window")
                .and_then(|window| window.get("notice"))
                .is_some_and(|notice| {
                    notice == "Connection to phone lost: the Call continues on your phone."
                })
        },
    );
    assert_eq!(state.connection_state, "disconnected");
    assert_eq!(
        phone.calls_on_phone(),
        [(
            call.rsplit('/').next().unwrap_or_default().to_owned(),
            "active".to_owned()
        )],
        "the Call carries on on the Phone"
    );
    // Not a Missed Call, nor anything else to notify.
    std::thread::sleep(Duration::from_secs(3));
    assert_eq!(harness.notifications().received(), []);
}

#[test]
fn a_phone_that_forgets_the_pairing_is_a_problem_not_out_of_range() {
    let harness = connected();

    forgets_the_pairing_and_the_app_says_so(&harness);
}

/// The Desktop was already paging the Phone when it came back into range, so
/// that connect is still under way when the Phone forgets the pairing: it
/// must fail as any connect made after the Phone forgot would (ticket #171).
#[test]
fn a_phone_that_forgets_the_pairing_while_a_connect_is_under_way_is_a_problem_too() {
    let harness = Harness::builder().phone_name(PHONE_NAME).start();
    harness.simulated_phone().wait_for_recorded(
        "the app pages the Phone before it is in range",
        DEADLINE,
        |recorded| connects(recorded) >= 1,
    );
    harness.simulated_phone().connect();
    wait_connected(&harness, DEADLINE);

    forgets_the_pairing_and_the_app_says_so(&harness);
}

fn forgets_the_pairing_and_the_app_says_so(harness: &Harness) {
    harness.simulated_phone().forget_pairing();

    let state = harness.wait_for_app("ConnectionState is pairing-lost", DEADLINE, |state| {
        state.connection_state == "pairing-lost"
    });
    assert_eq!(
        state
            .problems
            .iter()
            .map(|problem| problem.kind.as_str())
            .collect::<Vec<_>>(),
        ["pairing-lost"]
    );
    indicator_shows(
        harness,
        "NeedsAttention",
        "My Pixel not connected · Pairing lost",
    );
    // Each connect would ask whoever holds the Phone to pair.
    let tried = connects(&recorded(harness));
    let held = harness.simulated_phone().recorded_holds(
        "no more connects",
        Duration::from_secs(4),
        |recorded| connects(recorded) == tried,
    );
    assert_eq!(connects(&held), tried);
}
