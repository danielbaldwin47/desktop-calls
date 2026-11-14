//! Scenarios: removing Desktop Calls never leaves the Phone blocked in
//! BlueZ (ticket #215). `Device1.Blocked` outlives the app, so the removal
//! step turns calls back on first: it unblocks a Phone the app blocked
//! (the record in `calls-off.json` says so), through the running service
//! if there is one, and never touches a Phone it has no record of blocking.
//!
//! Every scenario installs into its own temporary home, with the Simulated
//! Phone as the only Bluetooth there is: no scenario can reach the user's
//! real Phone, BlueZ or systemd.

use std::fs;
use std::path::PathBuf;
use std::process::Output;
use std::time::Duration;

use test_harness::{AppState, Harness};

/// How long the app may take to notice a change on the Phone.
const DEADLINE: Duration = Duration::from_secs(5);
/// Long enough for anything that would unblock the Phone, or start the
/// stopped service, to have done it.
const SETTLE: Duration = Duration::from_secs(2);
const PHONE_NAME: &str = "My Pixel";
const BINARY: &str = ".local/bin/desktop-calls";
const FILES: [&str; 4] = [
    BINARY,
    ".local/share/systemd/user/desktop-calls.service",
    ".local/share/dbus-1/services/io.github.danielbaldwin47.DesktopCalls.service",
    ".local/share/applications/io.github.danielbaldwin47.DesktopCalls.desktop",
];

/// An installed app with its Phone connected, and calls off.
fn calls_off() -> Harness {
    let harness = calls_on_with_phone();
    let output = harness.cli(&["calls", "off"]);
    assert!(output.status.success(), "{output:?}");
    wait_off(&harness);
    assert!(harness.simulated_phone().blocked(), "the Phone is blocked");
    assert!(record(&harness).is_file(), "the app kept a record");
    harness
}

/// An installed app with its Phone connected, and calls on.
fn calls_on_with_phone() -> Harness {
    let harness = Harness::builder()
        .installed()
        .phone_name(PHONE_NAME)
        .start();
    harness.simulated_phone().connect();
    // Starts the service, through D-Bus activation.
    let _ = harness.cli(&["status"]);
    harness.wait_for_app("the Phone is connected", DEADLINE, |state| {
        state.connection_state == "connected"
    });
    harness
}

fn wait_off(harness: &Harness) -> AppState {
    harness.wait_for_app("calls are off", DEADLINE, |state| {
        state.calls_off == "off" && state.connection_state == "disconnected"
    })
}

fn record(harness: &Harness) -> PathBuf {
    harness
        .directory()
        .join("state/desktop-calls/calls-off.json")
}

fn home_file(harness: &Harness, path: &str) -> PathBuf {
    harness.directory().join("home").join(path)
}

fn files_left(harness: &Harness) -> Vec<&'static str> {
    FILES
        .into_iter()
        .filter(|path| home_file(harness, path).exists())
        .collect()
}

/// What the removal said, apart from the files it lists as removed.
fn said(output: &Output) -> String {
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let mut lines = String::new();
    for line in text.lines().filter(|line| !line.starts_with("Removed ")) {
        lines.push_str(line);
        lines.push('\n');
    }
    lines
}

/// How many times the Simulated Phone was asked to unblock the Phone.
fn unblocks_asked(harness: &Harness) -> usize {
    harness
        .simulated_phone()
        .recorded_holds("the Phone is asked about Blocked", Duration::ZERO, |_| true)
        .iter()
        .filter(|call| {
            call.member == "Set"
                && call.arguments.contains("Blocked")
                && call.arguments.contains("false")
        })
        .count()
}

#[test]
fn uninstalling_with_calls_off_unblocks_the_phone_through_the_running_service() {
    let harness = calls_off();
    assert!(harness.app_on_bus(), "the service runs");

    let output = harness.uninstall_command().output().expect("run install.py");

    assert!(output.status.success(), "{}", said(&output));
    assert!(!harness.simulated_phone().blocked(), "the Phone is unblocked");
    assert!(!record(&harness).exists(), "the record is gone");
    assert_eq!(files_left(&harness), Vec::<&str>::new());
    assert_eq!(
        said(&output),
        "Calls are on again: your phone is unblocked.\n"
    );
    // The running service did it, so it didn't block the Phone again, and
    // nothing else asked BlueZ.
    assert_eq!(unblocks_asked(&harness), 1);
    harness.app_holds("calls stay on", SETTLE, |state| state.calls_off == "on");
    assert!(!harness.simulated_phone().blocked());
}

