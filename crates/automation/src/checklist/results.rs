//! The dated result file: one per sitting, JSON, written after every step so
//! an interrupted run can resume. The scorecard reads the latest one.
//!
//! ```json
//! {
//!   "mode": "real",
//!   "started": "2026-11-05T14:32:07Z",
//!   "updated": "2026-11-05T14:51:40Z",
//!   "phone": "My Pixel",
//!   "runs": [
//!     { "run": 1, "started": "2026-11-05T14:32:07Z", "command": "just real-phone-checklist 2" }
//!   ],
//!   "steps": [
//!     { "id": "2.1", "title": "Answer on the Desktop", "result": "pass", "run": 1,
//!       "evidence": ["Banner shown 0.21 s after the Call appeared (target 1 s)."],
//!       "measurements": { "banner_latency_ms": 210, "codec": "msbc", "wideband": true },
//!       "started_us": 1793889127000000, "ended_us": 1793889190000000,
//!       "journal": [], "bus": "2.1-1793889127000000.txt" }
//!   ],
//!   "earlier": []
//! }
//! ```
//!
//! Each time the runner is started on the file is a run, listed in `runs`
//! (#380). `steps` holds each step's latest record, and its `run` says which
//! run recorded it; a record a later one replaced moves to `earlier`, so no
//! result is overwritten or loses its run. `run` is 0 in a file written
//! before runs were kept.
//!
//! `result` is `pass`, `fail`, `skipped` or `unexpected`. `unexpected` is
//! "observed, unexpected": the step ran, the app logged no error, and the
//! Phone did something the step did not anticipate, which its evidence
//! lines starting `Unexpected: ` describe. `journal` holds the app's info,
//! warning and error lines from the step, kept for failed and unexpected
//! steps only. No phone number is ever written. `bus` names the step's
//! Bluetooth D-Bus capture, in the directory beside the file with its
//! extension `.bus` (`bus_capture`); `null` for a step that didn't run or
//! a capture that couldn't be written.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{Map, Value, json};

/// Where the result files of runs against the real Phone go by default:
/// `$XDG_STATE_HOME/desktop-calls/real-phone-checklist`, or under
/// `~/.local/state`. They come from the real Phone and can hold real data,
/// so they stay out of the repository, which is published (ticket #120).
/// A relative `XDG_STATE_HOME` is ignored, as the XDG spec says. With no
/// absolute `HOME` either, it says why rather than fall back to a relative
/// path, which would put the results in the repository.
pub fn default_directory() -> Result<PathBuf, String> {
    let absolute = |name: &str| {
        std::env::var_os(name)
            .map(PathBuf::from)
            .filter(|path| path.is_absolute())
    };
    let state = absolute("XDG_STATE_HOME")
        .or_else(|| absolute("HOME").map(|home| home.join(".local/state")))
        .ok_or_else(|| {
            "HOME isn't set to an absolute path, so the Real-Phone Checklist has nowhere \
             outside the repository to keep its results. Set HOME, or pass --results DIR."
                .to_owned()
        })?;
    Ok(state.join("desktop-calls/real-phone-checklist"))
}

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
    /// Observed, unexpected: the step ran and the app logged no error, but
    /// the Phone did something the step did not anticipate. Its evidence
    /// says what; the run carries on, and the step isn't run again.
    Unexpected,
}

impl Outcome {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pass => "pass",
            Self::Fail => "fail",
            Self::Skipped => "skipped",
            Self::Unexpected => "unexpected",
        }
    }

    /// Whether the step is done with: passed, or observed something
    /// unexpected, which running it again would only observe again.
    pub fn is_done(self) -> bool {
        matches!(self, Self::Pass | Self::Unexpected)
    }
}

/// One step's record.
#[derive(Clone, Debug)]
pub struct StepRecord {
    pub id: String,
    pub title: String,
    pub outcome: Outcome,
    /// The run that recorded it, from 1; 0 if the file predates runs.
    pub run: u32,
    pub evidence: Vec<String>,
    pub measurements: Map<String, Value>,
    pub started_us: u64,
    pub ended_us: u64,
    pub journal: Vec<String>,
    /// The file name of the step's bus capture, if one was written.
    pub bus: Option<String>,
}

