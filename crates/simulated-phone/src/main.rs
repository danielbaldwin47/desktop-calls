//! The Simulated Phone: a stand-in for the Phone that the Desktop cannot tell
//! apart from a real one.
//!
//! It serves `org.bluez` on the system bus and `org.pipewire.Telephony` on the
//! session bus, shaped like the real Pixel 9a behind PipeWire 1.6.9, on
//! whichever buses `DBUS_SYSTEM_BUS_ADDRESS` and `DBUS_SESSION_BUS_ADDRESS`
//! name. Tests act as the Phone through its control interface,
//! `io.github.danielbaldwin47.DesktopCalls.SimulatedPhone1` on the session bus,
//! and read back every method call it received.
//!
//! With `--scenario FILE` it also acts out a declarative scenario file once
//! it is on the bus (see `scenario.rs`); `just demo` uses this.
//!
//! So far it can only be a paired Phone whose hands-free link comes and goes.

mod bluez;
mod objects;
mod scenario;
mod telephony;

use std::collections::HashMap;
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use clap::Parser;
use futures_util::StreamExt as _;
use zbus::object_server::SignalEmitter;
use zbus::{Connection, MessageStream, OwnedObjectPath, OwnedVariant, interface};

use crate::objects::{MethodCall, ObjectTree, PROPERTIES, interfaces_added_body};
use crate::scenario::Step;

const CONTROL_BUS_NAME: &str = "io.github.danielbaldwin47.DesktopCalls.SimulatedPhone";
const CONTROL_PATH: &str = "/io/github/danielbaldwin47/DesktopCalls/SimulatedPhone";

/// Stand in for the Phone on private buses.
#[derive(Parser)]
#[command(name = "simulated-phone")]
struct Arguments {
    /// The Phone's Bluetooth name.
    #[arg(long, default_value = "Pixel 9a")]
    name: String,
    /// The Phone's Bluetooth address.
    #[arg(long, default_value = "FC:41:16:00:00:01")]
    address: String,
    /// N in the first AudioGateway's path, `/org/pipewire/Telephony/agN`.
    /// Each reconnect registers the next one.
    #[arg(long, default_value_t = 1)]
    first_audio_gateway_index: u32,
    /// A scenario file to act out once on the bus.
    #[arg(long)]
    scenario: Option<PathBuf>,
}

/// Which bus a method call arrived on.
#[derive(Clone, Copy)]
enum Bus {
    System,
    Session,
}

impl Bus {
    fn as_str(self) -> &'static str {
        match self {
            Self::System => "system",
            Self::Session => "session",
        }
    }
}

/// Everything the Simulated Phone is and has been asked.
struct World {
    phone_address: String,
    bluez: ObjectTree,
    telephony: ObjectTree,
    next_audio_gateway_index: u32,
    audio_gateway: Option<OwnedObjectPath>,
    recorded: Vec<(Bus, MethodCall)>,
}

type SharedWorld = Arc<Mutex<World>>;

fn lock(world: &SharedWorld) -> MutexGuard<'_, World> {
    world.lock().unwrap_or_else(PoisonError::into_inner)
}

fn main() -> ExitCode {
    let arguments = Arguments::parse();
    let steps = match arguments.scenario.as_deref().map(read_scenario) {
        None => Vec::new(),
        Some(Ok(steps)) => steps,
        Some(Err(error)) => {
            eprintln!("simulated-phone: {error}");
            return ExitCode::FAILURE;
        }
    };
    match zbus::block_on(run(arguments, steps)) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("simulated-phone: {error}");
            ExitCode::FAILURE
        }
    }
}

fn read_scenario(path: &std::path::Path) -> Result<Vec<Step>, String> {
    let text = std::fs::read_to_string(path)
        .map_err(|error| format!("read {}: {error}", path.display()))?;
    scenario::parse(&text).map_err(|error| format!("{}: {error}", path.display()))
}

async fn run(arguments: Arguments, steps: Vec<Step>) -> zbus::Result<()> {
    let world = Arc::new(Mutex::new(World {
        bluez: bluez::new_tree(&arguments.name, &arguments.address)?,
        telephony: telephony::new_tree()?,
        phone_address: arguments.address,
        next_audio_gateway_index: arguments.first_audio_gateway_index,
        audio_gateway: None,
        recorded: Vec::new(),
    }));

    let system = zbus::connection::Builder::system().build().await?;
    let system_calls = MessageStream::from(&system);
    system.request_name(bluez::SERVICE).await?;

    let session = zbus::connection::Builder::session().build().await?;
    let session_calls = MessageStream::from(&session);
    session.request_name(telephony::SERVICE).await?;

    // The control name comes last, so a test that sees it can rely on the rest.
    let control = zbus::connection::Builder::session()
        .serve_at(
            CONTROL_PATH,
            Control {
                world: world.clone(),
                system: system.clone(),
                session: session.clone(),
            },
        )
        .name(CONTROL_BUS_NAME)
        .build()
        .await?;

    futures_util::future::try_join3(
        serve(system, system_calls, Bus::System, world.clone()),
        serve(session, session_calls, Bus::Session, world),
        act_out(&control, steps),
    )
    .await?;
    Ok(())
}

