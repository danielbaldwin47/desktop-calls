//! Scenarios: installing Desktop Calls for one user, as `just install` does,
//! puts it where the user's session finds it: on `PATH`, as a systemd user
//! unit that starts with the graphical session, as a D-Bus service the CLI
//! can start, and in the app launcher.
//!
//! Every scenario installs into its own temporary home. None touches the
//! user's real systemd: the unit is checked with `systemd-analyze verify`,
//! and D-Bus activation runs on the scenario's private session bus.

use std::fs;
use std::os::unix::fs::PermissionsExt as _;
use std::path::PathBuf;

use test_harness::Harness;

const UNIT: &str = ".local/share/systemd/user/desktop-calls.service";
const DBUS_SERVICE: &str =
    ".local/share/dbus-1/services/io.github.danielbaldwin47.DesktopCalls.service";
const DESKTOP_FILE: &str =
    ".local/share/applications/io.github.danielbaldwin47.DesktopCalls.desktop";
const BINARY: &str = ".local/bin/desktop-calls";

fn home(harness: &Harness) -> PathBuf {
    harness.directory().join("home")
}

fn read(harness: &Harness, path: &str) -> String {
    let path = home(harness).join(path);
    fs::read_to_string(&path).unwrap_or_else(|error| panic!("read {}: {error}", path.display()))
}

/// The values of every `key=` line in `text`, in order.
fn values<'a>(text: &'a str, key: &str) -> Vec<&'a str> {
    text.lines()
        .filter_map(|line| line.strip_prefix(key)?.strip_prefix('='))
        .collect()
}

#[test]
fn install_puts_every_file_in_the_users_own_directories() {
    let harness = Harness::builder().installed().start();

    for path in [BINARY, UNIT, DBUS_SERVICE, DESKTOP_FILE] {
        assert!(home(&harness).join(path).is_file(), "~/{path} is installed");
    }
    let mode = fs::metadata(home(&harness).join(BINARY))
        .expect("read the installed binary")
        .permissions()
        .mode();
    assert_eq!(mode & 0o777, 0o755, "the binary is executable");
}

#[test]
fn systemd_accepts_the_installed_unit() {
    let harness = Harness::builder().installed().start();

    let output = harness
        .external_command("systemd-analyze")
        .args(["--user", "verify"])
        .arg(home(&harness).join(UNIT))
        .output()
        .expect("run systemd-analyze");

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "systemd-analyze --user verify refused the unit: {stderr}"
    );
    assert!(
        !stderr.contains("desktop-calls"),
        "systemd-analyze warned about the unit: {stderr}"
    );
}

#[test]
fn the_unit_starts_with_the_graphical_session_and_restarts_after_a_crash() {
    let harness = Harness::builder().installed().start();

    let unit = read(&harness, UNIT);

    let binary = home(&harness).join(BINARY);
    assert_eq!(
        values(&unit, "ExecStart"),
        [format!("{} service", binary.display())]
    );
    assert_eq!(values(&unit, "WantedBy"), ["graphical-session.target"]);
    assert_eq!(values(&unit, "PartOf"), ["graphical-session.target"]);
    assert_eq!(values(&unit, "After"), ["graphical-session.target"]);
    assert_eq!(values(&unit, "Restart"), ["on-failure"]);
    // Started once its control interface is on the bus, so D-Bus activation
    // and systemd agree on when it is ready.
    assert_eq!(values(&unit, "Type"), ["dbus"]);
    assert_eq!(
        values(&unit, "BusName"),
        ["io.github.danielbaldwin47.DesktopCalls"]
    );
}

#[test]
fn the_dbus_service_starts_the_systemd_unit() {
    let harness = Harness::builder().installed().start();

    let service = read(&harness, DBUS_SERVICE);

    assert_eq!(
        values(&service, "Name"),
        ["io.github.danielbaldwin47.DesktopCalls"]
    );
    // dbus-broker, the user's session bus, starts only through systemd.
    assert_eq!(
        values(&service, "SystemdService"),
        ["desktop-calls.service"]
    );
    let binary = home(&harness).join(BINARY);
    assert_eq!(
        values(&service, "Exec"),
        [format!("{} service", binary.display())]
    );
}

#[test]
fn status_starts_the_stopped_service_through_dbus_activation() {
    let harness = Harness::builder().installed().start();
    assert!(!harness.app_on_bus(), "the service starts stopped");

    let output = harness.cli(&["status"]);

    assert!(
        output.status.success(),
        "desktop-calls status failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "No Phone is set up yet.\n"
    );
    assert!(harness.app_on_bus(), "the service is running");
}

#[test]
fn the_launcher_entry_runs_the_installed_binary_and_handles_tel_links() {
    let harness = Harness::builder().installed().start();

    let entry = read(&harness, DESKTOP_FILE);

    let binary = home(&harness).join(BINARY);
    assert_eq!(values(&entry, "Exec"), [format!("{} %u", binary.display())]);
    assert_eq!(values(&entry, "MimeType"), ["x-scheme-handler/tel;"]);
}
