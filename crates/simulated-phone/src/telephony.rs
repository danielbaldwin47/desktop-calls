//! `org.pipewire.Telephony` on the session bus, shaped like PipeWire 1.6.9
//! talking to the real Pixel 9a. Sources: the first-connection diagnosis, the
//! D-Bus snapshot in `docs/research/2026-11-04-dbus-snapshot/`, and for
//! answering and hanging up, PipeWire 1.6.9's source
//! (`docs/research/2026-11-04-incoming-call.md`).
//!
//! - `Introspect` answers with the XML captured from the real service
//!   (`introspection/`), which lists no child nodes. The manager has
//!   `org.ofono.Manager`, `ObjectManager` and `Introspectable`, and no
//!   `Properties`.
//! - An AudioGateway object exists only while hands-free is fully up, at
//!   `agN`, where N is not 0 and grows with every registration.
//! - `GetManagedObjects` and `InterfacesAdded` carry `AudioGateway1` and
//!   `AudioGatewayTransport1` only: `org.ofono.VoiceCallManager`, which has no
//!   properties, appears in `Introspect` alone.
//! - `GetModems` and `GetCalls` return `a{oa{sv}}`, and `GetModems` lists the
//!   AudioGateways with empty property lists.
//! - A Call is `agN/callM`, with `Call1` and `org.ofono.VoiceCall`, where M is
//!   the lowest number no live Call of the AudioGateway has, so consecutive
//!   Calls reuse a path. The AudioGateway's `ObjectManager` announces it with
//!   `InterfacesAdded` naming `Call1` only, followed by
//!   `org.ofono.VoiceCallManager.CallAdded`.
//! - `Answer` and `Hangup` behave alike on `Call1` and `org.ofono.VoiceCall`.
//!   The reply comes once the Phone confirms, and the state change after it:
//!   `PropertiesChanged` on `Call1`, then `org.ofono.VoiceCall.PropertyChanged`.
//!   A Call that is hung up goes `disconnected`, then `InterfacesRemoved` and
//!   `CallRemoved`. In a state that doesn't allow the method, the error is
//!   `org.pipewire.Telephony.Error.InvalidState`.
//! - Setting `RejectSCO` emits no signal. `Activate` does nothing while it is
//!   true; otherwise the transport reports its codec, then `pending`, then
//!   `active`, as the Pixel does.

use std::collections::{BTreeSet, HashMap};

use zbus::{Connection, Message, OwnedObjectPath, OwnedVariant, Str};

use crate::objects::{
    INTROSPECTABLE, Interfaces, MethodCall, OBJECT_MANAGER, ObjectTree, PROPERTIES, Properties,
    Reply, interfaces_added_body,
};

pub const SERVICE: &str = "org.pipewire.Telephony";
pub const MANAGER_PATH: &str = "/org/pipewire/Telephony";

const OFONO_MANAGER: &str = "org.ofono.Manager";
const AUDIO_GATEWAY: &str = "org.pipewire.Telephony.AudioGateway1";
const AUDIO_GATEWAY_TRANSPORT: &str = "org.pipewire.Telephony.AudioGatewayTransport1";
pub const VOICE_CALL_MANAGER: &str = "org.ofono.VoiceCallManager";
pub const CALL: &str = "org.pipewire.Telephony.Call1";
const VOICE_CALL: &str = "org.ofono.VoiceCall";

const INVALID_STATE: &str = "org.pipewire.Telephony.Error.InvalidState";
const INVALID_STATE_MESSAGE: &str = "The current state does not allow this method call";

/// `Introspect` on `/org/pipewire/Telephony`, captured from the real service.
const MANAGER_INTROSPECTION: &str = include_str!("../introspection/telephony-manager.xml");

/// `Introspect` on `/org/pipewire/Telephony/ag1`, captured from the real service.
const AUDIO_GATEWAY_INTROSPECTION: &str =
    include_str!("../introspection/telephony-audio-gateway.xml");

