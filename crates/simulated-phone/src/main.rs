//! The Simulated Phone: fake `org.pipewire.Telephony`, `org.bluez.obex` and
//! `org.bluez` objects on private buses, driven by tests through
//! `io.github.danielbaldwin47.DesktopCalls.SimulatedPhone1`.
//!
//! Stub until ticket #4 lands.

use std::process::ExitCode;

fn main() -> ExitCode {
    eprintln!("The Simulated Phone is not available yet (ticket #4).");
    ExitCode::FAILURE
}
