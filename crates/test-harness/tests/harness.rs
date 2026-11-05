//! The harness's own promises: a failing scenario keeps its logs, a passing
//! one leaves nothing behind unless asked to keep it, and nothing a scenario
//! started outlives its test process, however that process ends.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use test_harness::{Harness, KEEP_SCENARIO_LOGS, keep_scenario_logs};

#[test]
fn a_failing_scenario_keeps_its_logs() {
    let (directory_sender, directory) = mpsc::channel();
    let scenario = thread::spawn(move || {
        let harness = Harness::start();
        harness.wait_for_app("the Phone is known", Duration::from_secs(5), |state| {
            state.phone.is_some()
        });
        directory_sender
            .send(harness.directory().to_owned())
            .expect("the test is listening");
        panic!("this scenario fails on purpose");
    });

    assert!(scenario.join().is_err(), "the scenario should have failed");
    let directory: PathBuf = directory.recv().expect("the scenario started");
    let log = fs::read_to_string(directory.join("state/desktop-calls/desktop-calls.log"))
        .expect("the app's log file is kept");
    assert!(
        log.contains(r#""event_kind":"service-started""#),
        "the kept log holds the app's events: {log}"
    );
    fs::remove_dir_all(&directory).expect("clean up the kept logs");
}

/// Unless `KEEP_SCENARIO_LOGS` is set: then it keeps its directory, as a
/// failing scenario does.
#[test]
fn a_passing_scenario_leaves_nothing_behind() {
    let harness = Harness::start();
    let directory = harness.directory().to_owned();

    drop(harness);

    if keep_scenario_logs() {
        assert!(
            directory
                .join("state/desktop-calls/desktop-calls.log")
                .exists(),
            "{KEEP_SCENARIO_LOGS} is set, so {} keeps its logs",
            directory.display()
        );
        fs::remove_dir_all(&directory).expect("clean up the kept logs");
    } else {
        assert!(
            !directory.exists(),
            "{} was left behind",
            directory.display()
        );
    }
}

/// Where [`a_scenario_to_kill`] writes its directory, once it has started.
const REPORT: &str = "SCENARIO_TO_KILL_REPORT";

/// As a timeout or a stopped run does: SIGKILL a test process mid-scenario.
#[test]
fn nothing_a_scenario_started_outlives_its_killed_test_process() {
    let report = tempfile::NamedTempFile::new().expect("create the report file");
    let mut test_process = Command::new(std::env::current_exe().expect("find this test"))
        .args(["a_scenario_to_kill", "--exact", "--ignored"])
        .env(REPORT, report.path())
        .stdout(Stdio::null())
        .spawn()
        .expect("run the scenario to kill");
    let started = Instant::now();
    let directory = loop {
        let written = fs::read_to_string(report.path()).unwrap_or_default();
        if !written.is_empty() {
            break PathBuf::from(written);
        }
        assert!(
            started.elapsed() < Duration::from_secs(30),
            "the scenario to kill did not start within 30s"
        );
        thread::sleep(Duration::from_millis(50));
    };
    let running = processes_of(&directory);
    assert!(
        running.len() >= 5,
        "the scenario runs a guard, two buses, sway, the Simulated Phone and the app: {running:#?}"
    );

    test_process.kill().expect("SIGKILL the test process");
    test_process.wait().expect("reap the test process");

    let killed = Instant::now();
    let mut left = processes_of(&directory);
    while !left.is_empty() && killed.elapsed() < Duration::from_secs(5) {
        thread::sleep(Duration::from_millis(50));
        left = processes_of(&directory);
    }
    let _ = fs::remove_dir_all(&directory);
    assert!(
        left.is_empty(),
        "5s after the SIGKILL, still running: {left:#?}"
    );
}

#[test]
#[ignore = "only nothing_a_scenario_started_outlives_its_killed_test_process runs this"]
fn a_scenario_to_kill() {
    let Some(report) = std::env::var_os(REPORT) else {
        return;
    };
    let harness = Harness::builder().headless_screen().start();
    fs::write(report, harness.directory().as_os_str().as_encoded_bytes())
        .expect("report the scenario's directory");
    // Killed long before this ends; the limit only stops a stray run.
    thread::sleep(Duration::from_secs(60));
}

/// Every running process whose command line or environment names
/// `directory`, as `pid: command line`.
fn processes_of(directory: &Path) -> Vec<String> {
    let needle = directory.as_os_str().as_encoded_bytes();
    let names = |bytes: &[u8]| bytes.windows(needle.len()).any(|window| window == needle);
    let mut found = Vec::new();
    // An unreadable /proc finds nothing, which the check before the kill catches.
    for entry in fs::read_dir("/proc").into_iter().flatten().flatten() {
        let path = entry.path();
        if !entry
            .file_name()
            .to_string_lossy()
            .bytes()
            .all(|byte| byte.is_ascii_digit())
        {
            continue;
        }
        let command_line = fs::read(path.join("cmdline")).unwrap_or_default();
        let environment = fs::read(path.join("environ")).unwrap_or_default();
        if names(&command_line) || names(&environment) {
            let shown = String::from_utf8_lossy(&command_line).replace('\0', " ");
            found.push(format!("{}: {shown}", entry.file_name().to_string_lossy()));
        }
    }
    found
}
