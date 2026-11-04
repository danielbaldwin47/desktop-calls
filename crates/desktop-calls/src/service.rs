//! The long-running process that the systemd user unit starts.

use std::process::ExitCode;

use crate::logging::{self, EventKind};

/// Runs the service. Until the event loop lands (ticket #4) it only logs its
/// startup and exits.
pub fn run() -> ExitCode {
    let destination = logging::init();
    tracing::info!(
        event_kind = %EventKind::ServiceStarted,
        version = env!("CARGO_PKG_VERSION"),
        log_destination = %destination,
        "Desktop Calls started"
    );
    tracing::info!(
        event_kind = %EventKind::ServiceStopped,
        "Desktop Calls stopped: there is no event loop yet"
    );
    ExitCode::SUCCESS
}
