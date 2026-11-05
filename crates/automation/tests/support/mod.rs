//! The Real-Phone Checklist runner against the Simulated Phone, with a test
//! playing the user: shared by the checklist's test files.

// Clippy exempts `#[test]` functions but not their helpers. Like a test, a
// helper whose step fails must fail the test at once with its message.
#![expect(
    clippy::expect_used,
    reason = "test code: a failed helper step fails the test with its message"
)]

use std::io::{BufRead as _, BufReader, Write as _};
use std::path::Path;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::thread;
use std::time::Duration;

/// How long the runner may go without printing anything.
const QUIET_DEADLINE: Duration = Duration::from_secs(60);

/// A running checklist, and what it has printed.
pub struct Runner {
    pub child: Child,
    pub stdin: ChildStdin,
    pub printed: Receiver<String>,
    pub transcript: Vec<String>,
    /// The step at whose start the user types `quit`, if any.
    pub quit_at: Option<&'static str>,
    /// Demo actions the user does once instead of ones the runner
    /// suggests, as (suggested, done instead): slips, such as a mistyped
    /// number. What is done instead may be several actions, split by `;`.
    pub slips: Vec<(&'static str, &'static str)>,
    /// How many times the user does the suggested action as asked before
    /// slipping, for an action suggested more than once.
    pub slip_after: usize,
}

impl Runner {
    /// Starts the runner on a Desktop whose locale is in Britain, where
    /// the demo's Test Number is.
    pub fn start(arguments: &[&str], results: &Path) -> Self {
        Self::start_in_locale(arguments, results, "en_GB.UTF-8")
    }

    /// Starts the runner on a Desktop whose locale is `lang`.
    pub fn start_in_locale(arguments: &[&str], results: &Path, lang: &str) -> Self {
        let mut child = Command::new(env!("CARGO_BIN_EXE_automation"))
            .env_remove("LC_ALL")
            .env_remove("LC_TELEPHONE")
            .env("LANG", lang)
            .arg("real-phone-checklist")
            .args(arguments)
            .arg("--demo")
            .arg("--headless")
            .arg("--results")
            .arg(results)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .expect("start the checklist runner");
        let stdin = child.stdin.take().expect("the runner's standard input");
        let stdout = child.stdout.take().expect("the runner's standard output");
        let (sender, printed) = mpsc::channel();
        thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                let Ok(line) = line else { break };
                if sender.send(line).is_err() {
                    break;
                }
            }
        });
        Self {
            child,
            stdin,
            printed,
            transcript: Vec::new(),
            quit_at: None,
            slips: Vec::new(),
            slip_after: 0,
        }
    }

    pub fn type_line(&mut self, line: &str) {
        writeln!(self.stdin, "{line}").expect("type into the runner");
    }

    /// Plays the user until the runner exits: types each suggested action,
    /// starts each group, and answers each question with `answer(question)`.
    /// Returns every question asked.
    pub fn play(&mut self, mut answer: impl FnMut(&str) -> String) -> Vec<String> {
        let mut questions = Vec::new();
        loop {
            let line = match self.printed.recv_timeout(QUIET_DEADLINE) {
                Ok(line) => line,
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
                Err(mpsc::RecvTimeoutError::Timeout) => panic!(
                    "the runner printed nothing for {QUIET_DEADLINE:?}:\n{}",
                    self.transcript.join("\n")
                ),
            };
            self.transcript.push(line.clone());
            if self
                .quit_at
                .is_some_and(|id| line.starts_with(&format!("Step {id}:")))
            {
                self.type_line("quit");
            } else if let Some(action) = line.trim_start().strip_prefix("demo> ") {
                let slip = self
                    .slips
                    .iter()
                    .position(|(suggested, _)| *suggested == action);
                let done = match slip {
                    Some(_) if self.slip_after > 0 => {
                        self.slip_after -= 1;
                        action
                    }
                    Some(index) => self.slips.remove(index).1,
                    None => action,
                };
                for action in done.split(';') {
                    self.type_line(action);
                }
            } else if line.starts_with("Press Enter") {
                self.type_line("");
            } else if let Some(question) = line.trim_start().strip_prefix("Question: ") {
                let question = question.trim_end_matches(" [y/n]").to_owned();
                let reply = answer(&question);
                questions.push(question);
                self.type_line(&reply);
            }
        }
        let status = self.child.wait().expect("wait for the runner");
        assert!(
            status.success(),
            "the runner failed ({status}):\n{}",
            self.transcript.join("\n")
        );
        questions
    }

    pub fn printed(&self, text: &str) -> bool {
        self.transcript.iter().any(|line| line.contains(text))
    }

    /// The step ids the runner started, in order.
    pub fn steps_started(&self) -> Vec<String> {
        self.transcript
            .iter()
            .filter_map(|line| line.strip_prefix("Step "))
            .filter_map(|rest| rest.split(':').next())
            .map(str::to_owned)
            .collect()
    }
}

/// The only result file in `results`, as JSON.
pub fn result_file(results: &Path) -> serde_json::Value {
    let files: Vec<_> = std::fs::read_dir(results)
        .expect("list the results")
        .map(|entry| entry.expect("a result file").path())
        .collect();
    assert_eq!(files.len(), 1, "one dated result file: {files:?}");
    let name = files[0].file_name().and_then(|name| name.to_str());
    assert!(
        name.is_some_and(|name| name.starts_with("20"))
            && files[0]
                .extension()
                .is_some_and(|extension| extension == "json"),
        "the file is named by its date: {name:?}"
    );
    let text = std::fs::read_to_string(&files[0]).expect("read the result file");
    serde_json::from_str(&text).expect("the result file is JSON")
}

pub fn step<'a>(result: &'a serde_json::Value, id: &str) -> &'a serde_json::Value {
    result["steps"]
        .as_array()
        .expect("the steps")
        .iter()
        .find(|step| step["id"] == id)
        .unwrap_or_else(|| panic!("no step {id} in {result:#}"))
}

/// Whether any of `record`'s evidence lines contains `text`.
pub fn says(record: &serde_json::Value, text: &str) -> bool {
    record["evidence"].as_array().is_some_and(|evidence| {
        evidence
            .iter()
            .any(|line| line.as_str().is_some_and(|line| line.contains(text)))
    })
}
