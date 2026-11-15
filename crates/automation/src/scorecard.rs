//! The polish scorecard runner: each item of `docs/wiki/polish-scorecard.md`
//! with a status and its evidence (#27).
//!
//! Simulation evidence is the harness's scenarios, read from cargo-nextest's
//! `JUnit` reports ([`report`]), by the names in [`proofs`]. Real-Phone
//! evidence is the latest Real-Phone Checklist run against the real Phone,
//! whose result file keeps what the journal showed at each step. Item 8 is
//! the screenshot review's verdict ([`review`]).

mod proofs;
mod report;
mod review;

use std::fmt;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use crate::checklist::{
    self,
    results::{self, Mode, Outcome, ResultFile},
};
use report::{Ran, Report};
use review::Standing;

/// An item's standing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Status {
    Pass,
    Fail,
    /// The Simulated Phone half passes, or there is none; the rest waits on
    /// the user and the real Phone.
    Manual,
    /// What was reviewed has changed since: a screenshot review is due.
    NeedsReview,
}

impl fmt::Display for Status {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.pad(match self {
            Self::Pass => "PASS",
            Self::Fail => "FAIL",
            Self::Manual => "MANUAL",
            Self::NeedsReview => "NEEDS-REVIEW",
        })
    }
}

/// The eight items agreed with the user on 2026-11-04, worded as on the wiki.
const ITEMS: [&str; 8] = [
    "The Incoming Call Banner appears within 1 s of the Phone ringing.",
    "Audio flows within 1.5 s of pressing Answer.",
    "Wideband speech is negotiated with the Pixel 9a.",
    "From a fresh install, pairing through to the first Call takes 3 minutes or less.",
    "20 of 20 scripted Simulated Phone scenarios pass, and the Real-Phone Checklist passes.",
    "Reconnects within 10 s of the Phone returning to range, and survives suspend/resume.",
    "Every action is reachable by keyboard alone.",
    "A screenshot-review agent finds no visual defects.",
];

/// Item 1's bound on the real Phone, as the Checklist measures it.
const BANNER_MS: u64 = 1_000;
/// Item 2's.
const AUDIO_MS: u64 = 1_500;
/// Item 4's.
const FIRST_CALL_MS: u64 = 180_000;

/// One scored item.
struct Score {
    status: Status,
    evidence: String,
}

impl Score {
    fn new(status: Status, evidence: impl Into<String>) -> Self {
        Self {
            status,
            evidence: evidence.into(),
        }
    }
}

/// What the scorecard reads.
struct Evidence<'a> {
    scenarios: Option<&'a Report>,
    checklist: Option<&'a ResultFile>,
    root: &'a Path,
    verdict: &'a Path,
    gallery: Option<&'a str>,
}

/// How a list of scenarios fared: passed when all of them passed.
struct Tally {
    passed: usize,
    of: usize,
    /// The first few that failed, or that no report lists.
    not_passed: Vec<String>,
}

impl Tally {
    fn of(report: &Report, names: &[&str]) -> Self {
        let not_passed: Vec<String> = names
            .iter()
            .filter_map(|name| match report.ran(name) {
                Some(Ran::Passed) => None,
                Some(Ran::Failed) => Some(format!("{name} failed")),
                Some(Ran::Skipped) => Some(format!("{name} was skipped")),
                None => Some(format!("{name} is not in the report")),
            })
            .collect();
        Self {
            passed: names.len() - not_passed.len(),
            of: names.len(),
            not_passed,
        }
    }

    fn all_passed(&self) -> bool {
        self.not_passed.is_empty()
    }

    fn summary(&self, what: &str) -> String {
        let tally = format!("{} of {} {what} pass", self.passed, self.of);
        match self.not_passed.as_slice() {
            [] => tally,
            [only] => format!("{tally} ({only})"),
            [first, rest @ ..] => format!("{tally} ({first}, and {} more)", rest.len()),
        }
    }
}

/// Scores one item.
fn score(item: usize, evidence: &Evidence) -> Score {
    match item {
        1 => timed(
            evidence,
            proofs::BANNER,
            "banner_latency_ms",
            BANNER_MS,
            "group 2",
        ),
        2 => timed(
            evidence,
            proofs::AUDIO,
            "audio_latency_ms",
            AUDIO_MS,
            "group 2",
        ),
        3 => wideband(evidence.checklist),
        4 => first_call(evidence.checklist),
        5 => scenarios_and_checklist(evidence),
        6 => reconnects(evidence),
        7 => keyboard(evidence.scenarios),
        _ => screenshots(evidence),
    }
}