/// `Introspect` on `/org/pipewire/Telephony/ag1/call1`, captured from the real service.
const CALL_INTROSPECTION: &str = include_str!("../introspection/telephony-call.xml");

/// The interfaces `InterfacesRemoved` names when an AudioGateway goes.
pub const AUDIO_GATEWAY_INTERFACES: [&str; 2] = [AUDIO_GATEWAY, AUDIO_GATEWAY_TRANSPORT];

/// A departure from the Pixel's usual behaviour that other Phones, or other
/// moments, show. Tests switch them on through the control interface.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Quirk {
    /// `Call1.Hangup` fails as libdbus does for a method nobody handles, so
    /// only `org.ofono.VoiceCall.Hangup` works.
    Call1HangupMissing,
    /// `Hangup` replies `InvalidArgs` "Too few arguments" while the Call is
    /// torn down anyway.
    HangupTooFewArguments,
}

impl Quirk {
    /// The quirk with this control-interface name.
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "call1-hangup-missing" => Some(Self::Call1HangupMissing),
            "hangup-too-few-arguments" => Some(Self::HangupTooFewArguments),
            _ => None,
        }
    }
}

/// How the Phone behaves where Phones differ.
pub struct Behaviour {
    /// The codec id the audio link negotiates: 1 CVSD, 2 mSBC, 3 LC3-SWB.
    pub codec: u8,
    pub quirks: BTreeSet<Quirk>,
}

impl Default for Behaviour {
    /// The Pixel 9a: LC3-SWB, no quirks.
    fn default() -> Self {
        Self {
            codec: 3,
            quirks: BTreeSet::new(),
        }
    }
}

/// A signal the service emits.
pub enum Signal {
    PropertiesChanged {
        path: OwnedObjectPath,
        interface: &'static str,
        changed: Properties,
    },
    /// `org.ofono.VoiceCall.PropertyChanged`, one per changed property.
    VoiceCallPropertyChanged {
        path: OwnedObjectPath,
        name: &'static str,
        value: OwnedVariant,
    },
    InterfacesAdded {
        manager: OwnedObjectPath,
        object: OwnedObjectPath,
        interfaces: Interfaces,
    },
    InterfacesRemoved {
        manager: OwnedObjectPath,
        object: OwnedObjectPath,
        interfaces: Vec<&'static str>,
    },
    CallAdded {
        audio_gateway: OwnedObjectPath,
        call: OwnedObjectPath,
        properties: Properties,
    },
    CallRemoved {
        audio_gateway: OwnedObjectPath,
        call: OwnedObjectPath,
    },
}

impl Signal {
    pub async fn emit(&self, connection: &Connection) -> zbus::Result<()> {
        match self {
            Self::PropertiesChanged {
                path,
                interface,
                changed,
            } => {
                let changed: HashMap<&str, &OwnedVariant> = changed
                    .iter()
                    .map(|(name, value)| (name.as_str(), value))
                    .collect();
                connection
                    .emit_signal(
                        None::<&str>,
                        path.as_str(),
                        PROPERTIES,
                        "PropertiesChanged",
                        &(*interface, changed, Vec::<String>::new()),
                    )
                    .await
            }
            Self::VoiceCallPropertyChanged { path, name, value } => {
                connection
                    .emit_signal(
                        None::<&str>,
                        path.as_str(),
                        VOICE_CALL,
                        "PropertyChanged",
                        &(*name, value),
                    )
                    .await
            }
            Self::InterfacesAdded {
                manager,
                object,
                interfaces,
            } => {
                connection
                    .emit_signal(
                        None::<&str>,
                        manager.as_str(),
                        OBJECT_MANAGER,
                        "InterfacesAdded",
                        &(object, interfaces_added_body(interfaces)),
                    )
                    .await
            }
            Self::InterfacesRemoved {
                manager,
                object,
                interfaces,
            } => {
                connection
                    .emit_signal(
                        None::<&str>,
                        manager.as_str(),
                        OBJECT_MANAGER,
                        "InterfacesRemoved",
                        &(object, interfaces),
                    )
                    .await
            }
            Self::CallAdded {
                audio_gateway,
                call,
                properties,
            } => {
                let properties: HashMap<&str, &OwnedVariant> = properties
                    .iter()
                    .map(|(name, value)| (name.as_str(), value))
                    .collect();
                connection
                    .emit_signal(
                        None::<&str>,
                        audio_gateway.as_str(),
                        VOICE_CALL_MANAGER,
                        "CallAdded",
                        &(call, properties),
                    )
                    .await
            }
            Self::CallRemoved {
                audio_gateway,
                call,
            } => {
                connection
                    .emit_signal(
                        None::<&str>,
                        audio_gateway.as_str(),
                        VOICE_CALL_MANAGER,
                        "CallRemoved",
                        &(call,),
                    )
                    .await
            }
        }
    }
}

