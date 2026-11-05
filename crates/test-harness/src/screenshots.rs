//! Screenshots of the scenario's screen, taken once it has redrawn.

use std::fs;
use std::sync::{PoisonError, mpsc};
use std::thread;
use std::time::{Duration, Instant};

use super::{FRAMES_AHEAD, Harness, REDRAW_DEADLINE};
use crate::redraw::OnScreen;
use crate::screen::Screen;
use crate::screencopy::Recorder;
use crate::{Region, Screenshot, redraw};

/// The latest frame in the run of frames, starting with `found`, that
/// satisfy `shown`, up to the first that doesn't. Frames passed over, and
/// any left in `frames`, are deleted.
fn latest_shown(
    mut found: Screenshot,
    frames: &mpsc::Receiver<Screenshot>,
    shown: impl Fn(&Screenshot) -> bool,
) -> Screenshot {
    let mut run_ended = false;
    for frame in frames.try_iter() {
        if !run_ended && shown(&frame) {
            let _ = fs::remove_file(std::mem::replace(&mut found, frame).path);
        } else {
            run_ended = true;
            let _ = fs::remove_file(frame.path);
        }
    }
    found
}

impl Harness {
    /// Captures the screen as `<name>.png` in the screenshot directory, once
    /// it has finished redrawing (see `redraw.rs`), so the capture shows
    /// every state change reported so far.
    ///
    /// # Panics
    ///
    /// If the scenario has no screen, the screen doesn't finish redrawing,
    /// or the capture fails.
    #[must_use]
    pub fn screenshot(&self, name: &str) -> Screenshot {
        self.wait_for_redraw();
        Screen::screenshot(
            &self.environment,
            &self.group,
            &self.screenshot_directory.join(format!("{name}.png")),
        )
    }

    /// Captures only `region` of the screen, as `<name>.png` in the
    /// screenshot directory. A small region takes a fraction of a full
    /// capture's time, for checks that are timed. The capture's own
    /// coordinates start at the region's top-left corner. Like
    /// [`Harness::screenshot`], it waits for the screen to finish redrawing.
    ///
    /// # Panics
    ///
    /// If the scenario has no screen, the screen doesn't finish redrawing,
    /// or the capture fails.
    #[must_use]
    pub fn screenshot_of(&self, name: &str, region: Region) -> Screenshot {
        self.wait_for_redraw();
        Screen::screenshot_of(
            &self.environment,
            &self.group,
            &self.screenshot_directory.join(format!("{name}.png")),
            region,
        )
    }