/// Runs a scenario's steps in order through the control interface, as a
/// test would.
async fn act_out(control: &Connection, steps: Vec<Step>) -> zbus::Result<()> {
    let interface = control
        .object_server()
        .interface::<_, Control>(CONTROL_PATH)
        .await?;
    for step in steps {
        let emitter = interface.signal_emitter().clone();
        let control = interface.get().await;
        match step {
            Step::Connect => control.connect(emitter).await?,
            Step::Disconnect => control.disconnect(emitter).await?,
        }
    }
    Ok(())
}

/// Answers every method call that arrives on one bus, and records it.
async fn serve(
    connection: Connection,
    mut messages: MessageStream,
    bus: Bus,
    world: SharedWorld,
) -> zbus::Result<()> {
    while let Some(message) = messages.next().await {
        let message = message?;
        let Some(call) = MethodCall::from_message(&message) else {
            continue;
        };
        let reply = {
            let mut world = lock(&world);
            world.recorded.push((bus, call.clone()));
            match bus {
                Bus::System => bluez::answer(&world.bluez, &call, &message),
                Bus::Session => {
                    let audio_gateways: Vec<OwnedObjectPath> =
                        world.audio_gateway.iter().cloned().collect();
                    telephony::answer(&world.telephony, &audio_gateways, &call, &message)
                }
            }
        };
        reply.send(&connection, &message.header()).await?;
    }
    Ok(())
}

/// What tests use to act as the Phone.
struct Control {
    world: SharedWorld,
    system: Connection,
    session: Connection,
}

#[interface(interface = "io.github.danielbaldwin47.DesktopCalls.SimulatedPhone1")]
impl Control {
    /// Brings hands-free up, as a Phone coming into range does: BlueZ marks the
    /// Phone connected, then, once the service level connection is set up,
    /// PipeWire registers a new AudioGateway. Does nothing if it is up already.
    async fn connect(
        &self,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
    ) -> zbus::fdo::Result<()> {
        let (device, gateway, interfaces) = {
            let mut world = lock(&self.world);
            if world.audio_gateway.is_some() {
                return Ok(());
            }
            let device = bluez::device_path(&world.phone_address)?;
            world.bluez.set(
                &device,
                bluez::DEVICE,
                "Connected",
                OwnedVariant::from(true),
            );
            let gateway = telephony::audio_gateway_path(world.next_audio_gateway_index)?;
            world.next_audio_gateway_index += 1;
            let address = world.phone_address.clone();
            let interfaces = telephony::add_audio_gateway(&mut world.telephony, &gateway, &address);
            world.audio_gateway = Some(gateway.clone());
            (device, gateway, interfaces)
        };
        emit_properties_changed(&self.system, &device, bluez::DEVICE, "Connected", true).await?;
        self.session
            .emit_signal(
                None::<&str>,
                telephony::MANAGER_PATH,
                objects::OBJECT_MANAGER,
                "InterfacesAdded",
                &(gateway, interfaces_added_body(&interfaces)),
            )
            .await?;
        self.audio_gateway_changed(&emitter).await?;
        Ok(())
    }

    /// Drops hands-free, as a Phone going out of range does: PipeWire removes
    /// the AudioGateway, then BlueZ marks the Phone disconnected. Does nothing
    /// if it is down already.
    async fn disconnect(
        &self,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
    ) -> zbus::fdo::Result<()> {
        let (device, gateway) = {
            let mut world = lock(&self.world);
            let Some(gateway) = world.audio_gateway.take() else {
                return Ok(());
            };
            world.telephony.remove(&gateway);
            let device = bluez::device_path(&world.phone_address)?;
            world.bluez.set(
                &device,
                bluez::DEVICE,
                "Connected",
                OwnedVariant::from(false),
            );
            (device, gateway)
        };
        self.session
            .emit_signal(
                None::<&str>,
                telephony::MANAGER_PATH,
                objects::OBJECT_MANAGER,
                "InterfacesRemoved",
                &(gateway, telephony::AUDIO_GATEWAY_INTERFACES.to_vec()),
            )
            .await?;
        emit_properties_changed(&self.system, &device, bluez::DEVICE, "Connected", false).await?;
        self.audio_gateway_changed(&emitter).await?;
        Ok(())
    }

    /// The current AudioGateway's object path, or `/` while there is none.
    #[zbus(property)]
    fn audio_gateway(&self) -> OwnedObjectPath {
        lock(&self.world).audio_gateway.clone().unwrap_or_default()
    }

    /// Every method call received on the Phone's side of either bus, in
    /// order: the bus (`"system"` or `"session"`), object path, interface,
    /// member and the arguments in GVariant text form.
    fn recorded_method_calls(&self) -> Vec<(String, String, String, String, String)> {
        lock(&self.world)
            .recorded
            .iter()
            .map(|(bus, call)| {
                (
                    bus.as_str().to_owned(),
                    call.path.clone(),
                    call.interface.clone(),
                    call.member.clone(),
                    call.arguments.clone(),
                )
            })
            .collect()
    }
}

async fn emit_properties_changed(
    connection: &Connection,
    path: &OwnedObjectPath,
    interface: &str,
    name: &str,
    value: bool,
) -> zbus::Result<()> {
    let changed = HashMap::from([(name.to_owned(), OwnedVariant::from(value))]);
    connection
        .emit_signal(
            None::<&str>,
            path.as_str(),
            PROPERTIES,
            "PropertiesChanged",
            &(interface, changed, Vec::<String>::new()),
        )
        .await
}
