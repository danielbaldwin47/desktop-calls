//! The scenario's event timeline, printed whenever a wait times out, so a
//! failure says where the time went without another run.
//!
//! It merges two sources, in milliseconds since the scenario started:
//!
//! - Everything on the scenario's two private buses, seen by a monitor
//!   connection (`BecomeMonitor`) on each from the moment the bus starts:
//!   what the Simulated Phone signals and is asked, what the app publishes
//!   (its state and its `Surfaces`, which list a surface once the compositor
//!   has mapped it) and what the stand-ins receive. The harness's polling
//!   (`Get`, `GetAll`, `RecordedMethodCalls`) and the bus daemon's own
//!   chatter are left out.
//! - The app's own log, including `main-loop-stalled`, which the app logs
//!   when its main loop was held up for over 100 ms.
//!
//! The whole timeline is written to `timeline.txt` in the scenario's
//! directory; the panic shows its last [`SHOWN`] lines.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use futures_util::StreamExt as _;
use zbus::message::Type;
use zbus::{Connection, Message, MessageStream};

use crate::{APP_BUS_NAME, SIMULATED_PHONE_BUS_NAME};

/// How many of the timeline's last lines a panic shows.
const SHOWN: usize = 80;

/// How much of a message's arguments an entry shows.
const ARGUMENTS_SHOWN: usize = 240;

/// Method calls that only read, which the harness makes in a loop while it
/// waits: left out.
const POLLING: &[&str] = &[
    "Get",
    "GetAll",
    "Introspect",
    "Ping",
    "GetMachineId",
    "RecordedMethodCalls",
    "MostPbapSessionsAtOnce",
];

const BUS_DAEMON: &str = "org.freedesktop.DBus";

/// What the timeline calls the owners of these names. In a scenario the
/// Simulated Phone serves PipeWire's Telephony, BlueZ and obexd, each on a
/// connection of its own.
const WELL_KNOWN: &[(&str, &str)] = &[
    (APP_BUS_NAME, "app"),
    (SIMULATED_PHONE_BUS_NAME, "Simulated Phone"),
    ("org.pipewire.Telephony", "Simulated Phone (Telephony)"),
    ("org.bluez", "Simulated Phone (BlueZ)"),
    ("org.bluez.obex", "Simulated Phone (obex)"),
];

/// One scenario's timeline. See the module's documentation.
pub(crate) struct Timeline {
    started: Instant,
    started_at: SystemTime,
    directory: PathBuf,
    entries: Mutex<Vec<Entry>>,
    /// Names given to connections the monitors can't name from the bus,
    /// such as the harness's own.
    labels: Mutex<BTreeMap<String, String>>,
}

struct Entry {
    at: Duration,
    source: &'static str,
    text: String,
}

impl Timeline {
    /// Starts the clock: the scenario starts now.
    pub fn start(directory: &Path) -> Arc<Self> {
        Arc::new(Self {
            started: Instant::now(),
            started_at: SystemTime::now(),
            directory: directory.to_owned(),
            entries: Mutex::default(),
            labels: Mutex::default(),
        })
    }

    /// Notes something the harness saw happen, now.
    pub fn record(&self, source: &'static str, text: String) {
        let at = self.started.elapsed();
        self.entries
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(Entry { at, source, text });
    }

    /// Calls the connection with unique name `unique` `label` in entries.
    pub fn label(&self, unique: &str, label: &str) {
        self.labels
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(unique.to_owned(), label.to_owned());
    }

    /// Records everything on the bus at `address` from now on, as `bus`
    /// (`"system"` or `"session"`), until the bus stops.
    ///
    /// # Errors
    ///
    /// If the bus refuses a monitor.
    pub fn monitor(self: &Arc<Self>, address: &str, bus: &'static str) -> zbus::Result<()> {
        let connection = zbus::block_on(zbus::connection::Builder::address(address).build())?;
        // Made before becoming a monitor, so it misses nothing after.
        let mut stream = MessageStream::from(&connection);
        zbus::block_on(connection.call_method(
            Some(BUS_DAEMON),
            "/org/freedesktop/DBus",
            Some("org.freedesktop.DBus.Monitoring"),
            "BecomeMonitor",
            &(Vec::<&str>::new(), 0_u32),
        ))?;
        let timeline = Arc::clone(self);
        thread::spawn(move || {
            let _connection = connection;
            let mut names = Names::default();
            zbus::block_on(async {
                while let Some(Ok(message)) = stream.next().await {
                    let labels = timeline
                        .labels
                        .lock()
                        .unwrap_or_else(PoisonError::into_inner)
                        .clone();
                    if let Some(text) = names.describe(&message, &labels) {
                        timeline.record(bus, text);
                    }
                }
            });
        });
        Ok(())
    }

    /// The timeline so far, written in full to `timeline.txt` in the
    /// scenario's directory; returns its last [`SHOWN`] lines, introduced.
    pub fn render(&self) -> String {
        let mut entries: Vec<(Duration, &str, String)> = self
            .entries
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .map(|entry| (entry.at, entry.source, entry.text.clone()))
            .collect();
        entries.extend(self.app_log());
        entries.sort_by_key(|(at, _, _)| *at);
        let lines: Vec<String> = entries
            .iter()
            .map(|(at, source, text)| format!("{:>7} {source:<8} {text}", at.as_millis()))
            .collect();
        let file = self.directory.join("timeline.txt");
        let _ = fs::write(&file, lines.join("\n") + "\n");
        let skipped = lines.len().saturating_sub(SHOWN);
        let mut shown = format!(
            "\nThe scenario's timeline, in ms since it started{}:\n",
            if skipped > 0 {
                format!(
                    " (the last {SHOWN} of {} lines; all in {})",
                    lines.len(),
                    file.display()
                )
            } else {
                format!(" (also in {})", file.display())
            }
        );
        for line in &lines[skipped..] {
            let _ = writeln!(shown, "{line}");
        }
        shown
    }