#[test]
fn uninstalling_with_the_service_stopped_unblocks_the_phone_without_starting_it() {
    let harness = calls_off();
    harness.stop_service();
    harness.wait_until_app_off_bus();

    let output = harness.uninstall_command().output().expect("run install.py");

    assert!(output.status.success(), "{}", said(&output));
    assert!(!harness.simulated_phone().blocked(), "the Phone is unblocked");
    assert!(!record(&harness).exists(), "the record is gone");
    assert_eq!(files_left(&harness), Vec::<&str>::new());
    assert!(
        !harness.app_on_bus(),
        "the removal started the service it was removing"
    );
}

#[test]
fn uninstalling_finishes_a_turn_on_the_app_left_halfway() {
    let harness = calls_off();
    let address = harness.phone().address.clone();
    harness.stop_service();
    harness.wait_until_app_off_bus();
    fs::write(
        record(&harness),
        format!(r#"{{"version":1,"state":"on","address":"{address}"}}"#),
    )
    .expect("write the record");

    let output = harness.uninstall_command().output().expect("run install.py");

    assert!(output.status.success(), "{}", said(&output));
    assert!(!harness.simulated_phone().blocked(), "the Phone is unblocked");
    assert!(!record(&harness).exists(), "the record is gone");
}

#[test]
fn uninstalling_leaves_a_phone_blocked_with_no_record_blocked() {
    let harness = calls_off();
    harness.stop_service();
    harness.wait_until_app_off_bus();
    // As if the user had blocked the Phone themselves.
    fs::remove_file(record(&harness)).expect("remove the record");

    let output = harness.uninstall_command().output().expect("run install.py");

    assert!(output.status.success(), "{}", said(&output));
    assert!(harness.simulated_phone().blocked(), "the Phone is still blocked");
    assert_eq!(unblocks_asked(&harness), 0, "BlueZ was asked to unblock");
    assert_eq!(files_left(&harness), Vec::<&str>::new());
    assert_eq!(said(&output), "", "nothing to say about a Phone left alone");
}

#[test]
fn uninstalling_with_calls_on_changes_nothing_at_the_phone() {
    let harness = calls_on_with_phone();

    let output = harness.uninstall_command().output().expect("run install.py");

    assert!(output.status.success(), "{}", said(&output));
    assert!(!harness.simulated_phone().blocked());
    assert_eq!(unblocks_asked(&harness), 0, "BlueZ was asked to unblock");
    assert_eq!(files_left(&harness), Vec::<&str>::new());
    assert_eq!(said(&output), "");
}

#[test]
fn uninstalling_without_bluez_finishes_and_names_the_command_to_run_later() {
    let harness = calls_off();
    let address = harness.phone().address.clone();
    harness.stop_service();
    harness.wait_until_app_off_bus();

    let output = harness
        .uninstall_command()
        .env("DBUS_SYSTEM_BUS_ADDRESS", "unix:path=/nonexistent")
        .output()
        .expect("run install.py");

    assert!(output.status.success(), "{}", said(&output));
    assert_eq!(files_left(&harness), Vec::<&str>::new());
    assert_eq!(
        said(&output),
        format!(
            "Bluetooth didn't unblock your phone: once it is running, run `bluetoothctl unblock {address}` to let it connect to this computer again.\n"
        )
    );
    assert!(harness.simulated_phone().blocked(), "nothing reached the Phone");
}

#[test]
fn uninstalling_while_staging_a_package_makes_no_bluez_call() {
    let harness = calls_off();
    harness.stop_service();
    harness.wait_until_app_off_bus();
    let staging = harness.directory().join("stage");

    let output = harness
        .uninstall_command()
        .env("DESTDIR", &staging)
        .output()
        .expect("run install.py");

    assert!(output.status.success(), "{}", said(&output));
    assert!(harness.simulated_phone().blocked(), "the Phone is still blocked");
    assert!(record(&harness).is_file(), "the record is still there");
    assert_eq!(unblocks_asked(&harness), 0, "BlueZ was asked to unblock");
    assert_eq!(
        files_left(&harness).len(),
        FILES.len(),
        "the files outside the staging directory stay"
    );
    assert_eq!(said(&output), "");
}

#[test]
fn the_removal_flag_only_goes_with_calls_on() {
    let harness = calls_on_with_phone();

    let output = harness.cli(&["calls", "off", "--uninstalling"]);

    assert!(!output.status.success(), "{output:?}");
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("--uninstalling goes with `calls on`"),
        "{output:?}"
    );
    assert!(!harness.simulated_phone().blocked());
    assert_eq!(harness.app_state().calls_off, "on");
}