/// The Telephony service with no Phone connected: just the manager.
pub fn new_tree() -> zbus::Result<ObjectTree> {
    let mut tree = ObjectTree::new(false);
    let manager = OwnedObjectPath::try_from(MANAGER_PATH)?;
    tree.insert(
        &manager,
        Interfaces::new(),
        vec![OFONO_MANAGER, OBJECT_MANAGER, INTROSPECTABLE],
    );
    tree.capture_introspection(&manager, MANAGER_INTROSPECTION);
    Ok(tree)
}

/// The path of the AudioGateway registered with `index`.
pub fn audio_gateway_path(index: u32) -> zbus::Result<OwnedObjectPath> {
    OwnedObjectPath::try_from(format!("{MANAGER_PATH}/ag{index}"))
}

/// Registers an idle AudioGateway for the Phone at `address` at `path`, as
/// PipeWire does once the hands-free service level connection is up, and
/// returns its interfaces as `InterfacesAdded` carries them.
pub fn add_audio_gateway(
    tree: &mut ObjectTree,
    path: &OwnedObjectPath,
    address: &str,
) -> Interfaces {
    let mut gateway = Properties::new();
    gateway.insert(
        "Address".into(),
        OwnedVariant::from(Str::from(address.to_owned())),
    );
    gateway.insert("SpeakerVolume".into(), OwnedVariant::from(15_u8));
    gateway.insert("MicrophoneVolume".into(), OwnedVariant::from(15_u8));
    let mut transport = Properties::new();
    transport.insert("Codec".into(), OwnedVariant::from(0_u8));
    transport.insert("State".into(), OwnedVariant::from(Str::from("idle")));
    transport.insert("RejectSCO".into(), OwnedVariant::from(true));
    let mut interfaces = Interfaces::new();
    interfaces.insert(AUDIO_GATEWAY.into(), gateway);
    interfaces.insert(AUDIO_GATEWAY_TRANSPORT.into(), transport);
    tree.insert(
        path,
        interfaces.clone(),
        vec![
            VOICE_CALL_MANAGER,
            OBJECT_MANAGER,
            PROPERTIES,
            INTROSPECTABLE,
        ],
    );
    tree.capture_introspection(path, AUDIO_GATEWAY_INTROSPECTION);
    interfaces
}

/// Registers an incoming Call from `number` on the AudioGateway at
/// `audio_gateway`, as the Phone ringing does, and returns its path and the
/// signals that announce it. While no audio link is up, the Phone's attempt
/// to bring one up is refused, and only the codec shows it.
pub fn ring(
    tree: &mut ObjectTree,
    audio_gateway: &OwnedObjectPath,
    behaviour: &Behaviour,
    number: &str,
) -> zbus::Result<(OwnedObjectPath, Vec<Signal>)> {
    let mut call = Properties::new();
    call.insert(
        "LineIdentification".into(),
        OwnedVariant::from(Str::from(number.to_owned())),
    );
    call.insert("IncomingLine".into(), OwnedVariant::from(Str::from("")));
    call.insert("Name".into(), OwnedVariant::from(Str::from("")));
    call.insert("Multiparty".into(), OwnedVariant::from(false));
    call.insert("State".into(), OwnedVariant::from(Str::from("incoming")));
    let path = register_call(tree, audio_gateway, call.clone())?;
    let mut signals = announce_call(audio_gateway, &path, call);
    if transport_state(tree, audio_gateway).as_deref() == Some("idle") {
        signals.extend(set_transport(
            tree,
            audio_gateway,
            vec![("Codec", OwnedVariant::from(behaviour.codec))],
        ));
    }
    Ok((path, signals))
}