impl StepRecord {
    fn to_json(&self) -> Value {
        json!({
            "id": self.id,
            "title": self.title,
            "result": self.outcome.as_str(),
            "run": self.run,
            "evidence": self.evidence,
            "measurements": self.measurements,
            "started_us": self.started_us,
            "ended_us": self.ended_us,
            "journal": self.journal,
            "bus": self.bus,
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
                "unexpected" => Outcome::Unexpected,
                _ => Outcome::Skipped,
            },
            run: value["run"]
                .as_u64()
                .and_then(|run| u32::try_from(run).ok())
                .unwrap_or(0),
            evidence: lines("evidence"),
            measurements: value["measurements"]
                .as_object()
                .cloned()
                .unwrap_or_default(),
            started_us: value["started_us"].as_u64().unwrap_or(0),
            ended_us: value["ended_us"].as_u64().unwrap_or(0),
            journal: lines("journal"),
            bus: text("bus"),
        })
    }
}

/// One time the runner was started on a result file.
#[derive(Clone, Debug)]
pub struct Run {
    /// From 1, in the order the runs started.
    pub number: u32,
    pub started: String,
    /// The command that started it, such as `just real-phone-checklist 1`.
    pub command: String,
}

impl Run {
    fn to_json(&self) -> Value {
        json!({ "run": self.number, "started": self.started, "command": self.command })
    }

    fn from_json(value: &Value) -> Option<Self> {
        Some(Self {
            number: u32::try_from(value["run"].as_u64()?).ok()?,
            started: value["started"].as_str()?.to_owned(),
            command: value["command"].as_str().unwrap_or_default().to_owned(),
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
    pub runs: Vec<Run>,
    /// Each step's latest record.
    pub steps: Vec<StepRecord>,
    /// Records a later record of the same step replaced, oldest first.
    pub earlier: Vec<StepRecord>,
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
            runs: Vec::new(),
            steps: Vec::new(),
            earlier: Vec::new(),
        }
    }

    /// Starts a run on this file, started by `command`. Every step recorded
    /// from here on belongs to it.
    pub fn begin_run(&mut self, command: &str) {
        let number = self.runs.iter().map(|run| run.number).max().unwrap_or(0) + 1;
        self.runs.push(Run {
            number,
            started: utc_timestamp(SystemTime::now()),
            command: command.to_owned(),
        });
    }

    /// The run under way: the last one begun, or 0 for none.
    pub fn run(&self) -> u32 {
        self.runs.last().map_or(0, |run| run.number)
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
            runs: value["runs"]
                .as_array()
                .map(|runs| runs.iter().filter_map(Run::from_json).collect())
                .unwrap_or_default(),
            steps: value["steps"]
                .as_array()?
                .iter()
                .filter_map(StepRecord::from_json)
                .collect(),
            earlier: value["earlier"]
                .as_array()
                .map(|records| records.iter().filter_map(StepRecord::from_json).collect())
                .unwrap_or_default(),
        })
    }

    /// The record of step `id`, if it has one.
    pub fn step(&self, id: &str) -> Option<&StepRecord> {
        self.steps.iter().find(|step| step.id == id)
    }

    /// Records `step` for the run under way, in checklist order, and writes
    /// the file. An earlier record of the step moves to `earlier`.
    pub fn record(&mut self, mut step: StepRecord, order: &[&str]) -> io::Result<()> {
        step.run = self.run();
        if let Some(index) = self.steps.iter().position(|kept| kept.id == step.id) {
            self.earlier.push(self.steps.remove(index));
        }
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
            "runs": self.runs.iter().map(Run::to_json).collect::<Vec<_>>(),
            "steps": self.steps.iter().map(StepRecord::to_json).collect::<Vec<_>>(),
            "earlier": self.earlier.iter().map(StepRecord::to_json).collect::<Vec<_>>(),
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
