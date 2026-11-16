//! `automation real-phone-checklist` against an installed service that
//! logs no BlueZ method calls offers to turn them on before the first step
//! (#427). On yes it sets the switch in the user manager's environment,
//! restarts the service and, on leaving by quit or Ctrl-C, takes the switch
//! out again; on no it carries on and leaves the service alone.

#![expect(
    clippy::expect_used,
    reason = "test code: a failed step fails the test with its message"
)]

use std::io::{Read as _, Write as _};
use std::process::{Child, ChildStdin, Stdio};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use test_harness::Harness;

const DEADLINE: Duration = Duration::from_secs(30);
const SWITCH: &str = "RUST_LOG=info,bluez_dbus=debug";

/// The installed service, started as the user's session starts it: without
/// `RUST_LOG`, so it logs no BlueZ method calls.
fn installed_service() -> Harness {
    let harness = Harness::builder().installed().log_filter(None).start();
    harness.simulated_phone().connect();
    // Starts the service, through D-Bus activation.
    let _ = harness.cli(&["status"]);
    harness.wait_for_app("the Phone is connected", DEADLINE, |state| {
        state.connection_state == "connected"
    });
    harness.wait_for_service("active");
    harness
}

/// The runner on group 6, under `script` for the terminal it needs, with
/// what it says gathered as it says it.
struct Runner {
    child: Child,
    input: ChildStdin,
    said: Arc<Mutex<String>>,
}

impl Runner {
    fn start(harness: &Harness) -> Self {
        let runner = format!(
            "{} real-phone-checklist 6 --results {}",
            env!("CARGO_BIN_EXE_automation"),
            harness.directory().join("results").display()
        );
        let mut child = harness
            .external_command("script")
            .args(["--quiet", "--return", "--command", &runner, "/dev/null"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .expect("run the checklist under a pseudo-terminal");
        let input = child.stdin.take().expect("the runner's input");
        let mut output = child.stdout.take().expect("the runner's output");
        let said = Arc::new(Mutex::new(String::new()));
        let gathered = said.clone();
        thread::spawn(move || {
            let mut buffer = [0; 4096];
            while let Ok(read) = output.read(&mut buffer) {
                if read == 0 {
                    break;
                }
                let text = String::from_utf8_lossy(&buffer[..read]).replace('\r', "");
                gathered.lock().expect("unpoisoned").push_str(&text);
            }
        });
        Self { child, input, said }
    }

    fn said(&self) -> String {
        self.said.lock().expect("unpoisoned").clone()
    }

    fn wait_to_say(&self, text: &str) {
        let started = Instant::now();
        while !self.said().contains(text) {
            assert!(
                started.elapsed() < DEADLINE,
                "the runner didn't say {text:?}: {}",
                self.said()
            );
            thread::sleep(Duration::from_millis(50));
        }
    }

    fn type_in(&mut self, text: &str) {
        self.input
            .write_all(text.as_bytes())
            .expect("type into the runner");
    }

    /// Waits for the runner to end, and returns all it said.
    fn finish(mut self) -> String {
        let started = Instant::now();
        while self
            .child
            .try_wait()
            .expect("the runner's status")
            .is_none()
        {
            assert!(
                started.elapsed() < DEADLINE,
                "the runner didn't end: {}",
                self.said()
            );
            thread::sleep(Duration::from_millis(50));
        }
        thread::sleep(Duration::from_millis(200));
        self.said()
    }
}

/// Answers the offer yes, and waits until the service is back in a new
/// process with the switch on, and the runner is at the first step.
fn turn_on(harness: &Harness) -> Runner {
    let before = harness.service_process_id();
    let mut runner = Runner::start(harness);
    runner.wait_to_say("Turn them on for this sitting?");
    runner.type_in("y\n");
    runner.wait_to_say("On: Desktop Calls restarted with its BlueZ method calls logged.");
    assert!(
        harness
            .service_manager_environment()
            .contains(&SWITCH.to_owned()),
        "{:?}",
        harness.service_manager_environment()
    );
    let after = harness.service_process_id().expect("the service runs");
    assert_ne!(Some(after), before, "the service restarted");
    let environment = std::fs::read(format!("/proc/{after}/environ")).expect("its environment");
    assert!(
        environment
            .split(|byte| *byte == 0)
            .any(|variable| variable == SWITCH.as_bytes()),
        "the restarted service has the switch"
    );
    runner
}

fn assert_switched_off(harness: &Harness, said: &str) {
    assert!(
        said.contains("The app's BlueZ method lines are off again from Desktop Calls' next start"),
        "{said}"
    );
    assert!(
        !harness
            .service_manager_environment()
            .iter()
            .any(|each| each.starts_with("RUST_LOG=")),
        "{:?}",
        harness.service_manager_environment()
    );
}

#[test]
fn on_yes_the_switch_is_on_for_the_sitting_and_off_again_on_quit() {
    let harness = installed_service();
    let mut runner = turn_on(&harness);
    runner.type_in("quit\n");
    let said = runner.finish();
    assert_switched_off(&harness, &said);
}

#[test]
fn on_yes_then_ctrl_c_the_switch_is_off_again() {
    let harness = installed_service();
    let mut runner = turn_on(&harness);
    // Ctrl-C, as the terminal sends it.
    runner.type_in("\u{3}");
    let said = runner.finish();
    assert_switched_off(&harness, &said);
}

#[test]
fn on_no_the_service_is_left_alone_and_the_lines_are_not_captured() {
    let harness = installed_service();
    let before = harness.service_process_id();
    let mut runner = Runner::start(&harness);
    runner.wait_to_say("Turn them on for this sitting?");
    runner.type_in("n\n");
    runner.wait_to_say("Press Enter to start group 6");
    runner.type_in("\n");
    // Quit inside step 6.1, which keeps its capture.
    runner.wait_to_say("Take the Pixel out of range");
    thread::sleep(Duration::from_millis(500));
    runner.type_in("quit\n");
    let said = runner.finish();
    assert!(
        said.contains("Not captured: the app's BlueZ method calls"),
        "{said}"
    );
    assert!(!said.contains("lines are off again"), "{said}");
    assert_eq!(harness.service_process_id(), before, "not restarted");
    assert!(harness.service_manager_environment().is_empty());
    // The step quit part way keeps its capture, which says so at its top.
    let captures = std::fs::read_dir(harness.directory().join("results"))
        .unwrap_or_else(|error| panic!("the results: {error}: {said}"))
        .map(|entry| entry.expect("an entry").path())
        .find(|path| path.extension().is_some_and(|extension| extension == "bus"))
        .expect("the captures' directory");
    let step = std::fs::read_dir(&captures)
        .expect("the captures")
        .map(|entry| std::fs::read_to_string(entry.expect("an entry").path()).expect("a capture"))
        .next()
        .expect("step 6.1's capture");
    assert!(
        step.contains("Not captured: the app's BlueZ method calls: the app wasn't started with"),
        "{step}"
    );
}