/// The Simulated Phone half of an item: its scenarios' tally, or why there
/// is none.
fn simulated(scenarios: Option<&Report>, names: &[&str]) -> Result<String, Score> {
    let Some(report) = scenarios else {
        return Err(Score::new(
            Status::Fail,
            "no scenario report found (locally, `just scorecard` runs the scenarios first)",
        ));
    };
    let tally = Tally::of(report, names);
    let summary = format!("{}, {}", tally.summary("scenarios"), report.place());
    if tally.all_passed() {
        Ok(summary)
    } else {
        Err(Score::new(Status::Fail, summary))
    }
}

/// Items 1 and 2: the scenarios hold the bound against the Simulated
/// Phone, and the latest real Checklist measured it on the real Phone:
/// the slowest of its steps that measured it, since each must hold.
fn timed(evidence: &Evidence, names: &[&str], key: &str, bound: u64, group: &str) -> Score {
    let simulated = match simulated(evidence.scenarios, names) {
        Ok(summary) => summary,
        Err(failed) => return failed,
    };
    let measured = evidence.checklist.and_then(|checklist| {
        checklist
            .steps
            .iter()
            .filter_map(|step| {
                let millis = step.measurements.get(key)?.as_u64()?;
                Some((checklist, step.id.as_str(), millis))
            })
            .max_by_key(|(_, _, millis)| *millis)
    });
    match measured {
        Some((checklist, step, millis)) => Score::new(
            if millis <= bound {
                Status::Pass
            } else {
                Status::Fail
            },
            format!(
                "{simulated}; real Phone: {millis} ms against {bound} ms, Real-Phone Checklist {}, step {step}",
                checklist.started
            ),
        ),
        None => Score::new(
            Status::Manual,
            format!(
                "{simulated}; real Phone: not measured, run the Real-Phone Checklist's {group}"
            ),
        ),
    }
}

/// Item 3: the codec the runner recorded on the latest real Call.
fn wideband(checklist: Option<&ResultFile>) -> Score {
    let recorded = checklist.and_then(|checklist| {
        checklist.steps.iter().rev().find_map(|step| {
            let codec = step.measurements.get("codec")?.as_str()?;
            let wideband = step.measurements.get("wideband")?.as_bool()?;
            Some((checklist, step.id.as_str(), codec, wideband))
        })
    });
    match recorded {
        Some((checklist, step, codec, wideband)) => Score::new(
            if wideband { Status::Pass } else { Status::Fail },
            format!(
                "{codec} ({}) on {}, Real-Phone Checklist {}, step {step}",
                if wideband { "wideband" } else { "narrowband" },
                checklist.phone,
                checklist.started
            ),
        ),
        None => Score::new(
            Status::Manual,
            "no real Call's codec recorded: run the Real-Phone Checklist's group 2",
        ),
    }
}

/// Item 4: pairing to the first Call, timed by the Checklist's step 1.2.
fn first_call(checklist: Option<&ResultFile>) -> Score {
    let timed = checklist.and_then(|checklist| {
        checklist.steps.iter().rev().find_map(|step| {
            let millis = step
                .measurements
                .get("pairing_to_first_call_ms")?
                .as_u64()?;
            Some((checklist, step.id.as_str(), millis))
        })
    });
    match timed {
        Some((checklist, step, millis)) => Score::new(
            if millis <= FIRST_CALL_MS {
                Status::Pass
            } else {
                Status::Fail
            },
            format!(
                "{} s from pairing to the first Call, Real-Phone Checklist {}, step {step}",
                millis / 1_000,
                checklist.started
            ),
        ),
        None => Score::new(
            Status::Manual,
            "not timed on the real Phone: run the Real-Phone Checklist's group 1",
        ),
    }
}

