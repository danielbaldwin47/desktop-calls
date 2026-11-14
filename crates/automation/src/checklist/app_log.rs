//! The app's log for a stretch of time: the journal on the Desktop, or the
//! app's log file under the Simulated Phone. Only info, warning and error
//! lines are read, because debug lines carry phone numbers in full.

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
        let mut lines: Vec<LogLine> = match self {
            Self::Journal => journal(from_us, to_us),
            Self::File(path) => std::fs::read_to_string(path)
                .unwrap_or_default()
                .lines()
                .filter_map(file_line)
                .collect(),
        };
        lines.retain(|line| (from_us..=to_us).contains(&line.at_us));
        lines
    }
}

fn journal(from_us: u64, to_us: u64) -> Vec<LogLine> {
    let since = format!(
        "--since=@{}.{:06}",
        from_us / 1_000_000,
        from_us % 1_000_000
    );
    let until = format!("--until=@{}.{:06}", to_us / 1_000_000, to_us % 1_000_000);
    let Ok(output) = Command::new("journalctl")
        .args(["--user", "-t", "desktop-calls", "-o", "json", "--no-pager"])
        .args(["--priority=info", &since, &until])
        .output()
    else {
        return Vec::new();
    };
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(journal_line)
        .collect()
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
