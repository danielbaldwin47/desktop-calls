//! The polish scorecard runner: each item of `docs/wiki/polish-scorecard.md`
//! with a status and its evidence. Items gain real measurements as the
//! features that can pass them land.

use std::fmt;
use std::process::ExitCode;

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

/// Scores one item. Nothing can be measured yet: no feature an item
/// measures exists.
fn score(_item: usize) -> Score {
    Score {
        status: Status::Fail,
        evidence: "no evidence yet".to_owned(),
    }
}

/// `automation scorecard`: prints the table.
pub fn run() -> ExitCode {
    let scores: Vec<Score> = (1..=ITEMS.len()).map(score).collect();
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
