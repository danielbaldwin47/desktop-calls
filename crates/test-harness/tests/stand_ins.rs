//! The desktop stand-ins: a notification server, a tray watcher and Omarchy's
//! Do Not Disturb state file, exercised the way the Desktop's own clients
//! would use them.

use std::collections::HashMap;
use std::fs;
use std::io::Read as _;
use std::os::unix::fs::MetadataExt as _;
use std::path::Path;
use std::process::{Child, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use test_harness::{Harness, Notification, TrayItem};
use zbus::interface;
use zbus::object_server::SignalEmitter;

/// How long a stand-in may take to see what a client did.
const DEADLINE: Duration = Duration::from_secs(5);

#[test]
fn the_notification_server_records_notify_send_and_invokes_its_default_action() {
    let harness = Harness::start();
    let notify_send = harness
        .external_command("notify-send")
        .args([
            "--app-name=Desktop Calls",
            "--action=default=Call back",
            "Missed call from Alice",
            "Mobile, 14:02",
        ])
        .stdout(Stdio::piped())
        .spawn()
        .expect("run notify-send (libnotify)");

    let notification = harness.notifications().wait_for(
        "notify-send's notification arrives",
        DEADLINE,
        |notification| notification.summary == "Missed call from Alice",
    );
    assert_eq!(notification.app_name, "Desktop Calls");
    assert_eq!(notification.body, "Mobile, 14:02");
    assert_eq!(
        notification.actions,
        [("default".to_owned(), "Call back".to_owned())]
    );

    harness
        .notifications()
        .invoke_default_action(notification.id);

    // notify-send prints the name of the action the server reports invoked,
    // then exits.
    let (status, printed) = finish_within(notify_send, DEADLINE);
    assert!(status.success(), "notify-send failed: {printed}");
    assert_eq!(printed, "default\n");
}

#[test]
fn the_tray_watcher_records_a_registration_with_its_status_and_icon() {
    let harness = Harness::start();
    let item = TrayIcon {
        status: "Active".to_owned(),
        icon_name: "call-start-symbolic".to_owned(),
    };
    let connection = zbus::block_on(async {
        let connection = zbus::connection::Builder::address(harness.session_bus_address())
            .serve_at("/StatusNotifierItem", item)
            .build()
            .await?;
        connection
            .call_method(
                Some("org.kde.StatusNotifierWatcher"),
                "/StatusNotifierWatcher",
                Some("org.kde.StatusNotifierWatcher"),
                "RegisterStatusNotifierItem",
                &("/StatusNotifierItem",),
            )
            .await?;
        Ok::<_, zbus::Error>(connection)
    })
    .expect("register a tray icon");
    let bus_name = connection
        .unique_name()
        .expect("the tray icon's connection has a name")
        .to_string();

    // Desktop Calls' own Indicator registers too; this test's icon is the
    // one on the test's connection.
    let ours = |item: &TrayItem| item.bus_name == bus_name;
    let items = harness
        .tray()
        .wait_for("the tray icon is registered", DEADLINE, |items| {
            items.iter().any(ours)
        });
    assert_eq!(
        items.into_iter().filter(ours).collect::<Vec<_>>(),
        [TrayItem {
            bus_name: bus_name.clone(),
            path: "/StatusNotifierItem".to_owned(),
            status: Some("Active".to_owned()),
            icon_name: Some("call-start-symbolic".to_owned()),
            // This minimal icon has neither.
            icon_theme_path: None,
            tooltip: None,
        }]
    );

    zbus::block_on(async {
        let item = connection
            .object_server()
            .interface::<_, TrayIcon>("/StatusNotifierItem")
            .await?;
        item.get_mut().await.status = "NeedsAttention".to_owned();
        TrayIcon::new_status(item.signal_emitter(), "NeedsAttention").await
    })
    .expect("change the tray icon's status");

    harness
        .tray()
        .wait_for("the tray icon needs attention", DEADLINE, |items| {
            items
                .iter()
                .any(|item| ours(item) && item.status.as_deref() == Some("NeedsAttention"))
        });
}

/// Omarchy 4.5's notification state directory, under the scenario's home.
const OMARCHY_NOTIFICATIONS: &str = "home/.local/state/omarchy/notifications";

#[test]
fn omarchys_do_not_disturb_state_starts_empty_and_toggles_as_omarchy_45_writes_it() {
    let harness = Harness::start();
    let directory = harness.directory().join(OMARCHY_NOTIFICATIONS);
    let quiet = directory.join("quiet.json");
    // As Omarchy's shell prepares it at startup: a private directory and an
    // empty `quiet.json`, which means off.
    assert_eq!(mode(&directory), 0o700, "the directory is private");
    assert_eq!(read(&quiet), "", "empty until the first toggle");
    assert_eq!(mode(&quiet), 0o600);

    // `JSON.stringify({ dnd }) + "\n"`, from Omarchy 4.5's `Store.js`.
    harness.set_do_not_disturb(true);
    assert_eq!(read(&quiet), "{\"dnd\":true}\n");
    assert_eq!(mode(&quiet), 0o600, "an atomic write keeps the mode");

    harness.set_do_not_disturb(false);
    assert_eq!(read(&quiet), "{\"dnd\":false}\n");

    assert!(
        !harness
            .directory()
            .join("home/.local/state/omarchy/notifications.json")
            .exists(),
        "Omarchy 4.5 has no `notifications.json`; its migration deletes 4.0's"
    );
}

#[test]
fn a_scenario_can_start_with_do_not_disturb_on() {
    let harness = Harness::builder().do_not_disturb().start();
    let quiet = harness
        .directory()
        .join(OMARCHY_NOTIFICATIONS)
        .join("quiet.json");
    assert_eq!(read(&quiet), "{\"dnd\":true}\n");

    // The notification server silences from the start too.
    let notification = notify(
        &harness,
        "Desktop Calls",
        NORMAL,
        "Missed call from Alice",
        "",
    );
    assert!(notification.silenced, "{notification:?}");
}

#[test]
fn a_scenario_can_start_without_omarchys_notification_state() {
    let harness = Harness::builder().no_omarchy_notification_state().start();
    assert!(
        !harness.directory().join(OMARCHY_NOTIFICATIONS).exists(),
        "as on a Desktop without Omarchy"
    );
}

/// What Omarchy 4.5 makes of each notification text (summary, body): the
/// verification codes it finds, which it offers to copy and which let the
/// notification through Do Not Disturb. Worked out by running Omarchy's own
/// `Detect.js` and `Delivery.js` on these texts; see
/// `docs/research/2026-11-05-notification-text.md`.
const OMARCHY_VERDICTS: &[(&str, &str, &[&str])] = &[
    ("Missed call from Alice (mobile)", "08:30", &[]),
    ("Your verification code", "482913", &["482913"]),
    (
        "Missed call from Gate code 4821 (mobile)",
        "08:30",
        &["4821"],
    ),
    ("Door code", "482-913", &["482913"]),
    ("code", "G-482913", &["482913"]),
    ("2FA", "48 2139", &["482139"]),
    ("One-time password", "12 34 5678", &["1234", "5678"]),
    ("Two factor", "4821 and 9999 and 4821", &["4821", "9999"]),
    ("Confirmation", "4821", &["4821"]),
    ("Enter 482913 as your code", "", &["482913"]),
    // Uppercase letters and digits, when no digits alone are near.
    ("PIN", "A9F3K2", &["A9F3K2"]),
    // A capitalised word before "Code" makes a product name.
    ("Claude Code", "Build 48213 passed", &[]),
    // A year, too many digits, a digit run joined to a word.
    ("Verify by 2026", "", &[]),
    ("Code", "123456789", &[]),
    ("Security key", "4821_x", &[]),
    // The keyword must be a whole word, and the code near it.
    ("confirmations", "4821", &[]),
    ("Missed call from Pinkerton 4821", "08:30", &[]),
    (
        "code",
        "................................................................................482913",
        &[],
    ),
    // A zero-width non-joiner inside the keyword hides it.
    ("Missed call from P\u{200c}in 4821 (mobile)", "08:30", &[]),
];

/// Normal and critical urgency, from the notification specification.
const NORMAL: u8 = 1;
const CRITICAL: u8 = 2;

#[test]
fn under_do_not_disturb_the_notification_server_silences_what_omarchy_silences() {
    let harness = Harness::start();
    harness.set_do_not_disturb(true);

    for (summary, body, codes) in OMARCHY_VERDICTS {
        let notification = notify(&harness, "Desktop Calls", NORMAL, summary, body);
        assert_eq!(
            (notification.verification_codes, notification.silenced),
            (
                codes.iter().map(|code| (*code).to_owned()).collect(),
                codes.is_empty()
            ),
            "{summary:?} {body:?}"
        );
    }
}

#[test]
fn only_omarchys_own_senders_get_through_do_not_disturb_without_a_code() {
    let harness = Harness::start();
    harness.set_do_not_disturb(true);

    for (app_name, urgency, silenced) in [
        ("omarchy-action", NORMAL, false),
        ("notify-send", CRITICAL, false),
        ("notify-send", NORMAL, true),
        // Critical alone is not enough.
        ("Desktop Calls", CRITICAL, true),
    ] {
        let notification = notify(&harness, app_name, urgency, "Backup finished", "08:30");
        assert_eq!(notification.silenced, silenced, "{app_name} {urgency}");
    }
}

#[test]
fn without_do_not_disturb_nothing_is_silenced() {
    let harness = Harness::start();
    harness.set_do_not_disturb(true);
    harness.set_do_not_disturb(false);

    let notification = notify(
        &harness,
        "Desktop Calls",
        NORMAL,
        "Missed call from Alice",
        "",
    );
    assert!(!notification.silenced, "{notification:?}");
}

/// Sends a notification as an app would, and returns what the notification
/// server recorded of it.
#[expect(
    clippy::expect_used,
    reason = "a test helper fails its test the way the test itself would"
)]
fn notify(
    harness: &Harness,
    app_name: &str,
    urgency: u8,
    summary: &str,
    body: &str,
) -> Notification {
    let id: u32 = zbus::block_on(async {
        let connection = zbus::connection::Builder::address(harness.session_bus_address())
            .build()
            .await?;
        let reply = connection
            .call_method(
                Some("org.freedesktop.Notifications"),
                "/org/freedesktop/Notifications",
                Some("org.freedesktop.Notifications"),
                "Notify",
                &(
                    app_name,
                    0_u32,
                    "",
                    summary,
                    body,
                    Vec::<&str>::new(),
                    HashMap::from([("urgency", zbus::Variant::from(urgency))]),
                    -1_i32,
                ),
            )
            .await?;
        reply.body().deserialize::<u32>()
    })
    .expect("send a notification");
    harness
        .notifications()
        .received()
        .into_iter()
        .rfind(|notification| notification.id == id)
        .expect("the notification server recorded it")
}

