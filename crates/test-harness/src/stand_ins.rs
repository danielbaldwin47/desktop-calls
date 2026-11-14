//! The desktop stand-ins: what the Desktop's shell would provide, recording
//! what the app asks of it.
//!
//! - a notification server, `org.freedesktop.Notifications`, which records
//!   every `Notify`, decides as Omarchy 4.5 does whether Do Not Disturb
//!   silences it, and can click a notification (invoke its `default`
//!   action) the way Omarchy does;
//! - a tray watcher, `org.kde.StatusNotifierWatcher`, which records every
//!   registration and reads each item's status and icon;
//! - the settings portal, `org.freedesktop.portal.Settings`, which tells GTK
//!   whether the Desktop is light or dark;
//! - Omarchy 4.5's Do Not Disturb state file,
//!   `~/.local/state/omarchy/notifications/quiet.json`, kept in
//!   [`crate::omarchy_quiet`];
//! - Omarchy's current theme, `~/.local/state/omarchy/current/theme/`,
//!   switched the way `omarchy-theme-set` switches it;
//! - Omarchy's bar: `test-client --bar`, a layer-shell surface across the
//!   top of the output that reserves [`BAR_HEIGHT`], drawn in [`BAR_COLOUR`].
//!
//! Hyprland's sockets are in [`crate::hyprland`].
//!
//! The D-Bus stand-ins share one connection to the scenario's session bus,
//! served from the harness's process.

use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::path::Path;
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use futures_util::StreamExt as _;
use zbus::fdo::PropertiesProxy;
use zbus::message::{Header, Type};
use zbus::names::InterfaceName;
use zbus::object_server::SignalEmitter;
use zbus::{Connection, MatchRule, MessageStream, OwnedVariant, Variant, interface};

use crate::{omarchy_quiet, within};

/// The height of Omarchy's bar, in layout coordinates.
pub const BAR_HEIGHT: f64 = 26.0;
/// The bar stand-in's colour, so screenshots can tell it from the app.
pub const BAR_COLOUR: [u8; 3] = [0x30, 0x30, 0x60];

/// One of the harness's two Omarchy themes, for the theme stand-in. Their
/// `colors.toml` files are in `crates/test-harness/fixtures/omarchy-themes/`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OmarchyTheme {
    Light,
    Dark,
}

impl OmarchyTheme {
    /// Its name, for file names: `"omarchy-light"` or `"omarchy-dark"`.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Light => "omarchy-light",
            Self::Dark => "omarchy-dark",
        }
    }

    /// Whether the theme is light or dark, which Omarchy also tells the
    /// settings portal.
    #[must_use]
    pub fn mode(self) -> Theme {
        match self {
            Self::Light => Theme::Light,
            Self::Dark => Theme::Dark,
        }
    }

    /// The theme's `colors.toml`.
    #[must_use]
    pub fn colors_toml(self) -> &'static str {
        match self {
            Self::Light => include_str!("../fixtures/omarchy-themes/light/colors.toml"),
            Self::Dark => include_str!("../fixtures/omarchy-themes/dark/colors.toml"),
        }
    }

    /// The colour `colors.toml` gives `key`, such as `"background"`.
    ///
    /// # Panics
    ///
    /// If the theme has no such colour.
    #[must_use]
    pub fn colour(self, key: &str) -> [u8; 3] {
        self.colors_toml()
            .lines()
            .filter_map(|line| line.split_once('='))
            .find(|(name, _)| name.trim() == key)
            .and_then(|(_, value)| {
                let hex = value.trim().trim_matches('"').strip_prefix('#')?;
                let channel = |at: usize| u8::from_str_radix(hex.get(at..at + 2)?, 16).ok();
                Some([channel(0)?, channel(2)?, channel(4)?])
            })
            .unwrap_or_else(|| panic!("{} has no colour {key:?}", self.as_str()))
    }
}

