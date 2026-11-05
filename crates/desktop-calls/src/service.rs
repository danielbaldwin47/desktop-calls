//! The long-running process that the systemd user unit starts.
//!
//! One thread runs one GLib main context (a short-lived second one prepares
//! number formatting at startup). zbus and GTK run on it, zbus
//! through [`GlibRuntime`]; the BlueZ client, the Phone link, the control
//! interface, the Contacts sync and the surfaces turn what happens into
//! inputs; the Call state machine decides; this module carries out its effects, renders the
//! surfaces and publishes the state on the control interface.

use std::process::ExitCode;
use std::time::Duration;

use desktop_calls_core::{AudioLocation, Effect, Input, State, messages, step};
use futures_channel::mpsc::{UnboundedReceiver, UnboundedSender, unbounded};
use futures_util::StreamExt as _;
use futures_util::future::select;
use futures_util::stream;
use glib::MainContext;
use zbus::Connection;

use crate::control::{DialRequest, Publisher, ShownSurfaces};
use crate::glib_runtime::GlibRuntime;
use crate::logging::{self, EventKind};
use crate::notifications::Notifier;
use crate::phone_link::Command;
use crate::ringer::Ringer;
use crate::stall_watch::{self, StallWatch};
use crate::surfaces::Surfaces;
use crate::{bluez, contacts, earlier_run, phone_link};

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
    prepare_caller();
    // GTK needs a display. Without one (a headless session, a test with no
    // screen) Desktop Calls still runs and follows Calls, with no surfaces.
    let has_display = match adw::init() {
        Ok(()) => true,
        Err(error) => {
            tracing::warn!(
                event_kind = %EventKind::SurfacesUnavailable,
                error_kind = "no-display",
                detail = %error,
                "No display to show the Banner and Call Window on"
            );
            false
        }
    };
    let context = MainContext::default();
    let outcome = context.block_on(serve(GlibRuntime::new(context.clone()), has_display));
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

/// Formatting the first caller's number loads the phone-number metadata,
/// which held the main thread, and so the Banner, for 0.3 s on the Desktop
/// and over a second on a CI runner (debug builds). It loads on a thread of
/// its own while GTK starts; if the thread can't start, it loads now, still
/// before any Call.
fn prepare_caller() {
    let spawned = std::thread::Builder::new()
        .name("prepare-caller".to_owned())
        .spawn(messages::prepare_caller);
    if spawned.is_err() {
        messages::prepare_caller();
    }
}