/// Item 5: the 20 canonical scenarios and those for each quirk and setup
/// problem, then the latest real Checklist.
fn scenarios_and_checklist(evidence: &Evidence) -> Score {
    let Some(report) = evidence.scenarios else {
        return Score::new(
            Status::Fail,
            "no scenario report found (locally, `just scorecard` runs the scenarios first)",
        );
    };
    let canonical: Vec<(&str, Tally)> = proofs::CANONICAL
        .iter()
        .map(|(scenario, names)| (*scenario, Tally::of(report, names)))
        .collect();
    let passing = canonical
        .iter()
        .filter(|(_, tally)| tally.all_passed())
        .count();
    let quirks: Vec<&str> = proofs::QUIRKS_AND_PROBLEMS
        .iter()
        .flat_map(|(_, names)| names.iter().copied())
        .collect();
    let quirk_tally = Tally::of(report, &quirks);
    let simulated = format!(
        "{passing} of {} canonical scenarios pass, {}",
        canonical.len(),
        quirk_tally.summary("quirk and setup-problem scenarios")
    );
    if let Some((scenario, tally)) = canonical.iter().find(|(_, tally)| !tally.all_passed()) {
        return Score::new(
            Status::Fail,
            format!(
                "{simulated}; not passing: {scenario} ({})",
                tally.not_passed.join(", ")
            ),
        );
    }
    if !quirk_tally.all_passed() {
        return Score::new(Status::Fail, simulated);
    }
    let Some(checklist) = evidence.checklist else {
        return Score::new(
            Status::Manual,
            format!("{simulated}; no Real-Phone Checklist run against the real Phone yet"),
        );
    };
    let count = |outcome: Outcome| {
        checklist
            .steps
            .iter()
            .filter(|step| step.outcome == outcome)
            .count()
    };
    let (passed, failed, skipped) = (
        count(Outcome::Pass),
        count(Outcome::Fail),
        count(Outcome::Skipped),
    );
    let not_run = groups_not_run(checklist, "");
    let status = if failed > 0 {
        Status::Fail
    } else if skipped > 0 || !not_run.is_empty() {
        Status::Manual
    } else {
        Status::Pass
    };
    let not_run = if not_run.is_empty() {
        String::new()
    } else {
        format!(", not run: group {}", not_run.join(", "))
    };
    Score::new(
        status,
        format!(
            "{simulated}; Real-Phone Checklist {}: {passed} pass, {failed} fail, {skipped} skipped{not_run}, {}",
            checklist.started,
            checklist.path.display()
        ),
    )
}

/// The groups, by number, with a step whose id starts with `prefix` that
/// `checklist` has no record of: a run of one group, or one quit partway,
/// records only the steps it ran.
fn groups_not_run(checklist: &ResultFile, prefix: &str) -> Vec<String> {
    let mut groups: Vec<String> = Vec::new();
    for id in checklist::step_ids() {
        let recorded = checklist.steps.iter().any(|step| step.id == id);
        let group = id.split('.').next().unwrap_or(id).to_owned();
        if id.starts_with(prefix) && !recorded && !groups.contains(&group) {
            groups.push(group);
        }
    }
    groups
}

/// Item 6: the reconnect scenarios, then the Checklist's group 6 on the
/// real Phone.
fn reconnects(evidence: &Evidence) -> Score {
    let simulated = match simulated(evidence.scenarios, proofs::RECONNECT) {
        Ok(summary) => summary,
        Err(failed) => return failed,
    };
    let group: Vec<_> = evidence
        .checklist
        .map(|checklist| {
            checklist
                .steps
                .iter()
                .filter(|step| step.id.starts_with("6."))
                .collect()
        })
        .unwrap_or_default();
    let Some(checklist) = evidence.checklist.filter(|_| !group.is_empty()) else {
        return Score::new(
            Status::Manual,
            format!("{simulated}; real Phone: run the Real-Phone Checklist's group 6"),
        );
    };
    let failed: Vec<&str> = group
        .iter()
        .filter(|step| step.outcome == Outcome::Fail)
        .map(|step| step.id.as_str())
        .collect();
    let passed = group
        .iter()
        .filter(|step| step.outcome == Outcome::Pass)
        .count();
    // Every step of group 6 the Checklist has, run or not: a run quit
    // partway records only some.
    let scripted = checklist::step_ids()
        .iter()
        .filter(|id| id.starts_with("6."))
        .count();
    let status = if !failed.is_empty() {
        Status::Fail
    } else if passed < scripted {
        Status::Manual
    } else {
        Status::Pass
    };
    Score::new(
        status,
        format!(
            "{simulated}; real Phone: {passed} of {scripted} group 6 steps pass, Real-Phone Checklist {}",
            checklist.started
        ),
    )
}

/// Item 7: the keyboard-only suite, and the actions it lists as having no
/// key path yet.
fn keyboard(scenarios: Option<&Report>) -> Score {
    let simulated = match simulated(scenarios, proofs::KEYBOARD) {
        Ok(summary) => summary,
        Err(failed) => return failed,
    };
    let mut tickets: Vec<&str> = Vec::new();
    let mut missing = 0;
    for line in proofs::KEYBOARD_SUITE.lines() {
        if let Some(rest) = line.split("Missing { ticket: ").nth(1) {
            let ticket = rest.split(' ').next().unwrap_or_default();
            if ticket.chars().all(|character| character.is_ascii_digit()) {
                missing += 1;
                if !tickets.contains(&ticket) {
                    tickets.push(ticket);
                }
            }
        }
    }
    if missing == 0 {
        Score::new(
            Status::Pass,
            format!("{simulated}; every action has a key path"),
        )
    } else {
        let tickets: Vec<String> = tickets.iter().map(|ticket| format!("#{ticket}")).collect();
        Score::new(
            Status::Fail,
            format!(
                "{simulated}; {missing} actions have no key path yet ({}), crates/test-harness/tests/keyboard_only.rs",
                tickets.join(", ")
            ),
        )
    }
}

