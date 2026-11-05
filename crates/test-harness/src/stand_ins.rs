//! The desktop stand-ins: what the Desktop's shell would provide, recording
//! what the app asks of it.
//!
//! - a notification server, `org.freedesktop.Notifications`, which records
//!   every `Notify` and can click a notification (invoke its `default`
//!   action) the way Omarchy does;
//! - a tray watcher, `org.kde.StatusNotifierWatcher`, which records every
//!   registration and reads each item's status and icon;
//! - the settings portal, `org.freedesktop.portal.Settings`, which tells GTK
//!   whether the Desktop is light or dark;
//! - Omarchy's notification state file, whose `dnd` field is Do Not Disturb;
//! - Omarchy's bar: `test-client --bar`, a layer-shell surface across the
//!   top of the output that reserves [`BAR_HEIGHT`], drawn in [`BAR_COLOUR`];
//! - Hyprland's request socket, answering `j/layers` with an
//!   `omarchy-notifications` layer as wide as the scenario asks, the way
//!   `hyprctl layers -j` reports Omarchy's notification-toast column.
//!
//! The D-Bus stand-ins share one connection to the scenario's session bus,
//! served from the harness's process.

use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::io::{Read as _, Write as _};
use std::os::unix::net::UnixListener;
use std::path::Path;
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use futures_util::StreamExt as _;
use zbus::fdo::PropertiesProxy;
use zbus::message::{Header, Type};
use zbus::names::InterfaceName;
use zbus::object_server::SignalEmitter;
use zbus::{Connection, MatchRule, MessageStream, OwnedVariant, Variant, interface};

use crate::within;

/// The height of Omarchy's bar, in layout coordinates.
pub const BAR_HEIGHT: f64 = 26.0;
/// The bar stand-in's colour, so screenshots can tell it from the app.
pub const BAR_COLOUR: [u8; 3] = [0x30, 0x30, 0x60];

/// The instance signature the Hyprland stand-in serves under.
pub(crate) const HYPRLAND_SIGNATURE: &str = "desktop-calls-test";

/// Serves Hyprland's request socket under `runtime`, for the rest of the
/// test process, reporting an `omarchy-notifications` layer `toast_width`
/// wide at the top-right of the headless output.
pub(crate) fn serve_hyprland(runtime: &Path, toast_width: u32) -> std::io::Result<()> {
    let directory = runtime.join("hypr").join(HYPRLAND_SIGNATURE);
    fs::create_dir_all(&directory)?;
    let listener = UnixListener::bind(directory.join(".socket.sock"))?;
    let (width, height) = crate::SCREEN_SIZE;
    // `hyprctl layers -j` reports layout coordinates: at scale 1.6 the
    // 3840x2160 output is 2400x1350.
    let (width, height) = (width * 10 / 16, height * 10 / 16);
    let layers = format!(
        "{{\"HEADLESS-1\": {{\"levels\": {{\"0\": [], \"1\": [], \"2\": [], \"3\": [\
         {{\"address\": \"0x1\", \"x\": {x}, \"y\": 0, \"w\": {toast_width}, \
         \"h\": {height}, \"namespace\": \"omarchy-notifications\", \"pid\": 1, \
         \"alpha\": 1.0}}]}}}}}}",
        x = width.saturating_sub(toast_width),
    );
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            let mut request = [0; 256];
            let Ok(length) = stream.read(&mut request) else {
                continue;
            };
            let reply = if &request[..length] == b"j/layers" {
                layers.as_str()
            } else {
                "unknown request"
            };
            let _ = stream.write_all(reply.as_bytes());
        }
    });
    Ok(())
}

const NOTIFICATIONS_NAME: &str = "org.freedesktop.Notifications";
const NOTIFICATIONS_PATH: &str = "/org/freedesktop/Notifications";
const NOTIFICATIONS_INTERFACE: &str = "org.freedesktop.Notifications";

const WATCHER_NAME: &str = "org.kde.StatusNotifierWatcher";
const WATCHER_PATH: &str = "/StatusNotifierWatcher";
const ITEM_INTERFACE: &str = "org.kde.StatusNotifierItem";
/// Where an item that registers with only a bus name has its object.
const DEFAULT_ITEM_PATH: &str = "/StatusNotifierItem";

const PORTAL_NAME: &str = "org.freedesktop.portal.Desktop";
const PORTAL_PATH: &str = "/org/freedesktop/portal/desktop";

/// `NotificationClosed` reasons from the notification specification.
const CLOSED_BY_USER: u32 = 2;
const CLOSED_BY_CALL: u32 = 3;

