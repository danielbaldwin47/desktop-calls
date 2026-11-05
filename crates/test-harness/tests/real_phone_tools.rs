//! Scenarios: the Real-Phone tools never run the app beside the installed
//! `desktop-calls.service` (ticket #123, ADR-0006). `just test-call` and
//! `just run` stop the service for their run, and start it again when the
//! run ends, even when the tool is killed outright.
//!
//! The tools run against the Simulated Phone, with the harness's stand-in
//! for the systemd user manager running the installed service, so the
//! user's real service is never touched.

use std::ffi::OsString;
use std::fs;
use std::path::PathBuf;
use std::process::{Child, ExitStatus, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use test_harness::{AppEvent, Harness};

/// How long a step may take: the Phone, a tool or the stand-in reacting.
const DEADLINE: Duration = Duration::from_secs(10);
/// How long a tool may take to finish once its run ends: hanging up, then
/// starting the service again.
const FINISH_DEADLINE: Duration = Duration::from_secs(40);

/// An installed scenario, with the Phone connected and the installed
/// service running.
fn service_running() -> Harness {
    let harness = Harness::builder().phone_connected().installed().start();
    harness.start_service();
    harness.wait_for_app("ConnectionState is connected", DEADLINE, |state| {
        state.connection_state == "connected"
    });
    harness
}

/// A file whose appearance ends a hook's dev run.
fn done_file(harness: &Harness) -> PathBuf {
    harness.directory().join("hook-done")
}

/// A `just test-call` hook that runs a dev run of the app, as an agent's
/// hook would, until [`done_file`] appears.
fn dev_run_hook(harness: &Harness) -> String {
    format!(
        "{app} service & app=$!; until [ -e {done} ]; do sleep 0.1; done; kill $app; wait $app",
        app = harness.binary("desktop-calls").display(),
        done = done_file(harness).display(),
    )
}

/// Starts the Real-Phone tool `script` with `arguments`, logging to
/// `<name>.stdout` and `<name>.stderr` in the scenario.
fn start_tool(harness: &Harness, script: &str, name: &str, arguments: &[OsString]) -> Child {
    let log = |extension: &str| {
        let path = harness.directory().join(format!("{name}.{extension}"));
        fs::File::create(&path).unwrap_or_else(|error| panic!("create {}: {error}", path.display()))
    };
    harness
        .real_phone_tool(script)
        .args(arguments)
        .stdin(Stdio::null())
        .stdout(log("stdout"))
        .stderr(log("stderr"))
        .spawn()
        .unwrap_or_else(|error| panic!("start {script}: {error}"))
}

/// Starts `just test-call` with `arguments`.
fn start_test_call(harness: &Harness, arguments: &[&str]) -> Child {
    let arguments: Vec<OsString> = arguments.iter().map(OsString::from).collect();
    start_tool(harness, "test_call.py", "test-call", &arguments)
}

/// Waits until the Simulated Phone has the Test Call, and the person
/// called answers it.
fn callee_answers(harness: &Harness) {
    harness.simulated_phone().wait_for_recorded(
        "the Phone dials the Test Number",
        DEADLINE,
        |recorded| recorded.iter().any(|call| call.member == "Dial"),
    );
    let _ = harness.simulated_phone().alert();
    harness.simulated_phone().callee_answers();
}

/// Waits until a dev run, not the installed service, is on the bus.
fn dev_run_on_bus(harness: &Harness, service: Option<u32>) {
    wait_until("a dev run of the app is on the bus", || {
        harness
            .app_process_id()
            .is_some_and(|pid| Some(pid) != service)
    });
}

fn wait_until(what: &str, condition: impl Fn() -> bool) {
    let started = Instant::now();
    while !condition() {
        assert!(
            started.elapsed() < DEADLINE,
            "{what}: not within {DEADLINE:?}"
        );
        thread::sleep(Duration::from_millis(20));
    }
}

fn wait_for_exit(harness: &Harness, child: &mut Child, what: &str) -> ExitStatus {
    let started = Instant::now();
    loop {
        if let Some(status) = child
            .try_wait()
            .unwrap_or_else(|error| panic!("ask whether {what} exited: {error}"))
        {
            return status;
        }
        assert!(
            started.elapsed() < FINISH_DEADLINE,
            "{what} didn't finish within {FINISH_DEADLINE:?}; see {}",
            harness.directory().display()
        );
        thread::sleep(Duration::from_millis(50));
    }
}

/// Nothing was ever refused for running beside another copy.
fn never_two_apps(harness: &Harness) {
    assert!(
        !harness
            .app_logged(&AppEvent::kind("service-failed").with("error_kind", "already-running")),
        "an app started beside another"
    );
}

#[test]
fn test_call_runs_its_hook_with_the_installed_service_stopped_and_starts_it_again() {
    let harness = service_running();
    let service = harness.service_process_id();
    let hook = dev_run_hook(&harness);

    let mut test_call = start_test_call(&harness, &["--max-seconds", "30", &hook]);
    callee_answers(&harness);
    dev_run_on_bus(&harness, service);
    assert_eq!(
        harness.service_state(),
        "inactive",
        "the installed service is stopped while the dev run is up"
    );
    harness.wait_for_app("the dev run shows the Test Call", DEADLINE, |state| {
        state.calls.len() == 1
    });
    fs::write(done_file(&harness), "").expect("end the hook");

    let status = wait_for_exit(&harness, &mut test_call, "test_call.py");
    assert!(status.success(), "test_call.py: {status}");
    assert_eq!(harness.service_state(), "active", "the service runs again");
    assert_eq!(
        harness.app_process_id(),
        harness.service_process_id(),
        "the service, not the dev run, is on the bus"
    );
    assert_eq!(
        harness.service_history(),
        [
            "activating",
            "active",
            "deactivating",
            "inactive",
            "activating",
            "active"
        ],
        "the service was stopped once, and started once again"
    );
    assert!(harness.simulated_phone().calls_on_phone().is_empty());
    never_two_apps(&harness);
}

#[test]
fn a_killed_test_call_still_starts_the_installed_service_again() {
    let harness = service_running();
    let service = harness.service_process_id();
    let hook = dev_run_hook(&harness);
    let mut test_call = start_test_call(&harness, &["--max-seconds", "30", &hook]);
    callee_answers(&harness);
    dev_run_on_bus(&harness, service);

    test_call.kill().expect("kill test_call.py");
    let _ = test_call.wait();

    // Its guard hangs up, stops the hook's dev run, and starts the service.
    harness.wait_for_service_restart(service);
    wait_until("the service is on the bus", || {
        harness.app_process_id() == harness.service_process_id()
    });
    wait_until("the Test Call is hung up", || {
        harness.simulated_phone().calls_on_phone().is_empty()
    });
    never_two_apps(&harness);
}

#[test]
fn test_call_leaves_a_stopped_service_stopped() {
    let harness = Harness::builder().phone_connected().installed().start();

    let mut test_call = start_test_call(&harness, &["--max-seconds", "30", "true"]);
    callee_answers(&harness);

    let status = wait_for_exit(&harness, &mut test_call, "test_call.py");
    assert!(status.success(), "test_call.py: {status}");
    assert_eq!(harness.service_history(), Vec::<&str>::new());
}

/// Starts `just run`'s script on the scenario's app binary.
fn start_run(harness: &Harness) -> Child {
    let binary = harness.binary("desktop-calls").into_os_string();
    start_tool(harness, "run.py", "run", &[binary])
}

#[test]
fn just_run_runs_the_dev_build_in_place_of_the_installed_service_until_stopped() {
    let harness = service_running();
    let service = harness.service_process_id();

    let mut run = start_run(&harness);
    dev_run_on_bus(&harness, service);
    assert_eq!(harness.service_state(), "inactive");
    harness.wait_for_app("the dev run follows the Phone", DEADLINE, |state| {
        state.connection_state == "connected"
    });
    // Ctrl-C, as the user stops it.
    let interrupted = harness
        .external_command("kill")
        .arg("-INT")
        .arg(run.id().to_string())
        .status()
        .expect("send SIGINT to run.py");
    assert!(interrupted.success());

    wait_for_exit(&harness, &mut run, "run.py");
    assert_eq!(harness.service_state(), "active", "the service runs again");
    assert_eq!(harness.app_process_id(), harness.service_process_id());
    never_two_apps(&harness);
}

#[test]
fn a_killed_just_run_still_starts_the_installed_service_again() {
    let harness = service_running();
    let service = harness.service_process_id();
    let mut run = start_run(&harness);
    dev_run_on_bus(&harness, service);

    run.kill().expect("kill run.py");
    let _ = run.wait();

    harness.wait_for_service_restart(service);
    wait_until("the service is on the bus", || {
        harness.app_process_id() == harness.service_process_id()
    });
    never_two_apps(&harness);
}

#[test]
fn just_run_refuses_while_the_phone_has_a_call() {
    let harness = service_running();
    let path = harness.simulated_phone().ring("+447700900123");
    harness.simulated_phone().answer_on_phone(&path);
    harness.wait_for_app("the Call is live", DEADLINE, |state| state.calls.len() == 1);

    let mut run = start_run(&harness);

    let status = wait_for_exit(&harness, &mut run, "run.py");
    assert!(!status.success(), "run.py refused");
    assert_eq!(
        harness.service_history(),
        ["activating", "active"],
        "the service was never stopped"
    );
}
