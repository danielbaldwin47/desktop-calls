//! The Phone link: the PipeWire Telephony client.
//!
//! It is attached to a Phone by Bluetooth address, then reports when that
//! Phone connects and disconnects for Calls, its Calls and their audio link,
//! and carries out the Call state machine's Phone commands. What it hides:
//!
//! - AudioGateway objects are found only through `GetManagedObjects` and
//!   `InterfacesAdded`/`InterfacesRemoved` on `/org/pipewire/Telephony`, because
//!   PipeWire's `Introspect` lists no child nodes and `GetModems` carries no
//!   properties. Each AudioGateway is the `ObjectManager` of its own Calls.
//! - Their index is not `ag0` (the Pixel's is `ag1`) and changes on every
//!   reconnect, so they are matched by their `Address` property.
//! - An AudioGateway exists exactly while hands-free is fully up, so its
//!   lifetime is the Phone's connection.
//! - PipeWire reuses a Call's object path for the next Call, so each Call
//!   gets an app-assigned id when it appears, and everything known about it
//!   is forgotten when it goes.
//! - A Phone without `+CLCC` reports a ringing Call with no number, and the
//!   number later, in a `PropertiesChanged` of `LineIdentification`; the
//!   caller follows it.
//! - A Call that goes away without first going `disconnected` may come back
//!   within a second, as the same Call: the hands-free profile dropping
//!   briefly, or `WirePlumber` restarting, does that mid-Call. While
//!   hands-free is away, such a Call ends only if hands-free isn't back
//!   within [`GRACE`], or [`SERVICE_GRACE`] once the whole service has gone.
//!   Once hands-free is back, it is settled by the Calls the Phone lists:
//!   the same Call if listed, ended if not listed within [`LISTING_GRACE`]
//!   (ADR-0007). The AudioGateway comes back before its Calls, and nothing
//!   says when the Phone has listed them all.
//! - Going away is no proof that a Hang up worked: a `WirePlumber` restart
//!   takes the Calls away without `disconnected`, and a Hang up it never
//!   passed on leaves the Call live on the Phone. So a hung-up Call that
//!   goes without `disconnected` is vanished like any other, and the Hang up
//!   is sent again if it comes back. If hands-free isn't back in time to
//!   settle it, the Hang up is reported undelivered, not the Call ended
//!   (ticket #175).
//! - `Call1.Hangup` falls back to `org.ofono.VoiceCall.Hangup` where `Call1`'s
//!   method doesn't exist, and a hang-up error for a Call that is already
//!   going counts as success.
//! - `RejectSCO` is written with `Properties.Set`, and audio moves with
//!   `Activate()`, which does nothing while `RejectSCO` is true.
//! - Mute is the AudioGateway's `MicrophoneVolume`, a byte written with
//!   `Properties.Set`. PipeWire reads the value without checking its type,
//!   and a wrong one crashed `WirePlumber`, so it always goes as a byte.
//! - PipeWire sends `SendTones`' whole string in one `AT+VTS`, but Android
//!   takes exactly one tone per `AT+VTS` and refuses more. So each tone goes
//!   in a `SendTones` of its own, the next once the Phone has played the
//!   last.
//! - Telephony has no way to close the audio link from the Desktop: no
//!   `Deactivate()`. The audio goes back to the Phone by BlueZ dropping
//!   hands-free and connecting it again (ADR-0005), which takes the
//!   AudioGateway and its Calls away for about a second; they come back at
//!   the same paths, and the Calls as the same Calls. If hands-free doesn't
//!   come back, BlueZ is asked to connect it again (ticket #101).

mod commands;

use std::collections::HashMap;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use desktop_calls_core::{
    CallId, CallState, Caller, Codec, Input, PhoneCommand, Timestamp, Transport,
};
use futures_channel::mpsc::{UnboundedReceiver, UnboundedSender, unbounded};
use futures_util::StreamExt as _;
use futures_util::stream;
use glib::MainContext;
use zbus::{Connection, OwnedVariant};

use crate::logging::EventKind;
use crate::object_manager::{self, Interfaces, ObjectEvent, WatchError, Watched};
use commands::{command_name, execute};

const SERVICE: &str = "org.pipewire.Telephony";
const MANAGER_PATH: &str = "/org/pipewire/Telephony";
const AUDIO_GATEWAY: &str = "org.pipewire.Telephony.AudioGateway1";
const TRANSPORT: &str = "org.pipewire.Telephony.AudioGatewayTransport1";
const CALL: &str = "org.pipewire.Telephony.Call1";