/// Unregisters the Call at `call` and registers it again at the same path
/// with the same properties, as PipeWire does when the hands-free profile
/// briefly drops mid-Call. Returns the signals for each half.
pub fn reregister_call(
    tree: &mut ObjectTree,
    audio_gateway: &OwnedObjectPath,
    call: &OwnedObjectPath,
) -> Option<(Vec<Signal>, Vec<Signal>)> {
    let properties = tree.remove(call)?.remove(CALL)?;
    let removed = unannounce_call(audio_gateway, call);
    insert_call(tree, call, properties.clone());
    let added = announce_call(audio_gateway, call, properties);
    Some((removed, added))
}

/// The Calls on the AudioGateway at `audio_gateway`, by path.
pub fn calls(tree: &ObjectTree, audio_gateway: &str) -> Vec<(OwnedObjectPath, Properties)> {
    tree.objects_below(audio_gateway)
        .into_iter()
        .filter_map(|(path, interfaces)| Some((path, interfaces.get(CALL)?.clone())))
        .collect()
}

/// The signals that announce the Call at `call` going, as PipeWire emits them
/// when it unregisters a Call.
pub fn unannounce_call(audio_gateway: &OwnedObjectPath, call: &OwnedObjectPath) -> Vec<Signal> {
    vec![
        Signal::InterfacesRemoved {
            manager: audio_gateway.clone(),
            object: call.clone(),
            interfaces: vec![CALL],
        },
        Signal::CallRemoved {
            audio_gateway: audio_gateway.clone(),
            call: call.clone(),
        },
    ]
}

/// Answers a method call on the Telephony service, changing the tree as the
/// real service would. The signals go out after the reply.
pub fn answer(
    tree: &mut ObjectTree,
    audio_gateway: Option<&OwnedObjectPath>,
    behaviour: &Behaviour,
    call: &MethodCall,
    message: &Message,
) -> (Reply, Vec<Signal>) {
    match tree.answer(call, message) {
        Reply::Unhandled => {}
        reply => return (reply, Vec::new()),
    }
    // The AudioGateway the call is addressed to, if it is to the AudioGateway
    // itself or to one of its Calls.
    let to_audio_gateway = audio_gateway.filter(|gateway| gateway.as_str() == call.path);
    let to_call = audio_gateway.filter(|gateway| {
        call.path
            .strip_prefix(gateway.as_str())
            .is_some_and(|rest| rest.starts_with("/call"))
    });
    match (
        call.interface.as_str(),
        call.member.as_str(),
        to_audio_gateway,
        to_call,
    ) {
        (OFONO_MANAGER, "GetModems", _, _) if call.path == MANAGER_PATH => (
            Reply::body(
                audio_gateway
                    .iter()
                    .map(|path| ((*path).clone(), HashMap::<String, OwnedVariant>::new()))
                    .collect::<HashMap<_, _>>(),
            ),
            Vec::new(),
        ),
        (VOICE_CALL_MANAGER, "GetCalls", Some(_), _) => (
            Reply::body(
                calls(tree, &call.path)
                    .into_iter()
                    .map(|(path, properties)| (path, properties.into_iter().collect()))
                    .collect::<HashMap<OwnedObjectPath, HashMap<String, OwnedVariant>>>(),
            ),
            Vec::new(),
        ),
        (PROPERTIES, "Set", Some(_), _) => (set_property(tree, call, message), Vec::new()),
        (AUDIO_GATEWAY_TRANSPORT, "Activate", Some(gateway), _) => {
            (Reply::body(()), activate(tree, gateway, behaviour))
        }
        (CALL | VOICE_CALL, "Answer", _, Some(gateway)) => answer_call(tree, gateway, call),
        (CALL, "Hangup", _, Some(_)) if behaviour.quirks.contains(&Quirk::Call1HangupMissing) => (
            Reply::Error(
                "org.freedesktop.DBus.Error.UnknownMethod",
                format!(
                    "Method \"Hangup\" with signature \"\" on interface \"{CALL}\" doesn't exist\n"
                ),
            ),
            Vec::new(),
        ),
        (CALL | VOICE_CALL, "Hangup", _, Some(gateway)) => hang_up(tree, gateway, behaviour, call),
        _ => (Reply::unknown_method(call), Vec::new()),
    }
}

