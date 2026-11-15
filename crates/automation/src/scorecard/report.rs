//! The scenarios' results, read from the `JUnit` reports cargo-nextest writes
//! (`.config/nextest.toml`), one per scenario shard from CI's run on
//! `main`, or from what `cargo test` printed in `just scorecard`'s own run,
//! which needs no cargo-nextest.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

/// How one scenario ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ran {
    Passed,
    Failed,
    Skipped,
}

/// Every scenario in the reports read, by its full name.
pub struct Report {
    /// The files read, for the evidence.
    pub sources: Vec<PathBuf>,
    results: BTreeMap<String, Ran>,
}

impl Report {
    /// Reads `path`: a `JUnit` file or `cargo test`'s output, or a
    /// directory whose `.xml` files, at any depth, are each a `JUnit` file.
    /// `None` when there is no report there.
    pub fn read(path: &Path) -> Option<Self> {
        let mut sources = Vec::new();
        collect(path, &mut sources);
        sources.sort();
        // A file that can't be read is left out; the scenarios it held
        // then fail their items as not run.
        let read: Vec<(PathBuf, String)> = sources
            .into_iter()
            .filter_map(|source| {
                let text = fs::read_to_string(&source).ok()?;
                Some((source, text))
            })
            .collect();
        let sources: Vec<PathBuf> = read.iter().map(|(source, _)| source.clone()).collect();
        let mut results = BTreeMap::new();
        for (_, text) in &read {
            let found = if text.contains("<testcase ") {
                testcases(text)
            } else {
                printed_results(text)
            };
            for (name, ran) in found {
                // A scenario that failed in one report stays failed.
                let entry = results.entry(name).or_insert(ran);
                if ran == Ran::Failed {
                    *entry = Ran::Failed;
                }
            }
        }
        (!sources.is_empty()).then_some(Self { sources, results })
    }

    /// How the scenario `name` ended, if a report lists it.
    pub fn ran(&self, name: &str) -> Option<Ran> {
        self.results.get(name).copied()
    }

    /// Where the reports are, for the evidence.
    pub fn place(&self) -> String {
        match self.sources.as_slice() {
            [one] => one.display().to_string(),
            many => format!(
                "{} reports under {}",
                many.len(),
                common_parent(many).display()
            ),
        }
    }
}

fn collect(path: &Path, sources: &mut Vec<PathBuf>) {
    if path.is_file() {
        sources.push(path.to_owned());
    } else if let Ok(entries) = fs::read_dir(path) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                collect(&path, sources);
            } else if path.extension().is_some_and(|extension| extension == "xml") {
                sources.push(path);
            }
        }
    }
}

fn common_parent(paths: &[PathBuf]) -> PathBuf {
    let mut parent = paths
        .first()
        .and_then(|path| path.parent())
        .map(Path::to_path_buf)
        .unwrap_or_default();
    while !paths.iter().all(|path| path.starts_with(&parent)) {
        if !parent.pop() {
            break;
        }
    }
    parent
}

/// Each `<testcase name="…">` in `text` and how it ended: failed if it
/// holds a `<failure` or an `<error`, skipped if a `<skipped`.
fn testcases(text: &str) -> Vec<(String, Ran)> {
    let mut found = Vec::new();
    let mut rest = text;
    while let Some(start) = rest.find("<testcase ") {
        rest = &rest[start..];
        let head_end = rest.find('>').unwrap_or(rest.len());
        let head = &rest[..head_end];
        let body = if head.ends_with('/') {
            ""
        } else {
            let end = rest.find("</testcase>").unwrap_or(rest.len());
            &rest[head_end..end]
        };
        if let Some(name) = attribute(head, "name") {
            let ran = if body.contains("<failure") || body.contains("<error") {
                Ran::Failed
            } else if body.contains("<skipped") {
                Ran::Skipped
            } else {
                Ran::Passed
            };
            found.push((unescape(name), ran));
        }
        rest = &rest[head_end.min(rest.len())..];
    }
    found
}

/// Each `test <name> ... ok` line `cargo test` printed in `text`, and how
/// the test ended: `FAILED` failed, `ignored` skipped.
fn printed_results(text: &str) -> Vec<(String, Ran)> {
    text.lines()
        .filter_map(|line| {
            let (name, outcome) = line.strip_prefix("test ")?.split_once(" ... ")?;
            let ran = match outcome.trim() {
                "ok" => Ran::Passed,
                "FAILED" => Ran::Failed,
                outcome if outcome.starts_with("ignored") => Ran::Skipped,
                _ => return None,
            };
            Some((name.to_owned(), ran))
        })
        .collect()
}

fn attribute<'text>(head: &'text str, name: &str) -> Option<&'text str> {
    let marker = format!(" {name}=\"");
    let start = head.find(&marker)? + marker.len();
    let end = head[start..].find('"')? + start;
    Some(&head[start..end])
}

fn unescape(text: &str) -> String {
    text.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&amp;", "&")
}
