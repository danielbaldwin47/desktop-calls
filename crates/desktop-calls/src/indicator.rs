//! The Indicator: a `StatusNotifierItem` with a dbusmenu menu, the icon in the
//! Desktop's top bar.
//!
//! It shows what [`IndicatorView`] says, the Call state machine's decision,
//! and turns a left click and the menu's entries into inputs. It registers
//! with the tray's `org.kde.StatusNotifierWatcher`, again whenever the
//! watcher (Omarchy's bar) restarts.
//!
//! Its icons are symbolic SVGs, written at startup to the app's own data
//! directory, which `IconThemePath` names: Omarchy's bar looks there for an icon
//! name its icon theme lacks, and paints a `-symbolic` icon in the bar's
//! own colour, so the icons follow its light or dark theme. See
//! docs/research/2026-11-05-indicator.md.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::PathBuf;

use desktop_calls_core::{IndicatorEntry, IndicatorLook, IndicatorView, Input, messages};
use futures_channel::mpsc::UnboundedSender;
use futures_util::StreamExt as _;
use glib::MainContext;
use zbus::fdo::DBusProxy;
use zbus::object_server::{InterfaceRef, SignalEmitter};
use zbus::{Connection, OwnedObjectPath, OwnedVariant, Variant, interface};

use crate::control::Published;
use crate::logging::EventKind;

const WATCHER_NAME: &str = "org.kde.StatusNotifierWatcher";
const WATCHER_PATH: &str = "/StatusNotifierWatcher";
const ITEM_PATH: &str = "/StatusNotifierItem";
const MENU_PATH: &str = "/MenuBar";

/// The icons, by look, as file name stems and their SVG.
const ICONS: [(IndicatorLook, &str, &str); 5] = [
    (
        IndicatorLook::Connected,
        "desktop-calls-connected-symbolic",
        include_str!("../../../data/icons/desktop-calls-connected-symbolic.svg"),
    ),
    (
        IndicatorLook::Disconnected,
        "desktop-calls-disconnected-symbolic",
        include_str!("../../../data/icons/desktop-calls-disconnected-symbolic.svg"),
    ),
    (
        IndicatorLook::Ringing,
        "desktop-calls-ringing-symbolic",
        include_str!("../../../data/icons/desktop-calls-ringing-symbolic.svg"),
    ),
    (
        IndicatorLook::InCall,
        "desktop-calls-in-call-symbolic",
        include_str!("../../../data/icons/desktop-calls-in-call-symbolic.svg"),
    ),
    (
        IndicatorLook::Problem,
        "desktop-calls-problem-symbolic",
        include_str!("../../../data/icons/desktop-calls-problem-symbolic.svg"),
    ),
];

/// How many characters fit on one line of Omarchy's tray menu, which is
/// 232 px wide and cuts longer lines short (`Tray.qml`): a Problem's
/// explanation is wrapped to lines this long.
const MENU_LINE: usize = 26;

fn icon_name(look: IndicatorLook) -> &'static str {
    ICONS
        .iter()
        .find(|(icon, _, _)| *icon == look)
        .map_or("", |(_, name, _)| name)
}

/// Shows the Indicator and carries out what is chosen on it.
pub struct Indicator {
    item: InterfaceRef<Item>,
    menu: InterfaceRef<Menu>,
}

impl Indicator {
    /// Serves the Indicator on `session`, sending what is chosen on it to
    /// `intents`, and registers it with the tray, now and whenever the tray
    /// starts again.
    pub async fn serve(
        session: &Connection,
        intents: UnboundedSender<Input>,
    ) -> zbus::Result<Self> {
        let state = desktop_calls_core::State::default();
        let view = state.indicator(&state.surfaces());
        let server = session.object_server();
        server
            .at(
                ITEM_PATH,
                Item {
                    view: view.clone(),
                    icon_theme_path: write_icons(),
                    intents: intents.clone(),
                },
            )
            .await?;
        server
            .at(
                MENU_PATH,
                Menu {
                    view,
                    revision: 1,
                    intents,
                },
            )
            .await?;
        let item = server.interface::<_, Item>(ITEM_PATH).await?;
        let menu = server.interface::<_, Menu>(MENU_PATH).await?;
        let name = format!("org.kde.StatusNotifierItem-{}-1", std::process::id());
        session.request_name(name.as_str()).await?;
        MainContext::default().spawn_local(stay_registered(session.clone(), name));
        Ok(Self { item, menu })
    }