/// `Properties.Set` on an AudioGateway: only `RejectSCO` is writable here,
/// and setting it emits nothing.
fn set_property(tree: &mut ObjectTree, call: &MethodCall, message: &Message) -> Reply {
    let Ok((interface, name, value)) = message
        .body()
        .deserialize::<(String, String, OwnedVariant)>()
    else {
        return Reply::Error(
            "org.freedesktop.DBus.Error.InvalidArgs",
            "Invalid arguments".to_owned(),
        );
    };
    let Ok(path) = OwnedObjectPath::try_from(call.path.as_str()) else {
        return Reply::unknown_method(call);
    };
    match (interface.as_str(), name.as_str()) {
        (AUDIO_GATEWAY_TRANSPORT, "RejectSCO") if bool::try_from(&value).is_ok() => {
            tree.set(&path, AUDIO_GATEWAY_TRANSPORT, "RejectSCO", value);
            Reply::body(())
        }
        _ => Reply::Error(
            "org.freedesktop.DBus.Error.InvalidArgs",
            format!("No such writable property '{name}'"),
        ),
    }
}

/// `Activate`: brings the Call audio to the Desktop, unless SCO is rejected
/// or no Call is live.
fn activate(
    tree: &mut ObjectTree,
    audio_gateway: &OwnedObjectPath,
    behaviour: &Behaviour,
) -> Vec<Signal> {
    let rejected = tree
        .get(audio_gateway, AUDIO_GATEWAY_TRANSPORT, "RejectSCO")
        .and_then(|value| bool::try_from(value).ok())
        .unwrap_or(true);
    let live = calls(tree, audio_gateway.as_str())
        .iter()
        .any(|(_, properties)| call_state(properties) != "disconnected");
    if rejected || !live || transport_state(tree, audio_gateway).as_deref() != Some("idle") {
        return Vec::new();
    }
    let mut signals = set_transport(
        tree,
        audio_gateway,
        vec![
            ("Codec", OwnedVariant::from(behaviour.codec)),
            ("RejectSCO", OwnedVariant::from(false)),
        ],
    );
    signals.extend(set_transport(
        tree,
        audio_gateway,
        vec![("State", OwnedVariant::from(Str::from("pending")))],
    ));
    signals.extend(set_transport(
        tree,
        audio_gateway,
        vec![("State", OwnedVariant::from(Str::from("active")))],
    ));
    signals
}

fn answer_call(
    tree: &mut ObjectTree,
    audio_gateway: &OwnedObjectPath,
    call: &MethodCall,
) -> (Reply, Vec<Signal>) {
    let Some((path, properties)) = find_call(tree, audio_gateway, &call.path) else {
        return (Reply::unknown_method(call), Vec::new());
    };
    if call_state(&properties) != "incoming" {
        return (invalid_state(), Vec::new());
    }
    (Reply::body(()), set_call_state(tree, &path, "active"))
}

fn hang_up(
    tree: &mut ObjectTree,
    audio_gateway: &OwnedObjectPath,
    behaviour: &Behaviour,
    call: &MethodCall,
) -> (Reply, Vec<Signal>) {
    let Some((path, properties)) = find_call(tree, audio_gateway, &call.path) else {
        return (Reply::unknown_method(call), Vec::new());
    };
    if !matches!(
        call_state(&properties).as_str(),
        "incoming" | "waiting" | "active" | "dialing" | "alerting" | "held"
    ) {
        return (invalid_state(), Vec::new());
    }
    let signals = end_call(tree, audio_gateway, &path);
    let reply = if behaviour.quirks.contains(&Quirk::HangupTooFewArguments) {
        Reply::Error(
            "org.freedesktop.DBus.Error.InvalidArgs",
            "Too few arguments".to_owned(),
        )
    } else {
        Reply::body(())
    };
    (reply, signals)
}

