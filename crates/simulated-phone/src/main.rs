//! The Simulated Phone: a stand-in for the Phone that the Desktop cannot tell
//! apart from a real one.
//!
//! It serves `org.bluez` on the system bus, with logind's `PrepareForSleep`
//! beside it, and `org.pipewire.Telephony` and
//! `org.bluez.obex` on the session bus, shaped like the real Pixel 9a behind
//! PipeWire 1.6.9 and BlueZ 5.87, on whichever buses `DBUS_SYSTEM_BUS_ADDRESS`
//! and `DBUS_SESSION_BUS_ADDRESS` name. Each service has a connection of its
//! own, as each real service is a process of its own. Tests act as the Phone
//! through its control interface,
//! `io.github.danielbaldwin47.DesktopCalls.SimulatedPhone1` on the session bus,
//! and read back every method call it received. A conformance test
//! (`crates/test-harness/tests/conformance.rs`) holds it to the snapshot of
//! the real objects.
//!
//! With `--scenario FILE` it also acts out a declarative scenario file once
//! it is on the bus (see `scenario.rs`); `just demo` uses this.
//!
//! So far it can be a paired Phone that goes out of range and comes back
//! with its Calls carrying on, that the Desktop connects with `Connect`,
//! that forgets the pairing, whose hands-free link
//! drops and comes back for the Desktop with the Call carrying on, that
//! rings, that dials for the Desktop, whose Telephony service restarts as
//! `WirePlumber` does, whose Calls can be answered, hung up
//! and have their audio brought to the Desktop, and that opens a PBAP
//! session. Tests can pick the codec, how long the Phone takes to act on
//! `Activate`, and switch on quirks other Phones show.

mod bluez;
mod hands_free;
mod logind;
mod obex;
mod objects;
mod pairing;
mod range;
mod restart;
mod scenario;
mod telephony;

use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use clap::Parser;
use futures_util::StreamExt as _;
use zbus::object_server::SignalEmitter;
use zbus::{Connection, Message, MessageStream, OwnedObjectPath, interface};

use crate::objects::{MethodCall, ObjectTree, PROPERTIES, Reply};
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
    #[command(flatten)]
    nearby: pairing::Nearby,
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

/// Which real service a connection stands in for.
#[derive(Clone, Copy)]
pub(crate) enum Service {
    Bluez,
    Telephony,
    Obex,
}

impl Service {
    fn bus(self) -> Bus {
        match self {
            Self::Bluez => Bus::System,
            Self::Telephony | Self::Obex => Bus::Session,
        }
    }
}

/// Everything the Simulated Phone is and has been asked.
struct World {
    phone_address: String,
    bluez: ObjectTree,
    telephony: ObjectTree,
    obex: obex::Obex,
    next_audio_gateway_index: u32,
    audio_gateway: Option<OwnedObjectPath>,
    /// Hands-free dropped for `DisconnectProfile`, until it is back.
    dropped: Option<hands_free::Dropped>,
    range: range::Range,
    pairing: pairing::Pairing,
    behaviour: telephony::Behaviour,
    dialling: telephony::Dialling,
    recorded: Vec<(Bus, MethodCall)>,
}

type SharedWorld = Arc<Mutex<World>>;

