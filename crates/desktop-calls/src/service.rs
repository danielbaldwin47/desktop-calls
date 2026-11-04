//! The long-running process that the systemd user unit starts.
//!
//! One thread runs one GLib main context. zbus runs on it through
//! [`GlibRuntime`]; the BlueZ client and the Phone link turn D-Bus traffic
//! into inputs; the Call state machine decides; this module carries out its
//! effects and publishes the state on the control interface.

use std::process::ExitCode;

use desktop_calls_core::{Effect, Input, State, step};
use futures_channel::mpsc::{UnboundedReceiver, UnboundedSender, unbounded};
use futures_util::StreamExt as _;
use futures_util::future::select;
use glib::MainContext;
use zbus::Connection;

use crate::control::Publisher;
use crate::glib_runtime::GlibRuntime;
use crate::logging::{self, EventKind};
use crate::phone_link::Command;
use crate::{bluez, phone_link};

/// Why the service could not run.
#[derive(Clone, Copy, Debug)]
enum Failure {
    SessionBusUnreachable,
    SystemBusUnreachable,
    /// Another copy of Desktop Calls owns the control interface's name.
    AlreadyRunning,
}

impl Failure {
    fn as_str(self) -> &'static str {
        match self {
            Self::SessionBusUnreachable => "session-bus-unreachable",
            Self::SystemBusUnreachable => "system-bus-unreachable",
            Self::AlreadyRunning => "already-running",
        }
    }
}

/// Runs the service until SIGTERM or SIGINT.
pub fn run() -> ExitCode {
    let destination = logging::init();
    tracing::info!(
        event_kind = %EventKind::ServiceStarted,
        version = env!("CARGO_PKG_VERSION"),
        log_destination = %destination,
        "Desktop Calls started"
    );
    let context = MainContext::default();
    let outcome = context.block_on(serve(GlibRuntime::new(context.clone())));
    match outcome {
        Ok(()) => {
            tracing::info!(event_kind = %EventKind::ServiceStopped, "Desktop Calls stopped");
            ExitCode::SUCCESS
        }
        Err(failure) => {
            tracing::error!(
                event_kind = %EventKind::ServiceFailed,
                error_kind = failure.as_str(),
                "Desktop Calls could not run"
            );
            ExitCode::FAILURE
        }
    }
}

async fn serve(runtime: GlibRuntime) -> Result<(), Failure> {
    let session = connect(zbus::connection::Builder::session(), &runtime)
        .await
        .map_err(|error| raw_failure(Failure::SessionBusUnreachable, &error))?;
    let system = connect(zbus::connection::Builder::system(), &runtime)
        .await
        .map_err(|error| raw_failure(Failure::SystemBusUnreachable, &error))?;
    let publisher = Publisher::serve(&session)
        .await
        .map_err(|error| raw_failure(Failure::AlreadyRunning, &error))?;

    let (input_sender, inputs) = unbounded();
    let (command_sender, commands) = unbounded();
    let context = MainContext::default();
    context.spawn_local(watch_bluez(system, input_sender.clone()));
    context.spawn_local(follow_phone(session, commands, input_sender));
    context.spawn_local(decide(inputs, command_sender, publisher));

    select(
        glib_unix::signal_future(SIGTERM),
        glib_unix::signal_future(SIGINT),
    )
    .await;
    Ok(())
}

/// The signal numbers are fixed by Linux; no crate is needed to name them.
const SIGINT: i32 = 2;
const SIGTERM: i32 = 15;

async fn connect(
    builder: zbus::connection::Builder<'_>,
    runtime: &GlibRuntime,
) -> zbus::Result<Connection> {
    builder.runtime(runtime.clone()).build().await
}

fn raw_failure(failure: Failure, error: &zbus::Error) -> Failure {
    tracing::error!(
        event_kind = %EventKind::ServiceFailed,
        error_kind = failure.as_str(),
        detail = %error,
        "Desktop Calls could not start"
    );
    failure
}

async fn watch_bluez(system: Connection, inputs: UnboundedSender<Input>) {
    if let Err(error) = bluez::run(system, inputs).await {
        tracing::error!(
            event_kind = %EventKind::WatchFailed,
            error_kind = error.as_str(),
            "Stopped watching BlueZ for paired Phones"
        );
    }
}

async fn follow_phone(
    session: Connection,
    commands: UnboundedReceiver<Command>,
    inputs: UnboundedSender<Input>,
) {
    if let Err(error) = phone_link::run(session, commands, inputs).await {
        tracing::error!(
            event_kind = %EventKind::WatchFailed,
            error_kind = error.as_str(),
            "Stopped following the Phone"
        );
    }
}

/// Feeds every input to the Call state machine, carries out its effects and
/// publishes the new state.
async fn decide(
    mut inputs: UnboundedReceiver<Input>,
    commands: UnboundedSender<Command>,
    publisher: Publisher,
) {
    let mut state = State::default();
    while let Some(input) = inputs.next().await {
        let (next, effects) = step(state.clone(), input);
        for effect in effects {
            let command = match effect {
                Effect::AttachPhone { address } => Command::Attach { address },
                Effect::DetachPhone => Command::Detach,
            };
            // The Phone link only stops when the bus connection closes, and
            // then the service has nothing left to do anyway.
            let _ = commands.unbounded_send(command);
        }
        if next.connection_state() != state.connection_state() {
            tracing::info!(
                event_kind = %EventKind::ConnectionStateChanged,
                connection_state = %next.connection_state(),
                phone_address = next.phone().map_or("", |phone| phone.address.as_str()),
                "The Phone is {}",
                next.connection_state()
            );
        }
        if let Err(error) = publisher.publish(&next).await {
            tracing::warn!(
                event_kind = %EventKind::PublishFailed,
                error_kind = "publish-failed",
                detail = %error,
                "Couldn't publish the new state on the control interface"
            );
        }
        state = next;
    }
}