    /// Shows `view`, signalling the tray of each change, once the state it
    /// follows from is `_published` on the control interface.
    pub async fn render(&self, view: &IndicatorView, _published: &Published) -> zbus::Result<()> {
        let emitter = self.item.signal_emitter();
        let mut item = self.item.get_mut().await;
        let old = std::mem::replace(&mut item.view, view.clone());
        if old.look != view.look {
            Item::new_icon(emitter).await?;
        }
        if old.needs_attention != view.needs_attention {
            Item::new_status(emitter, status(view)).await?;
        }
        if messages::indicator_tooltip(&old) != messages::indicator_tooltip(view) {
            Item::new_tool_tip(emitter).await?;
        }
        drop(item);

        let emitter = self.menu.signal_emitter();
        let mut menu = self.menu.get_mut().await;
        let old = std::mem::replace(&mut menu.view, view.clone());
        if layout(&old) != layout(view) {
            menu.revision += 1;
            Menu::layout_updated(emitter, menu.revision, 0).await?;
        }
        Ok(())
    }
}

/// `Status`: never `Passive`, because Omarchy's bar hides Passive items.
fn status(view: &IndicatorView) -> &'static str {
    if view.needs_attention {
        "NeedsAttention"
    } else {
        "Active"
    }
}

/// Writes the icons to `$XDG_DATA_HOME/desktop-calls/icons` (`~/.local/share`
/// by default), the app's own data, and returns the directory, or empty if
/// they couldn't be written; the tray then shows whatever its icon theme
/// has. Written at every start, so they always match this version.
fn write_icons() -> String {
    let Some(directory) = icon_directory() else {
        icons_unwritten(
            "no-directory",
            "neither XDG_DATA_HOME nor HOME is an absolute path",
        );
        return String::new();
    };
    let written = fs::create_dir_all(&directory).and_then(|()| {
        ICONS.iter().try_for_each(|(_, name, svg)| {
            // Written whole and renamed, so a tray never reads half an icon.
            let temporary = directory.join(format!(".{name}.svg"));
            fs::write(&temporary, svg)?;
            fs::rename(&temporary, directory.join(format!("{name}.svg")))
        })
    });
    match written {
        Ok(()) => directory.to_string_lossy().into_owned(),
        Err(error) => {
            icons_unwritten("write-failed", &error.to_string());
            String::new()
        }
    }
}

fn icon_directory() -> Option<PathBuf> {
    let absolute = |name: &str| {
        std::env::var_os(name)
            .map(PathBuf::from)
            .filter(|path| path.is_absolute())
    };
    absolute("XDG_DATA_HOME")
        .or_else(|| absolute("HOME").map(|home| home.join(".local/share")))
        .map(|base| base.join("desktop-calls/icons"))
}

fn icons_unwritten(error_kind: &str, detail: &str) {
    tracing::warn!(
        event_kind = %EventKind::IndicatorIconsUnwritten,
        error_kind,
        detail,
        "Couldn't write the Indicator's icons"
    );
}

/// Registers the Indicator with the tray now, if there is one, and again
/// each time a tray takes the watcher's name.
async fn stay_registered(session: Connection, name: String) {
    let watched = async {
        let bus = DBusProxy::new(&session).await?;
        let mut owners = bus
            .receive_name_owner_changed_with_args(&[(0, WATCHER_NAME)])
            .await?;
        register(&session, &name).await;
        while let Some(change) = owners.next().await {
            if change.args()?.new_owner().is_some() {
                register(&session, &name).await;
            }
        }
        Ok::<_, zbus::Error>(())
    };
    if let Err(error) = watched.await {
        tracing::warn!(
            event_kind = %EventKind::WatchFailed,
            error_kind = "tray-unwatched",
            detail = %error,
            "Stopped watching for the tray; the Indicator won't come back if the tray restarts"
        );
    }
}

