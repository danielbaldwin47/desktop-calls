//! The long-running process that the systemd user unit starts.
//!
//! One thread runs one GLib main context (a short-lived second one prepares
//! number formatting at startup). zbus and GTK run on it, zbus
//! through [`GlibRuntime`]; the BlueZ client, the Phone link, the control
//! interface, the Contacts sync and the surfaces turn what happens into
//! inputs; the Call state machine decides; this module carries out its effects,
//! publishes the state on the control interface, renders the surfaces, then
//! signals the `Error` and `MissedCall` the step caused.

use std::collections::BTreeMap;
use std::process::ExitCode;
use std::time::Duration;

use desktop_calls_core::{
    AudioLocation, ConnectionState, DialProgress, Effect, Input, MissedCallNotice, State, messages,
    step,
};
use futures_channel::mpsc::{UnboundedReceiver, UnboundedSender, unbounded};
use futures_util::StreamExt as _;
use futures_util::future::select;
use futures_util::stream;
use glib::MainContext;
use zbus::Connection;

use crate::control::{
    DialRequest, DialVerdict, Published, Publisher, ShownSurfaces, publish_failed,
};
use crate::do_not_disturb::DoNotDisturb;
use crate::glib_runtime::GlibRuntime;
use crate::indicator::Indicator;
use crate::logging::{self, EventKind};
use crate::notifications::Notifier;
use crate::phone_link::Command;
use crate::ringer::Ringer;
use crate::setup_checks::SetupChecks;
use crate::stall_watch::{self, StallWatch};
use crate::surfaces::Surfaces;
use crate::{bluez, chosen_phone, contacts, earlier_run, keybinding, logind, pairing, phone_link};

/// Why the service could not run.
#[derive(Clone, Copy, Debug)]
enum Failure {
    SessionBusUnreachable,
    SystemBusUnreachable,
    /// Another copy of Desktop Calls owns the control interface's name.
    AlreadyRunning,
    /// The control interface couldn't be served for any other reason.
    ControlInterfaceUnavailable,
}

