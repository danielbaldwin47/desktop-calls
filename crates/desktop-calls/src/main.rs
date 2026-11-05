//! `desktop-calls`: the long-running per-user process and its CLI.

mod bluez;
mod cli;
mod contacts;
mod control;
mod glib_runtime;
mod hyprland;
mod logging;
mod object_manager;
mod phone_link;
mod ringer;
mod service;
mod surfaces;
mod theme;

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
    /// Say whether the Phone is connected.
    Status {
        /// Print JSON: `connection_state` and `phone` (`name`, `address`, or null).
        #[arg(long)]
        json: bool,
    },
    /// Mute the microphone during a Call, or unmute it if it is muted.
    Mute,
    /// Run the long-running process. The systemd user unit runs this.
    #[command(hide = true)]
    Service,
}

fn main() -> ExitCode {
    match Cli::parse().command {
        Some(Command::Status { json }) => cli::status(json),
        Some(Command::Mute) => cli::mute(),
        Some(Command::Service) => service::run(),
        // What the Desktop Calls keybinding runs.
        None => cli::activate(),
    }
}
