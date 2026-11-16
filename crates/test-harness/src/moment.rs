//! Capturing a state the app shows only for a moment (#248, #384).
//!
//! The app shows some states for a fixed time by design: the Banner says
//! "Identifying caller…" for 2 s, the Call Pill says "Call ended" for 2 s,
//! and the audio tile says "Moving…" until the audio arrives. On a loaded CI
//! runner, frames of the 3840 by 2160 screen came about 2 s apart, so no
//! frame fell inside the state, and the gallery failed.
//!
//! So the screen is grabbed into memory while the state lasts, each frame
//! with the instant it was grabbed, and judged only once the state could
//! have ended: the first frame grabbed before then that shows the state is
//! the capture. Nothing in the app changes, and nothing waits on it: the
//! app is neither held nor stopped, so the states last their real time, as
//! in an installed run.
//!
//! Two ways of doing it raced the state and lost on CI's two CPUs:
//!
//! - Until #384 the app was stopped (SIGSTOP) once the screen had held
//!   still for 100 ms. With reduced motion the audio tile's bar steps on
//!   twice a second, and the screen first held still 2.2 s into a state the
//!   gallery allowed 2 s.
//! - Then it was stopped on the first frame that showed the state. The
//!   pill was drawn at once, but judging a whole-screen frame, with four
//!   renders sharing the CPUs, took about 2 s, and the stop came too late
//!   (PR #391's first CI run); stopping the app while each frame was judged
//!   kept it from drawing the state at all (its second).
//!
//! Grabbing a frame costs a copy of the compositor's buffer, nothing more.
//! Frames are grabbed at most every [`SPACING`], at most [`MOST_FRAMES`] of
//! them, which bounds the memory a screen that keeps changing can take: a
//! change between grabs waits for the next, which then shows the screen as
//! it is.
//!
//! A frame shows the state only by what the caller's check sees in it, so
//! the check must tell the state from the screen before it. The capture
//! proves that it does, on the screen as it was just before the state, and
//! fails if it doesn't.
//!
//! A timer the app starts in answer to something the harness did can't
//! fire before that, plus its duration. So a frame grabbed before that
//! instant can't show what came after the state: the caller names the
//! instant. A frame is read only after the compositor copied it, so the
//! instant it was grabbed is never early.

use std::fmt::Write as _;
use std::fs;
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use super::Harness;
use crate::Screenshot;
use crate::screencopy::{RawFrame, Recorder};

/// The least time between two frames grabbed.
const SPACING: Duration = Duration::from_millis(200);
/// The most frames grabbed for one state: about 33 MB each.
const MOST_FRAMES: usize = 16;

impl Harness {
    /// Brings the app into a state that lasts only a moment with `reach`,
    /// grabs the screen until the state could have ended, and keeps the
    /// first frame grabbed that satisfies `shown` as `<name>.png` in the
    /// screenshot directory. `reach` returns the instant before which the
    /// state can't end.
    ///
    /// # Panics
    ///
    /// If the screen as it is before `reach`, once redrawn, already
    /// satisfies `shown`, so `shown` can't tell the state from what came
    /// before it; or if no frame grabbed before the instant satisfies
    /// `shown`: the panic gives when each frame was grabbed against the
    /// instant, and the timeline.
    #[must_use]
    pub fn capture_moment(
        &self,
        name: &str,
        shown: impl Fn(&Screenshot) -> bool,
        reach: impl FnOnce() -> Instant,
    ) -> Screenshot {
        self.wait_for_redraw();
        let before_it = self.capture_now(&format!("{name}-before.ppm"));
        let told_apart = !shown(&before_it);
        let _ = fs::remove_file(&before_it.path);
        assert!(
            told_apart,
            "the screen already passed the check for {name} before the app was in it, so the \
             check can't tell when the screen shows it"
        );
        let started = Instant::now();
        let before = reach();
        let reached = Instant::now();
        self.timeline.record(
            "harness",
            format!("{name} is reported; grabbing the screen"),
        );
        let (frames, ended) = self.grab_until(before);
        let at = |instant: Instant| instant.saturating_duration_since(started).as_millis();
        let mut grabs = String::new();
        let path = self.directory.join("captures").join(format!("{name}.ppm"));
        for (count, (grabbed, frame)) in frames.iter().enumerate() {
            let judged = frame.write_ppm(&path).map(|(width, height)| {
                let screenshot = Screenshot {
                    path: path.clone(),
                    width,
                    height,
                };
                shown(&screenshot).then_some(screenshot)
            });
            match judged {
                Ok(Some(screenshot)) => {
                    self.timeline.record(
                        "harness",
                        format!(
                            "frame {} of {name}, grabbed at {} ms, shows it",
                            count + 1,
                            at(*grabbed)
                        ),
                    );
                    let kept =
                        screenshot.save_png(&self.screenshot_directory.join(format!("{name}.png")));
                    let _ = fs::remove_file(&path);
                    return kept;
                }
                Ok(None) => {
                    let _ = write!(grabs, " {} ms (not it),", at(*grabbed));
                }
                Err(error) => {
                    let _ = write!(grabs, " {} ms (unreadable: {error}),", at(*grabbed));
                }
            }
        }
        let _ = fs::remove_file(&path);
        panic!(
            "no frame grabbed before {name} could have ended shows it. From asking for it: \
             reported at {} ms; frames grabbed at{} {}; it could end at {} ms{}",
            at(reached),
            if grabs.is_empty() { " none," } else { &grabs },
            ended.map_or_else(
                || "then it could have ended".to_owned(),
                |error| format!("then the recording failed ({error})"),
            ),
            at(before),
            self.timeline.render()
        );
    }

    /// The frames of the screen grabbed from now until `before`, each with
    /// when it was grabbed, at most every [`SPACING`] and at most
    /// [`MOST_FRAMES`] of them; and the error that ended the recording
    /// early, if one did.
    fn grab_until(&self, before: Instant) -> (Vec<(Instant, RawFrame)>, Option<String>) {
        let remaining = before.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return (Vec::new(), None);
        }
        let captures = self.directory.join("captures");
        let (mut recorder, stopper) =
            match Recorder::connect(&self.screen().socket, &captures, remaining) {
                Ok(connected) => connected,
                Err(error) => return (Vec::new(), Some(error)),
            };
        // A still screen sends no frame, so the recording is ended from
        // here at `before`.
        let (grabbing, cancelled) = mpsc::channel::<()>();
        thread::scope(|scope| {
            scope.spawn(move || {
                let remaining = before.saturating_duration_since(Instant::now());
                if let Err(mpsc::RecvTimeoutError::Timeout) = cancelled.recv_timeout(remaining) {
                    stopper.stop();
                }
            });
            let mut frames = Vec::new();
            let mut ended = None;
            while frames.len() < MOST_FRAMES {
                match recorder.next_raw_frame() {
                    Ok(frame) => {
                        let grabbed = Instant::now();
                        if grabbed >= before {
                            break;
                        }
                        frames.push((grabbed, frame));
                        let next = (grabbed + SPACING).min(before);
                        thread::sleep(next.saturating_duration_since(Instant::now()));
                    }
                    // Stopped at `before`, or failed before it.
                    Err(error) => {
                        if Instant::now() < before {
                            ended = Some(error);
                        }
                        break;
                    }
                }
            }
            drop(grabbing);
            (frames, ended)
        })
    }
}
