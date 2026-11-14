//! Scenarios: Calls Off (ticket #202). Turning calls off from the
//! Indicator or the CLI blocks the Simulated Phone, which drops it; it
//! stays disconnected through its own pages, the reconnect waits and an app
//! restart, and the Desktop makes no `Connect` or `ConnectProfile` while it
//! is blocked. Turning calls on connects it within 10 s, even when the
//! first `Connect` fails busy. A record a killed app left halfway is put
//! right at the next start, and a Phone blocked with no record is left
//! alone.

use std::fs;
use std::path::PathBuf;
use std::time::Duration;

use test_harness::{AppEvent, AppState, Bus, Harness, TrayItem};

/// How long the app may take to notice a change on the Phone.
const DEADLINE: Duration = Duration::from_secs(5);
/// How soon the Phone is connected again once calls are on (spec #198,
/// the polish scorecard's 10 s).
const RECONNECT_DEADLINE: Duration = Duration::from_secs(10);
/// Longer than the reconnect waits of 1 s, 2 s, then 3 s, and the
/// Simulated Phone's slow failure of a `Connect` on a blocked Phone, twice
/// over: anything that would connect it has had its chance.
const STAYS_OFF: Duration = Duration::from_secs(8);
const PHONE_NAME: &str = "My Pixel";
const NUMBER: &str = "+447700900123";

fn connected() -> Harness {
    let harness = Harness::builder().phone_name(PHONE_NAME).start();
    harness.simulated_phone().connect();
    wait_connected(&harness, DEADLINE);
    harness
}

fn wait_connected(harness: &Harness, deadline: Duration) -> AppState {
    harness.wait_for_app("the Phone is connected, calls on", deadline, |state| {
        state.connection_state == "connected" && state.calls_off == "on"
    })
}

fn wait_off(harness: &Harness) -> AppState {
    harness.wait_for_app(
        "calls are off and the Phone disconnected",
        DEADLINE,
        |state| state.calls_off == "off" && state.connection_state == "disconnected",
    )
}

fn stays_off(harness: &Harness, window: Duration) {
    harness.app_holds(
        "calls stay off and the Phone disconnected",
        window,
        |state| state.calls_off == "off" && state.connection_state == "disconnected",
    );
}

/// Waits until the Indicator, the only tray item, says `tooltip`.
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

fn record(harness: &Harness) -> PathBuf {
    harness
        .directory()
        .join("state/desktop-calls/calls-off.json")
}

/// How long the Phone took to connect once calls were turned on, by the
/// app's own log: from it unblocking the Phone to it logging the Phone
/// connected (the polish scorecard's way, tickets #80 and #83).
fn reconnect_took(harness: &Harness) -> Duration {
    harness.app_took(
        &AppEvent::kind("calls-turned-on"),
        &AppEvent::kind("connection-state-changed").with("connection_state", "connected"),
        RECONNECT_DEADLINE,
    )
}

fn cli_ok(harness: &Harness, arguments: &[&str]) -> String {
    let output = harness.cli(arguments);
    assert!(output.status.success(), "{arguments:?} failed: {output:?}");
    String::from_utf8_lossy(&output.stdout).into_owned()
}

