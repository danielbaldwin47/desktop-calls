//! Structured logging through `tracing`, with privacy redaction.
//!
//! Events go to exactly one destination, chosen once at startup with no
//! test-only branch:
//!
//! 1. the systemd journal, when its socket is reachable;
//! 2. otherwise `$XDG_STATE_HOME/desktop-calls/desktop-calls.log` (directory
//!    0700, file 0600), one JSON object per line;
//! 3. otherwise standard error, in the same JSON form.
//!
//! The level filter comes from `RUST_LOG` and defaults to `info`.
//!
//! Fields named in [`SENSITIVE_FIELDS`] (phone numbers and names) are written
//! as [`REDACTED`] on events at info, warn and error, and in full only on
//! debug and trace events. The redaction lives in this layer, rather than in
//! each call site, so a sensitive value logged at the wrong level is still
//! safe. That is also why this module writes the journal protocol itself:
//! `tracing-journald` formats fields on its own, so redaction cannot sit in
//! front of it.

use std::fmt;
use std::fs::{DirBuilder, File, OpenOptions};
use std::io::{self, Write as _};
use std::os::unix::fs::{DirBuilderExt as _, OpenOptionsExt as _};
use std::os::unix::net::UnixDatagram;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use tracing::field::{Field, Visit};
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::layer::{Context, Layer, SubscriberExt as _};
use tracing_subscriber::util::SubscriberInitExt as _;
use tracing_subscriber::{EnvFilter, Registry};

/// The journal's native-protocol socket (`systemd.journal-fields(7)`).
const JOURNAL_SOCKET: &str = "/run/systemd/journal/socket";

/// Identifies our entries in the journal: `journalctl --user -t desktop-calls`.
const SYSLOG_IDENTIFIER: &str = "desktop-calls";

/// Field names whose values identify people: redacted at info and above.
pub const SENSITIVE_FIELDS: &[&str] = &["phone_number", "contact_name"];

/// What a sensitive value is replaced with at info and above.
pub const REDACTED: &str = "[redacted]";

/// The kind of thing that happened. Every event carries one as `event_kind`.
#[derive(Clone, Copy, Debug)]
pub enum EventKind {
    ServiceStarted,
    ServiceStopped,
    ServiceFailed,
    ConnectionStateChanged,
    PhonePaired,
    PhoneUnpaired,
    PhoneNearby,
    SetupStarted,
    SetupStopped,
    SetupFailed,
    SetupStepReached,
    SetupPhoneListed,
    KeybindingAdded,
    KeybindingNotAdded,
    PairingFailed,
    HandsFreeRefused,
    HandsFreeUnreachable,
    ProfileAuthorized,
    PhonePresent,
    PhoneAbsent,
    PhoneConnectRequested,
    PhoneConnectFailed,
    DesktopSuspending,
    DesktopResumed,
    AudioGatewayAdded,
    AudioGatewayRemoved,
    ServiceOwnerChanged,
    ServiceUnavailable,
    MalformedSignal,
    WatchFailed,
    PublishFailed,
    CallAdded,
    CallChanged,
    CallEnded,
    CallVanished,
    CallReturned,
    CallerChanged,
    TransportChanged,
    HandoffRequested,
    AudioMoved,
    AnswerRequested,
    ActivateRequested,
    SurfaceMapped,
    SurfaceShown,
    SurfaceUnmapped,
    DialerQueryTyped,
    DialerQueryShown,
    PhoneCommandSent,
    PhoneCommandFailed,
    PhoneCommandFallback,
    RingingStarted,
    RingingStopped,
    RingingFailed,
    ErrorReported,
    SurfacesUnavailable,
    ToastColumnUnknown,
    ContactsLoaded,
    /// No Contacts are cached for the Phone attached.
    ContactsNotCached,
    /// A sync read part of the Call History, before or after Contacts.
    CallHistoryRead,
    ContactsSyncStarted,
    ContactsPermissionNeeded,
    ContactsPulled,
    ContactsSynced,
    ContactsSyncFailed,
    ContactsCacheFailed,
    CallHistoryRefreshed,
    CallHistoryRefreshFailed,
    MissedCall,
    MissedCallAction,
    NotificationFailed,
    ThemeChanged,
    ThemeShown,
    ThemeUnreadable,
    ThemeUnwatched,
    DoNotDisturbChanged,
    DoNotDisturbUnreadable,
    DoNotDisturbUnwatched,
    RoundingUnknown,
    IndicatorRegistered,
    IndicatorUnregistered,
    IndicatorIconsUnwritten,
    EarlierRunLoaded,
    EarlierRunForgotten,
    EarlierRunFailed,
    ChosenPhoneLoaded,
    ChosenPhoneKept,
    ChosenPhoneFailed,
    MainLoopStalled,
    FirstFramesPrepared,
    FirstFramesUnprepared,
    FirstFramesAbandoned,
    DesktopChecked,
    DesktopFixed,
    DesktopFixFailed,
}

