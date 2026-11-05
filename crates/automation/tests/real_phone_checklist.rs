//! `automation real-phone-checklist --demo`: the Real-Phone Checklist runner
//! against the Simulated Phone, with this test playing the user. It types
//! each action the runner suggests after `demo>`, presses Enter when asked
//! to start, and answers every yes-or-no question, as an agent playing the
//! user would. It then reads the dated result file.

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

/// The group-2 questions a machine can't answer today: what the helper
/// hears, what the notification server draws, and what the Desktop's
/// speakers play. The rest of group 2 is observed through the control
/// interface.
const HEARD_CLEARLY: &str = "Does your helper hear you clearly, with no echo of their own voice?";
const MISSED_CALL_SHOWN: &str =
    "Does a notification say you missed a call from your helper, by name or number, with the time?";
const WITHHELD_SHOWN: &str = "Does a notification say you missed a call from a withheld number, and that it can't be called back?";
const DESKTOP_SILENT: &str =
    "Apart from the Pixel itself, was the Desktop silent while the Call rang?";

/// A running checklist, and what it has printed.
struct Runner {
    child: Child,
    stdin: ChildStdin,
    printed: Receiver<String>,
    transcript: Vec<String>,
    /// The step at whose start the user types `quit`, if any.
    quit_at: Option<&'static str>,
}

impl Runner {
    fn start(arguments: &[&str], results: &Path) -> Self {
        let mut child = Command::new(env!("CARGO_BIN_EXE_automation"))
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
        }
    }

    fn type_line(&mut self, line: &str) {
        writeln!(self.stdin, "{line}").expect("type into the runner");
    }

    /// Plays the user until the runner exits: types each suggested action,
    /// starts each group, and answers each question with `answer(question)`.
    /// Returns every question asked.
    fn play(&mut self, mut answer: impl FnMut(&str) -> String) -> Vec<String> {
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
                self.type_line(action);
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

    fn printed(&self, text: &str) -> bool {
        self.transcript.iter().any(|line| line.contains(text))
    }

    /// The step ids the runner started, in order.
    fn steps_started(&self) -> Vec<String> {
        self.transcript
            .iter()
            .filter_map(|line| line.strip_prefix("Step "))
            .filter_map(|rest| rest.split(':').next())
            .map(str::to_owned)
            .collect()
    }
}

/// The only result file in `results`, as JSON.
fn result_file(results: &Path) -> serde_json::Value {
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

fn step<'a>(result: &'a serde_json::Value, id: &str) -> &'a serde_json::Value {
    result["steps"]
        .as_array()
        .expect("the steps")
        .iter()
        .find(|step| step["id"] == id)
        .unwrap_or_else(|| panic!("no step {id} in {result:#}"))
}