#[test]
fn calls_off_from_the_indicator_keeps_the_phone_away_through_its_pages_the_waits_and_a_restart() {
    let mut harness = connected();
    let drop_ins = harness.wireplumber_drop_ins();
    indicator_says(&harness, "My Pixel connected");

    harness.tray().click("Until I turn them on");

    wait_off(&harness);
    assert!(harness.simulated_phone().blocked(), "the Phone is blocked");
    let item = indicator_says(&harness, "Calls off");
    assert_eq!(item.status.as_deref(), Some("Active"), "no attention asked");
    assert_eq!(
        item.icon_name.as_deref(),
        Some("desktop-calls-off-symbolic")
    );
    let menu = harness.tray().menu();
    assert!(
        menu.iter()
            .any(|entry| entry.label == "Turn calls on" && entry.enabled),
        "{menu:?}"
    );
    let record_text = fs::read_to_string(record(&harness)).expect("read the record");
    assert!(record_text.contains(r#""state":"off""#), "{record_text}");

    // The Phone pages the Desktop: BlueZ refuses it.
    assert!(!harness.simulated_phone().page_desktop());
    stays_off(&harness, STAYS_OFF);

    harness.restart_app();
    wait_off(&harness);
    assert!(!harness.simulated_phone().page_desktop());
    stays_off(&harness, STAYS_OFF);

    assert!(harness.simulated_phone().blocked(), "the Phone is blocked");
    assert_eq!(
        harness.simulated_phone().connects_while_blocked(),
        0,
        "the Desktop made a Connect or ConnectProfile while calls were off"
    );
    assert_eq!(harness.wireplumber_drop_ins(), drop_ins);

    // The restarted app's Indicator turns them on again.
    harness.tray().click("Turn calls on");
    wait_connected(&harness, RECONNECT_DEADLINE);
}

#[test]
fn calls_on_from_the_cli_connects_the_phone_within_10_s_and_clears_the_record() {
    let harness = connected();
    let drop_ins = harness.wireplumber_drop_ins();

    let said = cli_ok(&harness, &["calls", "off"]);
    assert_eq!(
        said,
        "Calls are off: your phone stays disconnected until you turn calls on.\n"
    );
    wait_off(&harness);
    // Not "out of range": the Phone is away by the user's choice.
    assert_eq!(
        cli_ok(&harness, &["status"]),
        "Calls are off until you turn them on: your phone stays disconnected.\n"
    );

    let said = cli_ok(&harness, &["calls", "on"]);
    wait_connected(&harness, RECONNECT_DEADLINE);
    let took = reconnect_took(&harness);

    assert_eq!(said, "Calls are on: connecting your phone.\n");
    assert!(took <= RECONNECT_DEADLINE, "connected {took:?} after unblocking");
    assert!(!harness.simulated_phone().blocked());
    assert!(!record(&harness).exists(), "the record is cleared");
    assert_eq!(harness.simulated_phone().connects_while_blocked(), 0);
    assert_eq!(harness.wireplumber_drop_ins(), drop_ins);
    indicator_says(&harness, "My Pixel connected");
}

#[test]
fn calls_on_connects_within_10_s_when_the_first_connect_fails_busy() {
    let harness = connected();
    harness
        .simulated_phone()
        .enable_quirk("connect-busy-after-unblock");
    cli_ok(&harness, &["calls", "off"]);
    wait_off(&harness);

    cli_ok(&harness, &["calls", "on"]);
    wait_connected(&harness, RECONNECT_DEADLINE);
    let took = reconnect_took(&harness);

    assert!(took <= RECONNECT_DEADLINE, "connected {took:?} after unblocking");
    harness.wait_for_app_log("br-connection-busy", DEADLINE);
}

#[test]
fn calls_cant_be_turned_off_during_a_call() {
    let harness = connected();
    let _call = harness.simulated_phone().ring(NUMBER);
    harness.wait_for_app("the Call rings", DEADLINE, |state| state.ringing);

    let output = harness.cli(&["calls", "off"]);

    assert!(!output.status.success(), "{output:?}");
    assert_eq!(
        String::from_utf8_lossy(&output.stderr),
        "Calls can't be turned off during a Call; turn them off once it has ended.\n"
    );
    let menu = harness.tray().menu();
    assert!(
        menu.iter()
            .any(|entry| entry.label == "Turn calls off" && !entry.enabled),
        "{menu:?}"
    );
    assert!(!harness.simulated_phone().blocked());
    assert_eq!(harness.app_state().calls_off, "on");
}

#[test]
fn a_turn_on_a_killed_app_left_halfway_is_finished_at_the_next_start() {
    let mut harness = connected();
    let address = harness
        .app_state()
        .phone
        .map(|phone| phone.address)
        .expect("a Phone");
    cli_ok(&harness, &["calls", "off"]);
    wait_off(&harness);

    // Killed after keeping "on" and before unblocking the Phone.
    harness.kill_app_by_hand();
    fs::write(
        record(&harness),
        format!(r#"{{"version":1,"state":"on","address":"{address}"}}"#),
    )
    .expect("write the record");
    harness.start_app_by_hand();

    wait_connected(&harness, RECONNECT_DEADLINE);
    assert!(!harness.simulated_phone().blocked());
    assert!(!record(&harness).exists(), "the record is cleared");
}

#[test]
fn a_turn_off_a_killed_app_left_halfway_is_finished_at_the_next_start() {
    let mut harness = connected();
    let address = harness
        .app_state()
        .phone
        .map(|phone| phone.address)
        .expect("a Phone");

    // Killed after keeping "off" and before blocking the Phone.
    harness.kill_app_by_hand();
    fs::write(
        record(&harness),
        format!(r#"{{"version":1,"state":"off","address":"{address}"}}"#),
    )
    .expect("write the record");
    assert!(!harness.simulated_phone().blocked());
    harness.start_app_by_hand();

    wait_off(&harness);
    assert!(harness.simulated_phone().blocked(), "the Phone is blocked");
    stays_off(&harness, STAYS_OFF);
    assert_eq!(harness.simulated_phone().connects_while_blocked(), 0);
}

#[test]
fn unpairing_the_phone_while_calls_are_off_turns_them_on() {
    let harness = connected();
    let address = harness
        .app_state()
        .phone
        .map(|phone| phone.address)
        .expect("a Phone");
    cli_ok(&harness, &["calls", "off"]);
    wait_off(&harness);

    let path = format!("/org/bluez/hci0/dev_{}", address.replace(':', "_"));
    let path = zbus::ObjectPath::try_from(path.as_str()).expect("a device path");
    harness.call_phone_method(
        Bus::System,
        "org.bluez",
        "/org/bluez/hci0",
        "org.bluez.Adapter1",
        "RemoveDevice",
        &(path,),
    );

    harness.wait_for_app("calls on, with no Phone", DEADLINE, |state| {
        state.calls_off == "on" && state.phone.is_none()
    });
    assert!(!record(&harness).exists(), "the record is cleared");
    indicator_says(&harness, "No Phone set up");
}

#[test]
fn a_phone_blocked_with_no_record_is_left_blocked() {
    let mut harness = connected();
    cli_ok(&harness, &["calls", "off"]);
    wait_off(&harness);

    // As if the user had blocked it themselves.
    harness.kill_app_by_hand();
    fs::remove_file(record(&harness)).expect("remove the record");
    harness.start_app_by_hand();

    harness.app_holds("calls stay on, as nobody turned them off", DEADLINE, |state| {
        state.calls_off == "on"
    });
    assert!(harness.simulated_phone().blocked(), "the Phone is still blocked");
}