impl Failure {
    fn as_str(self) -> &'static str {
        match self {
            Self::SessionBusUnreachable => "session-bus-unreachable",
            Self::SystemBusUnreachable => "system-bus-unreachable",
            Self::AlreadyRunning => "already-running",
            Self::ControlInterfaceUnavailable => "control-interface-unavailable",
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
    let (dial_sender, dials) = unbounded();
    // First, so a copy started beside another stops before it reads or
    // touches anything.
    let publisher = Publisher::serve(&session, input_sender.clone(), dial_sender)
        .await
        .map_err(|error| {
            let failure = match error {
                zbus::Error::NameTaken => Failure::AlreadyRunning,
                _ => Failure::ControlInterfaceUnavailable,
            };
            raw_failure(failure, &error)
        })?;
    // Before anything the Phone reports, so its Calls are known for what
    // they were before a crash.
    if let Some(earlier) = earlier_run::load() {
        let _ = input_sender.unbounded_send(Input::EarlierRun(earlier));
    }
    // Before BlueZ lists the paired Phones, so the one chosen last is the
    // one followed.
    if let Some(address) = chosen_phone::load() {
        let _ = input_sender.unbounded_send(Input::ChosenPhoneKept { address });
    }

    let (command_sender, commands) = unbounded();
    let (contacts_sender, contacts_commands) = unbounded();
    let (report_sender, reports) = unbounded();
    let surfaces = has_display.then(|| Surfaces::new(&input_sender, report_sender));
    // Without the Indicator, Desktop Calls still works from its other
    // surfaces and the command line.
    let indicator = Indicator::serve(&session, input_sender.clone())
        .await
        .inspect_err(indicator_failed)
        .ok();
    let notifier = Notifier::start(&session, input_sender.clone(), publisher.clone());
    // Held until the service stops.
    let _do_not_disturb = DoNotDisturb::follow(input_sender.clone());
    let setup_checks = SetupChecks::start(system.clone(), session.clone(), input_sender.clone());
    let context = MainContext::default();
    context.spawn_local(watch_bluez(system.clone(), input_sender.clone()));
    context.spawn_local(watch_sleep(system.clone(), input_sender.clone()));
    context.spawn_local(contacts::run(
        session.clone(),
        contacts_commands,
        input_sender.clone(),
    ));
    context.spawn_local(follow_phone(
        session,
        system.clone(),
        commands,
        input_sender.clone(),
    ));
    context.spawn_local(publish_surfaces(reports, publisher.clone()));
    context.spawn_local(decide(
        inputs,
        dials,
        Shell {
            pairing: pairing::Pairing::new(system.clone(), input_sender.clone()),
            setup_checks,
            inputs: input_sender,
            system,
            commands: command_sender,
            contacts: contacts_sender,
            publisher,
            surfaces,
            indicator,
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

async fn watch_sleep(system: Connection, inputs: UnboundedSender<Input>) {
    if let Err(error) = logind::run(system, inputs).await {
        tracing::warn!(
            event_kind = %EventKind::WatchFailed,
            error_kind = "signals-unavailable",
            detail = %error,
            "Stopped watching logind for suspend and resume"
        );
    }
}

async fn follow_phone(
    session: Connection,
    system: Connection,
    commands: UnboundedReceiver<Command>,
    inputs: UnboundedSender<Input>,
) {
    if let Err(error) = phone_link::run(session, system, commands, inputs).await {
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

/// An effect that says something about the Call state its step reaches, so
/// it waits until that state is published.
enum Announcement {
    /// An `Error` signal.
    Error(messages::Failure),
    /// A Missed Call: its notification, then its `MissedCall` signal.
    MissedCall(MissedCallNotice),
}

/// What carries out the Call state machine's effects.
struct Shell {
    /// Where timers report back.
    inputs: UnboundedSender<Input>,
    /// The system bus, for connecting the Phone through BlueZ.
    system: Connection,
    commands: UnboundedSender<Command>,
    contacts: UnboundedSender<contacts::Command>,
    publisher: Publisher,
    surfaces: Option<Surfaces>,
    indicator: Option<Indicator>,
    ringer: Ringer,
    notifier: Notifier,
    stall_watch: StallWatch,
    /// Carries out the Setup Window's pairing on BlueZ.
    pairing: pairing::Pairing,
    /// Finds what the Desktop lacks, and puts it right.
    setup_checks: SetupChecks,
}

impl Shell {
    /// Carries out `effects`, except the ones that announce something about
    /// the new state, which it hands back for [`Shell::announce`] to carry
    /// out once that state is published.
    fn carry_out(&mut self, effects: Vec<Effect>) -> Vec<Announcement> {
        let mut phone_commands = Vec::new();
        let mut announcements = Vec::new();
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
                Effect::ConnectPhone { address, attempt } => {
                    let system = self.system.clone();
                    let inputs = self.inputs.clone();
                    // A Phone out of range takes BlueZ's page timeout to
                    // fail; the Desktop carries on meanwhile.
                    MainContext::default().spawn_local(async move {
                        let input = match bluez::connect_phone(&system, &address).await {
                            Ok(()) => Input::ConnectPhoneDone { attempt },
                            Err(failure) => Input::ConnectPhoneFailed { attempt, failure },
                        };
                        let _ = inputs.unbounded_send(input);
                    });
                }
                Effect::KeepChosenPhone { address } => chosen_phone::save(&address),
                Effect::SyncContacts => self.contacts_command(contacts::Command::Sync),
                Effect::RefreshCallHistory => {
                    self.contacts_command(contacts::Command::RefreshCallHistory);
                }
                Effect::ForgetContacts => self.contacts_command(contacts::Command::Forget),
                Effect::NotifyMissedCall(notice) => {
                    announcements.push(Announcement::MissedCall(notice));
                }
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
                effect @ (Effect::StartSetup
                | Effect::StopSetup
                | Effect::PairPhone { .. }
                | Effect::AnswerPairingCode { .. }
                | Effect::TrustPhone { .. }
                | Effect::ConnectHandsFree { .. }) => self.pairing.carry_out(effect),
                Effect::CheckDesktop => self.setup_checks.check_again(),
                Effect::Fix(fix) => self.setup_checks.fix(fix),
                Effect::CheckKeybinding => {
                    let inputs = self.inputs.clone();
                    std::thread::spawn(move || {
                        let _ =
                            inputs.unbounded_send(Input::KeybindingChecked(keybinding::check()));
                    });
                }
                Effect::AddKeybinding { file, seen, added } => {
                    let inputs = self.inputs.clone();
                    std::thread::spawn(move || {
                        let _ = inputs.unbounded_send(keybinding::add(&file, &seen, &added));
                    });
                }
                // The decide loop passes these on to whoever asked to dial.
                Effect::DialProgress(_) => {}
                Effect::EarlierCallsForgotten(calls) => earlier_run::forgotten(calls),
                Effect::ReportError(failure) => announcements.push(Announcement::Error(failure)),
            }
        }
        // One step's Phone commands run in order, as one batch.
        if !phone_commands.is_empty() {
            self.command(Command::Run(phone_commands));
        }
        announcements
    }

    /// Tells the user and the control interface's clients what `announcements`
    /// say, once the state they describe is `published`: an `Error` or a
    /// `MissedCall` signal never comes before the Call state it follows from
    /// (ticket #116). A Missed Call's notification waits for it too.
    async fn announce(&self, announcements: Vec<Announcement>, published: &Published) {
        for announcement in announcements {
            match announcement {
                Announcement::MissedCall(notice) => self.notifier.notify(notice, published),
                Announcement::Error(failure) => {
                    tracing::info!(
                        event_kind = %EventKind::ErrorReported,
                        error_kind = failure.as_str(),
                        "Told the user: {}",
                        messages::failure(failure).text
                    );
                    if let Err(error) = self.publisher.report_error(failure, published).await {
                        publish_failed(&error);
                    }
                }
            }
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
/// publishes the new state, renders the surfaces, then signals the `Error`
/// and `MissedCall` the step caused. A Dial request's
/// verdict waits for what the Call state machine says became of its dial
/// attempt: placed, or not placed and why.
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
    // Dial requests waiting for the Phone, by dial attempt.
    let mut waiting = BTreeMap::new();
    while let Some((input, verdict)) = inputs.next().await {
        requested(&input);
        let (next, effects, shown) = stall_watch::timed("deciding", || {
            let (next, effects) = step(state.clone(), input);
            let shown = next.surfaces();
            (next, effects, shown)
        });
        // Before carrying anything out, so a stall in this very turn shows.
        shell.stall_watch.watch(
            !next.calls().is_empty() || shown.banner.is_some() || shown.call_window.is_some(),
        );
        pass_on_dial_progress(&effects, verdict, &mut waiting);
        let announcements = shell.carry_out(effects);
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
            // Before publishing, so no first-frame work runs while it waits.
            surfaces.prepare_first_frames(
                *next.connection_state() == ConnectionState::Connected
                    && next.calls().is_empty()
                    && shown == desktop_calls_core::Surfaces::default(),
            );
        }
        // Cause, then effect: rendering needs the proof that `next` is
        // published (ticket #107).
        let published = shell.publisher.publish(&next).await;
        if let Some(surfaces) = &shell.surfaces {
            surfaces.render(&shown, &published);
        }
        if let Some(indicator) = &shell.indicator
            && let Err(error) = indicator.render(&next.indicator(&shown), &published).await
        {
            indicator_failed(&error);
        }
        // After the surfaces, which the user is waiting to see.
        shell.announce(announcements, &published).await;
        state = next;
    }
}

/// Gives the Dial requests waiting for a dial attempt its outcome. `verdict`
/// is where this step's own Dial request, if any, wants its outcome: the
/// step names its attempt, settled or asked of the Phone.
fn pass_on_dial_progress(
    effects: &[Effect],
    mut verdict: Option<DialVerdict>,
    waiting: &mut BTreeMap<u32, DialVerdict>,
) {
    for effect in effects {
        let Effect::DialProgress(progress) = effect else {
            continue;
        };
        let (attempt, outcome) = match *progress {
            DialProgress::Asked(attempt) => {
                if let Some(verdict) = verdict.take() {
                    waiting.insert(attempt, verdict);
                }
                continue;
            }
            DialProgress::Placed(attempt) => (attempt, None),
            DialProgress::NotPlaced(attempt, failure) => (attempt, Some(failure)),
        };
        if let Some(verdict) = waiting.remove(&attempt).or_else(|| verdict.take()) {
            // The caller may have given up waiting; nothing is lost.
            let _ = verdict.send(outcome);
        }
    }
}

fn indicator_failed(error: &zbus::Error) {
    tracing::warn!(
        event_kind = %EventKind::PublishFailed,
        error_kind = "indicator-failed",
        detail = %error,
        "Couldn't show the Indicator in the tray"
    );
}

/// When the user asked for something the polish scorecard times, from the
/// control interface or a surface alike, so the app's own time can be told
/// apart from its client's start-up (tickets #80 and #83).
fn requested(input: &Input) {
    match input {
        Input::MoveAudio(location) => handoff_requested(*location),
        Input::Answer(call) => tracing::info!(
            event_kind = %EventKind::AnswerRequested,
            call_id = call.0,
            "Asked to answer Call {call}"
        ),
        Input::Activate => tracing::info!(
            event_kind = %EventKind::ActivateRequested,
            "Asked to activate, as the keybinding does"
        ),
        // The Indicator, its menu, `desktop-calls dialer` and the control
        // interface open the Dialer this way: the Dialer's first frame is
        // timed from here too (ticket #125).
        Input::OpenDialer { .. } => tracing::info!(
            event_kind = %EventKind::ActivateRequested,
            "Asked to open the Dialer"
        ),
        _ => {}
    }
}

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