impl EventKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::ServiceStarted => "service-started",
            Self::ServiceStopped => "service-stopped",
            Self::ServiceFailed => "service-failed",
            Self::ConnectionStateChanged => "connection-state-changed",
            Self::PhonePaired => "phone-paired",
            Self::PhoneUnpaired => "phone-unpaired",
            Self::PhoneNearby => "phone-nearby",
            Self::SetupStarted => "setup-started",
            Self::SetupStopped => "setup-stopped",
            Self::SetupFailed => "setup-failed",
            Self::SetupStepReached => "setup-step-reached",
            Self::SetupPhoneListed => "setup-phone-listed",
            Self::KeybindingAdded => "keybinding-added",
            Self::KeybindingNotAdded => "keybinding-not-added",
            Self::PairingFailed => "pairing-failed",
            Self::HandsFreeRefused => "hands-free-refused",
            Self::HandsFreeUnreachable => "hands-free-unreachable",
            Self::ProfileAuthorized => "profile-authorized",
            Self::PhonePresent => "phone-present",
            Self::PhoneAbsent => "phone-absent",
            Self::PhoneConnectRequested => "phone-connect-requested",
            Self::PhoneConnectFailed => "phone-connect-failed",
            Self::DesktopSuspending => "desktop-suspending",
            Self::DesktopResumed => "desktop-resumed",
            Self::AudioGatewayAdded => "audio-gateway-added",
            Self::AudioGatewayRemoved => "audio-gateway-removed",
            Self::ServiceOwnerChanged => "service-owner-changed",
            Self::ServiceUnavailable => "service-unavailable",
            Self::MalformedSignal => "malformed-signal",
            Self::WatchFailed => "watch-failed",
            Self::PublishFailed => "publish-failed",
            Self::CallAdded => "call-added",
            Self::CallChanged => "call-changed",
            Self::CallEnded => "call-ended",
            Self::CallVanished => "call-vanished",
            Self::CallReturned => "call-returned",
            Self::CallerChanged => "caller-changed",
            Self::TransportChanged => "transport-changed",
            Self::HandoffRequested => "handoff-requested",
            Self::AudioMoved => "audio-moved",
            Self::AnswerRequested => "answer-requested",
            Self::ActivateRequested => "activate-requested",
            Self::SurfaceMapped => "surface-mapped",
            Self::SurfaceShown => "surface-shown",
            Self::SurfaceUnmapped => "surface-unmapped",
            Self::DialerQueryTyped => "dialer-query-typed",
            Self::DialerQueryShown => "dialer-query-shown",
            Self::PhoneCommandSent => "phone-command-sent",
            Self::PhoneCommandFailed => "phone-command-failed",
            Self::PhoneCommandFallback => "phone-command-fallback",
            Self::RingingStarted => "ringing-started",
            Self::RingingStopped => "ringing-stopped",
            Self::RingingFailed => "ringing-failed",
            Self::ErrorReported => "error-reported",
            Self::SurfacesUnavailable => "surfaces-unavailable",
            Self::ToastColumnUnknown => "toast-column-unknown",
            Self::ContactsLoaded => "contacts-loaded",
            Self::ContactsNotCached => "contacts-not-cached",
            Self::CallHistoryRead => "call-history-read",
            Self::ContactsSyncStarted => "contacts-sync-started",
            Self::ContactsPermissionNeeded => "contacts-permission-needed",
            Self::ContactsPulled => "contacts-pulled",
            Self::ContactsSynced => "contacts-synced",
            Self::ContactsSyncFailed => "contacts-sync-failed",
            Self::ContactsCacheFailed => "contacts-cache-failed",
            Self::CallHistoryRefreshed => "call-history-refreshed",
            Self::CallHistoryRefreshFailed => "call-history-refresh-failed",
            Self::MissedCall => "missed-call",
            Self::MissedCallAction => "missed-call-action",
            Self::NotificationFailed => "notification-failed",
            Self::ThemeChanged => "theme-changed",
            Self::ThemeShown => "theme-shown",
            Self::ThemeUnreadable => "theme-unreadable",
            Self::ThemeUnwatched => "theme-unwatched",
            Self::DoNotDisturbChanged => "do-not-disturb-changed",
            Self::DoNotDisturbUnreadable => "do-not-disturb-unreadable",
            Self::DoNotDisturbUnwatched => "do-not-disturb-unwatched",
            Self::RoundingUnknown => "rounding-unknown",
            Self::IndicatorRegistered => "indicator-registered",
            Self::IndicatorUnregistered => "indicator-unregistered",
            Self::IndicatorIconsUnwritten => "indicator-icons-unwritten",
            Self::EarlierRunLoaded => "earlier-run-loaded",
            Self::EarlierRunForgotten => "earlier-run-forgotten",
            Self::EarlierRunFailed => "earlier-run-failed",
            Self::ChosenPhoneLoaded => "chosen-phone-loaded",
            Self::ChosenPhoneKept => "chosen-phone-kept",
            Self::ChosenPhoneFailed => "chosen-phone-failed",
            Self::MainLoopStalled => "main-loop-stalled",
            Self::FirstFramesPrepared => "first-frames-prepared",
            Self::FirstFramesUnprepared => "first-frames-unprepared",
            Self::FirstFramesAbandoned => "first-frames-abandoned",
            Self::DesktopChecked => "desktop-checked",
            Self::DesktopFixed => "desktop-fixed",
            Self::DesktopFixFailed => "desktop-fix-failed",
        }
    }
}

