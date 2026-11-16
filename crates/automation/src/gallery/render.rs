//! Rendering the gallery's shots, several at a time, each in a scenario of
//! its own.

use std::num::NonZeroUsize;
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread;
use std::time::Duration;

use super::{DEADLINE, Look, State, file_stem};

/// Renders each of `shots` into `output`, several at a time: `workers`, or
/// one scenario per CPU. Each has its own compositor, buses and app, so they
/// don't meet. One at a time, the 56 shots took 139 s of a CI run (#118).
/// With a CPU's worth of them each on a machine others are building on, a
/// screen was late past its deadline (#167).
pub(super) fn render_all(
    shots: &[(&State, Look)],
    output: &Path,
    workers: Option<u32>,
    late: Option<Duration>,
) {
    let next = AtomicUsize::new(0);
    let workers = workers.map_or_else(
        || thread::available_parallelism().map_or(1, NonZeroUsize::get),
        |workers| workers as usize,
    );
    thread::scope(|scope| {
        for _ in 0..workers.min(shots.len()) {
            scope.spawn(|| {
                while let Some(&(state, look)) = shots.get(next.fetch_add(1, Ordering::Relaxed)) {
                    render(state, look, output, late);
                }
            });
        }
    });
}

/// Renders `state` under `look` into `output`.
fn render(state: &State, look: Look, output: &Path, late: Option<Duration>) {
    let harness = (state.prepare)(look.builder())
        .headless_screen()
        .omarchy_bar()
        .screenshot_directory(output.to_owned())
        .start();
    (state.reach)(&harness);
    // A state that lasts a moment is grabbed while it lasts and judged once
    // it could have ended (#248, #384), so it is captured however slow the
    // runner, and however late a capture would come.
    if let Some(moment) = state.moment {
        let _ = harness.capture_moment(
            &file_stem(state, look),
            |screenshot| (state.shown)(screenshot, look),
            || moment(&harness),
        );
        return;
    }
    if let Some(late) = late {
        thread::sleep(late);
    }
    // A frame is judged after it is taken. A surface queues its
    // new content for `Surfaces` as it renders it, and a task on
    // the same GLib main context publishes it. That task runs at
    // default priority, before GTK's redraw, which GLib runs at a
    // lower one. Only that scheduling order, not any proof in the
    // code, makes a frame judged while the app still reports the
    // state show that state.
    let _ = harness.screenshot_when(&file_stem(state, look), DEADLINE, |screenshot| {
        (state.shown)(screenshot, look) && (state.still)(&harness.app_state())
    });
}
