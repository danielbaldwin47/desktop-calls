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
        /// `docs/real-phone-checklist`.
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
        /// `docs/real-phone-checklist`, or under `target/` with --demo.
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
        } => gallery::run(&gallery::Options {
            output,
            only,
            repeat,
        }),
        Command::Demo { scenario } => demo::run(&scenario),
    }
}

/// `target/`: the directory above the one holding this binary.
fn target_directory() -> Option<PathBuf> {
    let executable = std::env::current_exe().ok()?;
    Some(executable.parent()?.parent()?.to_owned())
}
