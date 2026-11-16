//! The app's log for a stretch of time: the journal on the Desktop, or the
//! app's log file under the Simulated Phone. Only info, warning and error
//! lines are read, because debug lines carry phone numbers in full; and,
//! for a step's Bluetooth D-Bus capture, the app's debug lines of each
//! method call it makes to BlueZ, which carry none, and nothing else.

use std::path::PathBuf;
use std::process::Command;

use serde_json::Value;

/// Where the app logs.
pub enum AppLog {
    /// The systemd user journal, as `journalctl --user -t desktop-calls`.
    Journal,
    /// A file of JSON lines, as the app writes where there is no journal.
    File(PathBuf),
}

/// One line of the app's log.
pub struct LogLine {
    pub at_us: u64,
    pub level: &'static str,
    pub event_kind: String,
    pub message: String,
    /// The Call the line is about, if any.
    pub call_id: Option<u64>,
    /// The surface the line is about, if any, such as "dialer".
    pub surface: Option<String>,
    /// The connection state the line reports, if any, such as "connected".
    pub connection_state: Option<String>,
    /// Where the line says the Call audio is, or was asked to go, if it
    /// says: "desktop" or "phone".
    pub audio_location: Option<String>,
    /// What BlueZ said, on a line about a failed request, such as
    /// "br-connection-busy".
    pub detail: Option<String>,
}

impl LogLine {
    /// The line as it goes in a result file: time of day (UTC), level,
    /// event kind and message.
    pub fn excerpt(&self) -> String {
        let of_day = self.at_us / 1_000 % 86_400_000;
        format!(
            "{:02}:{:02}:{:02}.{:03}Z {} {}: {}",
            of_day / 3_600_000,
            of_day / 60_000 % 60,
            of_day / 1_000 % 60,
            of_day % 1_000,
            self.level,
            self.event_kind,
            self.message
        )
    }
}

impl AppLog {
    /// The app's info, warning and error lines from `from_us` to `to_us`
    /// (microseconds since the Unix epoch), oldest first. A log that can't
    /// be read gives no lines.
    pub fn lines(&self, from_us: u64, to_us: u64) -> Vec<LogLine> {
        self.read(from_us, to_us).unwrap_or_default()
    }

    /// As [`AppLog::lines`], or `None` if the log couldn't be read at all,
    /// for a check that must not read "no lines" as "nothing logged".
    pub fn read(&self, from_us: u64, to_us: u64) -> Option<Vec<LogLine>> {
        let mut lines: Vec<LogLine> = match self {
            Self::Journal => journal(from_us, to_us)?,
            Self::File(path) => std::fs::read_to_string(path)
                .ok()?
                .lines()
                .filter_map(file_line)
                .collect(),
        };
        lines.retain(|line| (from_us..=to_us).contains(&line.at_us));
        Some(lines)
    }
}

/// The app's `event_kind`s of a method call to BlueZ, its reply and its
/// error (`bluez_log.rs` in the app).
const BLUEZ_METHOD_KINDS: [&str; 3] = [
    "bluez-method-called",
    "bluez-method-replied",
    "bluez-method-failed",
];

impl AppLog {
    /// The app's lines of each method call it made to BlueZ, BlueZ's reply
    /// and its error, from `from_us` to `to_us`, oldest first, each with
    /// its time and as it goes in a step's capture:
    /// `app: call org.bluez.Device1.Connect on device`. Their switch is
    /// `RUST_LOG`; a log without them, or that can't be read, gives none.
    pub fn bluez_methods(&self, from_us: u64, to_us: u64) -> Vec<(u64, String)> {
        let records: Vec<(u64, Value)> = match self {
            Self::Journal => {
                let mut command = Command::new("journalctl");
                command
                    .args(["--user", "-t", "desktop-calls", "-o", "json", "--no-pager"])
                    .args(window(from_us, to_us));
                // Matches on one field are alternatives: only these lines
                // leave the journal, never another debug line.
                command.args(BLUEZ_METHOD_KINDS.map(|kind| format!("EVENT_KIND={kind}")));
                let Ok(output) = command.output() else {
                    return Vec::new();
                };
                String::from_utf8_lossy(&output.stdout)
                    .lines()
                    .filter_map(|text| {
                        let value: Value = serde_json::from_str(text).ok()?;
                        let at_us = value["__REALTIME_TIMESTAMP"].as_str()?.parse().ok()?;
                        Some((at_us, lower_case_keys(&value)))
                    })
                    .collect()
            }
            Self::File(path) => std::fs::read_to_string(path)
                .unwrap_or_default()
                .lines()
                .filter_map(|text| {
                    let value: Value = serde_json::from_str(text).ok()?;
                    Some((value["timestamp_us"].as_u64()?, value))
                })
                .collect(),
        };
        records
            .into_iter()
            .filter(|(at_us, _)| (from_us..=to_us).contains(at_us))
            .filter_map(|(at_us, value)| Some((at_us, bluez_method(&value)?)))
            .collect()
    }
}