/// Answers the ringing Call at `call` on the Phone itself, as its holder
/// does: the Call goes `active`, and the Phone tries to bring its audio link
/// up, which the Desktop accepts only while `RejectSCO` is false. Returns the
/// signals, or why the Phone can't.
pub fn answer_on_phone(
    tree: &mut ObjectTree,
    audio_gateway: &OwnedObjectPath,
    behaviour: &Behaviour,
    call: &str,
) -> Result<Vec<Signal>, String> {
    let (path, properties) = find_call(tree, audio_gateway, call)
        .ok_or_else(|| format!("there is no Call at {call}"))?;
    let state = call_state(&properties);
    if state != "incoming" {
        return Err(format!("the Call at {call} is {state}, not ringing"));
    }
    let mut signals = set_call_state(tree, &path, "active");
    signals.extend(open_audio_link(tree, audio_gateway, behaviour));
    Ok(signals)
}

/// Ends the Call at `call` from the Phone's side, as declining it on the
/// Phone or the caller hanging up does: PipeWire sees the Call vanish from
/// the Phone's list (`+CIEV` or `+CLCC`), marks it `disconnected`, then
/// unregisters it (`backend-native.c`, `hfp_hf_clcc_update` and the
/// `callsetup`/`call` indicators). `ringing_only` refuses a Call that is not
/// ringing.
pub fn end_call_on_phone(
    tree: &mut ObjectTree,
    audio_gateway: &OwnedObjectPath,
    call: &str,
    ringing_only: bool,
) -> Result<Vec<Signal>, String> {
    let (path, properties) = find_call(tree, audio_gateway, call)
        .ok_or_else(|| format!("there is no Call at {call}"))?;
    let state = call_state(&properties);
    if ringing_only && !matches!(state.as_str(), "incoming" | "waiting") {
        return Err(format!("the Call at {call} is {state}, not ringing"));
    }
    Ok(end_call(tree, audio_gateway, &path))
}

/// The Call at `path` goes `disconnected` and is unregistered; the audio
/// link drops with the last Call.
fn end_call(
    tree: &mut ObjectTree,
    audio_gateway: &OwnedObjectPath,
    path: &OwnedObjectPath,
) -> Vec<Signal> {
    let mut signals = set_call_state(tree, path, "disconnected");
    tree.remove(path);
    signals.extend(unannounce_call(audio_gateway, path));
    if calls(tree, audio_gateway.as_str()).is_empty()
        && transport_state(tree, audio_gateway).as_deref() != Some("idle")
    {
        signals.extend(set_transport(
            tree,
            audio_gateway,
            vec![
                ("Codec", OwnedVariant::from(0_u8)),
                ("State", OwnedVariant::from(Str::from("idle"))),
            ],
        ));
    }
    signals
}

/// The Phone brings its audio link up for a live Call, as Android does when
/// a hands-free unit is connected. While `RejectSCO` is true the Desktop
/// refuses it and nothing changes.
fn open_audio_link(
    tree: &mut ObjectTree,
    audio_gateway: &OwnedObjectPath,
    behaviour: &Behaviour,
) -> Vec<Signal> {
    let rejected = tree
        .get(audio_gateway, AUDIO_GATEWAY_TRANSPORT, "RejectSCO")
        .and_then(|value| bool::try_from(value).ok())
        .unwrap_or(true);
    if rejected || transport_state(tree, audio_gateway).as_deref() != Some("idle") {
        return Vec::new();
    }
    let mut signals = set_transport(
        tree,
        audio_gateway,
        vec![("Codec", OwnedVariant::from(behaviour.codec))],
    );
    for state in ["pending", "active"] {
        signals.extend(set_transport(
            tree,
            audio_gateway,
            vec![("State", OwnedVariant::from(Str::from(state)))],
        ));
    }
    signals
}