#[test]
fn group_2_measures_banner_and_audio_latency_and_the_codec_and_asks_only_what_it_cannot_see() {
    let results = tempfile::tempdir().expect("a results directory");
    let mut runner = Runner::start(&["2"], results.path());
    // As on Foner's handset: Call1.Hangup doesn't exist, so hanging up
    // needs the org.ofono.VoiceCall fallback, which the runner reports.
    runner.type_line("quirk call1-hangup-missing");
    let questions = runner.play(|_| "y".to_owned());

    assert_eq!(
        questions,
        [
            HEARD_CLEARLY,
            MISSED_CALL_SHOWN,
            WITHHELD_SHOWN,
            DESKTOP_SILENT
        ],
        "{}",
        runner.transcript.join("\n")
    );
    assert!(runner.printed("Before you start"), "the group's list");
    let result = result_file(results.path());
    assert_eq!(result["mode"], "demo");

    let answered = step(&result, "2.1");
    assert_eq!(answered["result"], "pass", "{answered:#}");
    let measured = &answered["measurements"];
    assert!(
        measured["banner_latency_ms"]
            .as_u64()
            .is_some_and(|ms| ms <= 1_000),
        "{answered:#}"
    );
    assert!(
        measured["audio_latency_ms"]
            .as_u64()
            .is_some_and(|ms| ms <= 1_500),
        "{answered:#}"
    );
    // The Simulated Phone negotiates LC3-SWB unless told otherwise.
    assert_eq!(measured["codec"], "lc3-swb", "{answered:#}");
    assert_eq!(measured["wideband"], true, "{answered:#}");
    assert_eq!(measured["hang_up"], "org.ofono.VoiceCall", "{answered:#}");
    // Timed from the app's own log of the Call appearing, not from the
    // control interface's later report of it.
    assert!(
        answered["evidence"][0]
            .as_str()
            .is_some_and(|evidence| evidence.contains("after the Desktop first saw the Call")),
        "{answered:#}"
    );

    for id in ["2.2", "2.3", "2.4", "2.5", "2.6", "2.7", "2.8"] {
        let observed = step(&result, id);
        assert_eq!(observed["result"], "pass", "{observed:#}");
        assert!(
            observed["evidence"]
                .as_array()
                .is_some_and(|e| !e.is_empty()),
            "{observed:#}"
        );
    }
    // 2.5: the Missed Call is notified, and its notification leads back to
    // the caller: the Call placed after Enter goes to the number that rang.
    let rang_out = step(&result, "2.5");
    assert!(
        rang_out["evidence"]
            .as_array()
            .is_some_and(|evidence| evidence.iter().any(|line| line
                .as_str()
                .is_some_and(|line| line.contains("called back")))),
        "{rang_out:#}"
    );
    // 2.4: the Call declined on the Pixel isn't in its missed Call History,
    // so no Missed Call is notified.
    let declined = step(&result, "2.4");
    assert!(
        declined["evidence"]
            .as_array()
            .is_some_and(|evidence| evidence.iter().any(|line| line
                .as_str()
                .is_some_and(|line| line.contains("No Missed Call")))),
        "{declined:#}"
    );
    // 2.7: under Do Not Disturb the Banner shows, the Desktop never rings,
    // and Decline on the Desktop and Answer on the Pixel are told apart.
    let quiet = step(&result, "2.7");
    for seen in [
        "saw Do Not Disturb turn on",
        "never rang",
        "after Decline",
        "Told apart from Decline on the Desktop: answered on the Pixel",
        "saw Do Not Disturb turn off",
    ] {
        assert!(
            quiet["evidence"].as_array().is_some_and(|evidence| evidence
                .iter()
                .any(|line| line.as_str().is_some_and(|line| line.contains(seen)))),
            "{seen:?} in {quiet:#}"
        );
    }
    // Only group 2 ran.
    assert!(
        result["steps"]
            .as_array()
            .expect("the steps")
            .iter()
            .all(|step| step["id"].as_str().is_some_and(|id| id.starts_with("2."))),
        "{result:#}"
    );
}

/// `automation scorecard --results results`, as text.
fn scorecard(results: &Path) -> String {
    let printed = Command::new(env!("CARGO_BIN_EXE_automation"))
        .arg("scorecard")
        .arg("--results")
        .arg(results)
        .output()
        .expect("run automation scorecard");
    assert!(printed.status.success(), "scorecard failed: {printed:?}");
    String::from_utf8(printed.stdout).expect("the scorecard is text")
}

fn row(scorecard: &str, number: u8) -> &str {
    scorecard
        .lines()
        .find(|line| line.trim_start().starts_with(&format!("{number} ")))
        .unwrap_or_else(|| panic!("no row {number}:\n{scorecard}"))
}