/// A minimal tray icon, as an app's `StatusNotifierItem` would be.
struct TrayIcon {
    status: String,
    icon_name: String,
}

#[interface(interface = "org.kde.StatusNotifierItem")]
impl TrayIcon {
    #[zbus(property)]
    fn status(&self) -> String {
        self.status.clone()
    }

    #[zbus(property)]
    fn icon_name(&self) -> String {
        self.icon_name.clone()
    }

    #[zbus(signal)]
    async fn new_status(emitter: &SignalEmitter<'_>, status: &str) -> zbus::Result<()>;
}

#[expect(
    clippy::expect_used,
    reason = "a test helper fails its test the way the test itself would"
)]
fn read(path: &Path) -> String {
    fs::read_to_string(path).expect("read the state file")
}

/// `path`'s permission bits.
#[expect(
    clippy::expect_used,
    reason = "a test helper fails its test the way the test itself would"
)]
fn mode(path: &Path) -> u32 {
    fs::metadata(path).expect("stat the state file").mode() & 0o777
}

/// Waits for `child` to exit, and returns its status and standard output.
#[expect(
    clippy::expect_used,
    reason = "a test helper fails its test the way the test itself would"
)]
fn finish_within(mut child: Child, deadline: Duration) -> (std::process::ExitStatus, String) {
    let mut stdout = child.stdout.take().expect("standard output is piped");
    let (sender, receiver) = mpsc::channel();
    thread::spawn(move || {
        let mut printed = String::new();
        let _ = stdout.read_to_string(&mut printed);
        let _ = sender.send(printed);
    });
    let Ok(printed) = receiver.recv_timeout(deadline) else {
        let _ = child.kill();
        panic!("the process did not exit within {deadline:?}");
    };
    (child.wait().expect("wait for the process"), printed)
}