    /// Every event in the app's log file, as entries.
    fn app_log(&self) -> Vec<(Duration, &'static str, String)> {
        let log = self.directory.join("state/desktop-calls/desktop-calls.log");
        let started_us = self
            .started_at
            .duration_since(UNIX_EPOCH)
            .map_or(0, |since| since.as_micros());
        crate::app_log::read(&log)
            .lines()
            .filter_map(|line| {
                let serde_json::Value::Object(mut event) = serde_json::from_str(line).ok()? else {
                    return None;
                };
                let at_us = u128::from(event.remove("timestamp_us")?.as_u64()?);
                let at = Duration::from_micros(
                    u64::try_from(at_us.saturating_sub(started_us)).unwrap_or(u64::MAX),
                );
                let kind = event.remove("event_kind");
                let message = event.remove("message");
                for noise in ["level", "target"] {
                    event.remove(noise);
                }
                let text = |value: Option<serde_json::Value>| match value {
                    Some(serde_json::Value::String(text)) => text,
                    Some(other) => other.to_string(),
                    None => String::new(),
                };
                let mut entry = format!("{}: {}", text(kind), text(message));
                for (name, value) in event {
                    let _ = write!(entry, " {name}={}", text(Some(value)));
                }
                Some((at, "app log", entry))
            })
            .collect()
    }
}

/// Which well-known names each connection owns, as the bus announced them.
#[derive(Default)]
struct Names {
    owned: BTreeMap<String, BTreeSet<String>>,
}

impl Names {
    /// A line for `message`, or `None` to leave it out.
    fn describe(&mut self, message: &Message, labels: &BTreeMap<String, String>) -> Option<String> {
        let header = message.header();
        let sender = header.sender().map(ToString::to_string).unwrap_or_default();
        let destination = header
            .destination()
            .map(ToString::to_string)
            .unwrap_or_default();
        let member = header.member().map(ToString::to_string).unwrap_or_default();
        let interface = header
            .interface()
            .map(ToString::to_string)
            .unwrap_or_default();
        let path = header.path().map(ToString::to_string).unwrap_or_default();
        if sender == BUS_DAEMON && member == "NameOwnerChanged" {
            let (name, _old, new) = message
                .body()
                .deserialize::<(String, String, String)>()
                .ok()?;
            if name.starts_with(':') {
                return None;
            }
            if new.is_empty() {
                for owned in self.owned.values_mut() {
                    owned.remove(&name);
                }
                return Some(format!("{name} released"));
            }
            self.owned
                .entry(new.clone())
                .or_default()
                .insert(name.clone());
            return Some(format!("{} owns {name}", self.label(&new, labels)));
        }
        if sender == BUS_DAEMON || destination == BUS_DAEMON || POLLING.contains(&member.as_str()) {
            return None;
        }
        let arguments = arguments(message);
        let from = self.label(&sender, labels);
        match message.message_type() {
            Type::Signal => {
                let short = interface.rsplit('.').next().unwrap_or_default();
                Some(format!(
                    "{from} signals {short}.{member} on {path} {arguments}"
                ))
            }
            Type::MethodCall => {
                let to = self.label(&destination, labels);
                let short = interface.rsplit('.').next().unwrap_or_default();
                Some(format!(
                    "{from} calls {to} {short}.{member} on {path} {arguments}"
                ))
            }
            Type::Error => {
                let name = header
                    .error_name()
                    .map(ToString::to_string)
                    .unwrap_or_default();
                let to = self.label(&destination, labels);
                Some(format!("{from} answers {to} with {name} {arguments}"))
            }
            Type::MethodReturn => None,
        }
    }

    /// A readable name for `name`: a connection's unique name, or a
    /// well-known name as a method call's destination.
    fn label(&self, name: &str, labels: &BTreeMap<String, String>) -> String {
        if let Some(label) = labels.get(name) {
            return label.clone();
        }
        let owned = self.owned.get(name);
        let well_known = owned.into_iter().flatten().map(String::as_str);
        well_known
            .chain([name])
            .find_map(|known| WELL_KNOWN.iter().find(|(name, _)| *name == known))
            .map_or_else(
                || {
                    owned
                        .and_then(|owned| owned.first())
                        .map_or(name, String::as_str)
                        .to_owned()
                },
                |(_, label)| (*label).to_owned(),
            )
    }
}

/// A message's arguments in GVariant text form, cut short.
fn arguments(message: &Message) -> String {
    let body = message.body();
    if body.signature().to_string().is_empty() {
        return String::new();
    }
    let Ok(structure) = body.deserialize::<zbus::Structure<'_>>() else {
        return format!("({})", body.signature());
    };
    let mut text = structure.to_string();
    if text.chars().count() > ARGUMENTS_SHOWN {
        text = text.chars().take(ARGUMENTS_SHOWN).collect::<String>() + "…";
    }
    text
}

/// A connection's unique name, for [`Timeline::label`].
pub(crate) fn unique_name(connection: &Connection) -> String {
    connection
        .unique_name()
        .map(ToString::to_string)
        .unwrap_or_default()
}