/// One `Notify` call the notification server received.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Notification {
    /// The id the server gave it.
    pub id: u32,
    pub app_name: String,
    pub replaces_id: u32,
    pub app_icon: String,
    pub summary: String,
    pub body: String,
    /// Pairs of action key and label, such as `("default", "Call back")`.
    pub actions: Vec<(String, String)>,
    /// Each hint's value in GVariant text form, such as `byte 0x02`.
    pub hints: BTreeMap<String, String>,
    pub expire_timeout: i32,
}

/// Whether the Desktop is light or dark, as the settings portal says.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Theme {
    #[default]
    Light,
    Dark,
}

impl Theme {
    /// Its name, for file names: `"light"` or `"dark"`.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Light => "light",
            Self::Dark => "dark",
        }
    }

    /// `org.freedesktop.appearance` `color-scheme`: 1 prefers dark, 2
    /// prefers light.
    fn color_scheme(self) -> u32 {
        match self {
            Self::Dark => 1,
            Self::Light => 2,
        }
    }
}

/// One item registered with the tray watcher, with its status and icon as
/// the item reports them now.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TrayItem {
    pub bus_name: String,
    pub path: String,
    /// `Status`: `"Passive"`, `"Active"` or `"NeedsAttention"`, or `None` if
    /// the item doesn't answer.
    pub status: Option<String>,
    /// `IconName`, or `None` if the item doesn't answer.
    pub icon_name: Option<String>,
}

/// What the stand-ins have recorded.
#[derive(Default)]
struct Records {
    notifications: Vec<Notification>,
    next_notification_id: u32,
    /// Registered tray items: bus name and object path.
    tray_items: Vec<(String, String)>,
}

#[derive(Default)]
struct Shared {
    records: Mutex<Records>,
    changed: Condvar,
}

impl Shared {
    fn records(&self) -> MutexGuard<'_, Records> {
        self.records.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// The running D-Bus stand-ins.
pub(crate) struct StandIns {
    connection: Connection,
    shared: Arc<Shared>,
}

impl StandIns {
    /// Serves the stand-ins on the session bus at `address`.
    pub fn start(address: &str, theme: Theme) -> zbus::Result<Self> {
        let shared = Arc::new(Shared::default());
        let connection = zbus::block_on(
            zbus::connection::Builder::address(address)
                .serve_at(
                    NOTIFICATIONS_PATH,
                    NotificationServer {
                        shared: shared.clone(),
                    },
                )
                .serve_at(
                    WATCHER_PATH,
                    TrayWatcher {
                        shared: shared.clone(),
                    },
                )
                .serve_at(PORTAL_PATH, SettingsPortal { theme })
                .name(NOTIFICATIONS_NAME)
                .name(WATCHER_NAME)
                .name(PORTAL_NAME)
                .build(),
        )?;
        Ok(Self { connection, shared })
    }

    /// Every notification received so far, in order.
    pub fn notifications(&self) -> Vec<Notification> {
        self.shared.records().notifications.clone()
    }

    /// Waits until a received notification satisfies `condition`.
    pub fn wait_for_notification(
        &self,
        deadline: Duration,
        condition: impl Fn(&Notification) -> bool,
    ) -> Result<Notification, Vec<Notification>> {
        let end = Instant::now() + deadline;
        let mut records = self.shared.records();
        loop {
            if let Some(found) = records.notifications.iter().find(|found| condition(found)) {
                return Ok(found.clone());
            }
            let left = end.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return Err(records.notifications.clone());
            }
            records = self
                .shared
                .changed
                .wait_timeout(records, left)
                .unwrap_or_else(PoisonError::into_inner)
                .0;
        }
    }

    /// Clicks notification `id`, as Omarchy does: invokes its `default`
    /// action, then closes it as dismissed by the user.
    pub fn invoke_default_action(&self, id: u32) -> Result<(), String> {
        let offered =
            self.shared.records().notifications.iter().any(|found| {
                found.id == id && found.actions.iter().any(|(key, _)| key == "default")
            });
        if !offered {
            return Err(format!("notification {id} has no default action"));
        }
        zbus::block_on(async {
            self.connection
                .emit_signal(
                    None::<&str>,
                    NOTIFICATIONS_PATH,
                    NOTIFICATIONS_INTERFACE,
                    "ActionInvoked",
                    &(id, "default"),
                )
                .await?;
            self.connection
                .emit_signal(
                    None::<&str>,
                    NOTIFICATIONS_PATH,
                    NOTIFICATIONS_INTERFACE,
                    "NotificationClosed",
                    &(id, CLOSED_BY_USER),
                )
                .await
        })
        .map_err(|error: zbus::Error| error.to_string())
    }

