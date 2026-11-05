//! Scenarios: the Phone goes out of range and comes back, the Desktop
//! suspends and resumes, a Call loses the Bluetooth connection, and the
//! Phone forgets the pairing (ticket #19; polish scorecard items 5 and 6,
//! scenarios 17 and 18).

use std::time::{Duration, Instant};

use test_harness::{AppState, Harness, RecordedMethodCall, TrayItem};

/// How long the app may take to notice a change on the Phone.
const DEADLINE: Duration = Duration::from_secs(5);
/// How soon the Phone is connected again once it is back in range, or once
/// the Desktop has resumed with it in range (spec stories 31 and 32).
const RECONNECT_DEADLINE: Duration = Duration::from_secs(10);
/// Long enough away for the app to have failed several connects.
const AWAY: Duration = Duration::from_secs(12);
const SYNC_DEADLINE: Duration = Duration::from_secs(15);
const PHONE_NAME: &str = "My Pixel";
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

/// Scenario 18.
#[test]
fn after_resume_the_phone_is_connected_within_10_s() {
    let harness = connected();
    let phone = harness.simulated_phone();

    phone.prepare_for_sleep(true);
    std::thread::sleep(Duration::from_millis(200));
    out_of_range(&harness);
    // Asleep, the Desktop doesn't page the Phone.
    let asleep = phone.recorded_holds(
        "no connect while suspended",
        Duration::from_secs(2),
        |recorded| connects(recorded) == 0,
    );
    assert_eq!(connects(&asleep), 0);
    phone.prepare_for_sleep(false);
    let resumed = Instant::now();
    phone.return_to_range();

    wait_connected(&harness, RECONNECT_DEADLINE);
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
        &harness,
        "NeedsAttention",
        "My Pixel not connected · Pairing lost",
    );
    // Each connect would ask whoever holds the Phone to pair.
    let tried = connects(&recorded(&harness));
    let held = harness.simulated_phone().recorded_holds(
        "no more connects",
        Duration::from_secs(4),
        |recorded| connects(recorded) == tried,
    );
    assert_eq!(connects(&held), tried);
}