/// How long a Call that vanished without going `disconnected` may take to
/// come back as the same Call while hands-free is away. The Pixel's took
/// 0.5 to 1.1 s.
const GRACE: Duration = Duration::from_secs(2);

/// How long, once hands-free is back, a vanished Call may take to be listed
/// again, whatever is left of its grace (ADR-0007). The Pixel lists its
/// Calls 30 to 60 ms after the AudioGateway; a loaded Desktop stalled for
/// 366 ms in ticket #170, and a busy `WirePlumber` may be slower still.
/// Waiting longer costs only this: a Call that ended on the Phone while
/// hands-free was away shows up to 3 s more, where ending a live Call
/// would show "Call ended" and bring it back as a new Call.
const LISTING_GRACE: Duration = Duration::from_secs(3);

/// How long the Calls of a Telephony service that left the bus may take to
/// come back with it. After a `WirePlumber` restart the real service was back
/// with its Call in 1.07 s; a busy or slower Desktop takes longer, and a
/// Call ended too soon is worse than one shown a little too long.
const SERVICE_GRACE: Duration = Duration::from_secs(10);

/// What the Phone link can be told to do.
pub enum Command {
    /// Follow the Phone with this Bluetooth address.
    Attach { address: String },
    /// Stop following any Phone.
    Detach,
    /// Carry out these Phone commands in order, stopping at the first that
    /// fails.
    Run(Vec<PhoneCommand>),
}

enum Event {
    Command(Command),
    Object(ObjectEvent),
    /// A grace period of a vanished Call ran out.
    GraceOver(Grace),
}

/// A Phone command with where it goes.
struct Resolved {
    command: PhoneCommand,
    target: Target,
}

/// Where a Phone command goes.
enum Target {
    /// This Telephony object.
    Object(String),
    /// The Phone with this Bluetooth address, through BlueZ.
    Phone(String),
}

/// Runs the Phone link until the connection closes, taking `commands` and
/// reporting to `inputs`. Telephony is on the `session` bus, BlueZ on the
/// `system` bus.
pub async fn run(
    session: Connection,
    system: Connection,
    commands: UnboundedReceiver<Command>,
    inputs: UnboundedSender<Input>,
) -> Result<(), WatchError> {
    let (object_sender, objects) = unbounded();
    let (grace_sender, graces) = unbounded();
    let (batch_sender, batches) = unbounded();
    let watching = async {
        let watched = Watched {
            service: SERVICE,
            root: MANAGER_PATH,
            nested_manager: Some(AUDIO_GATEWAY),
        };
        object_manager::watch(&session, watched, &object_sender).await?;
        drop(object_sender);
        Ok::<_, WatchError>(())
    };
    let following = async {
        let mut events = stream::select(
            stream::select(commands.map(Event::Command), objects.map(Event::Object)),
            graces.map(Event::GraceOver),
        );
        let mut link = PhoneLink::default();
        while let Some(event) = events.next().await {
            let outcome = link.handle(event);
            for (grace, lasting) in outcome.graces {
                let sender = grace_sender.clone();
                MainContext::default().spawn_local(async move {
                    glib::timeout_future(lasting).await;
                    let _ = sender.unbounded_send(grace);
                });
            }
            if !outcome.batch.is_empty() {
                let _ = batch_sender.unbounded_send(outcome.batch);
            }
            for input in outcome.inputs {
                inputs
                    .unbounded_send(input)
                    .map_err(|_| WatchError::ListenerGone)?;
            }
        }
        Ok(())
    };
    let executing = execute(&session, &system, batches, &inputs);
    futures_util::future::try_join3(watching, following, async {
        executing.await;
        Ok(())
    })
    .await
    .map(drop)
}

/// What handling one event led to.
#[derive(Default)]
struct Outcome {
    inputs: Vec<Input>,
    /// Grace periods of vanished Calls that start now, each with how long
    /// it lasts.
    graces: Vec<(Grace, Duration)>,
    /// Phone commands to carry out, in order.
    batch: Vec<Resolved>,
}

/// A Call object as the Phone reports it.
struct CallObject {
    state: String,
    number: String,
}