#[test]
fn the_scorecard_reads_the_latest_real_phone_result_and_never_a_demo() {
    let results = tempfile::tempdir().expect("a results directory");
    let mut runner = Runner::start(&["2"], results.path());
    runner.play(|_| "y".to_owned());

    // A run against the Simulated Phone is no evidence about the real one.
    let demo = scorecard(results.path());
    assert!(row(&demo, 3).ends_with("no evidence yet"), "{demo}");
    assert!(row(&demo, 5).ends_with("no evidence yet"), "{demo}");

    // The same file, as if the real Phone had produced it.
    let file = std::fs::read_dir(results.path())
        .expect("list the results")
        .next()
        .expect("a result file")
        .expect("a result file")
        .path();
    let mut result = result_file(results.path());
    result["mode"] = "real".into();
    std::fs::write(&file, result.to_string()).expect("rewrite the result file");

    let real = scorecard(results.path());
    let codec = row(&real, 3);
    assert!(codec.contains("PASS"), "{real}");
    assert!(codec.contains("lc3-swb") && codec.contains("2.1"), "{real}");
    let checklist = row(&real, 5);
    assert!(
        checklist.contains("FAIL"),
        "the scenarios aren't counted yet: {real}"
    );
    assert!(
        checklist.contains("Real-Phone Checklist")
            && checklist.contains("8 pass, 0 fail, 0 skipped"),
        "{real}"
    );
}

#[test]
fn an_interrupted_run_resumes_at_the_failed_step_and_keeps_what_passed() {
    let results = tempfile::tempdir().expect("a results directory");

    // The helper didn't hear clearly in 2.1, 2.2 passes, and the user stops
    // as 2.3 starts.
    let mut first = Runner::start(&["2"], results.path());
    first.quit_at = Some("2.3");
    first.play(|_| "n".to_owned());
    assert_eq!(first.steps_started(), ["2.1", "2.2", "2.3"]);
    let after_first = result_file(results.path());
    assert_eq!(step(&after_first, "2.1")["result"], "fail");
    assert_eq!(step(&after_first, "2.2")["result"], "pass");

    let mut second = Runner::start(&["2"], results.path());
    second.play(|_| "y".to_owned());
    assert!(
        second.printed("Resuming"),
        "{}",
        second.transcript.join("\n")
    );
    assert_eq!(
        second.steps_started(),
        ["2.1", "2.3", "2.4", "2.5", "2.6", "2.7", "2.8"],
        "the failed step and those not yet run, but not 2.2:\n{}",
        second.transcript.join("\n")
    );
    let after_second = result_file(results.path());
    for id in ["2.1", "2.2", "2.3", "2.4", "2.5", "2.6", "2.7", "2.8"] {
        assert_eq!(
            step(&after_second, id)["result"],
            "pass",
            "{after_second:#}"
        );
    }
    assert_eq!(after_second["started"], after_first["started"]);
}

#[test]
fn group_1_reads_a_second_page_of_call_history_and_reports_whether_the_phone_honours_paging() {
    let results = tempfile::tempdir().expect("a results directory");
    let mut runner = Runner::start(&["1"], results.path());
    let questions = runner.play(|_| "y".to_owned());

    // Paging is read from the Phone, not asked of the user.
    assert!(questions.is_empty(), "{}", runner.transcript.join("\n"));
    let result = result_file(results.path());
    let paging = step(&result, "1.4");
    assert_eq!(paging["result"], "pass", "{paging:#}");
    // The demo's Phone holds 120 Calls, newest first, and serves at most 50
    // per pull: Offset 50 gives entries 51 to 100, none of them on page 1,
    // all older than page 1's.
    let measured = &paging["measurements"];
    assert_eq!(measured["page_2_entries"], 50, "{paging:#}");
    assert_eq!(measured["page_2_entries_also_on_page_1"], 0, "{paging:#}");
    assert_eq!(measured["page_2_older_than_page_1"], true, "{paging:#}");
    for id in ["1.1", "1.2", "1.3"] {
        assert_eq!(step(&result, id)["result"], "skipped", "{result:#}");
    }
}