/// The switch that turns on the app's BlueZ method lines, as `just run`
/// sets it.
pub const BLUEZ_METHODS_SWITCH: &str = "RUST_LOG=info,bluez_dbus=debug";

/// Why the app on `session` doesn't log its BlueZ method calls, or `None`
/// if it does: it does if its `RUST_LOG` turns on debug for them. Read from
/// the environment it was started with, which only its own user can read.
pub fn bluez_methods_unlogged(session: &zbus::Connection) -> Option<String> {
    let environment = zbus::block_on(async {
        let bus = zbus::fdo::DBusProxy::new(session).await.ok()?;
        let name = zbus::names::BusName::try_from(super::watch::BUS_NAME).ok()?;
        let process = bus.get_connection_unix_process_id(name).await.ok()?;
        std::fs::read(format!("/proc/{process}/environ")).ok()
    });
    let Some(environment) = environment else {
        return Some("the app's BlueZ method calls: can't tell how the app was started".to_owned());
    };
    let rust_log = environment
        .split(|byte| *byte == 0)
        .find_map(|variable| variable.strip_prefix(b"RUST_LOG="))
        .map(String::from_utf8_lossy);
    (!logs_bluez_methods(rust_log.as_deref())).then(|| {
        format!(
            "the app's BlueZ method calls: the app wasn't started with {BLUEZ_METHODS_SWITCH}, which `just run` sets"
        )
    })
}

/// Whether a `RUST_LOG` of `filter` turns on the app's BlueZ method lines:
/// debug or trace for everything, or for `bluez_dbus`.
fn logs_bluez_methods(filter: Option<&str>) -> bool {
    filter.is_some_and(|filter| {
        filter.split(',').map(str::trim).any(|directive| {
            matches!(
                directive,
                "debug" | "trace" | "bluez_dbus=debug" | "bluez_dbus=trace" | "bluez_dbus"
            )
        })
    })
}

/// A journal entry's fields under the names the app's log file uses.
fn lower_case_keys(value: &Value) -> Value {
    value
        .as_object()
        .map(|fields| {
            fields
                .iter()
                .map(|(key, value)| (key.to_ascii_lowercase(), value.clone()))
                .collect()
        })
        .unwrap_or_default()
}

/// One of the app's BlueZ method lines as it goes in a capture, if
/// `value` is one.
fn bluez_method(value: &Value) -> Option<String> {
    let field = |name: &str| value[name].as_str().unwrap_or_default();
    let what = match field("event_kind") {
        "bluez-method-called" => "call",
        "bluez-method-replied" => "reply",
        "bluez-method-failed" => "error",
        _ => return None,
    };
    let property = match field("property") {
        "" => String::new(),
        property => format!(" ({property})"),
    };
    let detail = match field("detail") {
        "" => String::new(),
        detail => format!(": {detail}"),
    };
    Some(format!(
        "app: {what} {}.{}{property} {} {}{detail}",
        field("interface"),
        field("method"),
        if what == "call" { "on" } else { "from" },
        field("object"),
    ))
}

/// `journalctl`'s arguments for `from_us` to `to_us`.
fn window(from_us: u64, to_us: u64) -> [String; 2] {
    [
        format!(
            "--since=@{}.{:06}",
            from_us / 1_000_000,
            from_us % 1_000_000
        ),
        format!("--until=@{}.{:06}", to_us / 1_000_000, to_us % 1_000_000),
    ]
}

