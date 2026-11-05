//! `just demo`: the app against the Simulated Phone, on a nested compositor
//! that shows as a window on the Desktop, until the window is closed.

use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::Duration;

use test_harness::Harness;

/// How long the app may take to connect to the Simulated Phone.
const DEADLINE: Duration = Duration::from_secs(10);

/// `automation demo SCENARIO`.
pub fn run(scenario: &Path) -> ExitCode {
    let Some(parent_socket) = desktop_socket() else {
        eprintln!(
            "just demo opens a window, so it needs a Wayland session: WAYLAND_DISPLAY is not set."
        );
        return ExitCode::FAILURE;
    };
    if !scenario.is_file() {
        eprintln!("No scenario file at {}.", scenario.display());
        return ExitCode::FAILURE;
    }
    // Ctrl-C reaches every process in the demo. The compositor closes on it,
    // and this process lives on to stop the rest and clean up.
    if let Err(error) = signal_hook::flag::register(
        signal_hook::consts::SIGINT,
        Arc::new(AtomicBool::new(false)),
    ) {
        eprintln!("Can't handle Ctrl-C: {error}");
        return ExitCode::FAILURE;
    }

    let harness = Harness::builder()
        .nested_screen(parent_socket)
        .scenario(scenario.to_owned())
        .start();
    let state = harness.wait_for_app("ConnectionState is connected", DEADLINE, |state| {
        state.connection_state == "connected"
    });
    let phone = state.phone.map(|phone| phone.name).unwrap_or_default();
    println!("Demo: Desktop Calls is running in the nested compositor window.");
    println!("ConnectionState: {} ({phone}).", state.connection_state);
    println!("Scenario: {}", scenario.display());
    println!("Logs: {}", harness.directory().display());
    println!("Close the window, or press Ctrl-C, to stop.");

    harness.wait_until_screen_closes();
    ExitCode::SUCCESS
}

/// The Desktop compositor's socket, from `WAYLAND_DISPLAY`, which may be a
/// name in `XDG_RUNTIME_DIR` or a path.
fn desktop_socket() -> Option<PathBuf> {
    let display = std::env::var_os("WAYLAND_DISPLAY")?;
    let display = Path::new(&display);
    if display.is_absolute() {
        return Some(display.to_owned());
    }
    let runtime = std::env::var_os("XDG_RUNTIME_DIR")?;
    Some(Path::new(&runtime).join(display))
}