/// A Call of the followed Phone, as reported to the state machine.
struct Followed {
    id: CallId,
    /// The Phone said `disconnected`, or the Desktop asked to hang up: going
    /// away now is the end of the Call.
    ending: bool,
    /// The Call has been reported ended.
    ended: bool,
}

/// One grace period of a vanished Call. Only the latest one of each Call
/// counts: the one started when its whole service went replaces the shorter
/// one before it.
#[derive(Clone, Copy, PartialEq, Eq)]
struct Grace {
    call: CallId,
    number: u32,
}

/// A Call that went away without ending, during its grace period.
struct Vanished {
    id: CallId,
    grace: Grace,
    number: String,
    /// The Desktop asked to hang up meanwhile: the Call is hung up as soon
    /// as it is back.
    hang_up: bool,
}

#[derive(Default)]
struct PhoneLink {
    /// Every AudioGateway the service has, by object path: its address.
    audio_gateways: HashMap<String, String>,
    /// Each AudioGateway's latest `MicrophoneVolume`, by object path.
    microphone_volumes: HashMap<String, u8>,
    /// Each AudioGateway's latest transport properties, by object path, so
    /// a Phone attached after its AudioGateway appeared, as at startup
    /// mid-Call, reports where the audio is.
    transports: HashMap<String, HashMap<String, OwnedVariant>>,
    /// The address of the Phone being followed.
    attached: Option<String>,
    /// The AudioGateway of the followed Phone, while it is connected.
    connected: Option<String>,
    /// Every Call object the service has, by object path.
    call_objects: HashMap<String, CallObject>,
    /// The followed Phone's Calls, by object path.
    followed: HashMap<String, Followed>,
    vanished: Vec<Vanished>,
    last_call_id: u32,
    graces_started: u32,
}

impl PhoneLink {
    fn handle(&mut self, event: Event) -> Outcome {
        let mut outcome = Outcome::default();
        match event {
            Event::Command(Command::Attach { address }) => {
                self.attached = Some(address);
                self.connected = None;
                self.adopt_matching_gateway(&mut outcome);
            }
            Event::Command(Command::Detach) => {
                self.attached = None;
                self.connected = None;
                self.vanish_all(&mut outcome);
            }
            Event::Command(Command::Run(commands)) => {
                outcome.batch = self.resolve(commands, &mut outcome.inputs);
            }
            Event::Object(ObjectEvent::Added { path, interfaces }) => {
                self.added(path, &interfaces, &mut outcome);
            }
            Event::Object(ObjectEvent::Changed {
                path,
                interface,
                changed,
            }) => self.changed(&path, &interface, &changed, &mut outcome),
            Event::Object(ObjectEvent::Removed { path, interfaces }) => {
                if interfaces
                    .iter()
                    .any(|interface| interface == AUDIO_GATEWAY)
                {
                    self.forget(&path, &mut outcome);
                } else if interfaces.iter().any(|interface| interface == CALL) {
                    self.call_removed(&path, &mut outcome);
                }
            }
            Event::Object(ObjectEvent::Listed) => {}
            Event::Object(ObjectEvent::ServiceGone) => {
                let paths: Vec<String> = self.audio_gateways.keys().cloned().collect();
                for path in paths {
                    self.forget(&path, &mut outcome);
                }
                self.call_objects.clear();
                // The Calls come back with the service, which takes longer
                // than the hands-free profile bouncing.
                self.restart_graces(SERVICE_GRACE, &mut outcome);
            }
            Event::GraceOver(grace) => self.grace_over(grace, &mut outcome),
        }
        outcome
    }

    fn added(&mut self, path: String, interfaces: &Interfaces, outcome: &mut Outcome) {
        if let Some(gateway) = interfaces.get(AUDIO_GATEWAY) {
            let Some(address) = gateway.get("Address").and_then(string) else {
                return;
            };
            tracing::debug!(
                event_kind = %EventKind::AudioGatewayAdded,
                phone_address = %address,
                path = %path,
                "An AudioGateway appeared"
            );
            if let Some(volume) = gateway.get("MicrophoneVolume").and_then(byte) {
                self.microphone_volumes.insert(path.clone(), volume);
            }
            if let Some(transport) = interfaces.get(TRANSPORT) {
                self.transports.insert(path.clone(), transport.clone());
            }
            self.audio_gateways.insert(path, address);
            self.adopt_matching_gateway(outcome);
        } else if let Some(call) = interfaces.get(CALL) {
            let object = CallObject {
                state: call.get("State").and_then(string).unwrap_or_default(),
                number: call
                    .get("LineIdentification")
                    .and_then(string)
                    .unwrap_or_default(),
            };
            self.call_objects.insert(path.clone(), object);
            self.follow_call(&path, outcome);
        }
    }

