//! Stopping the app while a state that lasts only a moment is captured
//! (#248).
//!
//! The app shows some states for a fixed time by design: the Banner says
//! "Identifying caller…" for 2 s, and the Call Pill says "Call ended" for
//! 2 s. On a loaded CI runner, frames of the 3840 by 2160 screen came about
//! 2 s apart, so no frame fell inside the state, and the gallery failed.
//!
//! The harness stops the app's process with SIGSTOP, as a debugger would,
//! once the screen has finished drawing the state, and starts it again with
//! SIGCONT after the capture. Stopped, the app runs no timer and draws
//! nothing, and the compositor keeps showing its last frame. Nothing in the
//! app changes: it has no way to be held, so an installed run can't be, and
//! the states last their real time everywhere but in a capture.
//!
//! A timer the app starts in answer to something the harness did can't
//! fire before that, plus its duration. So the app stopped before that
//! instant still shows the state: the caller names the instant, and the
//! freeze fails, rather than capturing a later state, if it comes too late.

use std::fs;
use std::process::Command;
use std::thread;
use std::time::{Duration, Instant};

use super::Harness;

/// How long the app's threads may take to stop once sent SIGSTOP.
const STOP_DEADLINE: Duration = Duration::from_secs(5);

impl Harness {
    /// Stops the app's process until [`Harness::thaw_app`], or the end of
    /// the scenario, once the screen has finished redrawing.
    ///
    /// # Panics
    ///
    /// If the app has no process on the bus, if it can't be stopped, or if
    /// it was stopped at or after `before`: a timer the caller says can't
    /// fire before `before` may have fired by then.
    pub fn freeze_app_before(&self, before: Instant, what: &str) {
        self.wait_for_redraw();
        let pid = self
            .app_process_id()
            .expect("the app owns its name on the session bus");
        signal(pid, "-STOP");
        *self
            .frozen
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(pid);
        let stopped = wait_until_stopped(pid);
        self.timeline.record(
            "harness",
            format!("stopped the app (pid {pid}) to capture {what}"),
        );
        if let Some(late) = stopped.checked_duration_since(before) {
            self.thaw_app();
            panic!(
                "the app was stopped {late:?} after {what} could have ended, so a capture \
                 might show what came after it; the screen took that long to draw it"
            );
        }
    }

    /// Starts the app again after [`Harness::freeze_app_before`]: its timers
    /// that came due while it was stopped fire at once.
    pub fn thaw_app(&self) {
        let pid = self
            .frozen
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take();
        if let Some(pid) = pid {
            signal(pid, "-CONT");
            self.timeline
                .record("harness", format!("started the app (pid {pid}) again"));
        }
    }

    /// Whether the app is stopped, so its control interface can't answer.
    #[must_use]
    pub fn app_is_frozen(&self) -> bool {
        self.frozen
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .is_some()
    }
}

fn signal(pid: u32, signal: &str) {
    let sent = Command::new("kill")
        .arg(signal)
        .arg(pid.to_string())
        .status();
    assert!(
        sent.is_ok_and(|status| status.success()),
        "kill {signal} {pid} failed"
    );
}

/// When every thread of `pid` was first seen stopped. A thread seen stopped
/// was stopped a moment before, so the instant is late, never early.
fn wait_until_stopped(pid: u32) -> Instant {
    let give_up = Instant::now() + STOP_DEADLINE;
    loop {
        if every_thread_stopped(pid) {
            return Instant::now();
        }
        assert!(
            Instant::now() < give_up,
            "the app (pid {pid}) didn't stop within {STOP_DEADLINE:?} of SIGSTOP"
        );
        thread::sleep(Duration::from_millis(1));
    }
}

/// Whether each thread of `pid` is in state `T` (stopped by a signal).
fn every_thread_stopped(pid: u32) -> bool {
    let Ok(threads) = fs::read_dir(format!("/proc/{pid}/task")) else {
        return false;
    };
    threads.flatten().all(|thread| {
        fs::read_to_string(thread.path().join("stat")).is_ok_and(|stat| {
            // The state follows the command name, which is in parentheses
            // and may itself hold spaces or parentheses.
            stat.rsplit_once(") ")
                .and_then(|(_, rest)| rest.chars().next())
                .is_some_and(|state| state == 'T')
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Stdio;

    #[test]
    fn a_stopped_process_is_seen_stopped_and_a_running_one_is_not() {
        let mut child = Command::new("sleep")
            .arg("30")
            .stdout(Stdio::null())
            .spawn()
            .expect("start sleep");
        let pid = child.id();
        assert!(
            !every_thread_stopped(pid),
            "a sleeping process isn't stopped"
        );
        signal(pid, "-STOP");
        let seen = wait_until_stopped(pid);
        assert!(seen.elapsed() < STOP_DEADLINE);
        signal(pid, "-CONT");
        let give_up = Instant::now() + STOP_DEADLINE;
        while every_thread_stopped(pid) && Instant::now() < give_up {
            thread::sleep(Duration::from_millis(1));
        }
        assert!(!every_thread_stopped(pid), "SIGCONT started it again");
        let _ = child.kill();
        let _ = child.wait();
    }
}