fn journal(from_us: u64, to_us: u64) -> Option<Vec<LogLine>> {
    let Ok(output) = Command::new("journalctl")
        .args(["--user", "-t", "desktop-calls", "-o", "json", "--no-pager"])
        .arg("--priority=info")
        .args(window(from_us, to_us))
        .output()
    else {
        return None;
    };
    if !output.status.success() {
        return None;
    }
    Some(
        String::from_utf8_lossy(&output.stdout)
            .lines()
            .filter_map(journal_line)
            .collect(),
    )
}

fn journal_line(text: &str) -> Option<LogLine> {
    let value: Value = serde_json::from_str(text).ok()?;
    let level = match value["PRIORITY"].as_str()? {
        "0" | "1" | "2" | "3" => "error",
        "4" => "warn",
        "5" | "6" => "info",
        _ => return None,
    };
    Some(LogLine {
        at_us: value["__REALTIME_TIMESTAMP"].as_str()?.parse().ok()?,
        level,
        event_kind: value["EVENT_KIND"].as_str().unwrap_or_default().to_owned(),
        message: value["MESSAGE"].as_str().unwrap_or_default().to_owned(),
        call_id: value["CALL_ID"].as_str().and_then(|id| id.parse().ok()),
        surface: value["SURFACE"].as_str().map(str::to_owned),
        connection_state: value["CONNECTION_STATE"].as_str().map(str::to_owned),
        audio_location: value["AUDIO_LOCATION"].as_str().map(str::to_owned),
        detail: value["DETAIL"].as_str().map(str::to_owned),
    })
}

fn file_line(text: &str) -> Option<LogLine> {
    let value: Value = serde_json::from_str(text).ok()?;
    let level = match value["level"].as_str()? {
        "ERROR" => "error",
        "WARN" => "warn",
        "INFO" => "info",
        _ => return None,
    };
    Some(LogLine {
        at_us: value["timestamp_us"].as_u64()?,
        level,
        event_kind: value["event_kind"].as_str().unwrap_or_default().to_owned(),
        message: value["message"].as_str().unwrap_or_default().to_owned(),
        call_id: value["call_id"].as_u64(),
        surface: value["surface"].as_str().map(str::to_owned),
        connection_state: value["connection_state"].as_str().map(str::to_owned),
        audio_location: value["audio_location"].as_str().map(str::to_owned),
        detail: value["detail"].as_str().map(str::to_owned),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_switch_is_on_for_debug_for_everything_or_for_bluez_dbus() {
        for on in [
            "info,bluez_dbus=debug",
            "debug",
            "warn, bluez_dbus",
            "trace",
        ] {
            assert!(logs_bluez_methods(Some(on)), "{on}");
        }
        for off in ["info", "desktop_calls=debug", "bluez_dbus=info", ""] {
            assert!(!logs_bluez_methods(Some(off)), "{off}");
        }
        assert!(!logs_bluez_methods(None));
    }

    #[test]
    fn a_bluez_method_line_reads_as_a_capture_line() {
        let error = serde_json::json!({
            "event_kind": "bluez-method-failed",
            "object": "device",
            "interface": "org.bluez.Device1",
            "method": "ConnectProfile",
            "detail": "org.bluez.Error.Failed: br-connection-refused",
        });
        assert_eq!(
            bluez_method(&error).as_deref(),
            Some(
                "app: error org.bluez.Device1.ConnectProfile from device: org.bluez.Error.Failed: br-connection-refused"
            )
        );
        let set = serde_json::json!({
            "event_kind": "bluez-method-called",
            "object": "adapter",
            "interface": "org.freedesktop.DBus.Properties",
            "method": "Set",
            "property": "Discoverable",
        });
        assert_eq!(
            bluez_method(&set).as_deref(),
            Some("app: call org.freedesktop.DBus.Properties.Set (Discoverable) on adapter")
        );
        let other = serde_json::json!({"event_kind": "call-started", "phone_number": "5551234567"});
        assert_eq!(bluez_method(&other), None);
    }
}
