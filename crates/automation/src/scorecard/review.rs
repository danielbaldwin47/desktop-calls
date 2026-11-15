//! Item 8: the screenshot-review agent's latest verdict, recorded in
//! `docs/reviews/screenshot-review.md`, and whether what it reviewed has
//! changed since.
//!
//! The file records each watched path's git object id at the review, not
//! only the commit: a squash merge gives the commit a new id, but leaves
//! each path's content, and so its id, as it was.

use std::fs;
use std::path::Path;
use std::process::Command;

/// What the gallery shows comes from these: the surfaces, the core's views
/// and words, the Indicator's drawings, the gallery's states, the harness
/// that renders them, and the Omarchy looks. A change to any of them, or
/// one missing from the verdict or from `HEAD`, needs a new review.
pub const WATCHED: [&str; 7] = [
    "crates/desktop-calls/src",
    "crates/core/src",
    "data/icons",
    "crates/automation/src/gallery",
    "crates/automation/src/gallery.rs",
    "crates/test-harness/src",
    "crates/test-harness/fixtures/omarchy-themes",
];

/// Where the verdict is, from the repository's root.
pub const VERDICT_FILE: &str = "docs/reviews/screenshot-review.md";

/// What the review found, and whether it still stands.
pub enum Standing {
    /// No verdict file, or one that doesn't say what was reviewed.
    Unrecorded(String),
    /// The verdict stands: no defects, or the open defects' issues.
    Current {
        commit: String,
        defects: Vec<String>,
    },
    /// The watched paths that changed since the review of `commit`.
    Stale {
        commit: String,
        changed: Vec<String>,
    },
}

/// Reads the verdict at `verdict` and compares what it reviewed with the
/// `HEAD` of the repository at `root`.
pub fn standing(root: &Path, verdict: &Path) -> Standing {
    let Ok(text) = fs::read_to_string(verdict) else {
        return Standing::Unrecorded(format!("no review verdict at {}", verdict.display()));
    };
    let field = |name: &str| {
        text.lines()
            .find_map(|line| line.strip_prefix(name))
            .map(|value| value.trim().to_owned())
    };
    let (Some(commit), Some(verdict)) = (field("Reviewed commit:"), field("Verdict:")) else {
        return Standing::Unrecorded(format!(
            "{} has no \"Reviewed commit:\" or \"Verdict:\" line",
            verdict.display()
        ));
    };
    let reviewed: Vec<(&str, &str)> = text
        .lines()
        .filter_map(|line| line.strip_prefix("Reviewed path: ")?.split_once(' '))
        .collect();
    // Without git's answer, nothing can be said about the review: not that
    // every surface changed. A watched path gone from HEAD has changed.
    if let Err(why) = object_id(root, "") {
        return Standing::Unrecorded(format!(
            "can't compare the review with HEAD, git says: {why}"
        ));
    }
    let changed: Vec<String> = WATCHED
        .iter()
        .filter(|path| {
            let then = reviewed
                .iter()
                .find(|(reviewed, _)| reviewed == *path)
                .map(|(_, id)| id.trim());
            then.is_none() || then != object_id(root, path).ok().as_deref()
        })
        .map(|path| (*path).to_owned())
        .collect();
    if !changed.is_empty() {
        return Standing::Stale { commit, changed };
    }
    let mut defects: Vec<String> = text
        .lines()
        .filter_map(|line| line.strip_prefix("Defect: "))
        .map(|defect| defect.trim().to_owned())
        .collect();
    if verdict == "no defects" {
        defects.clear();
    } else if defects.is_empty() {
        defects.push(format!("the verdict is {verdict:?}, with no defect listed"));
    }
    Standing::Current { commit, defects }
}

/// The git object id of `path` at `HEAD` in the repository at `root`, or
/// what git said instead.
pub fn object_id(root: &Path, path: &str) -> Result<String, String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["rev-parse", &format!("HEAD:{path}")])
        .output()
        .map_err(|error| error.to_string())?;
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
    } else {
        Err(String::from_utf8_lossy(&output.stderr).trim().to_owned())
    }
}
