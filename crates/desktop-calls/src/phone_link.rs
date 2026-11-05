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
//! - A Call that goes away without first going `disconnected` may come back
//!   within a second, as the same Call: the hands-free profile dropping
//!   briefly, or `WirePlumber` restarting, does that mid-Call. Such a Call ends
//!   only if it isn't back within [`GRACE`].
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

use std::collections::HashMap;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use desktop_calls_core::{
    CallId, CallState, Caller, Codec, Input, PhoneCommand, Timestamp, Transport,
};
use futures_channel::mpsc::{UnboundedReceiver, UnboundedSender, unbounded};
use futures_util::StreamExt as _;
use futures_util::stream;
use glib::MainContext;
use zbus::{Connection, OwnedVariant, Variant};

use crate::logging::EventKind;
use crate::object_manager::{self, Interfaces, ObjectEvent, WatchError, Watched};

const SERVICE: &str = "org.pipewire.Telephony";
const MANAGER_PATH: &str = "/org/pipewire/Telephony";
const AUDIO_GATEWAY: &str = "org.pipewire.Telephony.AudioGateway1";
const TRANSPORT: &str = "org.pipewire.Telephony.AudioGatewayTransport1";
const CALL: &str = "org.pipewire.Telephony.Call1";
const OFONO_CALL: &str = "org.ofono.VoiceCall";
const PROPERTIES: &str = "org.freedesktop.DBus.Properties";

/// How long a Call that vanished without going `disconnected` may take to
/// come back as the same Call. The Pixel's took 0.5 to 1.1 s.
const GRACE: Duration = Duration::from_secs(2);

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
    /// The grace period of the vanished Call with this id ran out.
    GraceOver(CallId),
}

/// A Phone command with the object it goes to.
struct Resolved {
    command: PhoneCommand,
    path: String,
}

