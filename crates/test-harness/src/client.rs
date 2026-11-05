//! The harness's own Wayland client, `test-client`, which shows a window and
//! prints every keyboard and pointer event it receives, one per line. The
//! harness self-tests drive it to prove that synthetic input arrives.

use std::io::{BufRead as _, BufReader};
use std::process::{Child, ChildStdout};
use std::sync::{Arc, Condvar, Mutex, PoisonError};
use std::thread;
use std::time::{Duration, Instant};

use crate::timeline::Timeline;

/// The lines a running `test-client` has printed so far.
#[derive(Default)]
struct Printed {
    lines: Mutex<Vec<String>>,
    changed: Condvar,
}

/// A running `test-client`. Dropping it stops the client.
pub struct TestClient {
    child: Child,
    printed: Arc<Printed>,
    timeline: Arc<Timeline>,
}

impl TestClient {
    pub(crate) fn new(mut child: Child, timeline: Arc<Timeline>) -> Self {
        let printed = Arc::new(Printed::default());
        if let Some(stdout) = child.stdout.take() {
            let printed = printed.clone();
            thread::spawn(move || read_lines(stdout, &printed));
        }
        Self {
            child,
            printed,
            timeline,
        }
    }

    /// Every line the client has printed so far, in order.
    #[must_use]
    pub fn lines(&self) -> Vec<String> {
        self.printed
            .lines
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// Waits until the client prints a line that satisfies `condition`, and
    /// returns it.
    ///
    /// # Panics
    ///
    /// If `deadline` passes first. The panic names `description` and every
    /// line printed.
    pub fn wait_for(
        &self,
        description: &str,
        deadline: Duration,
        condition: impl Fn(&str) -> bool,
    ) -> String {
        let end = Instant::now() + deadline;
        let mut lines = self
            .printed
            .lines
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        loop {
            if let Some(line) = lines.iter().find(|line| condition(line)) {
                return line.clone();
            }
            let left = end.saturating_duration_since(Instant::now());
            assert!(
                !left.is_zero(),
                "\"{description}\" did not happen within {deadline:?}; the test client printed {lines:#?}{}",
                self.timeline.render()
            );
            lines = self
                .printed
                .changed
                .wait_timeout(lines, left)
                .unwrap_or_else(PoisonError::into_inner)
                .0;
        }
    }
}

impl Drop for TestClient {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn read_lines(stdout: ChildStdout, printed: &Printed) {
    for line in BufReader::new(stdout).lines() {
        let Ok(line) = line else { break };
        printed
            .lines
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(line);
        printed.changed.notify_all();
    }
}
