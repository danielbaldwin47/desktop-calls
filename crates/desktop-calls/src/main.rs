//! `desktop-calls`: the long-running per-user process and its CLI.

mod logging;
mod service;

use std::process::ExitCode;

use clap::{Parser, Subcommand};

/// Take and place your Phone's Calls on this Desktop.
#[derive(Parser)]
#[command(name = "desktop-calls", version)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// Run the long-running process. The systemd user unit runs this.
    #[command(hide = true)]
    Service,
}

fn main() -> ExitCode {
    match Cli::parse().command {
        Some(Command::Service) => service::run(),
        None => {
            eprintln!("Desktop Calls can't open the Dialer yet (ticket #11).");
            ExitCode::FAILURE
        }
    }
}