fn invalid_state() -> Reply {
    Reply::Error(INVALID_STATE, INVALID_STATE_MESSAGE.to_owned())
}

/// Registers a Call at the lowest free `callN` of the AudioGateway.
fn register_call(
    tree: &mut ObjectTree,
    audio_gateway: &OwnedObjectPath,
    properties: Properties,
) -> zbus::Result<OwnedObjectPath> {
    let taken: BTreeSet<String> = calls(tree, audio_gateway.as_str())
        .into_iter()
        .map(|(path, _)| path.to_string())
        .collect();
    let mut index = 1;
    let path = loop {
        let path = format!("{audio_gateway}/call{index}");
        if !taken.contains(&path) {
            break OwnedObjectPath::try_from(path)?;
        }
        index += 1;
    };
    insert_call(tree, &path, properties);
    Ok(path)
}

fn insert_call(tree: &mut ObjectTree, path: &OwnedObjectPath, properties: Properties) {
    tree.insert(
        path,
        Interfaces::from([(CALL.to_owned(), properties)]),
        vec![VOICE_CALL, PROPERTIES, INTROSPECTABLE],
    );
    tree.capture_introspection(path, CALL_INTROSPECTION);
}

fn announce_call(
    audio_gateway: &OwnedObjectPath,
    call: &OwnedObjectPath,
    properties: Properties,
) -> Vec<Signal> {
    vec![
        Signal::InterfacesAdded {
            manager: audio_gateway.clone(),
            object: call.clone(),
            interfaces: Interfaces::from([(CALL.to_owned(), properties.clone())]),
        },
        Signal::CallAdded {
            audio_gateway: audio_gateway.clone(),
            call: call.clone(),
            properties,
        },
    ]
}

fn find_call(
    tree: &ObjectTree,
    audio_gateway: &OwnedObjectPath,
    path: &str,
) -> Option<(OwnedObjectPath, Properties)> {
    calls(tree, audio_gateway.as_str())
        .into_iter()
        .find(|(call, _)| call.as_str() == path)
}

fn call_state(properties: &Properties) -> String {
    properties
        .get("State")
        .and_then(|state| String::try_from(state.clone()).ok())
        .unwrap_or_default()
}

fn set_call_state(
    tree: &mut ObjectTree,
    path: &OwnedObjectPath,
    state: &'static str,
) -> Vec<Signal> {
    tree.set(path, CALL, "State", OwnedVariant::from(Str::from(state)));
    vec![
        Signal::PropertiesChanged {
            path: path.clone(),
            interface: CALL,
            changed: Properties::from([("State".to_owned(), OwnedVariant::from(Str::from(state)))]),
        },
        Signal::VoiceCallPropertyChanged {
            path: path.clone(),
            name: "State",
            value: OwnedVariant::from(Str::from(state)),
        },
    ]
}

fn transport_state(tree: &ObjectTree, audio_gateway: &OwnedObjectPath) -> Option<String> {
    tree.get(audio_gateway, AUDIO_GATEWAY_TRANSPORT, "State")
        .and_then(|state| String::try_from(state.clone()).ok())
}

/// Sets transport properties and returns the one `PropertiesChanged` that
/// reports them together.
fn set_transport(
    tree: &mut ObjectTree,
    audio_gateway: &OwnedObjectPath,
    values: Vec<(&str, OwnedVariant)>,
) -> Vec<Signal> {
    let mut reported = Properties::new();
    for (name, value) in values {
        if let Ok(copy) = value.try_clone() {
            tree.set(audio_gateway, AUDIO_GATEWAY_TRANSPORT, name, copy);
            reported.insert(name.to_owned(), value);
        }
    }
    vec![Signal::PropertiesChanged {
        path: audio_gateway.clone(),
        interface: AUDIO_GATEWAY_TRANSPORT,
        changed: reported,
    }]
}