fn lock<T>(shared: &Mutex<T>) -> MutexGuard<'_, T> {
    shared.lock().unwrap_or_else(PoisonError::into_inner)
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
    let mut bluez = bluez::new_tree(&arguments.name, &arguments.address)?;
    let device = bluez::device_path(&arguments.address)?;
    let mut range = range::Range::new(&mut bluez, &device);
    let pairing = pairing::Pairing::new(&mut bluez, &device, &arguments.nearby, &mut range)?;
    let world = Arc::new(Mutex::new(World {
        bluez,
        range,
        pairing,
        telephony: telephony::new_tree()?,
        obex: obex::new()?,
        phone_address: arguments.address,
        next_audio_gateway_index: arguments.first_audio_gateway_index,
        audio_gateway: None,
        dropped: None,
        behaviour: telephony::Behaviour::default(),
        dialling: telephony::Dialling::default(),
        recorded: Vec::new(),
    }));

    let system = zbus::connection::Builder::system().build().await?;
    let system_calls = MessageStream::from(&system);
    system.request_name(bluez::SERVICE).await?;

    let session = zbus::connection::Builder::session().build().await?;
    let session_calls = MessageStream::from(&session);
    session.request_name(telephony::SERVICE).await?;

    let obex = zbus::connection::Builder::session().build().await?;
    let obex_calls = MessageStream::from(&obex);
    obex.request_name(obex::SERVICE).await?;

    let logind = logind::connect().await?;
    let telephony: TelephonyConnection = Arc::new(Mutex::new(Some(session.clone())));
    // The control name comes last, so a test that sees it can rely on the rest.
    let control = zbus::connection::Builder::session()
        .serve_at(
            CONTROL_PATH,
            Control {
                world: world.clone(),
                system: system.clone(),
                telephony: telephony.clone(),
                logind,
            },
        )
        .name(CONTROL_BUS_NAME)
        .build()
        .await?;

    futures_util::future::try_join4(
        serve(
            system,
            system_calls,
            Service::Bluez,
            world.clone(),
            telephony.clone(),
        ),
        serve(
            session,
            session_calls,
            Service::Telephony,
            world.clone(),
            telephony.clone(),
        ),
        serve(obex, obex_calls, Service::Obex, world, telephony),
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

/// Answers every method call that arrives for one service, and records it.
pub(crate) async fn serve(
    connection: Connection,
    mut messages: MessageStream,
    service: Service,
    world: SharedWorld,
    telephony: TelephonyConnection,
) -> zbus::Result<()> {
    while let Some(message) = messages.next().await {
        let message = message?;
        let Some(call) = MethodCall::from_message(&message) else {
            continue;
        };
        if matches!(service, Service::Bluez) && hands_free::handles(&call) {
            lock(&world).recorded.push((service.bus(), call.clone()));
            hands_free::answer(
                world.clone(),
                connection.clone(),
                telephony.clone(),
                call,
                message,
            );
            continue;
        }
        if matches!(service, Service::Bluez) && pairing::handles(&call) {
            lock(&world).recorded.push((service.bus(), call.clone()));
            pairing::answer(world.clone(), connection.clone(), call, message);
            continue;
        }
        if matches!(service, Service::Bluez) && range::handles(&call) {
            lock(&world).recorded.push((service.bus(), call.clone()));
            range::answer_connect(
                world.clone(),
                connection.clone(),
                telephony.clone(),
                call,
                message,
            );
            continue;
        }
        let (reply, signals, later, activate_later, slow) = {
            let mut world = lock(&world);
            world.recorded.push((service.bus(), call.clone()));
            let world = &mut *world;
            let slow = matches!(service, Service::Telephony)
                && call.interface == PROPERTIES
                && call.member == "Set"
                && world
                    .behaviour
                    .quirks
                    .contains(&telephony::Quirk::SlowPropertyWrites);
            match service {
                Service::Bluez => (
                    bluez::answer(&world.bluez, &call, &message),
                    Vec::new(),
                    None,
                    None,
                    slow,
                ),
                Service::Telephony => {
                    let answer = telephony::answer(
                        &mut world.telephony,
                        world.audio_gateway.as_ref(),
                        &mut world.behaviour,
                        &mut world.dialling,
                        &call,
                        &message,
                    );
                    (answer.reply, answer.signals, None, answer.later, slow)
                }
                Service::Obex => {
                    let answer =
                        obex::answer(&mut world.obex, &world.phone_address, &call, &message);
                    (answer.reply, answer.signals, answer.later, None, slow)
                }
            }
        };
        if slow {
            // The calls that follow are answered meanwhile.
            let connection = connection.clone();
            std::thread::spawn(move || {
                std::thread::sleep(telephony::SLOW_REPLY);
                zbus::block_on(answer(&connection, &message, reply, signals))
            });
            continue;
        }
        answer(&connection, &message, reply, signals).await?;
        if let Some((after, later)) = later {
            run_obex_later(world.clone(), connection.clone(), message, after, later);
        }
        if let Some(later) = activate_later {
            run_activate_later(world.clone(), connection.clone(), later);
        }
    }
    Ok(())
}

/// Has the Phone act on an `Activate` once its delay has passed, on a
/// thread of its own so the service keeps answering meanwhile.
fn run_activate_later(world: SharedWorld, connection: Connection, later: telephony::ActivateLater) {
    std::thread::spawn(move || {
        std::thread::sleep(later.after);
        let signals = {
            let mut world = lock(&world);
            let world = &mut *world;
            telephony::activate_later(
                &mut world.telephony,
                world.audio_gateway.as_ref(),
                &mut world.behaviour,
                &later,
            )
        };
        zbus::block_on(async {
            for signal in signals {
                if signal.emit(&connection).await.is_err() {
                    break;
                }
            }
        });
    });
}

/// Sends `reply` to `message`, then emits `signals`: as the real service,
/// which replies once the Phone confirms, and then reports the change the
/// Phone makes.
async fn answer(
    connection: &Connection,
    message: &Message,
    reply: Reply,
    signals: Vec<telephony::Signal>,
) -> zbus::Result<()> {
    reply.send(connection, &message.header()).await?;
    for signal in signals {
        signal.emit(connection).await?;
    }
    Ok(())
}

/// Does what obexd does `after` a method call, on a thread of its own so
/// the service keeps answering meanwhile: replies to `message` if a reply is
/// still owed, and sends the signals.
fn run_obex_later(
    world: SharedWorld,
    connection: Connection,
    message: zbus::Message,
    after: std::time::Duration,
    later: obex::Later,
) {
    std::thread::spawn(move || {
        std::thread::sleep(after);
        let (reply, signals) = obex::run_later(&mut lock(&world).obex, later);
        zbus::block_on(answer(&connection, &message, reply, signals))
    });
}

/// The connection the Telephony service is on. A restart replaces it; it is
/// `None` while the service is off the bus.
type TelephonyConnection = Arc<Mutex<Option<Connection>>>;

/// What tests use to act as the Phone.
struct Control {
    world: SharedWorld,
    system: Connection,
    telephony: TelephonyConnection,
    /// Where logind's signals come from.
    logind: Connection,
}

#[interface(interface = "io.github.danielbaldwin47.DesktopCalls.SimulatedPhone1")]
impl Control {
    /// Comes into range and connects, as a Phone does by itself: BlueZ marks
    /// the Phone connected, then, once the service level connection is set
    /// up, the device gains `Battery1`, PipeWire registers a new AudioGateway
    /// and sends `ModemAdded`, and announces any Call the Phone carried on
    /// while away. Does nothing if hands-free is up already.
    async fn connect(
        &self,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
    ) -> zbus::fdo::Result<()> {
        let change = {
            let mut world = lock(&self.world);
            world.range.reachable = true;
            range::hands_free_up(&mut world)?
        };
        self.range_changed(change, &emitter).await
    }

    /// Goes out of range, as a Phone carried away does: PipeWire unannounces
    /// each Call, without ending it, then the AudioGateway, and sends
    /// `ModemRemoved`; the device loses `Battery1`, and BlueZ marks the Phone
    /// disconnected. The Calls carry on on the Phone. The Desktop's
    /// `Connect` fails after BlueZ's page timeout until the Phone is back.
    async fn disconnect(
        &self,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
    ) -> zbus::fdo::Result<()> {
        let change = {
            let mut world = lock(&self.world);
            world.range.reachable = false;
            range::hands_free_down(&mut world)?
        };
        self.range_changed(change, &emitter).await
    }

    /// Comes back into range without connecting: the Phone answers the
    /// Desktop's next page, and its `Connect` succeeds.
    fn return_to_range(&self) {
        lock(&self.world).range.reachable = true;
    }

    /// Comes back into range `delay_ms` from now, as [`return_to_range`]
    /// does: a Phone that answers the Desktop's page only after a while.
    /// Returns at once.
    ///
    /// [`return_to_range`]: Self::return_to_range
    fn return_to_range_after(&self, delay_ms: u32) {
        let world = self.world.clone();
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(u64::from(delay_ms)));
            lock(&world).range.reachable = true;
        });
    }

    /// The person holding the Phone forgets the Desktop in its Bluetooth
    /// settings: the Phone drops the connection, as out of range, and fails
    /// each `Connect` from then on with `"br-connection-key-missing"`.
    async fn forget_pairing(
        &self,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
    ) -> zbus::fdo::Result<()> {
        let change = {
            let mut world = lock(&self.world);
            world.range.forgot_pairing = true;
            range::hands_free_down(&mut world)?
        };
        self.range_changed(change, &emitter).await
    }

    /// The person holding the Phone taps the Desktop on its "Pair new
    /// device" screen, which needs the Desktop discoverable. Returns the
    /// passkey the Phone shows; `PairingRequests` says how it went.
    fn pair_from_phone(&self) -> zbus::fdo::Result<u32> {
        pairing::pair_from_phone(&self.world, &self.system, &self.telephony)
            .map_err(zbus::fdo::Error::Failed)
    }

    /// Each pairing so far: its passkey, and `waiting`, `confirmed` or
    /// `rejected`.
    fn pairing_requests(&self) -> Vec<(u32, String)> {
        lock(&self.world).pairing.requests()
    }

    /// Whether the Phone shares Calls: `shared`, `when-asked` (media alone
    /// until `ConnectProfile`) or `refused` ("Phone calls" off).
    fn set_hands_free(&self, mode: &str) -> zbus::fdo::Result<()> {
        lock(&self.world).pairing.hands_free = pairing::HandsFree::from_name(mode)
            .ok_or_else(|| zbus::fdo::Error::InvalidArgs(format!("unknown mode {mode:?}")))?;
        Ok(())
    }

    /// The Calls on the Phone, as `(path, State)`: those on the current
    /// AudioGateway, and those carrying on on the Phone while hands-free is
    /// down, by the last part of their path.
    fn calls_on_phone(&self) -> Vec<(String, String)> {
        let world = lock(&self.world);
        let mut calls: Vec<(String, String)> = world
            .audio_gateway
            .iter()
            .flat_map(|gateway| telephony::calls(&world.telephony, gateway.as_str()))
            .map(|(call, properties)| (call.to_string(), range::call_state(&properties)))
            .collect();
        calls.extend(world.range.calls_away());
        calls.extend(
            world
                .dropped
                .iter()
                .flat_map(hands_free::Dropped::calls_away),
        );
        calls
    }

    /// Sends logind's `PrepareForSleep(start)` on the system bus: `true` as
    /// the Desktop suspends, `false` once it has resumed.
    async fn prepare_for_sleep(&self, start: bool) -> zbus::fdo::Result<()> {
        Ok(logind::prepare_for_sleep(&self.logind, start).await?)
    }

    /// Rings the Phone from `number`, as a caller does: PipeWire registers an
    /// incoming Call on the AudioGateway and announces it. Returns the Call's
    /// object path. During a live Call the new one is `waiting`, as with
    /// `+CCWA`. Fails while hands-free is down. With the
    /// `late-line-identification` quirk the Call appears with no number,
    /// and the number follows a moment later, while it still rings.
    async fn ring(&self, number: String) -> zbus::fdo::Result<OwnedObjectPath> {
        let (call, signals, late) = {
            let mut world = lock(&self.world);
            let world = &mut *world;
            let Some(gateway) = world.audio_gateway.clone() else {
                return Err(zbus::fdo::Error::Failed(
                    "hands-free is not connected, so the Phone cannot ring".to_owned(),
                ));
            };
            let (call, signals) =
                telephony::ring(&mut world.telephony, &gateway, &world.behaviour, &number)?;
            let late = world
                .behaviour
                .quirks
                .contains(&telephony::Quirk::LateLineIdentification);
            (call, signals, late)
        };
        let session = self.telephony()?;
        for signal in signals {
            signal.emit(&session).await?;
        }
        if late {
            let world = self.world.clone();
            let call = call.clone();
            std::thread::spawn(move || {
                std::thread::sleep(telephony::LATE_LINE_IDENTIFICATION);
                let signals =
                    telephony::identify_caller(&mut lock(&world).telephony, &call, &number);
                zbus::block_on(async {
                    for signal in signals {
                        if signal.emit(&session).await.is_err() {
                            break;
                        }
                    }
                });
            });
        }
        Ok(call)
    }

    /// Rings the Phone with this caller ID as the Phone sends it in
    /// `+CLIP` and `+CLCC`, such as `"+447700900123",145`, and PipeWire
    /// 1.6.9 reads it: the number between the quotes, 1 to 16 characters,
    /// becomes `LineIdentification`; anything else, such as a withheld
    /// caller's `"",129`, fails PipeWire's parse and leaves it empty.
    async fn ring_with_caller_id(&self, caller_id: String) -> zbus::fdo::Result<OwnedObjectPath> {
        self.ring(telephony::number_from_caller_id(&caller_id))
            .await
    }

    /// The other end of the Call the Desktop dialled starts ringing: PipeWire
    /// registers the Call, `alerting`, and the Phone brings the audio to the
    /// Desktop if `RejectSCO` allows. Returns the Call's object path. Fails
    /// if the Phone has no accepted `Dial` waiting.
    async fn alert(&self) -> zbus::fdo::Result<OwnedObjectPath> {
        let (call, signals) = {
            let mut world = lock(&self.world);
            let world = &mut *world;
            let Some(gateway) = world.audio_gateway.clone() else {
                return Err(zbus::fdo::Error::Failed(
                    "hands-free is not connected".to_owned(),
                ));
            };
            telephony::alert(
                &mut world.telephony,
                &gateway,
                &world.behaviour,
                &mut world.dialling,
            )?
            .ok_or_else(|| {
                zbus::fdo::Error::Failed("the Desktop hasn't dialled anything".to_owned())
            })?
        };
        for signal in signals {
            signal.emit(&self.telephony()?).await?;
        }
        Ok(call)
    }

    /// The person holding the Phone places a Call to `number` on it: PipeWire
    /// registers the Call, `dialing`, and the audio stays on the Phone, since
    /// the Desktop set nothing. `CalleeAnswers` connects it. Returns the
    /// Call's object path.
    async fn dial_on_phone(&self, number: String) -> zbus::fdo::Result<OwnedObjectPath> {
        let (call, signals) = {
            let mut world = lock(&self.world);
            let world = &mut *world;
            let Some(gateway) = world.audio_gateway.clone() else {
                return Err(zbus::fdo::Error::Failed(
                    "hands-free is not connected".to_owned(),
                ));
            };
            telephony::dial_on_phone(&mut world.telephony, &gateway, &world.behaviour, &number)?
        };
        for signal in signals {
            signal.emit(&self.telephony()?).await?;
        }
        Ok(call)
    }

    /// The person the Desktop called answers: the dialled Call goes
    /// `active`. Fails if no Call is `dialing` or `alerting`.
    async fn callee_answers(&self) -> zbus::fdo::Result<()> {
        let signals = {
            let mut world = lock(&self.world);
            let world = &mut *world;
            world
                .audio_gateway
                .clone()
                .and_then(|gateway| telephony::callee_answers(&mut world.telephony, &gateway))
                .ok_or_else(|| zbus::fdo::Error::Failed("no dialled Call is ringing".to_owned()))?
        };
        for signal in signals {
            signal.emit(&self.telephony()?).await?;
        }
        Ok(())
    }

    /// Makes the next `Dial` fail with a Telephony error: `"invalid-format"`,
    /// `"invalid-state"`, `"not-supported"`, `"in-progress"`, `"failed"`, or
    /// `"cme:N"` for the Phone's `+CME ERROR: N`.
    fn refuse_next_dial(&self, error: &str) -> zbus::fdo::Result<()> {
        let refusal = telephony::dial_refusal(error).ok_or_else(|| {
            zbus::fdo::Error::InvalidArgs(format!(
                "unknown Dial error {error:?}; the errors are invalid-format, invalid-state, not-supported, in-progress, failed and cme:N"
            ))
        })?;
        lock(&self.world).dialling.refusal = Some(refusal);
        Ok(())
    }

    /// Makes the Phone's audio link negotiate the codec with this hands-free
    /// id from now on: 1 CVSD, 2 mSBC, 3 LC3-SWB (the Pixel's).
    fn set_codec(&self, codec: u8) -> zbus::fdo::Result<()> {
        if codec == 0 {
            return Err(zbus::fdo::Error::InvalidArgs(
                "codec 0 means no audio link; pick 1, 2 or 3".to_owned(),
            ));
        }
        lock(&self.world).behaviour.codec = codec;
        Ok(())
    }

    /// Makes the Phone report this hands-free microphone gain, as its `+VGM`
    /// does: 0 to 15, or 255, which is how PipeWire sends its -1 ("the Phone
    /// hasn't said"). The current AudioGateway reports it at once, and later
    /// ones start with it.
    async fn set_microphone_volume(&self, volume: u8) -> zbus::fdo::Result<()> {
        let signals = {
            let mut world = lock(&self.world);
            let world = &mut *world;
            world.behaviour.microphone_volume = volume;
            match world.audio_gateway.clone() {
                Some(gateway) => {
                    telephony::set_microphone_volume(&mut world.telephony, &gateway, volume)
                }
                None => Vec::new(),
            }
        };
        self.emit(signals).await
    }

    /// Switches on a quirk other Phones, or other moments, show:
    /// `"call1-hangup-missing"`, `"hangup-too-few-arguments"` or
    /// `"get-calls-empty-while-dialling"`, `"slow-property-writes"`,
    /// `"activate-ignored"`, `"activate-ignored-once"`,
    /// `"earbuds-keep-audio"`, `"disconnect-profile-refused"`,
    /// `"connect-profile-fails"`, `"connect-profile-fails-once"`,
    /// `"late-line-identification"`, `"chld-unsupported"`,
    /// `"hangup-ignored"` or `"slow-service-level"`.
    fn enable_quirk(&self, name: &str) -> zbus::fdo::Result<()> {
        let quirk = telephony::Quirk::from_name(name).ok_or_else(|| {
            zbus::fdo::Error::InvalidArgs(format!(
                "unknown quirk {name:?}; the quirks are call1-hangup-missing, hangup-too-few-arguments, get-calls-empty-while-dialling, slow-property-writes, activate-ignored, activate-ignored-once, earbuds-keep-audio, disconnect-profile-refused, connect-profile-fails, connect-profile-fails-once, late-line-identification, chld-unsupported, hangup-ignored and slow-service-level"
            ))
        })?;
        lock(&self.world).behaviour.quirks.insert(quirk);
        Ok(())
    }

    /// Unregisters the Call at `call` and registers it again `after_ms`
    /// later, at the same path and in the same state, as PipeWire does when
    /// the hands-free profile briefly drops mid-Call. Returns once the Call
    /// is gone.
    async fn reregister_call(&self, call: OwnedObjectPath, after_ms: u32) -> zbus::fdo::Result<()> {
        let (removed, added) = {
            let mut world = lock(&self.world);
            let world = &mut *world;
            let gateway = world.audio_gateway.clone().ok_or_else(|| {
                zbus::fdo::Error::Failed("hands-free is not connected".to_owned())
            })?;
            telephony::reregister_call(&mut world.telephony, &gateway, &call).ok_or_else(|| {
                zbus::fdo::Error::InvalidArgs(format!("there is no Call at {call}"))
            })?
        };
        for signal in removed {
            signal.emit(&self.telephony()?).await?;
        }
        let session = self.telephony()?;
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(u64::from(after_ms)));
            zbus::block_on(async {
                for signal in added {
                    if signal.emit(&session).await.is_err() {
                        break;
                    }
                }
            });
        });
        Ok(())
    }

    /// Restarts the Telephony service, as `systemctl --user restart
    /// wireplumber` does. The service unannounces the connected
    /// AudioGateway's Calls, then the AudioGateway, and leaves the bus.
    /// `gap_ms` later a new connection takes `org.pipewire.Telephony` and
    /// announces the AudioGateway again at the same path, then, 40 ms later,
    /// its Calls at theirs. Returns once the service has left the bus. On the
    /// real Pixel the gap was 0.73 s.
    async fn restart_telephony(&self, gap_ms: u32) -> zbus::fdo::Result<()> {
        let old = self.telephony()?;
        let restart = {
            let mut world = lock(&self.world);
            let world = &mut *world;
            match world.audio_gateway.clone() {
                Some(gateway) => Some(telephony::restart(&mut world.telephony, &gateway)?),
                None => None,
            }
        };
        if let Some(restart) = &restart {
            for signal in &restart.gone {
                signal.emit(&old).await?;
            }
        }
        *lock(&self.telephony) = None;
        old.release_name(telephony::SERVICE).await?;
        let holder = self.telephony.clone();
        let world = self.world.clone();
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(u64::from(gap_ms)));
            if let Err(error) = restart::come_back(&holder, &world, restart) {
                eprintln!("simulated-phone: the Telephony service didn't come back: {error}");
            }
        });
        Ok(())
    }

    /// Gives the Phone the PBAP folder `folder` (`pb`, `ich`, `och`,
    /// `mch` or `cch`) with exactly this vCard text.
    fn set_pbap_folder(&self, folder: &str, vcards: String) -> zbus::fdo::Result<()> {
        if !PBAP_FOLDERS.contains(&folder) {
            return Err(unknown_pbap_folder(folder));
        }
        lock(&self.world).obex.set_pbap_folder(folder, vcards);
        Ok(())
    }

    /// Takes the PBAP folder `folder` away from the Phone, as some
    /// Phones lack `cch`.
    fn remove_pbap_folder(&self, folder: &str) -> zbus::fdo::Result<()> {
        if !PBAP_FOLDERS.contains(&folder) {
            return Err(unknown_pbap_folder(folder));
        }
        lock(&self.world).obex.remove_pbap_folder(folder);
        Ok(())
    }

    /// How the person holding the Phone answers its prompt to share
    /// Contacts, for each new PBAP session: `"grant"`, `"deny"` or
    /// `"ignore"`.
    fn set_contacts_prompt(&self, answer: &str) -> zbus::fdo::Result<()> {
        let prompt = obex::Prompt::from_name(answer).ok_or_else(|| {
            zbus::fdo::Error::InvalidArgs(format!(
                "unknown answer {answer:?}; the answers are grant, deny and ignore"
            ))
        })?;
        lock(&self.world).obex.prompt = prompt;
        Ok(())
    }

    /// Makes every PBAP pull take `delay_ms` before it completes.
    fn set_pull_delay(&self, delay_ms: u32) {
        lock(&self.world).obex.pull_delay = std::time::Duration::from_millis(u64::from(delay_ms));
    }

    /// Makes the Phone take `delay_ms` after each `Activate` before it acts
    /// on it, as the Pixel took 62 to 313 ms to start opening the audio
    /// link. The reply still comes at once, as PipeWire's does.
    fn set_activate_delay(&self, delay_ms: u32) {
        lock(&self.world).behaviour.activate_delay =
            std::time::Duration::from_millis(u64::from(delay_ms));
    }

    /// The most PBAP sessions open at once so far, counting each from its
    /// `CreateSession` until it fails or is removed.
    #[zbus(property)]
    fn most_pbap_sessions_at_once(&self) -> u32 {
        lock(&self.world).obex.most_sessions_at_once
    }

    /// Answers the ringing Call at `call` on the Phone, as the person holding
    /// it does. The Phone then brings its audio link up, which lands on the
    /// Desktop only if `RejectSCO` is false. Returns once the Call is active.
    async fn answer_on_phone(&self, call: OwnedObjectPath) -> zbus::fdo::Result<()> {
        let signals = {
            let mut world = lock(&self.world);
            let world = &mut *world;
            let gateway = connected_audio_gateway(world)?;
            telephony::answer_on_phone(&mut world.telephony, &gateway, &mut world.behaviour, &call)
                .map_err(zbus::fdo::Error::InvalidArgs)?
        };
        self.emit(signals).await
    }

    /// Connects earbuds to the Phone beside the Desktop (`true`), or
    /// disconnects them. Connected earbuds become the Phone's active
    /// hands-free device: a Call answered on them, or placed on the Phone,
    /// has its audio there, and the Phone ignores `Activate` until the
    /// Desktop answers or dials, which makes the Desktop the active device
    /// unless the `earbuds-keep-audio` quirk is on.
    fn set_earbuds(&self, connected: bool) {
        lock(&self.world).behaviour.earbuds.set_connected(connected);
    }

    /// Declines the ringing Call at `call` on the Phone, as the person
    /// holding it does. Returns once the Call is gone.
    async fn decline_on_phone(&self, call: OwnedObjectPath) -> zbus::fdo::Result<()> {
        self.end_call_on_phone(&call, true).await
    }

    /// Ends the Call at `call` from the other end, ringing or live, as the
    /// caller hanging up does. Returns once the Call is gone. On the
    /// Desktop's side of the bus it looks just like a decline on the Phone.
    async fn caller_hangs_up(&self, call: OwnedObjectPath) -> zbus::fdo::Result<()> {
        self.end_call_on_phone(&call, false).await
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

/// The PBAP folders a test can give the Phone.
const PBAP_FOLDERS: [&str; 5] = ["pb", "ich", "och", "mch", "cch"];

fn unknown_pbap_folder(folder: &str) -> zbus::fdo::Error {
    zbus::fdo::Error::InvalidArgs(format!(
        "unknown folder {folder:?}; the folders are pb, ich, och, mch and cch"
    ))
}

impl Control {
    fn telephony(&self) -> zbus::fdo::Result<Connection> {
        lock(&self.telephony).clone().ok_or_else(|| {
            zbus::fdo::Error::Failed("the Telephony service is restarting".to_owned())
        })
    }

    async fn end_call_on_phone(
        &self,
        call: &OwnedObjectPath,
        ringing_only: bool,
    ) -> zbus::fdo::Result<()> {
        let signals = {
            let mut world = lock(&self.world);
            let world = &mut *world;
            let gateway = connected_audio_gateway(world)?;
            telephony::end_call_on_phone(&mut world.telephony, &gateway, call, ringing_only)
                .map_err(zbus::fdo::Error::InvalidArgs)?
        };
        self.emit(signals).await
    }

    /// Sends a change in range's signals, if there was a change.
    async fn range_changed(
        &self,
        change: Option<range::Change>,
        emitter: &SignalEmitter<'_>,
    ) -> zbus::fdo::Result<()> {
        if let Some(change) = change {
            range::emit(&self.system, &self.telephony, change).await?;
            self.audio_gateway_changed(emitter).await?;
        }
        Ok(())
    }

    async fn emit(&self, signals: Vec<telephony::Signal>) -> zbus::fdo::Result<()> {
        for signal in signals {
            signal.emit(&self.telephony()?).await?;
        }
        Ok(())
    }
}

fn connected_audio_gateway(world: &World) -> zbus::fdo::Result<OwnedObjectPath> {
    world
        .audio_gateway
        .clone()
        .ok_or_else(|| zbus::fdo::Error::Failed("hands-free is not connected".to_owned()))
}