/// Makes `theme` Omarchy's current theme under `home`, as `omarchy-theme-set`
/// does: it builds the new theme in `current/next-theme`, deletes
/// `current/theme`, moves the new one into its place, then writes
/// `theme.generation` and `theme.name`.
pub(crate) fn switch_omarchy_theme(home: &Path, theme: OmarchyTheme) -> std::io::Result<()> {
    let current = home.join(".local/state/omarchy/current");
    let next = current.join("next-theme");
    fs::create_dir_all(&next)?;
    fs::write(next.join("colors.toml"), theme.colors_toml())?;
    match fs::remove_dir_all(current.join("theme")) {
        Err(error) if error.kind() != std::io::ErrorKind::NotFound => return Err(error),
        _ => {}
    }
    fs::rename(&next, current.join("theme"))?;
    let generation = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |since| since.as_micros());
    fs::write(current.join("theme.generation"), format!("{generation}\n"))?;
    fs::write(current.join("theme.name"), format!("{}\n", theme.as_str()))
}

const NOTIFICATIONS_NAME: &str = "org.freedesktop.Notifications";
const NOTIFICATIONS_PATH: &str = "/org/freedesktop/Notifications";
const NOTIFICATIONS_INTERFACE: &str = "org.freedesktop.Notifications";

const WATCHER_NAME: &str = "org.kde.StatusNotifierWatcher";
const WATCHER_PATH: &str = "/StatusNotifierWatcher";
const ITEM_INTERFACE: &str = "org.kde.StatusNotifierItem";
const MENU_INTERFACE: &str = "com.canonical.dbusmenu";
/// Where an item that registers with only a bus name has its object.
const DEFAULT_ITEM_PATH: &str = "/StatusNotifierItem";

const PORTAL_NAME: &str = "org.freedesktop.portal.Desktop";
const PORTAL_PATH: &str = "/org/freedesktop/portal/desktop";

/// `NotificationClosed` reasons from the notification specification.
const CLOSED_BY_USER: u32 = 2;
const CLOSED_BY_CALL: u32 = 3;
/// The urgency a notification without an `urgency` hint has.
const URGENCY_NORMAL: u8 = 1;

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
    /// What Omarchy 4.5 takes for verification codes in its summary and
    /// body, which it offers to copy.
    pub verification_codes: Vec<String>,
    /// Whether Omarchy 4.5 keeps it off the screen: Do Not Disturb was on,
    /// and it neither came from Omarchy's own senders nor carried a
    /// verification code.
    pub silenced: bool,
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

/// One item registered with the tray watcher, with what it reports now.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TrayItem {
    pub bus_name: String,
    pub path: String,
    /// `Status`: `"Passive"`, `"Active"` or `"NeedsAttention"`, or `None` if
    /// the item doesn't answer.
    pub status: Option<String>,
    /// `IconName`, or `None` if the item doesn't answer.
    pub icon_name: Option<String>,
    /// `IconThemePath`: where a tray looks for `icon_name` when the icon
    /// theme has no such icon, or `None` if the item doesn't answer.
    pub icon_theme_path: Option<String>,
    /// The title of `ToolTip`, which a tray shows on hover, or `None` if the
    /// item doesn't answer.
    pub tooltip: Option<String>,
}

/// One entry of a tray item's menu, as a tray reads it over dbusmenu.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MenuEntry {
    pub id: i32,
    /// What the entry says; empty for a separator.
    pub label: String,
    pub separator: bool,
    pub enabled: bool,
    /// For an entry with a checkmark, whether it is ticked.
    pub ticked: Option<bool>,
    /// A submenu's entries.
    pub children: Vec<MenuEntry>,
}

impl MenuEntry {
    /// The entries' labels, a separator as `"---"`, as a tray lists them.
    #[must_use]
    pub fn labels(entries: &[Self]) -> Vec<String> {
        entries
            .iter()
            .map(|entry| {
                if entry.separator {
                    "---".to_owned()
                } else {
                    entry.label.clone()
                }
            })
            .collect()
    }

    /// The entry labelled `label`, here or in a submenu.
    #[must_use]
    pub fn find<'a>(entries: &'a [Self], label: &str) -> Option<&'a Self> {
        entries.iter().find_map(|entry| {
            if entry.label == label {
                Some(entry)
            } else {
                Self::find(&entry.children, label)
            }
        })
    }
}

