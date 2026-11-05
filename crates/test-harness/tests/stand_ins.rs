//! The desktop stand-ins: a notification server, a tray watcher and Omarchy's
//! Do Not Disturb state file, exercised the way the Desktop's own clients
//! would use them.

use std::io::Read as _;
use std::process::{Child, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use test_harness::{Harness, TrayItem};
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

#[test]
fn toggling_do_not_disturb_rewrites_omarchys_notification_state_file() {
    let harness = Harness::start();
    // Where Omarchy keeps it: `~/.local/state/omarchy/notifications.json`.
    let state_file = harness
        .directory()
        .join("home/.local/state/omarchy/notifications.json");
    assert!(
        !state_file.exists(),
        "no state file until the first toggle, which a reader treats as off"
    );

    harness.set_do_not_disturb(true);
    assert_eq!(
        read_json(&state_file),
        serde_json::json!({ "version": 3, "dnd": true })
    );

    harness.set_do_not_disturb(false);
    assert_eq!(
        read_json(&state_file),
        serde_json::json!({ "version": 3, "dnd": false })
    );
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
fn read_json(path: &std::path::Path) -> serde_json::Value {
    let text = std::fs::read_to_string(path).expect("read the state file");
    serde_json::from_str(&text).expect("the state file is JSON")
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