#[test]
fn step_7_3_sees_the_indicator_open_the_dialer_and_its_menu_mute_and_hang_up() {
    let results = tempfile::tempdir().expect("a results directory");
    let mut runner = Runner::start(&["7"], results.path());
    let questions = runner.play(|_| "y".to_owned());

    // Only what the user sees in Omarchy's bar is asked.
    let asked: Vec<&String> = questions
        .iter()
        .filter(|question| question.contains("icon"))
        .collect();
    assert_eq!(asked.len(), 3, "{}", runner.transcript.join("\n"));
    let result = result_file(results.path());
    let indicator = step(&result, "7.3");
    assert_eq!(indicator["result"], "pass", "{indicator:#}");
    let evidence = indicator["evidence"].to_string();
    for seen in [
        "opened the Dialer",
        "muted the microphone",
        "unmuted the microphone",
        "ended the Call",
    ] {
        assert!(evidence.contains(seen), "{seen}: {indicator:#}");
    }

    // Step 7.4 times the Dialer by the app's log; whether it meets 150 ms is
    // judged on the user's Desktop, not in this demo.
    let dialer = step(&result, "7.4");
    assert!(
        dialer["measurements"]["dialer_first_frame_ms"].is_u64(),
        "{dialer:#}"
    );
    assert!(
        dialer["evidence"]
            .to_string()
            .contains("drew its first frame"),
        "{dialer:#}"
    );
}

/// What the runner listed under "Before you start", in order.
fn needs_listed(runner: &Runner) -> Vec<String> {
    runner
        .transcript
        .iter()
        .skip_while(|line| !line.starts_with("Before you start"))
        .skip(1)
        .map_while(|line| line.strip_prefix("  - ").map(str::to_owned))
        .collect()
}

#[test]
fn before_a_group_starts_the_runner_lists_only_what_the_steps_about_to_run_need() {
    let results = tempfile::tempdir().expect("a results directory");
    // 3.2 needs a helper and 3.3 a number with a keypad menu. The demo
    // can't place a Call from the Phone's side, so 3.3 is skipped.
    let mut first = Runner::start(&["3"], results.path());
    first.play(|_| "y".to_owned());
    let listed = needs_listed(&first);
    assert!(
        listed.iter().any(|need| need.contains("helper"))
            && listed.iter().any(|need| need.contains("keypad menu")),
        "{}",
        first.transcript.join("\n")
    );
    assert_eq!(step(&result_file(results.path()), "3.2")["result"], "pass");

    // The rerun runs only 3.3, so no helper is asked for.
    let mut second = Runner::start(&["3"], results.path());
    second.play(|_| "y".to_owned());
    assert_eq!(second.steps_started(), ["3.3"]);
    let listed = needs_listed(&second);
    assert!(
        listed.iter().any(|need| need.contains("keypad menu"))
            && !listed.iter().any(|need| need.contains("helper")),
        "{}",
        second.transcript.join("\n")
    );
}

#[test]
fn real_phone_results_live_in_the_users_state_directory_not_the_repository() {
    // Results come from the real Phone and can hold real data, so they stay
    // out of the repository: under $XDG_STATE_HOME (ticket #120).
    let home = tempfile::tempdir().expect("a temporary home");
    let results = home.path().join("state/desktop-calls/real-phone-checklist");
    std::fs::create_dir_all(&results).expect("the results directory");
    std::fs::write(
        results.join("2026-11-05T13-25-50Z.json"),
        r#"{"mode": "real", "started": "2026-11-05T13:25:50Z", "phone": "Pixel",
            "steps": [{"id": "2.1", "title": "Answer on the Desktop", "result": "pass",
                       "evidence": [], "measurements": {"codec": "msbc", "wideband": true}}]}"#,
    )
    .expect("a result file");

    let printed = Command::new(env!("CARGO_BIN_EXE_automation"))
        .arg("scorecard")
        .current_dir(home.path())
        .env("HOME", home.path())
        .env("XDG_STATE_HOME", home.path().join("state"))
        .output()
        .expect("run automation scorecard");
    assert!(printed.status.success(), "{printed:?}");
    let scorecard = String::from_utf8(printed.stdout).expect("the scorecard is text");
    let codec = row(&scorecard, 3);
    assert!(
        codec.contains("PASS") && codec.contains("msbc"),
        "{scorecard}"
    );
}