/// What the stand-ins have recorded.
#[derive(Default)]
struct Records {
    notifications: Vec<Notification>,
    next_notification_id: u32,
    /// Whether Omarchy's Do Not Disturb is on.
    do_not_disturb: bool,
    /// Registered tray items: bus name and object path.
    tray_items: Vec<(String, String)>,
    /// Every status the registered items have reported, in order, each
    /// change once.
    tray_statuses: Vec<String>,
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

    /// Records the statuses `items` report, each change once.
    fn record_statuses(&self, items: Vec<TrayItem>) {
        let mut records = self.records();
        for status in items.into_iter().filter_map(|item| item.status) {
            if records.tray_statuses.last() != Some(&status) {
                records.tray_statuses.push(status);
            }
        }
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
        record_tray_statuses(address.to_owned(), shared.clone());
        Ok(Self { connection, shared })
    }

    /// Every notification received so far, in order.
    pub fn notifications(&self) -> Vec<Notification> {
        self.shared.records().notifications.clone()
    }

    /// Turns Do Not Disturb on or off for the notifications that arrive
    /// from now on.
    pub fn set_do_not_disturb(&self, on: bool) {
        self.shared.records().do_not_disturb = on;
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
        self.invoke_action(id, "default")
    }

    /// Presses notification `id`'s button for action `key`: invokes the
    /// action, then closes the notification as dismissed by the user, as
    /// Omarchy does.
    pub fn invoke_action(&self, id: u32, key: &str) -> Result<(), String> {
        let offered =
            self.shared.records().notifications.iter().any(|found| {
                found.id == id && found.actions.iter().any(|(offered, _)| offered == key)
            });
        if !offered {
            return Err(format!("notification {id} has no {key:?} action"));
        }
        zbus::block_on(async {
            self.connection
                .emit_signal(
                    None::<&str>,
                    NOTIFICATIONS_PATH,
                    NOTIFICATIONS_INTERFACE,
                    "ActionInvoked",
                    &(id, key),
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

    /// Waits until the one registered tray item's menu satisfies
    /// `condition`, reading it again on each signal, such as the menu's
    /// `LayoutUpdated`.
    pub fn wait_for_tray_menu(
        &self,
        deadline: Duration,
        condition: impl Fn(&[MenuEntry]) -> bool,
    ) -> Result<Vec<MenuEntry>, String> {
        let waited = zbus::block_on(within(deadline, async {
            // Subscribe before reading, so no change slips between the two.
            let rule = MatchRule::builder().msg_type(Type::Signal).build()?;
            let mut signals = MessageStream::for_match_rule(rule, &self.connection, None).await?;
            loop {
                if let Ok((bus_name, path)) = self.only_tray_item()
                    && let Ok(menu) = read_tray_menu(&self.connection, &bus_name, &path).await
                    && condition(&menu)
                {
                    return Ok::<_, zbus::Error>(menu);
                }
                signals.next().await;
            }
        }));
        match waited {
            Some(Ok(menu)) => Ok(menu),
            Some(Err(error)) => Err(error.to_string()),
            None => Err(format!("the menu is {:?}", self.tray_menu())),
        }
    }

    /// Every status the registered items have reported, in order.
    pub fn tray_statuses(&self) -> Vec<String> {
        self.shared.records().tray_statuses.clone()
    }

    /// The one registered tray item, as a tray addresses it.
    fn only_tray_item(&self) -> Result<(String, String), String> {
        match self.shared.records().tray_items.as_slice() {
            [item] => Ok(item.clone()),
            items => Err(format!("expected one tray item, found {items:?}")),
        }
    }

    /// Clicks the one registered tray item, as Omarchy's bar does on a left
    /// click: `Activate` at the pointer's position.
    pub fn activate_tray_item(&self) -> Result<(), String> {
        let (bus_name, path) = self.only_tray_item()?;
        zbus::block_on(self.connection.call_method(
            Some(bus_name.as_str()),
            path.as_str(),
            Some(ITEM_INTERFACE),
            "Activate",
            &(0_i32, 0_i32),
        ))
        .map(drop)
        .map_err(|error| error.to_string())
    }

    /// The one registered tray item's menu, read as a tray reads it when it
    /// opens: `AboutToShow`, then the whole layout.
    pub fn tray_menu(&self) -> Result<Vec<MenuEntry>, String> {
        let (bus_name, path) = self.only_tray_item()?;
        zbus::block_on(read_tray_menu(&self.connection, &bus_name, &path))
            .map_err(|error| error.to_string())
    }

    /// Clicks the entry labelled `label` in the one registered tray item's
    /// menu, as a tray does: a dbusmenu `clicked` event.
    pub fn click_tray_menu(&self, label: &str) -> Result<(), String> {
        let entries = self.tray_menu()?;
        let entry = MenuEntry::find(&entries, label)
            .ok_or_else(|| format!("the menu has no {label:?}: {entries:#?}"))?;
        if !entry.enabled {
            return Err(format!("{label:?} is disabled: {entries:#?}"));
        }
        let (bus_name, path) = self.only_tray_item()?;
        zbus::block_on(async {
            let menu = menu_path(&self.connection, &bus_name, &path).await?;
            self.connection
                .call_method(
                    Some(bus_name.as_str()),
                    menu.as_str(),
                    Some(MENU_INTERFACE),
                    "Event",
                    &(entry.id, "clicked", Variant::from(0_i32), 0_u32),
                )
                .await
        })
        .map(drop)
        .map_err(|error| error.to_string())
    }
}

/// A dbusmenu layout node: id, properties, and its children, each a node in
/// a variant.
type MenuNode = (i32, HashMap<String, OwnedVariant>, Vec<OwnedVariant>);

/// The menu of the tray item `path` at `bus_name`, read as a tray reads it
/// when it opens: `AboutToShow`, then the whole layout.
async fn read_tray_menu(
    connection: &Connection,
    bus_name: &str,
    path: &str,
) -> zbus::Result<Vec<MenuEntry>> {
    let menu = menu_path(connection, bus_name, path).await?;
    connection
        .call_method(
            Some(bus_name),
            menu.as_str(),
            Some(MENU_INTERFACE),
            "AboutToShow",
            &(0_i32,),
        )
        .await?;
    let reply = connection
        .call_method(
            Some(bus_name),
            menu.as_str(),
            Some(MENU_INTERFACE),
            "GetLayout",
            &(0_i32, -1_i32, Vec::<String>::new()),
        )
        .await?;
    let (_revision, (_id, _properties, children)): (u32, MenuNode) = reply.body().deserialize()?;
    menu_entries(children)
}

async fn menu_path(connection: &Connection, bus_name: &str, path: &str) -> zbus::Result<String> {
    let properties = PropertiesProxy::builder(connection)
        .destination(bus_name.to_owned())
        .path(path.to_owned())
        .build()
        .await?;
    let value = properties
        .get(InterfaceName::try_from(ITEM_INTERFACE)?, "Menu")
        .await?;
    Ok(zbus::OwnedObjectPath::try_from(value)?.to_string())
}

fn menu_entries(children: Vec<OwnedVariant>) -> zbus::Result<Vec<MenuEntry>> {
    children
        .into_iter()
        .map(|child| {
            let (id, properties, children): MenuNode = child.try_into()?;
            let text = |name: &str| {
                properties
                    .get(name)
                    .and_then(|value| String::try_from(value.try_clone().ok()?).ok())
            };
            let flag = |name: &str| {
                properties
                    .get(name)
                    .and_then(|value| bool::try_from(value.try_clone().ok()?).ok())
            };
            let ticked = (text("toggle-type").as_deref() == Some("checkmark")).then(|| {
                properties
                    .get("toggle-state")
                    .and_then(|value| i32::try_from(value.try_clone().ok()?).ok())
                    == Some(1)
            });
            Ok(MenuEntry {
                id,
                label: text("label").unwrap_or_default(),
                separator: text("type").as_deref() == Some("separator"),
                enabled: flag("enabled").unwrap_or(true),
                ticked,
                children: menu_entries(children)?,
            })
        })
        .collect()
}

/// Records each status the registered tray items report, as a tray sees
/// them: read again whenever a signal arrives, for the rest of the
/// stand-ins' life.
fn record_tray_statuses(address: String, shared: Arc<Shared>) {
    std::thread::spawn(move || {
        zbus::block_on(async {
            // A connection of its own, listening only to the tray's
            // signals: a stream that fills while this waits for property
            // replies would otherwise stall the stand-ins' own connection.
            let connection = zbus::connection::Builder::address(address.as_str())
                .build()
                .await?;
            let items = MatchRule::builder()
                .msg_type(Type::Signal)
                .interface(ITEM_INTERFACE)
                .build()?;
            let registrations = MatchRule::builder()
                .msg_type(Type::Signal)
                .interface(WATCHER_NAME)
                .member("StatusNotifierItemRegistered")
                .build()?;
            let mut signals = futures_util::stream::select(
                MessageStream::for_match_rule(items, &connection, None).await?,
                MessageStream::for_match_rule(registrations, &connection, None).await?,
            );
            loop {
                let registered = shared.records().tray_items.clone();
                let items = read_tray_items(&connection, registered).await;
                shared.record_statuses(items);
                if signals.next().await.is_none() {
                    return Ok::<_, zbus::Error>(());
                }
            }
        })
    });
}

async fn read_tray_items(
    connection: &Connection,
    registered: Vec<(String, String)>,
) -> Vec<TrayItem> {
    let mut items = Vec::new();
    for (bus_name, path) in registered {
        // One GetAll, as a tray reads an item: a snapshot no change can
        // fall in the middle of.
        let mut properties = all_properties(connection, &bus_name, &path)
            .await
            .unwrap_or_default();
        let mut text = |name: &str| {
            properties
                .remove(name)
                .and_then(|value| String::try_from(value).ok())
        };
        let status = text("Status");
        let icon_name = text("IconName");
        let icon_theme_path = text("IconThemePath");
        let tooltip = properties
            .remove("ToolTip")
            .and_then(|value| ToolTip::try_from(value).ok())
            .map(|tooltip| tooltip.2);
        items.push(TrayItem {
            bus_name,
            path,
            status,
            icon_name,
            icon_theme_path,
            tooltip,
        });
    }
    items
}

/// A tray item's `ToolTip`: icon name, icon pixmaps, title and description.
type ToolTip = (String, Vec<(i32, i32, Vec<u8>)>, String, String);

async fn all_properties(
    connection: &Connection,
    bus_name: &str,
    path: &str,
) -> zbus::Result<HashMap<String, OwnedVariant>> {
    let properties = PropertiesProxy::builder(connection)
        .destination(bus_name.to_owned())
        .path(path.to_owned())
        .build()
        .await?;
    Ok(properties
        .get_all(InterfaceName::try_from(ITEM_INTERFACE)?)
        .await?)
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
        let urgency = hints
            .get("urgency")
            .and_then(|urgency| u8::try_from(&**urgency).ok())
            .unwrap_or(URGENCY_NORMAL);
        let verification_codes = omarchy_quiet::verification_codes(&summary, &body);
        let mut records = self.shared.records();
        let silenced = omarchy_quiet::silenced(
            records.do_not_disturb,
            &app_name,
            urgency,
            &verification_codes,
        );
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
            verification_codes,
            silenced,
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
        // As Omarchy's watcher does, an item whose owner has left the bus,
        // such as an app restarted since, is gone.
        let dbus = zbus::fdo::DBusProxy::new(emitter.connection()).await?;
        let earlier = self.shared.records().tray_items.clone();
        let mut kept = Vec::new();
        for (name, item_path) in earlier {
            let owned = match zbus::names::BusName::try_from(name.as_str()) {
                Ok(owner) => dbus.name_has_owner(owner).await.unwrap_or(false),
                Err(_) => false,
            };
            if owned {
                kept.push((name, item_path));
            }
        }
        kept.push((bus_name, path));
        self.shared.records().tray_items = kept;
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
