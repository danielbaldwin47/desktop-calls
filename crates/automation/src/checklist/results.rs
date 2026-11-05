//! The dated result file: one per sitting, JSON, written after every step so
//! an interrupted run can resume. The scorecard reads the latest one.
//!
//! ```json
//! {
//!   "mode": "real",
//!   "started": "2026-11-05T14:32:07Z",
//!   "updated": "2026-11-05T14:51:40Z",
//!   "phone": "My Pixel",
//!   "steps": [
//!     { "id": "2.1", "title": "Answer on the Desktop", "result": "pass",
//!       "evidence": ["Banner shown 0.21 s after the Call appeared (target 1 s)."],
//!       "measurements": { "banner_latency_ms": 210, "codec": "msbc", "wideband": true },
//!       "started_us": 1793889127000000, "ended_us": 1793889190000000,
//!       "journal": [] }
//!   ]
//! }
//! ```
//!
//! `result` is `pass`, `fail` or `skipped`. `journal` holds the app's info,
//! warning and error lines from the step, kept for failed steps only. No
//! phone number is ever written.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{Map, Value, json};

/// Where the result files of runs against the real Phone go by default,
/// relative to the repository, where `just` runs.
pub const DEFAULT_DIRECTORY: &str = "docs/real-phone-checklist";

/// Whether a run was against the real Phone or the Simulated Phone.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Real,
    Demo,
}

impl Mode {
    fn as_str(self) -> &'static str {
        match self {
            Self::Real => "real",
            Self::Demo => "demo",
        }
    }
}

/// How a step ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    Pass,
    Fail,
    Skipped,
}

impl Outcome {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pass => "pass",
            Self::Fail => "fail",
            Self::Skipped => "skipped",
        }
    }
}

/// One step's record.
#[derive(Clone, Debug)]
pub struct StepRecord {
    pub id: String,
    pub title: String,
    pub outcome: Outcome,
    pub evidence: Vec<String>,
    pub measurements: Map<String, Value>,
    pub started_us: u64,
    pub ended_us: u64,
    pub journal: Vec<String>,
}

impl StepRecord {
    fn to_json(&self) -> Value {
        json!({
            "id": self.id,
            "title": self.title,
            "result": self.outcome.as_str(),
            "evidence": self.evidence,
            "measurements": self.measurements,
            "started_us": self.started_us,
            "ended_us": self.ended_us,
            "journal": self.journal,
        })
    }

    fn from_json(value: &Value) -> Option<Self> {
        let text = |name: &str| value[name].as_str().map(str::to_owned);
        let lines = |name: &str| -> Vec<String> {
            value[name]
                .as_array()
                .map(|lines| {
                    lines
                        .iter()
                        .filter_map(|line| line.as_str().map(str::to_owned))
                        .collect()
                })
                .unwrap_or_default()
        };
        Some(Self {
            id: text("id")?,
            title: text("title").unwrap_or_default(),
            outcome: match value["result"].as_str()? {
                "pass" => Outcome::Pass,
                "fail" => Outcome::Fail,
                _ => Outcome::Skipped,
            },
            evidence: lines("evidence"),
            measurements: value["measurements"]
                .as_object()
                .cloned()
                .unwrap_or_default(),
            started_us: value["started_us"].as_u64().unwrap_or(0),
            ended_us: value["ended_us"].as_u64().unwrap_or(0),
            journal: lines("journal"),
        })
    }
}

/// One sitting's results, and the file they are kept in.
#[derive(Debug)]
pub struct ResultFile {
    pub path: PathBuf,
    pub mode: Mode,
    pub started: String,
    pub phone: String,
    pub steps: Vec<StepRecord>,
}

impl ResultFile {
    /// A new, empty sitting in `directory`, named by the time it starts.
    pub fn new(directory: &Path, mode: Mode, phone: &str) -> Self {
        let started = utc_timestamp(SystemTime::now());
        // Colons are awkward in file names; the name keeps the moment.
        let name = format!("{}.json", started.replace(':', "-"));
        Self {
            path: directory.join(name),
            mode,
            started,
            phone: phone.to_owned(),
            steps: Vec::new(),
        }
    }

