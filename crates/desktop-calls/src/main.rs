//! `desktop-calls`: the long-running per-user process and its CLI.

mod bluez;
mod calls_off;
mod chosen_phone;
mod cli;
mod clock;
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
mod missed_calls;
mod notifications;
mod object_manager;
mod pairing;
mod phone_link;
mod quiet_hours;
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

/// Take and place your phone's calls on this computer.
#[derive(Parser)]
#[command(
    name = "desktop-calls",
    version,
    args_conflicts_with_subcommands = true
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
    /// A `tel:` link to call, as passed when you open one.
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
    /// Show whether your phone is connected.
    Status {
        /// Print JSON: `connection_state` and `phone` (`name`, `address`, or null).
        #[arg(long)]
        json: bool,
    },
    /// Open the dialer, to find someone in your contacts or call history
    /// and call them.
    Dialer {
        /// Text to start the search with: part of a name or number.
        #[arg(default_value = "")]
        query: String,
    },
    /// Mute or unmute the microphone during a call.
    Mute,
    /// Open the window that pairs your phone with this computer: to set it
    /// up, pair it again, or switch to another phone.
    Connect,
    /// Move the call's audio to this computer or to your phone.
    Audio {
        /// Where the audio goes.
        #[arg(value_enum)]
        to: AudioTarget,
    },
    /// Turn calls off or on. While calls are off, your phone stays
    /// disconnected from this computer.
    Calls {
        #[command(subcommand)]
        turn: CallsTurn,
        /// What removing Desktop Calls runs with `on`, while it is still
        /// installed: unblocks the Phone if Desktop Calls blocked it,
        /// through the running service if there is one.
        #[arg(long, hide = true, global = true)]
        uninstalling: bool,
    },
    /// Set quiet hours, the daily times when calls are off, such as
    /// 22:00-07:00, or turn them off. With no argument, open the quiet
    /// hours window.
    QuietHours {
        /// HH:MM-HH:MM on the 24-hour clock, or off.
        #[arg(value_name = "HH:MM-HH:MM|off")]
        hours: Option<String>,
    },
    /// Run the long-running process. The systemd user unit runs this.
    #[command(hide = true)]
    Service,
}

/// What `desktop-calls calls` does.
#[derive(clap::Subcommand)]
enum CallsTurn {
    /// Turn calls off until you turn them on, or until a time you choose.
    Off {
        /// For this long: minutes or hours, such as 90m or 2h, up to 24h.
        #[arg(long = "for", value_name = "LENGTH", group = "deadline")]
        length: Option<String>,
        /// Until this time: HH:MM on the 24-hour clock.
        #[arg(long, value_name = "HH:MM", group = "deadline")]
        until: Option<String>,
        /// Until morning: 07:00, or when your quiet hours end.
        #[arg(long, group = "deadline")]
        until_morning: bool,
    },
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
            turn: CallsTurn::Off { .. },
            uninstalling: true,
        }) => Cli::command()
            .error(
                ErrorKind::ArgumentConflict,
                "--uninstalling goes with `calls on`",
            )
            .exit(),
        Some(Command::Calls {
            turn:
                CallsTurn::Off {
                    length,
                    until,
                    until_morning,
                },
            ..
        }) => cli::calls_off(length.as_deref(), until.as_deref(), until_morning),
        Some(Command::Calls {
            turn: CallsTurn::On,
            ..
        }) => cli::calls_on(),
        Some(Command::QuietHours { hours: Some(hours) }) => cli::quiet_hours(&hours),
        Some(Command::QuietHours { hours: None }) => cli::quiet_hours_window(),
        Some(Command::Service) => service::run(),
        None if let Some(link) = arguments.link => cli::dial(&link),
        // What the Desktop Calls keybinding runs.
        None => cli::activate(),
    }
}
