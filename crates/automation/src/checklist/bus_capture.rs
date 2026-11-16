//! The Bluetooth D-Bus conversation around each step, kept beside the
//! result file for the agent who reads a failed step (retro candidate 6,
//! in the runner only: nothing is added to the app).
//!
//! On the system bus it hears BlueZ's signals and reads BlueZ's objects as
//! each step starts and ends. Without root the system bus refuses a
//! monitor, so the app's calls to BlueZ, and BlueZ's replies and errors,
//! are not in it. On the session bus, where a monitor needs no root, it
//! hears every message to and from obexd, PipeWire's Telephony service and
//! the app's control interface.
//!
//! Bluetooth addresses become `dev-` and a short hash, the same within a
//! file; names, aliases and caller lines become `…`; runs of seven digits
//! or more become `<number>`. On the session bus, where the app's calls,
//! contacts and windows go by, every string in a body becomes `…` but
//! keys, object paths, D-Bus names and the values in [`STATE_KEYS`]. Lines are timed in microseconds since the
//! Unix epoch, as the record's `started_us` and `ended_us` are.

use std::collections::HashMap;
use std::fmt::Write as _;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};

use futures_util::StreamExt;
use zbus::message::Type;
use zbus::{Connection, MatchRule, Message, MessageStream};

use super::results::unix_micros;

/// The session-bus services whose messages are kept, both ways.
const SESSION_SERVICES: [&str; 3] = [
    "org.bluez.obex",
    "org.pipewire.Telephony",
    super::watch::BUS_NAME,
];

/// Properties whose values may name a person or carry a number.
const PRIVATE_PROPERTIES: [&str; 5] = [
    "Name",
    "Alias",
    "LineIdentification",
    "IncomingLine",
    "Number",
];

/// Keys whose values, on the session bus, say a state and are kept.
const STATE_KEYS: [&str; 4] = ["ConnectionState", "ContactsState", "State", "Status"];

type Lines = Arc<Mutex<Vec<(u64, String)>>>;

/// A capture running for the whole sitting, written out a step at a time.
pub struct BusCapture {
    directory: PathBuf,
    lines: Lines,
    system: Option<Connection>,
    /// What can't be listened to, at the top of every file.
    unheard: Vec<String>,
}

impl BusCapture {
    /// Starts listening on `system` and `session`, each a connection of its
    /// own (the session one becomes a monitor), for files in `directory`.
    /// A bus that can't be listened to is said on the terminal now and at
    /// the top of each file.
    pub fn start(
        system: zbus::Result<Connection>,
        session: zbus::Result<Connection>,
        directory: PathBuf,
    ) -> Self {
        let lines = Lines::default();
        let mut unheard = Vec::new();
        let system = match system {
            Ok(system) => match zbus::block_on(listen_to_bluez(&system)) {
                Ok(stream) => {
                    spawn(stream, "system", Arc::clone(&lines), |_| true);
                    Some(system)
                }
                Err(error) => {
                    unheard.push(format!("system: BlueZ's signals can't be heard: {error}"));
                    None
                }
            },
            Err(error) => {
                unheard.push(format!("system: no connection: {error}"));
                None
            }
        };
        match session.and_then(|session| zbus::block_on(monitor(session))) {
            Ok((stream, mut owners)) => {
                spawn(stream, "session", Arc::clone(&lines), move |message| {
                    owners.keeps(message)
                });
            }
            Err(error) => unheard.push(format!("session: can't monitor: {error}")),
        }
        for line in &unheard {
            println!("Bluetooth D-Bus capture, not all of it: {line}");
        }
        Self {
            directory,
            lines,
            system,
            unheard,
        }
    }

    /// Notes what BlueZ's objects say now, with `when` ("start", "end").
    pub fn snapshot(&self, when: &str) {
        let Some(system) = &self.system else { return };
        match zbus::block_on(bluez_objects(system)) {
            Ok(objects) => note(
                &self.lines,
                format!("system: BlueZ at the step's {when}:\n{objects}"),
            ),
            Err(error) => note(
                &self.lines,
                format!("system: BlueZ unread at the step's {when}: {error}"),
            ),
        }
    }