    /// Every registered tray item, with its status and icon read now.
    pub fn tray_items(&self) -> Vec<TrayItem> {
        let registered = self.shared.records().tray_items.clone();
        zbus::block_on(read_tray_items(&self.connection, registered))
    }

    /// Waits until the registered tray items satisfy `condition`.
    pub fn wait_for_tray(
        &self,
        deadline: Duration,
        condition: impl Fn(&[TrayItem]) -> bool,
    ) -> Result<Vec<TrayItem>, String> {
        let waited = zbus::block_on(within(deadline, async {
            // Every change to an item or to the registrations is a signal:
            // subscribe before reading, so none slips between the two.
            let rule = MatchRule::builder().msg_type(Type::Signal).build()?;
            let mut signals = MessageStream::for_match_rule(rule, &self.connection, None).await?;
            loop {
                let registered = self.shared.records().tray_items.clone();
                let items = read_tray_items(&self.connection, registered).await;
                if condition(&items) {
                    return Ok::<_, zbus::Error>(items);
                }
                signals.next().await;
            }
        }));
        match waited {
            Some(Ok(items)) => Ok(items),
            Some(Err(error)) => Err(error.to_string()),
            None => Err(format!("the tray items are {:?}", self.tray_items())),
        }
    }
}

async fn read_tray_items(
    connection: &Connection,
    registered: Vec<(String, String)>,
) -> Vec<TrayItem> {
    let mut items = Vec::new();
    for (bus_name, path) in registered {
        let text = |name: &'static str| {
            let (bus_name, path) = (bus_name.clone(), path.clone());
            async move {
                let properties = PropertiesProxy::builder(connection)
                    .destination(bus_name)
                    .path(path)
                    .build()
                    .await
                    .ok()?;
                let interface = InterfaceName::try_from(ITEM_INTERFACE).ok()?;
                let value = properties.get(interface, name).await.ok()?;
                String::try_from(value).ok()
            }
        };
        let status = text("Status").await;
        let icon_name = text("IconName").await;
        items.push(TrayItem {
            bus_name,
            path,
            status,
            icon_name,
        });
    }
    items
}

/// Writes Omarchy's notification state file under `home` as Omarchy does on
/// each toggle: the whole file, replaced atomically.
pub(crate) fn write_do_not_disturb(home: &Path, on: bool) -> std::io::Result<()> {
    let directory = home.join(".local/state/omarchy");
    fs::create_dir_all(&directory)?;
    let temporary = directory.join("notifications.json.tmp");
    // `JSON.stringify({ version: 3, dnd }, null, 2) + "\n"`.
    fs::write(
        &temporary,
        format!("{{\n  \"version\": 3,\n  \"dnd\": {on}\n}}\n"),
    )?;
    fs::rename(&temporary, directory.join("notifications.json"))
}

struct NotificationServer {
    shared: Arc<Shared>,
}

#[expect(
    clippy::unused_self,
    reason = "zbus serves methods that take &self, even constant ones"
)]
#[interface(interface = "org.freedesktop.Notifications")]
impl NotificationServer {
    /// Omarchy's capabilities, as its server reports them.
    fn get_capabilities(&self) -> Vec<&'static str> {
        vec![
            "persistence",
            "body",
            "body-markup",
            "body-hyperlinks",
            "actions",
            "icon-static",
        ]
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "the notification specification's Notify takes eight"
    )]
    fn notify(
        &self,
        app_name: String,
        replaces_id: u32,
        app_icon: String,
        summary: String,
        body: String,
        actions: Vec<String>,
        hints: HashMap<String, OwnedVariant>,
        expire_timeout: i32,
    ) -> u32 {
        let mut records = self.shared.records();
        let id = if replaces_id != 0
            && records
                .notifications
                .iter()
                .any(|found| found.id == replaces_id)
        {
            replaces_id
        } else {
            records.next_notification_id += 1;
            records.next_notification_id
        };
        records.notifications.push(Notification {
            id,
            app_name,
            replaces_id,
            app_icon,
            summary,
            body,
            actions: pairs(actions),
            hints: hints
                .into_iter()
                .map(|(name, value)| (name, value.to_string()))
                .collect(),
            expire_timeout,
        });
        drop(records);
        self.shared.changed.notify_all();
        id
    }

    async fn close_notification(
        &self,
        id: u32,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
    ) -> zbus::fdo::Result<()> {
        Ok(Self::notification_closed(&emitter, id, CLOSED_BY_CALL).await?)
    }

    fn get_server_information(&self) -> (&'static str, &'static str, &'static str, &'static str) {
        ("quickshell", "Desktop Calls test harness", "1.2", "1.2")
    }

    #[zbus(signal)]
    async fn notification_closed(
        emitter: &SignalEmitter<'_>,
        id: u32,
        reason: u32,
    ) -> zbus::Result<()>;
}