    fn changed(
        &mut self,
        path: &str,
        interface: &str,
        changed: &HashMap<String, OwnedVariant>,
        outcome: &mut Outcome,
    ) {
        match interface {
            TRANSPORT => {
                if let Some(known) = self.transports.get_mut(path) {
                    for (name, value) in changed {
                        if let Ok(value) = value.try_clone() {
                            known.insert(name.clone(), value);
                        }
                    }
                }
                if self.connected.as_deref() == Some(path) {
                    transport_changed(changed, outcome);
                }
            }
            AUDIO_GATEWAY => {
                if let Some(volume) = changed.get("MicrophoneVolume").and_then(byte) {
                    self.microphone_volumes.insert(path.to_owned(), volume);
                    if self.connected.as_deref() == Some(path) {
                        outcome.inputs.push(Input::MicrophoneVolumeChanged(volume));
                    }
                }
            }
            CALL => self.call_object_changed(path, changed, outcome),
            _ => {}
        }
    }

    /// The Call object at `path` changed. The number goes first, so that a
    /// change that also ends the Call ends it with the right caller.
    fn call_object_changed(
        &mut self,
        path: &str,
        changed: &HashMap<String, OwnedVariant>,
        outcome: &mut Outcome,
    ) {
        let Some(object) = self.call_objects.get_mut(path) else {
            return;
        };
        // A Phone without `+CLCC` reports a ringing Call before its number,
        // which PipeWire sets from the `+CLIP` after the first `RING`.
        let number = changed
            .get("LineIdentification")
            .and_then(string)
            .filter(|number| *number != object.number);
        if let Some(number) = &number {
            object.number.clone_from(number);
        }
        let state = changed.get("State").and_then(string);
        if let Some(state) = &state {
            object.state.clone_from(state);
        }
        let Some(followed) = self.followed.get_mut(path) else {
            return;
        };
        if followed.ended {
            return;
        }
        if let Some(number) = number {
            tracing::info!(
                event_kind = %EventKind::CallerChanged,
                call_id = followed.id.0,
                "The Phone named Call {}'s caller",
                followed.id
            );
            tracing::debug!(
                event_kind = %EventKind::CallerChanged,
                call_id = followed.id.0,
                phone_number = %number,
                path,
                "Call {} is with this number",
                followed.id
            );
            outcome.inputs.push(Input::CallerChanged {
                call: followed.id,
                caller: caller(&number),
            });
        }
        let Some(state) = state else {
            return;
        };
        match call_state(&state) {
            Some(call_state) => {
                tracing::info!(
                    event_kind = %EventKind::CallChanged,
                    call_id = followed.id.0,
                    call_state = call_state.as_str(),
                    "Call {} is {}",
                    followed.id,
                    call_state.as_str()
                );
                outcome.inputs.push(Input::CallChanged {
                    call: followed.id,
                    state: call_state,
                    at: now(),
                });
            }
            None if state == "disconnected" => {
                followed.ending = true;
                followed.ended = true;
                tracing::info!(
                    event_kind = %EventKind::CallEnded,
                    call_id = followed.id.0,
                    "Call {} ended",
                    followed.id
                );
                outcome.inputs.push(Input::CallEnded { call: followed.id });
            }
            None => unknown_state(&state),
        }
    }