    /// Writes what was captured from `from_us` to `to_us` for step `id`,
    /// redacted, and forgets it. Returns the file's name in the directory.
    pub fn write(&self, id: &str, from_us: u64, to_us: u64) -> Option<String> {
        let mut lines = self.lines.lock().unwrap_or_else(PoisonError::into_inner);
        let mut text = String::new();
        for line in &self.unheard {
            let _ = writeln!(text, "Not captured: {line}");
        }
        let mut addresses = HashMap::new();
        for (at_us, line) in lines
            .iter()
            .filter(|(at, _)| (from_us..=to_us).contains(at))
        {
            let _ = writeln!(text, "{at_us} {}", redact(line, &mut addresses));
        }
        lines.retain(|(at_us, _)| *at_us > to_us);
        drop(lines);
        let name = format!("{id}-{from_us}.txt");
        std::fs::create_dir_all(&self.directory).ok()?;
        std::fs::write(self.directory.join(&name), text).ok()?;
        Some(name)
    }
}

/// The directory beside `results` (a result file) that its captures go in.
pub fn directory_for(results: &Path) -> PathBuf {
    results.with_extension("bus")
}

fn note(lines: &Lines, line: String) {
    lines
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .push((unix_micros(), line));
}

/// BlueZ's signals on `system`, by a match rule: no monitor, no root.
async fn listen_to_bluez(system: &Connection) -> zbus::Result<MessageStream> {
    let rule = MatchRule::builder()
        .msg_type(Type::Signal)
        .sender("org.bluez")
        .build()?;
    MessageStream::for_match_rule(rule, system, None).await
}

/// Every message on the session bus, as a monitor, with who owns each of
/// [`SESSION_SERVICES`] now. A match rule can't name a well-known name as
/// a destination, so the messages kept are chosen by [`Owners::keeps`].
async fn monitor(session: Connection) -> zbus::Result<(MessageStream, Owners)> {
    let bus = zbus::fdo::DBusProxy::new(&session).await?;
    let mut owners = Owners::default();
    for service in SESSION_SERVICES {
        let name = zbus::names::BusName::try_from(service)?;
        if let Ok(owner) = bus.get_name_owner(name).await {
            owners.0.insert(owner.to_string(), service);
        }
    }
    let stream = MessageStream::from(&session);
    zbus::fdo::MonitoringProxy::new(&session)
        .await?
        .become_monitor(&[], 0)
        .await?;
    Ok((stream, owners))
}

