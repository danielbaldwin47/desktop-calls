//! Automation for Desktop Calls: the scorecard runner, the Real-Phone
//! Checklist runner, the screenshot gallery and the demo.

mod checklist;
mod demo;
mod gallery;
mod scorecard;

use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand};

/// Automation for Desktop Calls. `just --list` shows the recipes that run it.
#[derive(Parser)]
#[command(name = "automation")]
struct Arguments {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Report every polish-scorecard item with its status and evidence.
    Scorecard {
        /// Where the Real-Phone Checklist's result files are. Default:
        /// `~/.local/state/desktop-calls/real-phone-checklist`.
        #[arg(long)]
        results: Option<PathBuf>,
    },
    /// Walk through the Real-Phone Checklist one step at a time, observing
    /// what a machine can and asking only what it can't.
    RealPhoneChecklist {
        /// The group to run, 1 to 7. Default: every group.
        group: Option<u8>,
        /// Run against the Simulated Phone instead of the real Phone, and
        /// print the action that plays each instruction after `demo>`.
        #[arg(long)]
        demo: bool,
        /// With --demo, keep the compositor off-screen.
        #[arg(long, requires = "demo")]
        headless: bool,
        /// Where to keep the dated result files. Default:
        /// `~/.local/state/desktop-calls/real-phone-checklist` (under
        /// `$XDG_STATE_HOME` if set), or under `target/` with --demo.
        #[arg(long)]
        results: Option<PathBuf>,
        /// Start a new result file even if the last run is unfinished.
        #[arg(long)]
        fresh: bool,
    },
    /// Render every surface in every state, light and dark, stock and under
    /// an Omarchy theme, into a gallery.
    Screenshot {
        /// Where to write the gallery. Default: `target/screenshots/gallery`.
        #[arg(long)]
        output: Option<PathBuf>,
        /// Render only this state, in every look, keeping the rest of the
        /// gallery: to exercise a capture fix for one state in minutes.
        #[arg(long, value_name = "STATE")]
        only: Option<String>,
        /// Render it this many times, stopping at the first failure.
        #[arg(long, value_name = "N", default_value_t = 1,
              value_parser = clap::value_parser!(u32).range(1..))]
        repeat: u32,
        /// Render this many states at a time. Default: one per CPU. Each
        /// has its own compositor and app, so on a busy machine fewer are
        /// more reliable.
        #[arg(long, value_name = "N", value_parser = clap::value_parser!(u32).range(1..))]
        workers: Option<u32>,
        /// Render only part K of N of the states (every N-th, from the
        /// K-th), as CI's gallery jobs do, one part each.
        #[arg(long, value_name = "K/N", conflicts_with = "only")]
        shard: Option<gallery::Shard>,
        /// Print the gallery's states, in order, and render nothing.
        #[arg(long)]
        list_states: bool,
        /// Wait this many milliseconds between reaching each state and
        /// capturing it, as a slow runner might: the states that last a
        /// moment must still be captured (#248).
        #[arg(long, value_name = "MS")]
        late: Option<u64>,
    },
    /// Run the app against the Simulated Phone in a nested compositor window.
    Demo {
        /// The scenario file the Simulated Phone acts out.
        scenario: PathBuf,
    },
}

fn main() -> ExitCode {
    match Arguments::parse().command {
        Command::Scorecard { results } => scorecard::run(results),
        Command::RealPhoneChecklist {
            group,
            demo,
            headless,
            results,
            fresh,
        } => checklist::run(&checklist::Options {
            group,
            demo,
            headless,
            results,
            fresh,
        }),
        Command::Screenshot {
            output,
            only,
            repeat,
            workers,
            shard,
            list_states,
            late,
        } => gallery::run(&gallery::Options {
            output,
            only,
            repeat,
            workers,
            shard,
            list_states,
            late: late.map(std::time::Duration::from_millis),
        }),
        Command::Demo { scenario } => demo::run(&scenario),
    }
}

/// `target/`: the directory above the one holding this binary.
fn target_directory() -> Option<PathBuf> {
    let executable = std::env::current_exe().ok()?;
    Some(executable.parent()?.parent()?.to_owned())
}
