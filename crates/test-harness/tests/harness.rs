//! The harness's own promises: a failing scenario keeps its logs, a passing
//! one leaves nothing behind.

use std::path::PathBuf;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use test_harness::Harness;

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
    let log = std::fs::read_to_string(directory.join("state/desktop-calls/desktop-calls.log"))
        .expect("the app's log file is kept");
    assert!(
        log.contains(r#""event_kind":"service-started""#),
        "the kept log holds the app's events: {log}"
    );
    std::fs::remove_dir_all(&directory).expect("clean up the kept logs");
}

#[test]
fn a_passing_scenario_leaves_nothing_behind() {
    let harness = Harness::start();
    let directory = harness.directory().to_owned();

    drop(harness);

    assert!(
        !directory.exists(),
        "{} was left behind",
        directory.display()
    );
}