async fn serve(runtime: GlibRuntime, has_display: bool) -> Result<(), Failure> {
    let session = connect(zbus::connection::Builder::session(), &runtime)
        .await
        .map_err(|error| raw_failure(Failure::SessionBusUnreachable, &error))?;
    let system = connect(zbus::connection::Builder::system(), &runtime)
        .await
        .map_err(|error| raw_failure(Failure::SystemBusUnreachable, &error))?;
    let (input_sender, inputs) = unbounded();
    // Before anything the Phone reports, so its Calls are known for what
    // they were before a crash.
    if let Some(earlier) = earlier_run::load() {
        let _ = input_sender.unbounded_send(Input::EarlierRun(earlier));
    }
    let (dial_sender, dials) = unbounded();
    let publisher = Publisher::serve(&session, input_sender.clone(), dial_sender)
        .await
        .map_err(|error| raw_failure(Failure::AlreadyRunning, &error))?;

    let (command_sender, commands) = unbounded();
    let (contacts_sender, contacts_commands) = unbounded();
    let (report_sender, reports) = unbounded();
    let surfaces = has_display.then(|| Surfaces::new(&input_sender, report_sender));
    let notifier = Notifier::start(&session, input_sender.clone(), publisher.clone());
    let context = MainContext::default();
    context.spawn_local(watch_bluez(system, input_sender.clone()));
    context.spawn_local(contacts::run(
        session.clone(),
        contacts_commands,
        input_sender.clone(),
    ));
    context.spawn_local(follow_phone(session, commands, input_sender.clone()));
    context.spawn_local(publish_surfaces(reports, publisher.clone()));
    context.spawn_local(decide(
        inputs,
        dials,
        Shell {
            inputs: input_sender,
            commands: command_sender,
            contacts: contacts_sender,
            publisher,
            surfaces,
            ringer: Ringer::default(),
            notifier,
            stall_watch: StallWatch::default(),
        },
    ));

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

/// Publishes what the surfaces show, as they report it.
async fn publish_surfaces(mut reports: UnboundedReceiver<ShownSurfaces>, publisher: Publisher) {
    while let Some(shown) = reports.next().await {
        if let Err(error) = publisher.publish_surfaces(shown).await {
            publish_failed(&error);
        }
    }
}

/// What carries out the Call state machine's effects.
struct Shell {
    /// Where timers report back.
    inputs: UnboundedSender<Input>,
    commands: UnboundedSender<Command>,
    contacts: UnboundedSender<contacts::Command>,
    publisher: Publisher,
    surfaces: Option<Surfaces>,
    ringer: Ringer,
    notifier: Notifier,
    stall_watch: StallWatch,
}

impl Shell {
    async fn carry_out(&mut self, effects: Vec<Effect>) {
        let mut phone_commands = Vec::new();
        for effect in effects {
            match effect {
                Effect::AttachPhone { address } => {
                    self.contacts_command(contacts::Command::Attach {
                        address: address.clone(),
                    });
                    self.command(Command::Attach { address });
                }
                Effect::DetachPhone => {
                    self.contacts_command(contacts::Command::Detach);
                    self.command(Command::Detach);
                }
                Effect::SyncContacts => self.contacts_command(contacts::Command::Sync),
                Effect::RefreshCallHistory => {
                    self.contacts_command(contacts::Command::RefreshCallHistory);
                }
                Effect::ForgetContacts => self.contacts_command(contacts::Command::Forget),
                Effect::NotifyMissedCall(notice) => self.notifier.notify(notice),
                Effect::Phone(command) => phone_commands.push(command),
                Effect::StartRinging => self.ringer.start(),
                Effect::StopRinging => self.ringer.stop(),
                Effect::StartTimer { timer, after_ms } => {
                    let inputs = self.inputs.clone();
                    MainContext::default().spawn_local(async move {
                        glib::timeout_future(Duration::from_millis(u64::from(after_ms))).await;
                        let _ = inputs.unbounded_send(Input::TimerFired(timer));
                    });
                }
                Effect::ReportError(failure) => {
                    tracing::info!(
                        event_kind = %EventKind::ErrorReported,
                        error_kind = failure.as_str(),
                        "Told the user: {}",
                        desktop_calls_core::messages::failure(failure).text
                    );
                    if let Err(error) = self.publisher.report_error(failure).await {
                        publish_failed(&error);
                    }
                }
            }
        }
        // One step's Phone commands run in order, as one batch.
        if !phone_commands.is_empty() {
            self.command(Command::Run(phone_commands));
        }
    }

    fn contacts_command(&self, command: contacts::Command) {
        // As for the Phone link: the sync stops only with the bus.
        let _ = self.contacts.unbounded_send(command);
    }

    fn command(&self, command: Command) {
        // The Phone link only stops when the bus connection closes, and then
        // the service has nothing left to do anyway.
        let _ = self.commands.unbounded_send(command);
    }
}

/// Feeds every input to the Call state machine, carries out its effects,
/// renders the surfaces and publishes the new state. A Dial request's
/// verdict is the failure its step reports, if any.
async fn decide(
    inputs: UnboundedReceiver<Input>,
    dials: UnboundedReceiver<DialRequest>,
    mut shell: Shell,
) {
    let mut inputs = stream::select(
        inputs.map(|input| (input, None)),
        dials.map(|(number, verdict)| (Input::Dial(number), Some(verdict))),
    );
    let mut state = State::default();
    while let Some((input, verdict)) = inputs.next().await {
        if let Input::MoveAudio(location) = &input {
            handoff_requested(*location);
        }
        let (next, effects, shown) = stall_watch::timed("deciding", || {
            let (next, effects) = step(state.clone(), input);
            let shown = next.surfaces();
            (next, effects, shown)
        });
        // Before carrying anything out, so a stall in this very turn shows.
        shell.stall_watch.watch(
            !next.calls().is_empty() || shown.banner.is_some() || shown.call_window.is_some(),
        );
        if let Some(verdict) = verdict {
            let refusal = effects.iter().find_map(|effect| match effect {
                Effect::ReportError(failure) => Some(*failure),
                _ => None,
            });
            // The caller may have given up waiting; nothing is lost.
            let _ = verdict.send(refusal);
        }
        shell.carry_out(effects).await;
        let next_run = next.for_next_run();
        if next_run != state.for_next_run() {
            earlier_run::save(&next_run);
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
        if let (Some(from), Some(to)) = (state.audio_location(), next.audio_location())
            && from != to
        {
            audio_moved(to);
        }
        if let Some(surfaces) = &shell.surfaces {
            surfaces.render(&shown);
        }
        if let Err(error) = shell.publisher.publish(&next).await {
            publish_failed(&error);
        }
        state = next;
    }
}

/// When a Handoff was asked for, from the control interface or the Call
/// Window alike, so its own time can be told apart from its client's
/// start-up (ticket #80).
fn handoff_requested(location: AudioLocation) {
    tracing::info!(
        event_kind = %EventKind::HandoffRequested,
        audio_location = location.as_str(),
        "Asked for a Handoff to the {}",
        place(location)
    );
}

/// When a live Call's audio changed place, by a Handoff or the Phone's own
/// button.
fn audio_moved(location: AudioLocation) {
    tracing::info!(
        event_kind = %EventKind::AudioMoved,
        audio_location = location.as_str(),
        "The Call audio is on the {}",
        place(location)
    );
}

fn place(location: AudioLocation) -> &'static str {
    match location {
        AudioLocation::Desktop => "Desktop",
        AudioLocation::Phone => "Phone",
    }
}

fn publish_failed(error: &zbus::Error) {
    tracing::warn!(
        event_kind = %EventKind::PublishFailed,
        error_kind = "publish-failed",
        detail = %error,
        "Couldn't publish the new state on the control interface"
    );
}
