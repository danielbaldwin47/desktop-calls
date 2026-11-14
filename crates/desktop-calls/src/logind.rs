//! Suspend and resume, from logind's `PrepareForSleep` (ADR-0004): the
//! Desktop stops connecting the Phone as it suspends, and connects it again
//! at once when it resumes, rather than waiting out its backoff. On resume
//! the wall clock is read again too, since timers stopped while the Desktop
//! slept (ticket #203).

use desktop_calls_core::Input;
use futures_channel::mpsc::UnboundedSender;
use futures_util::StreamExt as _;
use zbus::Connection;

use crate::clock;
use crate::logging::EventKind;

const SERVICE: &str = "org.freedesktop.login1";
const PATH: &str = "/org/freedesktop/login1";
const MANAGER: &str = "org.freedesktop.login1.Manager";

/// Reports each suspend and resume to `inputs` until the connection closes,
/// and each resume to `clock`, after the resume itself.
///
/// # Errors
///
/// The bus refused to deliver logind's signals.
pub async fn run(
    system: Connection,
    inputs: UnboundedSender<Input>,
    clock: UnboundedSender<clock::Wake>,
) -> zbus::Result<()> {
    // Its properties are never read, so none are fetched.
    let manager = zbus::Proxy::new(&system, SERVICE, PATH, MANAGER).await?;
    let mut signals = manager.receive_signal("PrepareForSleep").await?;
    while let Some(signal) = signals.next().await {
        let Ok((start,)) = signal.body().deserialize::<(bool,)>() else {
            tracing::warn!(
                event_kind = %EventKind::MalformedSignal,
                "logind's PrepareForSleep had no boolean"
            );
            continue;
        };
        let input = if start {
            tracing::info!(event_kind = %EventKind::DesktopSuspending, "The Desktop is suspending");
            Input::Suspending
        } else {
            tracing::info!(event_kind = %EventKind::DesktopResumed, "The Desktop resumed");
            Input::Resumed
        };
        if inputs.unbounded_send(input).is_err() {
            break;
        }
        if !start {
            let _ = clock.unbounded_send(clock::Wake::Resumed);
        }
    }
    Ok(())
}