    /// Reports the Call object at `path` if it belongs to the followed,
    /// connected Phone and isn't reported yet.
    fn follow_call(&mut self, path: &str, outcome: &mut Outcome) {
        let Some(connected) = &self.connected else {
            return;
        };
        let belongs = path
            .strip_prefix(connected.as_str())
            .is_some_and(|rest| rest.starts_with('/'));
        let Some(object) = self.call_objects.get(path) else {
            return;
        };
        if !belongs || self.followed.contains_key(path) {
            return;
        }
        let Some(state) = call_state(&object.state) else {
            if object.state != "disconnected" {
                unknown_state(&object.state);
            }
            return;
        };
        // A Call that vanished and is back, by its number, is the same Call.
        if let Some(index) = self
            .vanished
            .iter()
            .position(|call| call.number == object.number)
        {
            let Vanished { id, hang_up, .. } = self.vanished.remove(index);
            tracing::info!(
                event_kind = %EventKind::CallReturned,
                call_id = id.0,
                "Call {id} came back"
            );
            self.followed.insert(
                path.to_owned(),
                Followed {
                    id,
                    ending: hang_up,
                    ended: false,
                },
            );
            if hang_up {
                outcome.batch.push(Resolved {
                    command: PhoneCommand::HangUp(id),
                    target: Target::Object(path.to_owned()),
                });
            }
            outcome.inputs.push(Input::CallChanged {
                call: id,
                state,
                at: now(),
            });
            return;
        }
        self.last_call_id += 1;
        let id = CallId(self.last_call_id);
        tracing::info!(
            event_kind = %EventKind::CallAdded,
            call_id = id.0,
            call_state = state.as_str(),
            "Call {id} appeared, {}",
            state.as_str()
        );
        tracing::debug!(
            event_kind = %EventKind::CallAdded,
            call_id = id.0,
            phone_number = %object.number,
            path,
            "Call {id} is with this number"
        );
        let caller = caller(&object.number);
        self.followed.insert(
            path.to_owned(),
            Followed {
                id,
                ending: false,
                ended: false,
            },
        );
        outcome.inputs.push(Input::CallAdded {
            call: id,
            state,
            caller,
            at: now(),
        });
    }

    fn call_removed(&mut self, path: &str, outcome: &mut Outcome) {
        let object = self.call_objects.remove(path);
        let Some(followed) = self.followed.remove(path) else {
            return;
        };
        if followed.ended {
            return;
        }
        self.vanish(&followed, object.map(|object| object.number), outcome);
    }

    /// The followed Call went away without `disconnected`. It may come back,
    /// and so may a Call the Desktop asked to hang up, if the Hang up never
    /// reached the Phone: it is hung up again then.
    fn vanish(&mut self, followed: &Followed, number: Option<String>, outcome: &mut Outcome) {
        let id = followed.id;
        if followed.ending {
            tracing::info!(
                event_kind = %EventKind::CallVanished,
                call_id = id.0,
                "Call {id} went away without ending after the Hang up; it is hung up again if it comes back"
            );
        } else {
            tracing::info!(
                event_kind = %EventKind::CallVanished,
                call_id = id.0,
                "Call {id} went away without ending; waiting for it to come back"
            );
        }
        let grace = self.start_grace(id, GRACE, outcome);
        self.vanished.push(Vanished {
            id,
            grace,
            number: number.unwrap_or_default(),
            hang_up: followed.ending,
        });
    }

    /// The latest grace of a vanished Call ran out. Once hands-free is back,
    /// a Call the Phone didn't list has ended, hung up or not. While it is
    /// away, nobody knows: a Hang up kept for the Call's return is reported
    /// undelivered, as the Call may carry on on the Phone.
    fn grace_over(&mut self, grace: Grace, outcome: &mut Outcome) {
        let id = grace.call;
        let Some(index) = self.vanished.iter().position(|call| call.grace == grace) else {
            return;
        };
        let vanished = self.vanished.remove(index);
        if vanished.hang_up && self.connected.is_none() {
            tracing::warn!(
                event_kind = %EventKind::PhoneCommandFailed,
                error_kind = "hang-up-undelivered",
                call_id = id.0,
                "Call {id} didn't come back, and the Hang up never reached the Phone"
            );
            outcome.inputs.push(Input::HangUpUndelivered(id));
            return;
        }
        tracing::info!(
            event_kind = %EventKind::CallEnded,
            call_id = id.0,
            "Call {id} ended: it didn't come back"
        );
        outcome.inputs.push(Input::CallEnded { call: id });
    }

    /// Starts a grace period of `lasting` for the vanished Call `call`.
    fn start_grace(&mut self, call: CallId, lasting: Duration, outcome: &mut Outcome) -> Grace {
        self.graces_started += 1;
        let grace = Grace {
            call,
            number: self.graces_started,
        };
        outcome.graces.push((grace, lasting));
        grace
    }

    /// Gives every vanished Call a grace of `lasting` from now, in place of
    /// the one it had.
    fn restart_graces(&mut self, lasting: Duration, outcome: &mut Outcome) {
        let ids: Vec<CallId> = self.vanished.iter().map(|call| call.id).collect();
        for id in ids {
            let grace = self.start_grace(id, lasting, outcome);
            if let Some(vanished) = self.vanished.iter_mut().find(|call| call.id == id) {
                vanished.grace = grace;
            }
        }
    }

