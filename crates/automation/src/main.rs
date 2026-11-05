//! Automation for Desktop Calls: the scorecard runner, the screenshot
//! gallery and the demo. The Real-Phone Checklist runner lands with ticket
//! #24.

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
    Scorecard,
    /// Render every surface in every state, light and dark, stock and under
    /// an Omarchy theme, into a gallery.
    Screenshot {
        /// Where to write the gallery. Default: `target/screenshots/gallery`.
        #[arg(long)]
        output: Option<PathBuf>,
    },
    /// Run the app against the Simulated Phone in a nested compositor window.
    Demo {
        /// The scenario file the Simulated Phone acts out.
        scenario: PathBuf,
    },
}

fn main() -> ExitCode {
    match Arguments::parse().command {
        Command::Scorecard => scorecard::run(),
        Command::Screenshot { output } => gallery::run(output),
        Command::Demo { scenario } => demo::run(&scenario),
    }
}

/// `target/`: the directory above the one holding this binary.
fn target_directory() -> Option<PathBuf> {
    let executable = std::env::current_exe().ok()?;
    Some(executable.parent()?.parent()?.to_owned())
}