/// The unique names that own [`SESSION_SERVICES`], as owners change.
#[derive(Default)]
struct Owners(HashMap<String, &'static str>);

impl Owners {
    /// Whether `message` is to or from one of [`SESSION_SERVICES`], or says
    /// one changed owner; following that change.
    fn keeps(&mut self, message: &Message) -> bool {
        let header = message.header();
        if header
            .member()
            .is_some_and(|member| member == "NameOwnerChanged")
            && header
                .sender()
                .is_some_and(|sender| sender == "org.freedesktop.DBus")
        {
            let Ok((name, old, new)) = message.body().deserialize::<(String, String, String)>()
            else {
                return false;
            };
            let Some(service) = SESSION_SERVICES
                .into_iter()
                .find(|service| *service == name)
            else {
                return false;
            };
            self.0.remove(&old);
            if !new.is_empty() {
                self.0.insert(new, service);
            }
            return true;
        }
        let ours = |name: Option<String>| {
            name.is_some_and(|name| {
                SESSION_SERVICES.contains(&name.as_str()) || self.0.contains_key(&name)
            })
        };
        ours(header.sender().map(ToString::to_string))
            || ours(header.destination().map(ToString::to_string))
    }
}

fn spawn(
    mut stream: MessageStream,
    bus: &'static str,
    lines: Lines,
    mut keeps: impl FnMut(&Message) -> bool + Send + 'static,
) {
    std::thread::spawn(move || {
        zbus::block_on(async move {
            while let Some(Ok(message)) = stream.next().await {
                if keeps(&message) {
                    note(&lines, describe(bus, &message));
                }
            }
        });
    });
}

/// One message as a line: bus, kind, sender, destination, path, member,
/// error and body.
fn describe(bus: &str, message: &Message) -> String {
    let header = message.header();
    let kind = match message.message_type() {
        Type::MethodCall => "call",
        Type::MethodReturn => "reply",
        Type::Error => "error",
        Type::Signal => "signal",
    };
    let text = |part: Option<String>| part.unwrap_or_else(|| "-".to_owned());
    let body = message
        .body()
        .deserialize::<zbus::Structure>()
        .map(|body| body.to_string())
        .unwrap_or_default();
    format!(
        "{bus}: {kind} {} -> {} {} {}.{}{} {body}",
        text(header.sender().map(ToString::to_string)),
        text(header.destination().map(ToString::to_string)),
        text(header.path().map(ToString::to_string)),
        text(header.interface().map(ToString::to_string)),
        text(header.member().map(ToString::to_string)),
        header
            .error_name()
            .map(|name| format!(" {name}"))
            .unwrap_or_default(),
    )
}

/// BlueZ's adapters and devices, a line each, with their properties.
async fn bluez_objects(system: &Connection) -> zbus::Result<String> {
    let objects = zbus::fdo::ObjectManagerProxy::builder(system)
        .destination("org.bluez")
        .path("/")
        .build()
        .await?
        .get_managed_objects()
        .await?;
    let mut paths: Vec<_> = objects.iter().collect();
    paths.sort_by(|(one, _), (other, _)| one.as_str().cmp(other.as_str()));
    let mut text = String::new();
    for (path, interfaces) in paths {
        for (interface, properties) in interfaces {
            if !matches!(
                interface.as_str(),
                "org.bluez.Adapter1" | "org.bluez.Device1"
            ) {
                continue;
            }
            let mut names: Vec<_> = properties.iter().collect();
            names.sort_by_key(|(name, _)| *name);
            let listed: Vec<String> = names
                .iter()
                .map(|(name, value)| format!("{name}: {}", ***value))
                .collect();
            let _ = writeln!(text, "  {path} {interface} {{{}}}", listed.join(", "));
        }
    }
    Ok(text)
}

/// `line` without Bluetooth addresses, names or numbers. `addresses` keeps
/// each address's stand-in, so one Phone reads the same all through.
fn redact(line: &str, addresses: &mut HashMap<String, String>) -> String {
    let line = hide_addresses(line, addresses);
    let line = if line.starts_with("session: ") {
        hide_session_strings(&line)
    } else {
        hide_private_values(&line)
    };
    hide_numbers(&line)
}

/// Each string in `line` as `…`, but a key, an object path, a D-Bus name,
/// and any string inside the value of one of [`STATE_KEYS`].
fn hide_session_strings(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut characters = line.chars().peekable();
    let mut depth = 0_usize;
    let mut state_key = false;
    let mut state_depth = None;
    while let Some(character) = characters.next() {
        match character {
            '<' => {
                depth += 1;
                if std::mem::take(&mut state_key) {
                    state_depth = Some(depth);
                }
            }
            '>' => {
                if state_depth == Some(depth) {
                    state_depth = None;
                }
                depth = depth.saturating_sub(1);
            }
            '"' => {
                let mut text = String::new();
                while let Some(inner) = characters.next() {
                    match inner {
                        '"' => break,
                        '\\' => text.extend(characters.next()),
                        _ => text.push(inner),
                    }
                }
                let key = characters.peek() == Some(&':');
                state_key = key && STATE_KEYS.contains(&text.as_str());
                let kept =
                    key || state_depth.is_some() || out.ends_with("objectpath ") || is_name(&text);
                out.push('"');
                out.push_str(if kept { &text } else { "…" });
                out.push('"');
                continue;
            }
            _ => {}
        }
        out.push(character);
    }
    out
}

/// Whether `text` is empty, a unique name (`:1.10`) or a dotted D-Bus name
/// (`org.bluez.obex.Client1`): no spaces, no person.
fn is_name(text: &str) -> bool {
    text.is_empty()
        || text.strip_prefix(':').is_some_and(|rest| {
            rest.chars()
                .all(|character| character.is_ascii_digit() || character == '.')
        })
        || (text.contains('.')
            && text.starts_with(|character: char| character.is_ascii_lowercase())
            && text
                .chars()
                .all(|character| character.is_ascii_alphanumeric() || "._-".contains(character)))
}

/// Each `AA:BB:CC:DD:EE:FF` or `AA_BB_CC_DD_EE_FF` as `dev-` and a hash.
fn hide_addresses(line: &str, addresses: &mut HashMap<String, String>) -> String {
    let bytes = line.as_bytes();
    let mut out = String::with_capacity(line.len());
    let mut at = 0;
    while at < bytes.len() {
        if let Some(address) = address_at(&line[at..]) {
            let key = address.to_ascii_uppercase().replace('_', ":");
            let stand_in = addresses.entry(key.clone()).or_insert_with(|| {
                let mut hasher = DefaultHasher::new();
                key.hash(&mut hasher);
                format!("dev-{:04x}", hasher.finish() & 0xffff)
            });
            out.push_str(stand_in);
            at += address.len();
        } else {
            let next = line[at..].chars().next().map_or(1, char::len_utf8);
            out.push_str(&line[at..at + next]);
            at += next;
        }
    }
    out
}

/// The Bluetooth address `text` starts with, if any.
fn address_at(text: &str) -> Option<&str> {
    let candidate = text.get(..17)?;
    let bytes = candidate.as_bytes();
    let separator = bytes[2];
    if separator != b':' && separator != b'_' {
        return None;
    }
    let shaped = bytes.iter().enumerate().all(|(index, byte)| {
        if index % 3 == 2 {
            *byte == separator
        } else {
            byte.is_ascii_hexdigit()
        }
    });
    shaped.then_some(candidate)
}

/// The value after each `"Name": <"…">` (and the other private
/// properties) as `…`.
fn hide_private_values(line: &str) -> String {
    let mut out = line.to_owned();
    for name in PRIVATE_PROPERTIES {
        for key in [format!("\"{name}\": <\""), format!("{name}: \"")] {
            let mut from = 0;
            while let Some(found) = out[from..].find(&key) {
                let start = from + found + key.len();
                let Some(length) = out[start..].find('"') else {
                    break;
                };
                out.replace_range(start..start + length, "…");
                from = start + "…".len();
            }
        }
    }
    out
}

/// Each run of seven digits or more as `<number>`.
fn hide_numbers(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut digits = String::new();
    for character in line.chars().chain(std::iter::once('\0')) {
        if character.is_ascii_digit() {
            digits.push(character);
            continue;
        }
        if digits.len() >= 7 {
            out.push_str("<number>");
        } else {
            out.push_str(&digits);
        }
        digits.clear();
        if character != '\0' {
            out.push(character);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn addresses_become_one_stand_in_each() {
        let mut addresses = HashMap::new();
        let line = "/org/bluez/hci0/dev_FC_41_16_00_00_01 Address: \"fc:41:16:00:00:01\"";
        let redacted = hide_addresses(line, &mut addresses);
        assert!(!redacted.contains("41"), "{redacted}");
        assert_eq!(addresses.len(), 1, "{redacted}");
    }

    #[test]
    fn names_and_numbers_are_hidden() {
        let line = r#"{"Alias": <"Ann's Pixel">, "LineIdentification": <"+15551234567">, "Connected": <true>}"#;
        let redacted = redact(line, &mut HashMap::new());
        assert_eq!(
            redacted,
            r#"{"Alias": <"…">, "LineIdentification": <"…">, "Connected": <true>}"#
        );
        assert_eq!(
            hide_numbers("call 5551234 at 12:30"),
            "call <number> at 12:30"
        );
    }

    #[test]
    fn session_strings_are_hidden_but_states_and_names() {
        let line = r#"session: signal :1.10 -> - /p org.freedesktop.DBus.Properties.PropertiesChanged ("io.github.danielbaldwin47.DesktopCalls1", {"ConnectionState": <"connected">, "Calls": <[(uint32 1, "incoming", "ann")]>, "ContactsState": <("shared", uint32 2)>}, @as [], objectpath "/org/bluez/obex/client/session0")"#;
        assert_eq!(
            hide_session_strings(line),
            r#"session: signal :1.10 -> - /p org.freedesktop.DBus.Properties.PropertiesChanged ("io.github.danielbaldwin47.DesktopCalls1", {"ConnectionState": <"connected">, "Calls": <[(uint32 1, "…", "…")]>, "ContactsState": <("shared", uint32 2)>}, @as [], objectpath "/org/bluez/obex/client/session0")"#
        );
    }
}