/// `["default", "Call back", ...]` as `[("default", "Call back"), ...]`.
fn pairs(actions: Vec<String>) -> Vec<(String, String)> {
    let mut actions = actions.into_iter();
    let mut pairs = Vec::new();
    while let Some(key) = actions.next() {
        pairs.push((key, actions.next().unwrap_or_default()));
    }
    pairs
}

struct TrayWatcher {
    shared: Arc<Shared>,
}

#[expect(
    clippy::unused_self,
    reason = "zbus serves methods that take &self, even constant ones"
)]
#[interface(interface = "org.kde.StatusNotifierWatcher")]
impl TrayWatcher {
    /// Registers an item by its bus name, or by its object path on the
    /// caller's connection.
    async fn register_status_notifier_item(
        &self,
        service: String,
        #[zbus(header)] header: Header<'_>,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
    ) -> zbus::fdo::Result<()> {
        let sender = header
            .sender()
            .map(ToString::to_string)
            .ok_or_else(|| zbus::fdo::Error::Failed("no sender".to_owned()))?;
        let (bus_name, path) = if service.starts_with('/') {
            (sender, service)
        } else {
            (service, DEFAULT_ITEM_PATH.to_owned())
        };
        let registered = format!("{bus_name}{path}");
        self.shared.records().tray_items.push((bus_name, path));
        self.shared.changed.notify_all();
        Self::status_notifier_item_registered(&emitter, &registered).await?;
        Ok(())
    }

    /// The tray itself registering. The stand-in always has one, so this
    /// changes nothing.
    fn register_status_notifier_host(&self, service: &str) {
        let _ = service;
    }

    #[zbus(property)]
    fn registered_status_notifier_items(&self) -> Vec<String> {
        self.shared
            .records()
            .tray_items
            .iter()
            .map(|(bus_name, path)| format!("{bus_name}{path}"))
            .collect()
    }

    /// Omarchy's bar is a tray host, so there always is one.
    #[zbus(property)]
    fn is_status_notifier_host_registered(&self) -> bool {
        true
    }

    #[zbus(property)]
    fn protocol_version(&self) -> i32 {
        0
    }

    #[zbus(signal)]
    async fn status_notifier_item_registered(
        emitter: &SignalEmitter<'_>,
        service: &str,
    ) -> zbus::Result<()>;
}

/// The settings portal, reduced to the one setting GTK and libadwaita read
/// for light and dark.
struct SettingsPortal {
    theme: Theme,
}

impl SettingsPortal {
    fn settings(&self) -> HashMap<String, HashMap<String, OwnedVariant>> {
        HashMap::from([(
            "org.freedesktop.appearance".to_owned(),
            HashMap::from([(
                "color-scheme".to_owned(),
                OwnedVariant::from(self.theme.color_scheme()),
            )]),
        )])
    }

    fn setting(&self, namespace: &str, key: &str) -> zbus::fdo::Result<OwnedVariant> {
        self.settings()
            .get(namespace)
            .and_then(|keys| keys.get(key))
            .map(OwnedVariant::try_clone)
            .transpose()
            .map_err(|error| zbus::fdo::Error::Failed(error.to_string()))?
            .ok_or_else(|| {
                zbus::fdo::Error::UnknownProperty(format!("no setting {namespace} {key}"))
            })
    }
}

#[expect(
    clippy::unused_self,
    reason = "zbus serves methods that take &self, even constant ones"
)]
#[interface(interface = "org.freedesktop.portal.Settings")]
impl SettingsPortal {
    /// Every setting in the namespaces asked for; an empty list asks for all.
    fn read_all(&self, namespaces: Vec<String>) -> HashMap<String, HashMap<String, OwnedVariant>> {
        let mut settings = self.settings();
        let asked: std::collections::HashSet<String> = namespaces.into_iter().collect();
        if !asked.is_empty() {
            settings.retain(|namespace, _| asked.contains(namespace));
        }
        settings
    }

    fn read_one(&self, namespace: &str, key: &str) -> zbus::fdo::Result<OwnedVariant> {
        self.setting(namespace, key)
    }

    /// The deprecated `Read`, which wraps the value in a second variant.
    fn read(&self, namespace: &str, key: &str) -> zbus::fdo::Result<OwnedVariant> {
        let value = Variant::from(self.setting(namespace, key)?);
        OwnedVariant::try_from(Variant::Variant(Box::new(value)))
            .map_err(|error| zbus::fdo::Error::Failed(error.to_string()))
    }

    #[zbus(property)]
    fn version(&self) -> u32 {
        2
    }
}