/// Item 8: the screenshot review's verdict, and the gallery it judged.
fn screenshots(evidence: &Evidence) -> Score {
    let gallery = evidence
        .gallery
        .map_or_else(String::new, |gallery| format!("; gallery: {gallery}"));
    match review::standing(evidence.root, evidence.verdict) {
        Standing::Unrecorded(why) => Score::new(Status::NeedsReview, format!("{why}{gallery}")),
        Standing::Stale { commit, changed } => Score::new(
            Status::NeedsReview,
            format!(
                "{} changed since the review of {commit}, {}{gallery}",
                changed.join(", "),
                review::VERDICT_FILE
            ),
        ),
        Standing::Current { commit, defects } if defects.is_empty() => Score::new(
            Status::Pass,
            format!("no defects at {commit}, {}{gallery}", review::VERDICT_FILE),
        ),
        Standing::Current { commit, defects } => Score::new(
            Status::Fail,
            format!(
                "{} defects at {commit}: {}, {}{gallery}",
                defects.len(),
                defects.join("; "),
                review::VERDICT_FILE
            ),
        ),
    }
}

/// What `automation scorecard` is given.
pub struct Options {
    /// The Real-Phone Checklist's result files.
    pub results: Option<PathBuf>,
    /// cargo-nextest's `JUnit` report of the scenarios, or a directory of them.
    pub scenarios: Option<PathBuf>,
    /// The screenshot review's verdict. Default: `docs/reviews/screenshot-review.md`.
    pub review: Option<PathBuf>,
    /// Where the gallery is, for item 8's evidence.
    pub gallery: Option<String>,
    /// Print the name of every scenario the scorecard reads, one a line,
    /// and score nothing.
    pub names: bool,
    /// Print every Real-Phone Checklist step's id, one a line, and score
    /// nothing.
    pub checklist_steps: bool,
}

/// `automation scorecard`: prints the table.
pub fn run(options: Options) -> ExitCode {
    if options.names || options.checklist_steps {
        let lines = if options.names {
            proofs::every_scenario()
        } else {
            checklist::step_ids()
        };
        for line in lines {
            println!("{line}");
        }
        return ExitCode::SUCCESS;
    }
    let directory = match options.results.map_or_else(results::default_directory, Ok) {
        Ok(directory) => directory,
        Err(why) => {
            eprintln!("{why}");
            return ExitCode::FAILURE;
        }
    };
    let checklist = ResultFile::latest(&directory, Mode::Real);
    let scenarios = options.scenarios.as_deref().and_then(Report::read);
    let root = PathBuf::from(".");
    let verdict = options
        .review
        .unwrap_or_else(|| root.join(review::VERDICT_FILE));
    let evidence = Evidence {
        scenarios: scenarios.as_ref(),
        checklist: checklist.as_ref(),
        root: &root,
        verdict: &verdict,
        gallery: options.gallery.as_deref(),
    };
    let scores: Vec<Score> = (1..=ITEMS.len())
        .map(|item| score(item, &evidence))
        .collect();
    print_table(&scores);
    ExitCode::SUCCESS
}

fn print_table(scores: &[Score]) {
    let count = |status: Status| scores.iter().filter(|score| score.status == status).count();
    let item_width = ITEMS.iter().map(|item| item.len()).max().unwrap_or(0);
    let status_width = "NEEDS-REVIEW".len();

    println!(
        "Polish scorecard: {} of {} items pass ({} fail, {} manual, {} need review).",
        count(Status::Pass),
        ITEMS.len(),
        count(Status::Fail),
        count(Status::Manual),
        count(Status::NeedsReview)
    );
    println!();
    println!(
        " #  {:<item_width$}  {:<status_width$}  Evidence",
        "Item", "Status"
    );
    for (index, (item, score)) in ITEMS.iter().zip(scores).enumerate() {
        println!(
            "{:>2}  {item:<item_width$}  {:<status_width$}  {}",
            index + 1,
            score.status,
            score.evidence
        );
    }
}