    /// The latest result file in `directory` of `mode`, if there is one.
    pub fn latest(directory: &Path, mode: Mode) -> Option<Self> {
        let mut paths: Vec<PathBuf> = fs::read_dir(directory)
            .ok()?
            .filter_map(|entry| Some(entry.ok()?.path()))
            .filter(|path| {
                path.extension()
                    .is_some_and(|extension| extension == "json")
            })
            .collect();
        paths.sort();
        paths
            .into_iter()
            .rev()
            .filter_map(|path| Self::read(&path))
            .find(|file| file.mode == mode)
    }

    fn read(path: &Path) -> Option<Self> {
        let value: Value = serde_json::from_str(&fs::read_to_string(path).ok()?).ok()?;
        let mode = match value["mode"].as_str()? {
            "real" => Mode::Real,
            "demo" => Mode::Demo,
            _ => return None,
        };
        Some(Self {
            path: path.to_owned(),
            mode,
            started: value["started"].as_str()?.to_owned(),
            phone: value["phone"].as_str().unwrap_or_default().to_owned(),
            steps: value["steps"]
                .as_array()?
                .iter()
                .filter_map(StepRecord::from_json)
                .collect(),
        })
    }

    /// The record of step `id`, if it has one.
    pub fn step(&self, id: &str) -> Option<&StepRecord> {
        self.steps.iter().find(|step| step.id == id)
    }

    /// Records `step`, replacing any earlier record of it, in checklist
    /// order, and writes the file.
    pub fn record(&mut self, step: StepRecord, order: &[&str]) -> io::Result<()> {
        self.steps.retain(|kept| kept.id != step.id);
        self.steps.push(step);
        let position = |id: &str| order.iter().position(|known| *known == id);
        self.steps.sort_by_key(|step| position(&step.id));
        self.write()
    }

    fn write(&self) -> io::Result<()> {
        if let Some(directory) = self.path.parent() {
            fs::create_dir_all(directory)?;
        }
        let value = json!({
            "mode": self.mode.as_str(),
            "started": self.started,
            "updated": utc_timestamp(SystemTime::now()),
            "phone": self.phone,
            "steps": self.steps.iter().map(StepRecord::to_json).collect::<Vec<_>>(),
        });
        let mut text = serde_json::to_string_pretty(&value).map_err(io::Error::other)?;
        text.push('\n');
        // Write beside the file, then rename, so an interrupted write never
        // leaves half a file.
        let partial = self.path.with_extension("json.partial");
        fs::write(&partial, text)?;
        fs::rename(&partial, &self.path)
    }
}

/// Microseconds since the Unix epoch, now.
pub fn unix_micros() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| {
            u64::try_from(elapsed.as_micros()).unwrap_or(u64::MAX)
        })
}

/// `time` as an ISO 8601 UTC timestamp to the second, such as
/// `2026-11-05T14:32:07Z`.
fn utc_timestamp(time: SystemTime) -> String {
    let seconds = time
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs());
    let days = i64::try_from(seconds / 86_400).unwrap_or(0);
    let of_day = seconds % 86_400;
    let (year, month, day) = civil_from_days(days);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        of_day / 3_600,
        of_day % 3_600 / 60,
        of_day % 60
    )
}

/// `time` as a PBAP Call History entry gives it, such as `20261105T143207`.
pub fn irmc_timestamp(time: SystemTime) -> String {
    utc_timestamp(time).replace(['-', ':', 'Z'], "")
}

/// The proleptic Gregorian date of `days` since 1970-01-01, by Howard
/// Hinnant's `civil_from_days`.
fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let days = days + 719_468;
    let era = days.div_euclid(146_097);
    let of_era = days.rem_euclid(146_097);
    let year_of_era = (of_era - of_era / 1_460 + of_era / 36_524 - of_era / 146_096) / 365;
    let day_of_year = of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let shifted_month = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * shifted_month + 2) / 5 + 1;
    let month = if shifted_month < 10 {
        shifted_month + 3
    } else {
        shifted_month - 9
    };
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    (year, month, day)
}