async fn register(session: &Connection, name: &str) {
    let registered = session
        .call_method(
            Some(WATCHER_NAME),
            WATCHER_PATH,
            Some(WATCHER_NAME),
            "RegisterStatusNotifierItem",
            &(name,),
        )
        .await;
    match registered {
        Ok(_) => tracing::info!(
            event_kind = %EventKind::IndicatorRegistered,
            "The Indicator is in the tray"
        ),
        // No tray yet: the Indicator registers when one starts.
        Err(error) => tracing::info!(
            event_kind = %EventKind::IndicatorUnregistered,
            error_kind = "no-tray",
            detail = %error,
            "No tray to show the Indicator in yet"
        ),
    }
}

/// The `StatusNotifierItem`.
struct Item {
    view: IndicatorView,
    icon_theme_path: String,
    intents: UnboundedSender<Input>,
}

/// An icon as pixels: width, height and ARGB32 data. The Indicator has
/// none; its icons are named.
type Pixmaps = Vec<(i32, i32, Vec<u8>)>;

#[expect(
    clippy::unused_self,
    reason = "zbus serves methods that take &self, even constant ones"
)]
#[interface(interface = "org.kde.StatusNotifierItem")]
impl Item {
    #[zbus(property)]
    fn category(&self) -> &'static str {
        "Communications"
    }

    /// What Omarchy's bar files the Indicator's pinned or hidden setting
    /// under.
    #[zbus(property)]
    fn id(&self) -> &'static str {
        "desktop-calls"
    }

    #[zbus(property)]
    fn title(&self) -> &'static str {
        "Desktop Calls"
    }

    #[zbus(property)]
    fn status(&self) -> &'static str {
        status(&self.view)
    }

    #[zbus(property)]
    fn window_id(&self) -> i32 {
        0
    }

    #[zbus(property)]
    fn icon_name(&self) -> &'static str {
        icon_name(self.view.look)
    }

    #[zbus(property)]
    fn icon_theme_path(&self) -> String {
        self.icon_theme_path.clone()
    }

    #[zbus(property)]
    fn icon_pixmap(&self) -> Pixmaps {
        Vec::new()
    }

    #[zbus(property)]
    fn overlay_icon_name(&self) -> &'static str {
        ""
    }

    #[zbus(property)]
    fn overlay_icon_pixmap(&self) -> Pixmaps {
        Vec::new()
    }

    /// Empty, so a tray shows `IconName` while asking for attention too.
    #[zbus(property)]
    fn attention_icon_name(&self) -> &'static str {
        ""
    }

    #[zbus(property)]
    fn attention_icon_pixmap(&self) -> Pixmaps {
        Vec::new()
    }

    #[zbus(property)]
    fn attention_movie_name(&self) -> &'static str {
        ""
    }

    /// Icon name, icon pixmaps, title and description.
    #[zbus(property)]
    fn tool_tip(&self) -> (String, Pixmaps, String, String) {
        (
            String::new(),
            Vec::new(),
            messages::indicator_tooltip(&self.view),
            String::new(),
        )
    }

    /// A left click activates the Indicator rather than opening its menu.
    #[zbus(property)]
    fn item_is_menu(&self) -> bool {
        false
    }

    #[zbus(property)]
    fn menu(&self) -> OwnedObjectPath {
        OwnedObjectPath::try_from(MENU_PATH).unwrap_or_default()
    }

    /// A left click expands the collapsed Call Window during a Call, and
    /// otherwise opens the Dialer: the Call state machine decides which.
    fn activate(&self, x: i32, y: i32) {
        let _ = (x, y);
        // The decision loop outlives the Indicator; a closed channel only
        // means the app is stopping.
        let _ = self.intents.unbounded_send(Input::IndicatorActivated);
    }

    /// A middle click does nothing.
    fn secondary_activate(&self, x: i32, y: i32) {
        let _ = (x, y);
    }

    /// The menu is exported as `Menu`, which the tray draws itself.
    fn context_menu(&self, x: i32, y: i32) {
        let _ = (x, y);
    }

    fn scroll(&self, delta: i32, orientation: &str) {
        let _ = (delta, orientation);
    }

    #[zbus(signal)]
    async fn new_title(emitter: &SignalEmitter<'_>) -> zbus::Result<()>;

    #[zbus(signal)]
    async fn new_icon(emitter: &SignalEmitter<'_>) -> zbus::Result<()>;

    #[zbus(signal)]
    async fn new_attention_icon(emitter: &SignalEmitter<'_>) -> zbus::Result<()>;

    #[zbus(signal)]
    async fn new_overlay_icon(emitter: &SignalEmitter<'_>) -> zbus::Result<()>;

    #[zbus(signal)]
    async fn new_tool_tip(emitter: &SignalEmitter<'_>) -> zbus::Result<()>;

    #[zbus(signal)]
    async fn new_status(emitter: &SignalEmitter<'_>, status: &str) -> zbus::Result<()>;
}

/// One entry of the menu as dbusmenu sends it: its id, its properties, and
/// what choosing it does.
#[derive(Clone, Debug, PartialEq)]
struct MenuItem {
    id: i32,
    label: String,
    kind: MenuItemKind,
    children: Vec<MenuItem>,
}

#[derive(Clone, Debug, PartialEq)]
enum MenuItemKind {
    Separator,
    /// An entry; `ticked` for one with a checkmark.
    Standard {
        enabled: bool,
        ticked: Option<bool>,
    },
}

/// The menu's entries for `view`. Each kind of entry keeps its id from one
/// layout to the next, so a click on a menu drawn just before a change
/// still means what it said.
fn layout(view: &IndicatorView) -> Vec<MenuItem> {
    let mut separators = 0;
    view.menu
        .iter()
        .map(|entry| {
            let id = match entry {
                IndicatorEntry::Status => 1,
                IndicatorEntry::OpenDialer => 2,
                IndicatorEntry::HangUp(_) => 3,
                IndicatorEntry::Mute(_) => 4,
                IndicatorEntry::MoveAudio { .. } => 5,
                IndicatorEntry::ConnectPhone => 6,
                IndicatorEntry::HangUpOnPhone => 7,
                IndicatorEntry::Separator => {
                    separators += 1;
                    9 + separators
                }
                IndicatorEntry::Problem(problem) => problem_id(*problem),
            };
            let kind = match entry {
                IndicatorEntry::Separator => MenuItemKind::Separator,
                IndicatorEntry::Mute(mute) => MenuItemKind::Standard {
                    enabled: entry.enabled(),
                    ticked: Some(*mute == desktop_calls_core::Mute::On),
                },
                _ => MenuItemKind::Standard {
                    enabled: entry.enabled(),
                    ticked: None,
                },
            };
            let children = match entry {
                IndicatorEntry::Problem(problem) => {
                    wrap(messages::problem(*problem).text, MENU_LINE)
                        .into_iter()
                        .zip(id + 1..)
                        .map(|(line, id)| MenuItem {
                            id,
                            label: line,
                            kind: MenuItemKind::Standard {
                                enabled: true,
                                ticked: None,
                            },
                            children: Vec::new(),
                        })
                        .collect()
                }
                _ => Vec::new(),
            };
            MenuItem {
                id,
                label: messages::indicator_entry(entry, view),
                kind,
                children,
            }
        })
        .collect()
}

/// A Problem's entry id; the lines of its explanation follow it.
fn problem_id(problem: messages::ProblemKind) -> i32 {
    match problem {
        messages::ProblemKind::ContactsDenied => 100,
        messages::ProblemKind::ContactsPromptUnanswered => 200,
        messages::ProblemKind::PairingLost => 300,
        messages::ProblemKind::BluetoothOff => 400,
        messages::ProblemKind::BluetoothBlocked => 500,
        messages::ProblemKind::ContactsSupportMissing => 600,
        messages::ProblemKind::AudioNotSetUp => 700,
        messages::ProblemKind::BluezHandsFree => 800,
    }
}

/// `text` broken at spaces into lines of at most `width` characters, where
/// it can be.
fn wrap(text: &str, width: usize) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    for word in text.split_whitespace() {
        match lines.last_mut() {
            Some(line) if line.chars().count() + 1 + word.chars().count() <= width => {
                line.push(' ');
                line.push_str(word);
            }
            _ => lines.push(word.to_owned()),
        }
    }
    lines
}

/// A dbusmenu layout node: id, properties, and its children, each a node in
/// a variant.
type MenuNode = (i32, HashMap<String, OwnedVariant>, Vec<OwnedVariant>);

impl MenuItem {
    fn properties(&self) -> HashMap<String, OwnedVariant> {
        let mut properties = HashMap::new();
        match self.kind {
            MenuItemKind::Separator => {
                properties.insert("type".to_owned(), text("separator"));
            }
            MenuItemKind::Standard { enabled, ticked } => {
                properties.insert("label".to_owned(), text(&self.label));
                properties.insert("enabled".to_owned(), OwnedVariant::from(enabled));
                if let Some(ticked) = ticked {
                    properties.insert("toggle-type".to_owned(), text("checkmark"));
                    properties.insert(
                        "toggle-state".to_owned(),
                        OwnedVariant::from(i32::from(ticked)),
                    );
                }
                if !self.children.is_empty() {
                    properties.insert("children-display".to_owned(), text("submenu"));
                }
            }
        }
        properties
    }

    fn node(&self) -> zbus::Result<MenuNode> {
        let children = self
            .children
            .iter()
            .map(|child| OwnedVariant::try_from(Variant::from(child.node()?)))
            .collect::<zbus::Result<_>>()?;
        Ok((self.id, self.properties(), children))
    }

    fn find(items: &[Self], id: i32) -> Option<&Self> {
        items.iter().find_map(|item| {
            (item.id == id)
                .then_some(item)
                .or_else(|| Self::find(&item.children, id))
        })
    }
}

/// The Indicator's menu, served over dbusmenu.
struct Menu {
    view: IndicatorView,
    /// Goes up each time the layout changes.
    revision: u32,
    intents: UnboundedSender<Input>,
}

impl Menu {
    fn root(&self) -> zbus::Result<MenuNode> {
        let children = layout(&self.view)
            .iter()
            .map(|item| OwnedVariant::try_from(Variant::from(item.node()?)))
            .collect::<zbus::Result<_>>()?;
        let mut properties = HashMap::new();
        properties.insert("children-display".to_owned(), text("submenu"));
        Ok((0, properties, children))
    }

    /// The entry `id` names, if it is one choosing does something for.
    fn chosen(&self, id: i32) -> Option<Input> {
        let items = layout(&self.view);
        let index = items.iter().position(|item| item.id == id)?;
        self.view.menu.get(index)?.intent()
    }
}

#[expect(
    clippy::unused_self,
    reason = "zbus serves methods that take &self, even constant ones"
)]
#[interface(interface = "com.canonical.dbusmenu")]
impl Menu {
    /// The whole layout, whatever is asked for: it is small.
    fn get_layout(
        &self,
        parent_id: i32,
        recursion_depth: i32,
        property_names: Vec<String>,
    ) -> zbus::fdo::Result<(u32, MenuNode)> {
        let _ = (recursion_depth, property_names);
        let root = self.root().map_err(|error| failed(&error))?;
        if parent_id == 0 {
            return Ok((self.revision, root));
        }
        let item = MenuItem::find(&layout(&self.view), parent_id)
            .ok_or_else(|| zbus::fdo::Error::InvalidArgs(format!("no menu entry {parent_id}")))?
            .node()
            .map_err(|error| failed(&error))?;
        Ok((self.revision, item))
    }

    fn get_group_properties(
        &self,
        ids: Vec<i32>,
        property_names: Vec<String>,
    ) -> Vec<(i32, HashMap<String, OwnedVariant>)> {
        let items = layout(&self.view);
        // Empty asks for every property.
        let wanted: HashSet<String> = property_names.into_iter().collect();
        ids.into_iter()
            .filter_map(|id| MenuItem::find(&items, id))
            .map(|item| {
                let mut properties = item.properties();
                if !wanted.is_empty() {
                    properties.retain(|name, _| wanted.contains(name));
                }
                (item.id, properties)
            })
            .collect()
    }

    fn get_property(&self, id: i32, name: &str) -> zbus::fdo::Result<OwnedVariant> {
        MenuItem::find(&layout(&self.view), id)
            .and_then(|item| item.properties().remove(name))
            .ok_or_else(|| zbus::fdo::Error::InvalidArgs(format!("no property {name} on {id}")))
    }

    fn event(&self, id: i32, event_id: &str, data: OwnedVariant, timestamp: u32) {
        let _ = (data, timestamp);
        if event_id == "clicked"
            && let Some(intent) = self.chosen(id)
        {
            let _ = self.intents.unbounded_send(intent);
        }
    }

    /// Each event, as [`Self::event`]; returns the ids of entries the menu
    /// doesn't have.
    fn event_group(&self, events: Vec<(i32, String, OwnedVariant, u32)>) -> Vec<i32> {
        let items = layout(&self.view);
        events
            .into_iter()
            .filter_map(|(id, event_id, data, timestamp)| {
                let known = MenuItem::find(&items, id).is_some();
                self.event(id, &event_id, data, timestamp);
                (!known).then_some(id)
            })
            .collect()
    }

    /// The menu is always up to date.
    fn about_to_show(&self, id: i32) -> bool {
        let _ = id;
        false
    }

    /// No entry needs updating; returns, second, the ids of entries the
    /// menu doesn't have.
    fn about_to_show_group(&self, ids: Vec<i32>) -> (Vec<i32>, Vec<i32>) {
        let items = layout(&self.view);
        let unknown = ids
            .into_iter()
            .filter(|id| MenuItem::find(&items, *id).is_none())
            .collect();
        (Vec::new(), unknown)
    }

    #[zbus(property)]
    fn version(&self) -> u32 {
        3
    }

    #[zbus(property)]
    fn text_direction(&self) -> &'static str {
        "ltr"
    }

    #[zbus(property)]
    fn status(&self) -> &'static str {
        "normal"
    }

    #[zbus(property)]
    fn icon_theme_path(&self) -> Vec<String> {
        Vec::new()
    }

    #[zbus(signal)]
    async fn layout_updated(
        emitter: &SignalEmitter<'_>,
        revision: u32,
        parent: i32,
    ) -> zbus::Result<()>;

    #[zbus(signal)]
    async fn items_properties_updated(
        emitter: &SignalEmitter<'_>,
        updated_props: Vec<(i32, HashMap<String, OwnedVariant>)>,
        removed_props: Vec<(i32, Vec<String>)>,
    ) -> zbus::Result<()>;

    #[zbus(signal)]
    async fn item_activation_requested(
        emitter: &SignalEmitter<'_>,
        id: i32,
        timestamp: u32,
    ) -> zbus::Result<()>;
}

/// A string property value.
fn text(value: &str) -> OwnedVariant {
    OwnedVariant::from(zbus::Str::from(value.to_owned()))
}

fn failed(error: &zbus::Error) -> zbus::fdo::Error {
    zbus::fdo::Error::Failed(error.to_string())
}