/// Runs the Phone link until the connection closes, taking `commands` and
/// reporting to `inputs`.
pub async fn run(
    session: Connection,
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
            for id in outcome.graces {
                let sender = grace_sender.clone();
                MainContext::default().spawn_local(async move {
                    glib::timeout_future(GRACE).await;
                    let _ = sender.unbounded_send(id);
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
    let executing = execute(&session, batches, &inputs);
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
    /// Vanished Calls whose grace period starts now.
    graces: Vec<CallId>,
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

/// A Call that went away without ending, during its grace period.
struct Vanished {
    id: CallId,
    number: String,
}

#[derive(Default)]
struct PhoneLink {
    /// Every AudioGateway the service has, by object path: its address.
    audio_gateways: HashMap<String, String>,
    /// Each AudioGateway's latest `MicrophoneVolume`, by object path.
    microphone_volumes: HashMap<String, u8>,
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
            Event::Object(ObjectEvent::ServiceGone) => {
                let paths: Vec<String> = self.audio_gateways.keys().cloned().collect();
                for path in paths {
                    self.forget(&path, &mut outcome);
                }
                self.call_objects.clear();
            }
            Event::GraceOver(id) => {
                if let Some(index) = self.vanished.iter().position(|call| call.id == id) {
                    self.vanished.remove(index);
                    tracing::info!(
                        event_kind = %EventKind::CallEnded,
                        call_id = id.0,
                        "Call {id} ended: it didn't come back"
                    );
                    outcome.inputs.push(Input::CallEnded { call: id });
                }
            }
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
            self.audio_gateways.insert(path, address);
            self.adopt_matching_gateway(outcome);
            if let Some(transport) = interfaces.get(TRANSPORT)
                && self.connected.is_some()
            {
                transport_changed(transport, outcome);
            }
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
            TRANSPORT if self.connected.as_deref() == Some(path) => {
                transport_changed(changed, outcome);
            }
            AUDIO_GATEWAY => {
                if let Some(volume) = changed.get("MicrophoneVolume").and_then(byte) {
                    self.microphone_volumes.insert(path.to_owned(), volume);
                    if self.connected.as_deref() == Some(path) {
                        outcome.inputs.push(Input::MicrophoneVolumeChanged(volume));
                    }
                }
            }
            CALL => {
                let Some(state) = changed.get("State").and_then(string) else {
                    return;
                };
                let Some(object) = self.call_objects.get_mut(path) else {
                    return;
                };
                object.state.clone_from(&state);
                let Some(followed) = self.followed.get_mut(path) else {
                    return;
                };
                if followed.ended {
                    return;
                }
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
            _ => {}
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
            let id = self.vanished.remove(index).id;
            tracing::info!(
                event_kind = %EventKind::CallReturned,
                call_id = id.0,
                "Call {id} came back"
            );
            self.followed.insert(
                path.to_owned(),
                Followed {
                    id,
                    ending: false,
                    ended: false,
                },
            );
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
        let caller = if object.number.is_empty() {
            Caller::Withheld
        } else {
            Caller::Number(object.number.clone())
        };
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
        if followed.ending {
            tracing::info!(
                event_kind = %EventKind::CallEnded,
                call_id = followed.id.0,
                "Call {} ended",
                followed.id
            );
            outcome.inputs.push(Input::CallEnded { call: followed.id });
            return;
        }
        self.vanish(followed.id, object.map(|object| object.number), outcome);
    }

    fn vanish(&mut self, id: CallId, number: Option<String>, outcome: &mut Outcome) {
        tracing::info!(
            event_kind = %EventKind::CallVanished,
            call_id = id.0,
            "Call {id} went away without ending; waiting for it to come back"
        );
        self.vanished.push(Vanished {
            id,
            number: number.unwrap_or_default(),
        });
        outcome.graces.push(id);
    }

    /// Every followed Call goes away without ending, as when the
    /// AudioGateway or the whole service does.
    fn vanish_all(&mut self, outcome: &mut Outcome) {
        let followed: Vec<(String, Followed)> = self.followed.drain().collect();
        for (path, followed) in followed {
            if followed.ended {
                continue;
            }
            if followed.ending {
                outcome.inputs.push(Input::CallEnded { call: followed.id });
                continue;
            }
            let number = self
                .call_objects
                .get(&path)
                .map(|object| object.number.clone());
            self.vanish(followed.id, number, outcome);
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
        let calls: Vec<String> = self.call_objects.keys().cloned().collect();
        for call in calls {
            self.follow_call(&call, outcome);
        }
    }

    /// The AudioGateway at `path` is gone. If it was the followed Phone's, the
    /// Phone disconnected, unless another of its AudioGateways remains.
    fn forget(&mut self, path: &str, outcome: &mut Outcome) {
        self.microphone_volumes.remove(path);
        let Some(address) = self.audio_gateways.remove(path) else {
            return;
        };
        tracing::debug!(
            event_kind = %EventKind::AudioGatewayRemoved,
            phone_address = %address,
            path,
            "An AudioGateway disappeared"
        );
        let prefix = format!("{path}/");
        self.call_objects
            .retain(|call, _| !call.starts_with(&prefix));
        if self.connected.as_deref() != Some(path) {
            return;
        }
        self.vanish_all(outcome);
        self.connected = None;
        let before = outcome.inputs.len();
        self.adopt_matching_gateway(outcome);
        if outcome.inputs.len() == before {
            outcome.inputs.push(Input::PhoneDisconnected { address });
        }
    }

    /// Finds the object each command goes to. A command that can't be
    /// carried out fails at once.
    fn resolve(&mut self, commands: Vec<PhoneCommand>, inputs: &mut Vec<Input>) -> Vec<Resolved> {
        let mut batch = Vec::new();
        for command in commands {
            let path = match &command {
                PhoneCommand::Answer(id) | PhoneCommand::HangUp(id) => {
                    let found = self
                        .followed
                        .iter_mut()
                        .find(|(_, followed)| followed.id == *id && !followed.ended);
                    match found {
                        Some((path, followed)) => {
                            if matches!(command, PhoneCommand::HangUp(_)) {
                                followed.ending = true;
                            }
                            Some(path.clone())
                        }
                        None => None,
                    }
                }
                PhoneCommand::MoveAudioToDesktop
                | PhoneCommand::KeepAudioOnPhone
                | PhoneCommand::SetMicrophoneVolume(_)
                | PhoneCommand::SendTones(_) => self.connected.clone(),
            };
            match path {
                Some(path) => batch.push(Resolved { command, path }),
                // With no Phone connected there is no audio to keep; the
                // next connect asks again.
                None if command == PhoneCommand::KeepAudioOnPhone => {}
                None => {
                    tracing::warn!(
                        event_kind = %EventKind::PhoneCommandFailed,
                        error_kind = "no-target",
                        command = ?command,
                        "Couldn't send {command:?}: the Phone has no such Call or isn't connected"
                    );
                    inputs.push(Input::PhoneCommandFailed(command));
                    break;
                }
            }
        }
        batch
    }
}

/// Carries out each batch of Phone commands in order, one D-Bus call at a
/// time, and reports each command done, or the first failure of each batch.
async fn execute(
    session: &Connection,
    mut batches: UnboundedReceiver<Vec<Resolved>>,
    inputs: &UnboundedSender<Input>,
) {
    while let Some(batch) = batches.next().await {
        for Resolved { command, path } in batch {
            let result = match &command {
                PhoneCommand::Answer(_) => call(session, &path, CALL, "Answer", &()).await,
                PhoneCommand::HangUp(_) => hang_up(session, &path).await,
                PhoneCommand::MoveAudioToDesktop => {
                    match set_reject_sco(session, &path, false).await {
                        Ok(()) => call(session, &path, TRANSPORT, "Activate", &()).await,
                        error => error,
                    }
                }
                PhoneCommand::KeepAudioOnPhone => set_reject_sco(session, &path, true).await,
                PhoneCommand::SetMicrophoneVolume(volume) => {
                    set_microphone_volume(session, &path, *volume).await
                }
                PhoneCommand::SendTones(tones) => send_tones(session, &path, tones).await,
            };
            match result {
                Ok(()) => {
                    tracing::debug!(
                        event_kind = %EventKind::PhoneCommandSent,
                        command = ?command,
                        "The Phone did {command:?}"
                    );
                    let _ = inputs.unbounded_send(Input::PhoneCommandDone(command));
                }
                Err(error) => {
                    tracing::warn!(
                        event_kind = %EventKind::PhoneCommandFailed,
                        error_kind = error_kind(&error),
                        command = ?command,
                        detail = %error,
                        "The Phone refused {command:?}"
                    );
                    let _ = inputs.unbounded_send(Input::PhoneCommandFailed(command));
                    break;
                }
            }
        }
    }
}

async fn call<B>(
    session: &Connection,
    path: &str,
    interface: &str,
    member: &str,
    body: &B,
) -> zbus::Result<()>
where
    B: zbus::export::serde::Serialize + zbus::DynamicType,
{
    session
        .call_method(Some(SERVICE), path, Some(interface), member, body)
        .await
        .map(drop)
}

async fn set_reject_sco(session: &Connection, path: &str, reject: bool) -> zbus::Result<()> {
    call(
        session,
        path,
        PROPERTIES,
        "Set",
        &(TRANSPORT, "RejectSCO", Variant::from(reject)),
    )
    .await
}

async fn set_microphone_volume(session: &Connection, path: &str, volume: u8) -> zbus::Result<()> {
    call(
        session,
        path,
        PROPERTIES,
        "Set",
        &(AUDIO_GATEWAY, "MicrophoneVolume", Variant::from(volume)),
    )
    .await
}

/// Plays `tones` one at a time, each once the Phone has played the last.
async fn send_tones(session: &Connection, path: &str, tones: &str) -> zbus::Result<()> {
    let mut tone = [0; 4];
    for character in tones.chars() {
        let tone: &str = character.encode_utf8(&mut tone);
        call(session, path, AUDIO_GATEWAY, "SendTones", &(tone,)).await?;
    }
    Ok(())
}

/// Hangs up the Call at `path`: through `Call1`, or `org.ofono.VoiceCall`
/// where `Call1`'s method doesn't exist. An error that says the Call is
/// already going, or gone, means it ended.
async fn hang_up(session: &Connection, path: &str) -> zbus::Result<()> {
    let result = match call(session, path, CALL, "Hangup", &()).await {
        Err(error) if is_missing_method(&error) => {
            tracing::debug!(
                event_kind = %EventKind::PhoneCommandFallback,
                detail = %error,
                "Call1.Hangup doesn't exist here; hanging up through org.ofono.VoiceCall"
            );
            call(session, path, OFONO_CALL, "Hangup", &()).await
        }
        result => result,
    };
    match result {
        Err(error) if is_already_ending(&error) => {
            tracing::debug!(
                event_kind = %EventKind::PhoneCommandSent,
                detail = %error,
                "The Call was already ending"
            );
            Ok(())
        }
        result => result,
    }
}

fn error_name(error: &zbus::Error) -> Option<&str> {
    match error {
        zbus::Error::MethodError(name, _, _) => Some(name.as_str()),
        zbus::Error::FDO(error) => Some(match **error {
            zbus::fdo::Error::UnknownMethod(_) => "org.freedesktop.DBus.Error.UnknownMethod",
            zbus::fdo::Error::UnknownObject(_) => "org.freedesktop.DBus.Error.UnknownObject",
            zbus::fdo::Error::InvalidArgs(_) => "org.freedesktop.DBus.Error.InvalidArgs",
            _ => return None,
        }),
        _ => None,
    }
}

fn error_text(error: &zbus::Error) -> String {
    match error {
        zbus::Error::MethodError(_, Some(text), _) => text.clone(),
        zbus::Error::FDO(error) => error.to_string(),
        _ => String::new(),
    }
}

/// libdbus's reply for a method nobody handles: 'Method "Hangup" with
/// signature "" on interface "…" doesn't exist'.
fn is_missing_method(error: &zbus::Error) -> bool {
    error_name(error) == Some("org.freedesktop.DBus.Error.UnknownMethod")
        && error_text(error).contains("doesn't exist")
}

/// A hang-up error for a Call that is already being torn down or is gone:
/// "Too few arguments" while it tears down, `InvalidState` once it is
/// `disconnected`, or no object at all.
fn is_already_ending(error: &zbus::Error) -> bool {
    match error_name(error) {
        Some("org.freedesktop.DBus.Error.InvalidArgs") => {
            error_text(error).contains("Too few arguments")
        }
        Some(
            "org.pipewire.Telephony.Error.InvalidState"
            | "org.freedesktop.DBus.Error.UnknownObject"
            | "org.freedesktop.DBus.Error.UnknownMethod",
        ) => true,
        _ => false,
    }
}

/// The kind of a Phone command failure, for the log.
fn error_kind(error: &zbus::Error) -> &'static str {
    match error_name(error) {
        Some("org.pipewire.Telephony.Error.InvalidState") => "invalid-state",
        Some("org.pipewire.Telephony.Error.InProgress") => "in-progress",
        Some("org.pipewire.Telephony.Error.NotSupported") => "not-supported",
        Some("org.pipewire.Telephony.Error.CME") => "cme",
        Some("org.pipewire.Telephony.Error.Failed") => "failed",
        Some(_) => "dbus-error",
        None => "no-reply",
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