impl fmt::Display for EventKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Where log events are written.
pub enum Destination {
    Journal,
    File(PathBuf),
    Stderr,
}

impl fmt::Display for Destination {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Journal => f.write_str("journal"),
            Self::File(path) => write!(f, "{}", path.display()),
            Self::Stderr => f.write_str("stderr"),
        }
    }
}

/// Installs the global subscriber and returns where events will go.
///
/// Never fails: if no destination can be opened, events go to standard error.
/// If a subscriber is already installed, it is kept.
pub fn init() -> Destination {
    let (sink, destination) = choose_sink();
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    let installed = Registry::default()
        .with(filter)
        .with(PrivacyLayer { sink })
        .try_init();
    if let Err(error) = installed {
        eprintln!("desktop-calls: logging was already set up: {error}");
    }
    destination
}

fn choose_sink() -> (Sink, Destination) {
    if let Some(socket) = journal_socket() {
        return (Sink::Journal(socket), Destination::Journal);
    }
    if let Some(path) = log_file_path()
        && let Ok(file) = open_log_file(&path)
    {
        return (Sink::File(file), Destination::File(path));
    }
    (Sink::Stderr, Destination::Stderr)
}

/// An unbound socket for sending to the journal, if the journal is listening.
fn journal_socket() -> Option<UnixDatagram> {
    let probe = UnixDatagram::unbound().ok()?;
    probe.connect(JOURNAL_SOCKET).ok()?;
    // Events are sent with `send_to`, so a journald restart doesn't strand us
    // on a dead connection.
    UnixDatagram::unbound().ok()
}

