//! Stopping the app while a state that lasts only a moment is captured
//! (#248, #384).
//!
//! The app shows some states for a fixed time by design: the Banner says
//! "Identifying caller…" for 2 s, the Call Pill says "Call ended" for 2 s,
//! and the audio tile says "Moving…" for at most the 3 s a Handoff may
//! take. On a loaded CI runner, frames of the 3840 by 2160 screen came about
//! 2 s apart, so no frame fell inside the state, and the gallery failed.
//!
//! The harness stops the app's process with SIGSTOP, as a debugger would,
//! on the first frame of the screen that shows the state, and starts it
//! again with SIGCONT after the capture. Stopped, the app runs no timer and
//! draws nothing, and the compositor keeps showing its last frame. Nothing
//! in the app changes: it has no way to be held, so an installed run can't
//! be, and the states last their real time everywhere but in a capture.
//!
//! Until #384 the app was stopped once the screen had held still for 100 ms
//! instead. A screen with anything moving on it never does: the audio
//! tile's bar steps on twice a second with reduced motion, and on CI's two
//! CPUs the screen first held still 2.2 s into a state the gallery allowed
//! 2 s. A frame, by contrast, is whole: the compositor shows a surface's
//! commits one at a time. So the first frame that shows the state is the
//! picture, and the state's whole time is the margin.
//!
//! A frame shows the state only by what the caller's check sees in it, so
//! the check must tell the state from the screen before it. The freeze
//! proves that it does, on the screen as it was just before the state, and
//! fails if it doesn't.
//!
//! A timer the app starts in answer to something the harness did can't
//! fire before that, plus its duration. So the app stopped before that
//! instant still shows the state: the caller names the instant, and the
//! freeze fails, rather than capturing a later state, if it comes too late.
//! The screen can't show the state after it either: its next state can't
//! be drawn before that instant.

use std::fs;
use std::process::Command;
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use super::Harness;
use crate::Screenshot;
use crate::screencopy::Recorder;

/// How long the app's threads may take to stop once sent SIGSTOP.
const STOP_DEADLINE: Duration = Duration::from_secs(5);

impl Harness {
    /// Brings the app into a state that lasts only a moment with `reach`,
    /// and stops the app's process, until [`Harness::thaw_app`] or the end
    /// of the scenario, on the first frame of the screen that satisfies
    /// `shown`. `reach` returns the instant before which the state can't
    /// end. `what` names the state.
    ///
    /// # Panics
    ///
    /// If the screen as it is before `reach`, once redrawn, already
    /// satisfies `shown`, so `shown` can't tell the state from what came
    /// before it; if no frame satisfies `shown` before the instant; if the
    /// app has no process on the bus or can't be stopped; or if it was
    /// stopped at or after the instant: the state may have ended by then.
    pub fn freeze_app_once_shown(
        &self,
        what: &str,
        shown: impl Fn(&Screenshot) -> bool,
        reach: impl FnOnce() -> Instant,
    ) {
        self.wait_for_redraw();
        let before_it = self.capture_now("before-the-moment.ppm");
        let told_apart = !shown(&before_it);
        let _ = fs::remove_file(&before_it.path);
        assert!(
            told_apart,
            "the screen already passed the check for {what} before the app was in it, so the \
             check can't tell when the screen shows it"
        );
        let before = reach();
        let first = self.first_frame_shown(what, before, &shown);
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
            format!(
                "stopped the app (pid {pid}) to capture {what}, {:?} after the screen first \
                 showed it",
                stopped.saturating_duration_since(first)
            ),
        );
        if let Some(late) = stopped.checked_duration_since(before) {
            self.thaw_app();
            panic!(
                "the app was stopped {late:?} after {what} could have ended, so a capture \
                 might show what came after it{}",
                self.timeline.render()
            );
        }
    }

    /// When the screen's first frame that satisfies `shown` was judged,
    /// recording it from now until `before`.
    fn first_frame_shown(
        &self,
        what: &str,
        before: Instant,
        shown: &impl Fn(&Screenshot) -> bool,
    ) -> Instant {
        let captures = self.directory.join("captures");
        let never_shown = |detail: String| -> ! {
            panic!(
                "the screen didn't show {what} before it could have ended{detail}{}",
                self.timeline.render()
            )
        };
        let remaining = before.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            never_shown(String::new());
        }
        let (mut recorder, stopper) =
            Recorder::connect(&self.screen().socket, &captures, remaining)
                .unwrap_or_else(|error| never_shown(format!(" ({error})")));
        let path = captures.join("the-moment.ppm");
        // A still screen sends no frame, so the recording is ended from
        // here at `before`.
        let (judged, cancelled) = mpsc::channel::<()>();
        thread::scope(|scope| {
            scope.spawn(move || {
                let remaining = before.saturating_duration_since(Instant::now());
                if let Err(mpsc::RecvTimeoutError::Timeout) = cancelled.recv_timeout(remaining) {
                    stopper.stop();
                }
            });
            let first = loop {
                let frame = recorder
                    .next_frame(&path)
                    .map(|(width, height)| Screenshot {
                        path: path.clone(),
                        width,
                        height,
                    });
                let Ok(frame) = frame else {
                    break None;
                };
                let showing = shown(&frame);
                let _ = fs::remove_file(&frame.path);
                if showing {
                    break Some(Instant::now());
                }
            };
            drop(judged);
            first.map_or_else(
                || never_shown(String::new()),
                |first| {
                    self.timeline
                        .record("harness", format!("the screen showed {what}"));
                    first
                },
            )
        })
    }

    /// Starts the app again after [`Harness::freeze_app_once_shown`]: its timers
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
