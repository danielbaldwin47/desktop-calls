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
#[command(
    name = "desktop-calls",
    version,
    args_conflicts_with_subcommands = true
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
    /// A `tel:` link to Call, as the desktop file passes it when a link is
    /// opened.
    #[arg(value_name = "TEL_LINK")]
    link: Option<String>,
}

#[derive(Subcommand)]
enum Command {
    /// Call a number, with the audio on this computer.
    Dial {
        /// The number as typed, with spaces if you like, or a `tel:` link.
        number: String,
    },
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
    let arguments = Cli::parse();
    match arguments.command {
        Some(Command::Dial { number }) => cli::dial(&number),
        Some(Command::Status { json }) => cli::status(json),
        Some(Command::Mute) => cli::mute(),
        Some(Command::Service) => service::run(),
        None if let Some(link) = arguments.link => cli::dial(&link),
        // What the Desktop Calls keybinding runs.
        None => cli::activate(),
    }
}
