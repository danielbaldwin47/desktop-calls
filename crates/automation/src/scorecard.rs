//! The polish scorecard runner: each item of `docs/wiki/polish-scorecard.md`
//! with a status and its evidence. Items gain real measurements as the
//! features that can pass them land.

use std::fmt;
use std::path::PathBuf;
use std::process::ExitCode;

use crate::checklist::results::{self, Mode, Outcome, ResultFile};

/// An item's standing. The spec also names MANUAL (waits on the user and
/// the real Phone) and NEEDS-REVIEW (waits on a screenshot-review agent);
/// they arrive with the first item that needs them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Status {
    Pass,
    Fail,
}

impl fmt::Display for Status {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.pad(match self {
            Self::Pass => "PASS",
            Self::Fail => "FAIL",
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

/// One scored item.
struct Score {
    status: Status,
    evidence: String,
}

impl Score {
    fn none() -> Self {
        Self {
            status: Status::Fail,
            evidence: "no evidence yet".to_owned(),
        }
    }
}

/// Scores one item. Items 3 and 5 read the latest Real-Phone Checklist run
/// against the real Phone, if there is one; nothing else is measured yet.
fn score(item: usize, checklist: Option<&ResultFile>) -> Score {
    match (item, checklist) {
        (3, Some(checklist)) => wideband(checklist),
        (5, Some(checklist)) => checklist_result(checklist),
        _ => Score::none(),
    }
}

/// Item 3: the codec the runner recorded on the latest real Call.
fn wideband(checklist: &ResultFile) -> Score {
    let recorded = checklist.steps.iter().rev().find_map(|step| {
        let codec = step.measurements.get("codec")?.as_str()?;
        let wideband = step.measurements.get("wideband")?.as_bool()?;
        Some((step.id.as_str(), codec, wideband))
    });
    match recorded {
        Some((step, codec, wideband)) => Score {
            status: if wideband { Status::Pass } else { Status::Fail },
            evidence: format!(
                "{codec} ({}) on {}, Real-Phone Checklist {}, step {step}",
                if wideband { "wideband" } else { "narrowband" },
                checklist.phone,
                checklist.started
            ),
        },
        None => Score::none(),
    }
}

/// Item 5: the Real-Phone Checklist part. The scenario count joins it with
/// ticket #27, so the item can't pass yet.
fn checklist_result(checklist: &ResultFile) -> Score {
    let count = |outcome: Outcome| {
        checklist
            .steps
            .iter()
            .filter(|step| step.outcome == outcome)
            .count()
    };
    Score {
        status: Status::Fail,
        evidence: format!(
            "Real-Phone Checklist {}: {} pass, {} fail, {} skipped; scenarios not yet counted",
            checklist.started,
            count(Outcome::Pass),
            count(Outcome::Fail),
            count(Outcome::Skipped)
        ),
    }
}

/// `automation scorecard`: prints the table.
pub fn run(results: Option<PathBuf>) -> ExitCode {
    let directory = results.unwrap_or_else(|| PathBuf::from(results::DEFAULT_DIRECTORY));
    let checklist = ResultFile::latest(&directory, Mode::Real);
    let scores: Vec<Score> = (1..=ITEMS.len())
        .map(|item| score(item, checklist.as_ref()))
        .collect();
    let passing = scores
        .iter()
        .filter(|score| score.status == Status::Pass)
        .count();
    let item_width = ITEMS.iter().map(|item| item.len()).max().unwrap_or(0);
    let status_width = "Status".len();

    println!("Polish scorecard: {passing} of {} items pass.", ITEMS.len());
    println!();
    println!(
        " #  {:<item_width$}  {:<status_width$}  Evidence",
        "Item", "Status"
    );
    for (index, (item, score)) in ITEMS.iter().zip(&scores).enumerate() {
        println!(
            "{:>2}  {item:<item_width$}  {:<status_width$}  {}",
            index + 1,
            score.status,
            score.evidence
        );
    }
    ExitCode::SUCCESS
}