/// `$XDG_STATE_HOME/desktop-calls`, with the XDG default of `~/.local/state`
/// when the variable is unset, empty or relative: where the app keeps its
/// log and what it must remember between runs.
pub fn state_directory() -> Option<PathBuf> {
    let state_home = std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .or_else(|| {
            std::env::var_os("HOME")
                .map(PathBuf::from)
                .filter(|home| home.is_absolute())
                .map(|home| home.join(".local/state"))
        })?;
    Some(state_home.join("desktop-calls"))
}

fn log_file_path() -> Option<PathBuf> {
    Some(state_directory()?.join("desktop-calls.log"))
}

/// Opens the log file for appending. Only the user may read it, because debug
/// events carry phone numbers and names in full.
fn open_log_file(path: &Path) -> io::Result<File> {
    if let Some(directory) = path.parent() {
        DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(directory)?;
    }
    OpenOptions::new()
        .append(true)
        .create(true)
        .mode(0o600)
        .open(path)
}

enum Sink {
    Journal(UnixDatagram),
    File(File),
    Stderr,
}

impl Sink {
    fn write(&self, record: &Record) {
        let written = match self {
            Self::Journal(socket) => socket
                .send_to(&record.to_journal(), JOURNAL_SOCKET)
                .map(drop),
            Self::File(file) => (&*file).write_all(&record.to_json_line()),
            Self::Stderr => io::stderr().write_all(&record.to_json_line()),
        };
        // Logging must never take the app down. If the destination fails, keep
        // the event on standard error, which systemd also collects.
        if written.is_err() && !matches!(self, Self::Stderr) {
            let _ = io::stderr().write_all(&record.to_json_line());
        }
    }
}

/// Writes every event to the chosen sink, redacting sensitive fields by level.
struct PrivacyLayer {
    sink: Sink,
}

impl<S: Subscriber> Layer<S> for PrivacyLayer {
    fn on_event(&self, event: &Event<'_>, _context: Context<'_, S>) {
        self.sink.write(&Record::from_event(event));
    }
}

/// One event, with sensitive fields already redacted for its level.
struct Record {
    level: Level,
    target: &'static str,
    file: Option<&'static str>,
    line: Option<u32>,
    message: String,
    fields: Vec<(&'static str, FieldValue)>,
}

enum FieldValue {
    Text(String),
    Signed(i64),
    Unsigned(u64),
    Float(f64),
    Bool(bool),
}

impl fmt::Display for FieldValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Text(text) => f.write_str(text),
            Self::Signed(n) => write!(f, "{n}"),
            Self::Unsigned(n) => write!(f, "{n}"),
            Self::Float(n) => write!(f, "{n}"),
            Self::Bool(b) => write!(f, "{b}"),
        }
    }
}

impl Record {
    fn from_event(event: &Event<'_>) -> Self {
        let metadata = event.metadata();
        let mut visitor = RecordVisitor {
            redact: matches!(*metadata.level(), Level::ERROR | Level::WARN | Level::INFO),
            message: String::new(),
            fields: Vec::new(),
        };
        event.record(&mut visitor);
        Self {
            level: *metadata.level(),
            target: metadata.target(),
            file: metadata.file(),
            line: metadata.line(),
            message: visitor.message,
            fields: visitor.fields,
        }
    }

    /// The journal's native protocol: `KEY=value\n`, or for values containing
    /// a newline, `KEY\n`, the length as little-endian u64, the value and `\n`.
    fn to_journal(&self) -> Vec<u8> {
        let mut payload = Vec::new();
        put_journal_field(&mut payload, "MESSAGE", &self.message);
        put_journal_field(&mut payload, "PRIORITY", journal_priority(self.level));
        put_journal_field(&mut payload, "SYSLOG_IDENTIFIER", SYSLOG_IDENTIFIER);
        put_journal_field(&mut payload, "TARGET", self.target);
        if let Some(file) = self.file {
            put_journal_field(&mut payload, "CODE_FILE", file);
        }
        if let Some(line) = self.line {
            put_journal_field(&mut payload, "CODE_LINE", &line.to_string());
        }
        for (name, value) in &self.fields {
            put_journal_field(&mut payload, &journal_field_name(name), &value.to_string());
        }
        payload
    }