    /// Every followed Call goes away without ending, as when the
    /// AudioGateway or the whole service does.
    fn vanish_all(&mut self, outcome: &mut Outcome) {
        let followed: Vec<(String, Followed)> = self.followed.drain().collect();
        for (path, followed) in followed {
            if followed.ended {
                continue;
            }
            let number = self
                .call_objects
                .get(&path)
                .map(|object| object.number.clone());
            self.vanish(&followed, number, outcome);
        }
    }

    /// If the followed Phone isn't connected yet but has an AudioGateway,
    /// it is now connected, with whatever Calls it has.
    fn adopt_matching_gateway(&mut self, outcome: &mut Outcome) {
        if self.connected.is_some() {
            return;
        }
        let Some(attached) = self.attached.as_ref() else {
            return;
        };
        let Some((path, address)) = self
            .audio_gateways
            .iter()
            .find(|(_, address)| address.eq_ignore_ascii_case(attached))
        else {
            return;
        };
        self.connected = Some(path.clone());
        outcome.inputs.push(Input::PhoneConnected {
            address: address.clone(),
        });
        if let Some(volume) = self.microphone_volumes.get(path) {
            outcome.inputs.push(Input::MicrophoneVolumeChanged(*volume));
        }
        if let Some(transport) = self.transports.get(path) {
            transport_changed(transport, outcome);
        }
        // Hands-free is back: the Calls that vanished are settled by what
        // the Phone lists from now on, not by when they went (ADR-0007).
        self.restart_graces(LISTING_GRACE, outcome);
        let calls: Vec<String> = self.call_objects.keys().cloned().collect();
        for call in calls {
            self.follow_call(&call, outcome);
        }
    }

    /// The AudioGateway at `path` is gone. If it was the followed Phone's, the
    /// Phone disconnected, unless another of its AudioGateways remains.
    fn forget(&mut self, path: &str, outcome: &mut Outcome) {
        self.microphone_volumes.remove(path);
        self.transports.remove(path);
        let Some(address) = self.audio_gateways.remove(path) else {
            return;
        };
        tracing::debug!(
            event_kind = %EventKind::AudioGatewayRemoved,
            phone_address = %address,
            path,
            "An AudioGateway disappeared"
        );
        let followed = self.connected.as_deref() == Some(path);
        // Vanished Calls are known again by their number, so they vanish
        // before their objects are forgotten: the service may go without
        // unannouncing them first, as when WirePlumber stops.
        if followed {
            self.vanish_all(outcome);
        }
        let prefix = format!("{path}/");
        self.call_objects
            .retain(|call, _| !call.starts_with(&prefix));
        if !followed {
            return;
        }
        self.connected = None;
        let before = outcome.inputs.len();
        self.adopt_matching_gateway(outcome);
        if outcome.inputs.len() == before {
            outcome.inputs.push(Input::PhoneDisconnected { address });
        }
    }

    /// Whether `command` hangs up a Call that vanished and may come back,
    /// which is then hung up as soon as it does.
    fn hang_up_when_back(&mut self, command: &PhoneCommand) -> bool {
        let PhoneCommand::HangUp(id) = command else {
            return false;
        };
        let Some(vanished) = self.vanished.iter_mut().find(|call| call.id == *id) else {
            return false;
        };
        tracing::info!(
            event_kind = %EventKind::PhoneCommandSent,
            call_id = id.0,
            "Call {id} will be hung up once it is back"
        );
        vanished.hang_up = true;
        true
    }

