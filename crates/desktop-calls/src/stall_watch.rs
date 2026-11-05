//! Notices when the main loop is held up, and logs it as
//! `main-loop-stalled`.
//!
//! GTK, zbus and the Call state machine all run on the one main loop, so
//! anything slow on it (formatting the first caller's number once took over
//! a second on CI, ticket #41) holds up the Banner. The log says so, with
//! how long, which turns "the Banner was late" into "the main loop was busy
//! for 1.2 s right after the Call appeared".
//!
//! A timer on the main loop fires every [`TICK`]; a tick that comes over
//! [`STALL`] late means the loop was busy for at least that long, give or
//! take one tick. It runs only while there is a Call or a surface is shown,
//! when a held-up main loop is what the user sees, so an idle Desktop Calls
//! doesn't wake 50 times a second. The turn that decides a Call has
//! appeared runs before the timer starts, so [`timed`] measures that work
//! directly.

use std::cell::Cell;
use std::rc::Rc;
use std::time::{Duration, Instant};

use glib::{ControlFlow, SourceId};

use crate::logging::EventKind;

/// How often the timer fires while watching.
const TICK: Duration = Duration::from_millis(20);

/// How late a tick may come before it counts as a stall.
const STALL: Duration = Duration::from_millis(100);

/// The timer, while watching.
#[derive(Default)]
pub struct StallWatch {
    timer: Option<SourceId>,
}

impl StallWatch {
    /// Starts or stops watching the main loop.
    pub fn watch(&mut self, watching: bool) {
        match (watching, self.timer.take()) {
            (true, None) => {
                let last = Rc::new(Cell::new(Instant::now()));
                self.timer = Some(glib::timeout_add_local(TICK, move || {
                    let now = Instant::now();
                    let late = now.duration_since(last.replace(now)).saturating_sub(TICK);
                    if late > STALL {
                        stalled(late, "between ticks");
                    }
                    ControlFlow::Continue
                }));
            }
            (true, Some(timer)) => self.timer = Some(timer),
            (false, Some(timer)) => timer.remove(),
            (false, None) => {}
        }
    }
}

/// Runs `work` on the main loop, and logs a stall if it took over [`STALL`].
pub fn timed<T>(during: &'static str, work: impl FnOnce() -> T) -> T {
    let started = Instant::now();
    let done = work();
    let took = started.elapsed();
    if took > STALL {
        stalled(took, during);
    }
    done
}

fn stalled(how_long: Duration, during: &'static str) {
    let stalled_ms = u64::try_from(how_long.as_millis()).unwrap_or(u64::MAX);
    tracing::info!(
        event_kind = %EventKind::MainLoopStalled,
        stalled_ms,
        during,
        "The main loop was held up for {stalled_ms} ms"
    );
}