    fn to_json_line(&self) -> Vec<u8> {
        let mut object = serde_json::Map::new();
        object.insert("timestamp_us".into(), unix_micros().into());
        object.insert("level".into(), self.level.as_str().into());
        object.insert("target".into(), self.target.into());
        object.insert("message".into(), self.message.as_str().into());
        for (name, value) in &self.fields {
            let json = match value {
                FieldValue::Text(text) => text.as_str().into(),
                FieldValue::Signed(n) => (*n).into(),
                FieldValue::Unsigned(n) => (*n).into(),
                FieldValue::Float(n) => (*n).into(),
                FieldValue::Bool(b) => (*b).into(),
            };
            object.insert((*name).into(), json);
        }
        let mut line = serde_json::Value::Object(object).to_string().into_bytes();
        line.push(b'\n');
        line
    }
}

struct RecordVisitor {
    redact: bool,
    message: String,
    fields: Vec<(&'static str, FieldValue)>,
}

impl RecordVisitor {
    fn push(&mut self, field: &Field, value: FieldValue) {
        let name = field.name();
        if name == "message" {
            self.message = value.to_string();
        } else if self.redact && SENSITIVE_FIELDS.contains(&name) {
            self.fields
                .push((name, FieldValue::Text(REDACTED.to_owned())));
        } else {
            self.fields.push((name, value));
        }
    }
}

impl Visit for RecordVisitor {
    fn record_debug(&mut self, field: &Field, value: &dyn fmt::Debug) {
        self.push(field, FieldValue::Text(format!("{value:?}")));
    }

    fn record_str(&mut self, field: &Field, value: &str) {
        self.push(field, FieldValue::Text(value.to_owned()));
    }

    fn record_i64(&mut self, field: &Field, value: i64) {
        self.push(field, FieldValue::Signed(value));
    }

    fn record_u64(&mut self, field: &Field, value: u64) {
        self.push(field, FieldValue::Unsigned(value));
    }

    fn record_f64(&mut self, field: &Field, value: f64) {
        self.push(field, FieldValue::Float(value));
    }

    fn record_bool(&mut self, field: &Field, value: bool) {
        self.push(field, FieldValue::Bool(value));
    }

    fn record_error(&mut self, field: &Field, value: &(dyn std::error::Error + 'static)) {
        self.push(field, FieldValue::Text(value.to_string()));
    }
}

fn put_journal_field(payload: &mut Vec<u8>, name: &str, value: &str) {
    payload.extend_from_slice(name.as_bytes());
    if value.contains('\n') {
        payload.push(b'\n');
        let length = u64::try_from(value.len()).unwrap_or(u64::MAX);
        payload.extend_from_slice(&length.to_le_bytes());
    } else {
        payload.push(b'=');
    }
    payload.extend_from_slice(value.as_bytes());
    payload.push(b'\n');
}

/// Journal field names are upper-case ASCII letters, digits and underscores,
/// and must not start with a digit or underscore (those are trusted fields).
fn journal_field_name(name: &str) -> String {
    let mut journal_name: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_uppercase()
            } else {
                '_'
            }
        })
        .collect();
    if !journal_name.starts_with(|c: char| c.is_ascii_uppercase()) {
        journal_name.insert_str(0, "F_");
    }
    journal_name
}

fn journal_priority(level: Level) -> &'static str {
    match level {
        Level::ERROR => "3",
        Level::WARN => "4",
        Level::INFO => "6",
        Level::DEBUG | Level::TRACE => "7",
    }
}

fn unix_micros() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| {
            u64::try_from(elapsed.as_micros()).unwrap_or(u64::MAX)
        })
}