    /// Finds the object each command goes to. A command that can't be
    /// carried out fails at once.
    fn resolve(&mut self, commands: Vec<PhoneCommand>, inputs: &mut Vec<Input>) -> Vec<Resolved> {
        let mut batch = Vec::new();
        for command in commands {
            let target = match &command {
                PhoneCommand::Answer(id) | PhoneCommand::HangUp(id) => {
                    let found = self
                        .followed
                        .iter_mut()
                        .find(|(_, followed)| followed.id == *id && !followed.ended)
                        .map(|(path, followed)| {
                            if matches!(command, PhoneCommand::HangUp(_)) {
                                followed.ending = true;
                            }
                            path.clone()
                        });
                    // While the Telephony service restarts, a Call is away
                    // for about a second; it is hung up once back.
                    if found.is_none() && self.hang_up_when_back(&command) {
                        continue;
                    }
                    found.map(Target::Object)
                }
                // Only a connected Phone has hands-free to drop.
                PhoneCommand::MoveAudioToPhone => self
                    .connected
                    .as_ref()
                    .and_then(|path| self.audio_gateways.get(path))
                    .cloned()
                    .map(Target::Phone),
                // Hands-free is down, so there is no AudioGateway: the
                // attached Phone's address is all there is.
                PhoneCommand::ConnectHandsFree => self.attached.clone().map(Target::Phone),
                PhoneCommand::MoveAudioToDesktop
                | PhoneCommand::KeepAudioOnPhone
                | PhoneCommand::AllowAudioOnDesktop
                | PhoneCommand::Dial(_)
                | PhoneCommand::SetMicrophoneVolume(_)
                | PhoneCommand::SendTones(_)
                | PhoneCommand::HoldAndAnswer(_)
                | PhoneCommand::EndAndAnswer(_)
                | PhoneCommand::AnswerBesideHeld(_)
                | PhoneCommand::Swap => self.connected.clone().map(Target::Object),
            };
            match target {
                Some(target) => batch.push(Resolved { command, target }),
                // With no Phone connected there is no audio to keep; the
                // next connect asks again.
                None if command == PhoneCommand::KeepAudioOnPhone => {}
                None => {
                    let name = command_name(&command);
                    tracing::warn!(
                        event_kind = %EventKind::PhoneCommandFailed,
                        error_kind = "no-target",
                        command = %name,
                        "Couldn't send {name}: the Phone has no such Call or isn't connected"
                    );
                    inputs.push(Input::PhoneCommandFailed(command));
                    break;
                }
            }
        }
        batch
    }
}

fn transport_changed(properties: &HashMap<String, OwnedVariant>, outcome: &mut Outcome) {
    if let Some(codec) = properties
        .get("Codec")
        .and_then(|codec| u8::try_from(codec).ok())
    {
        outcome
            .inputs
            .push(Input::CodecChanged(Codec::from_id(codec)));
    }
    if let Some(state) = properties.get("State").and_then(string) {
        let transport = match state.as_str() {
            "idle" => Transport::Idle,
            "pending" => Transport::Pending,
            "active" => Transport::Active,
            other => {
                tracing::warn!(
                    event_kind = %EventKind::MalformedSignal,
                    error_kind = "unknown-transport-state",
                    transport_state = other,
                    "Ignored an unknown transport state"
                );
                return;
            }
        };
        tracing::debug!(
            event_kind = %EventKind::TransportChanged,
            transport_state = %state,
            "The Call audio link is {state}"
        );
        outcome.inputs.push(Input::TransportChanged(transport));
    }
}

/// The Call state machine's name for a PipeWire Call state; `None` for
/// `disconnected` and anything unknown.
fn call_state(state: &str) -> Option<CallState> {
    match state {
        "incoming" => Some(CallState::Ringing),
        "waiting" => Some(CallState::Waiting),
        "dialing" => Some(CallState::Dialing),
        "alerting" => Some(CallState::Alerting),
        "active" => Some(CallState::Active),
        "held" => Some(CallState::Held),
        _ => None,
    }
}

/// The caller a Call's `LineIdentification` names. oFono's convention, which
/// PipeWire's Telephony API follows, gives a withheld number as "withheld";
/// PipeWire itself leaves it empty. `IncomingLine` is no fallback: it is the
/// Phone's own number the caller dialled, never the caller's.
fn caller(number: &str) -> Caller {
    if number.is_empty() || number.eq_ignore_ascii_case("withheld") {
        Caller::Withheld
    } else {
        Caller::Number(number.to_owned())
    }
}

fn unknown_state(state: &str) {
    tracing::warn!(
        event_kind = %EventKind::MalformedSignal,
        error_kind = "unknown-call-state",
        call_state = state,
        "Ignored an unknown Call state"
    );
}

fn byte(value: &OwnedVariant) -> Option<u8> {
    u8::try_from(value).ok()
}

fn string(value: &OwnedVariant) -> Option<String> {
    String::try_from(value.clone()).ok()
}

fn now() -> Timestamp {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_millis());
    Timestamp(u64::try_from(millis).unwrap_or(u64::MAX))
}
