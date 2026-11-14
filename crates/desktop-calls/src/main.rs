//! `desktop-calls`: the long-running per-user process and its CLI.

mod bluez;
mod calls_off;
mod chosen_phone;
mod cli;
mod contacts;
mod control;
mod do_not_disturb;
mod earlier_run;
mod glib_runtime;
mod hyprland;
mod indicator;
mod keybinding;
mod logging;
mod logind;
mod notifications;
mod object_manager;
mod pairing;
mod phone_link;
mod ringer;
mod service;
mod setup_checks;
mod stall_watch;
mod surfaces;
mod theme;
mod uninstall;
mod wireplumber;

use std::process::ExitCode;

use clap::error::ErrorKind;
use clap::{CommandFactory as _, Parser, Subcommand};
use desktop_calls_core::AudioLocation;

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
    /// Open the Dialer, to find someone in Contacts or the Call History and
    /// Call them.
    Dialer {
        /// Text to start the search with: part of a name or number.
        #[arg(default_value = "")]
        query: String,
    },
    /// Mute the microphone during a Call, or unmute it if it is muted.
    Mute,
    /// Open the window that pairs your phone with this computer: to set it
    /// up, pair it again, or switch to another phone.
    Connect,
    /// Move the Call's audio to this computer or to your phone.
    Audio {
        /// Where the audio goes.
        #[arg(value_enum)]
        to: AudioTarget,
    },
    /// Turn calls off, so your phone stays disconnected from this computer
    /// until you turn them on, or turn them on.
    Calls {
        /// Off or on.
        #[arg(value_enum)]
        turn: CallsTurn,
        /// What removing Desktop Calls runs with `on`, while it is still
        /// installed: unblocks the Phone if Desktop Calls blocked it, and
        /// never connects it.
        #[arg(long, hide = true)]
        uninstalling: bool,
    },
    /// Run the long-running process. The systemd user unit runs this.
    #[command(hide = true)]
    Service,
}

/// What `desktop-calls calls` does.
#[derive(Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
enum CallsTurn {
    /// Turn calls off until you turn them on.
    Off,
    /// Turn calls on.
    On,
}

/// Where `desktop-calls audio` moves the audio.
#[derive(Clone, Copy, clap::ValueEnum)]
enum AudioTarget {
    /// This computer.
    Here,
    /// Your phone.
    Phone,
}

impl From<AudioTarget> for AudioLocation {
    fn from(target: AudioTarget) -> Self {
        match target {
            AudioTarget::Here => Self::Desktop,
            AudioTarget::Phone => Self::Phone,
        }
    }
}

fn main() -> ExitCode {
    let arguments = Cli::parse();
    match arguments.command {
        Some(Command::Dial { number }) => cli::dial(&number),
        Some(Command::Status { json }) => cli::status(json),
        Some(Command::Dialer { query }) => cli::dialer(&query),
        Some(Command::Mute) => cli::mute(),
        Some(Command::Connect) => cli::connect_phone(),
        Some(Command::Audio { to }) => cli::audio(to.into()),
        Some(Command::Calls {
            turn: CallsTurn::On,
            uninstalling: true,
        }) => uninstall::calls_on(),
        Some(Command::Calls {
            turn: CallsTurn::Off,
            uninstalling: true,
        }) => Cli::command()
            .error(
                ErrorKind::ArgumentConflict,
                "--uninstalling goes with `calls on`",
            )
            .exit(),
        Some(Command::Calls { turn, .. }) => cli::calls(turn == CallsTurn::Off),
        Some(Command::Service) => service::run(),
        None if let Some(link) = arguments.link => cli::dial(&link),
        // What the Desktop Calls keybinding runs.
        None => cli::activate(),
    }
}