    /// Captures the screen until a capture satisfies `shown`, and returns
    /// that capture, kept as `<name>.png`. What the app reports can run a
    /// frame ahead of what is on screen; this waits for the screen.
    ///
    /// The screen is recorded on a thread of its own: its content when this
    /// is called, then a frame each time anything on screen changes, as the
    /// compositor reports it (see `screencopy.rs`). `shown` judges the frames
    /// in order as they arrive. So a state on screen for less time than one
    /// `shown` takes, such as the Call Window's "Call ended" for 2 s, is
    /// still caught, and a still screen costs no CPU that the app needs to
    /// draw. At most [`FRAMES_AHEAD`] frames wait for `shown` at a time.
    ///
    /// Once a frame satisfies `shown`, this waits for the screen to finish
    /// redrawing, as every action does (see `redraw.rs`): a window can match
    /// while still mid-resize, and a test that clicks where the capture shows
    /// a button needs where it ends up. It then returns a capture of the
    /// redrawn screen if that still satisfies `shown`; otherwise the state
    /// was brief and has gone, and it returns the latest frame of the run
    /// that satisfied `shown`.
    ///
    /// # Panics
    ///
    /// If no frame within `deadline` satisfies `shown`; the panic names the
    /// last frame, kept as `<name>.png`. Also if the recording fails.
    #[must_use]
    pub fn screenshot_when(
        &self,
        name: &str,
        deadline: Duration,
        shown: impl Fn(&Screenshot) -> bool,
    ) -> Screenshot {
        let give_up = Instant::now() + deadline;
        let captures = self.directory.join("captures");
        let (mut recorder, stopper) = Recorder::connect(&self.screen().socket, &captures)
            .unwrap_or_else(|error| panic!("record the screen: {error}"));
        let (found, last, failed) = thread::scope(|scope| {
            let (sender, receiver) = mpsc::sync_channel(FRAMES_AHEAD);
            let recording = scope.spawn(move || {
                for count in 1_u32.. {
                    let path = captures.join(format!("{name}-{count}.ppm"));
                    let (width, height) = recorder.next_frame(&path)?;
                    if sender
                        .send(Screenshot {
                            path,
                            width,
                            height,
                        })
                        .is_err()
                    {
                        break;
                    }
                }
                Ok::<_, String>(())
            });
            let mut last: Option<Screenshot> = None;
            let mut found = None;
            let mut ended_early = false;
            while let Some(remaining) = give_up.checked_duration_since(Instant::now()) {
                let frame = match receiver.recv_timeout(remaining) {
                    Ok(frame) => frame,
                    Err(mpsc::RecvTimeoutError::Timeout) => break,
                    Err(mpsc::RecvTimeoutError::Disconnected) => {
                        ended_early = true;
                        break;
                    }
                };
                if shown(&frame) {
                    self.wait_for_redraw();
                    found = Some(latest_shown(frame, &receiver, &shown));
                    break;
                }
                // Each frame is 25 MB; keep only the latest, for the panic.
                if let Some(previous) = last.replace(frame) {
                    let _ = fs::remove_file(previous.path);
                }
            }
            stopper.stop();
            drop(receiver);
            let ended = recording
                .join()
                .unwrap_or_else(|panic| std::panic::resume_unwind(panic));
            (found, last, ended.err().filter(|_| ended_early))
        });
        if let Some(error) = failed {
            panic!("recording the screen for {name} failed: {error}");
        }
        let kept = |capture: &Screenshot| {
            capture.save_png(&self.screenshot_directory.join(format!("{name}.png")))
        };
        match (found, last) {
            (Some(capture), _) => {
                let redrawn = self.capture_now(&format!("{name}-redrawn.ppm"));
                if shown(&redrawn) {
                    let _ = fs::remove_file(&capture.path);
                    kept(&redrawn)
                } else {
                    let _ = fs::remove_file(&redrawn.path);
                    kept(&capture)
                }
            }
            (None, Some(capture)) => panic!(
                "the screen never showed {name} within {deadline:?}; see {}{}",
                kept(&capture).path.display(),
                self.timeline.render()
            ),
            (None, None) => panic!(
                "no capture of {name} was made within {deadline:?}{}",
                self.timeline.render()
            ),
        }
    }

    /// One frame of the screen as it is now, as a PPM under `captures/`.
    fn capture_now(&self, file: &str) -> Screenshot {
        let captures = self.directory.join("captures");
        let path = captures.join(file);
        let (width, height) = Recorder::connect(&self.screen().socket, &captures)
            .and_then(|(mut recorder, _)| recorder.next_frame(&path))
            .unwrap_or_else(|error| panic!("capture the screen: {error}"));
        Screenshot {
            path,
            width,
            height,
        }
    }

    /// Waits until the screen has finished redrawing after every state
    /// change so far. Every action that clicks, types or takes a screenshot
    /// calls this first, so no scenario can race a redraw. See `redraw.rs`.
    pub(super) fn wait_for_redraw(&self) {
        let compositor = self.screen().pid;
        let on_screen = || {
            let app = self
                .processes
                .iter()
                .filter(|(name, _)| *name == "desktop-calls")
                .map(|(_, app)| app.id());
            let test_clients = self
                .test_clients
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .clone();
            OnScreen {
                compositor,
                clients: app.chain(test_clients).collect(),
            }
        };
        if let Err(error) = redraw::wait_for_redraw(on_screen, REDRAW_DEADLINE) {
            panic!("{error}{}", self.timeline.render());
        }
    }
}
