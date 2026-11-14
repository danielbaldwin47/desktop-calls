//! Waiting for the screen to finish redrawing, which every harness action
//! that clicks, types or takes a screenshot does first, by itself (#53).
//!
//! The app reports a state change on its control interface as soon as it
//! has told its surfaces what to show, before GTK has drawn it, so a
//! scenario that acts straight after one raced the redraw: in CI a keypad
//! click landed on "4" instead of "1" because the Call Window was still
//! growing (#71), and a click to focus the muted Call Window came before it
//! was drawn (#39).
//!
//! The screen has finished redrawing when, for [`QUIET`]:
//!
//! - the compositor used no CPU at all. Every redraw ends with the client
//!   committing a buffer, and the compositor rendering it, so a compositor
//!   that has done nothing has had nothing new to show. Its runtime is the
//!   kernel's own count, per thread, in `/proc/<pid>/task/<tid>/schedstat`;
//!   and
//! - at the end, no thread of the compositor or of any client on the screen
//!   (the app and the harness's test clients) is runnable. A client starved
//!   of CPU mid-redraw, as on a loaded CI runner, is runnable, not asleep;
//!   so is one busy drawing.
//!
//! [`QUIET`] is several of the headless output's frames, so a frame the
//! compositor had already scheduled (it renders on a 60 Hz timer) is
//! rendered within it, and counts as activity.
//!
//! The text cursor's blinking is off in every scenario (`desktop_settings` in
//! `builder.rs`), or a focused entry would redraw for 10 s after each key.

use std::fs;
use std::thread;
use std::time::{Duration, Instant};

/// How long the compositor must stay idle for the screen to count as
/// redrawn: six frames of the headless output's 60 Hz.
pub(crate) const QUIET: Duration = Duration::from_millis(100);

/// What the screen is made of, as processes: the compositor and the clients
/// that draw on it.
pub(crate) struct OnScreen {
    pub compositor: u32,
    pub clients: Vec<u32>,
}

/// What kept the screen from settling the last time it was checked.
#[derive(Debug)]
enum Busy {
    /// The compositor used CPU: something was committed or rendered.
    Compositor,
    /// A thread was runnable, with its process's and its own name.
    Runnable { process: String, thread: String },
}

impl std::fmt::Display for Busy {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Compositor => write!(formatter, "the compositor kept rendering"),
            Self::Runnable { process, thread } => {
                write!(formatter, "{process}'s thread {thread} stayed runnable")
            }
        }
    }
}

/// Waits until the screen has finished redrawing, as the module says, for
/// at most `deadline`. `on_screen` is asked afresh on each check, so a
/// client that starts or stops meanwhile is followed.
///
/// # Errors
///
/// If `deadline` passes first: says what kept the screen busy.
pub(crate) fn wait_for_redraw(
    on_screen: impl Fn() -> OnScreen,
    deadline: Duration,
) -> Result<(), String> {
    let give_up = Instant::now() + deadline;
    loop {
        let before = runtime(on_screen().compositor);
        thread::sleep(QUIET);
        let now = on_screen();
        let busy = if runtime(now.compositor) == before {
            runnable(&now)
        } else {
            Some(Busy::Compositor)
        };
        let Some(busy) = busy else {
            return Ok(());
        };
        if Instant::now() >= give_up {
            return Err(format!(
                "the screen never held still for {QUIET:?} within {deadline:?}: {busy}"
            ));
        }
    }
}

/// The CPU time all of `pid`'s threads have used, in nanoseconds. A thread
/// that has gone takes its time with it, so this is compared only for a
/// change.
fn runtime(pid: u32) -> u64 {
    threads(pid)
        .filter_map(|task| fs::read_to_string(format!("{task}/schedstat")).ok())
        .filter_map(|schedstat| schedstat.split_whitespace().next()?.parse::<u64>().ok())
        .sum()
}

/// The first runnable thread on screen, if any. `R` is running or waiting
/// for a CPU; `D` is waiting on the disk, in the middle of something.
fn runnable(on_screen: &OnScreen) -> Option<Busy> {
    std::iter::once(on_screen.compositor)
        .chain(on_screen.clients.iter().copied())
        .find_map(|pid| {
            threads(pid).find_map(|task| {
                let stat = fs::read_to_string(format!("{task}/stat")).ok()?;
                // The name, in parentheses, may itself hold spaces and
                // parentheses; the state follows the last `)`.
                let (name, rest) = stat.split_once('(')?.1.rsplit_once(')')?;
                let state = rest.trim_start().chars().next()?;
                matches!(state, 'R' | 'D').then(|| Busy::Runnable {
                    process: process_name(pid),
                    thread: name.to_owned(),
                })
            })
        })
}

fn process_name(pid: u32) -> String {
    fs::read_to_string(format!("/proc/{pid}/comm"))
        .map_or_else(|_| format!("process {pid}"), |name| name.trim().to_owned())
}

/// `/proc/<pid>/task/<tid>` for each of `pid`'s threads; none once it has
/// exited.
fn threads(pid: u32) -> impl Iterator<Item = String> {
    fs::read_dir(format!("/proc/{pid}/task"))
        .into_iter()
        .flatten()
        .flatten()
        .map(|task| task.path().display().to_string())
}
