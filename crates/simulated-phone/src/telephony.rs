//! `org.pipewire.Telephony` on the session bus, shaped like PipeWire 1.6.9
//! talking to the real Pixel 9a. Sources: the first-connection diagnosis and
//! the D-Bus snapshot in `docs/research/2026-11-04-dbus-snapshot/`.
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
//! - A Call is `agN/callM`, with `Call1` and `org.ofono.VoiceCall`. The
//!   AudioGateway's `ObjectManager` announces it with `InterfacesAdded`
//!   naming `Call1` only, followed by `org.ofono.VoiceCallManager.CallAdded`.

use std::collections::HashMap;

use zbus::{Message, OwnedObjectPath, OwnedVariant, Str};

use crate::objects::{
    INTROSPECTABLE, Interfaces, MethodCall, OBJECT_MANAGER, ObjectTree, PROPERTIES, Properties,
    Reply,
};

pub const SERVICE: &str = "org.pipewire.Telephony";
pub const MANAGER_PATH: &str = "/org/pipewire/Telephony";

const OFONO_MANAGER: &str = "org.ofono.Manager";
const AUDIO_GATEWAY: &str = "org.pipewire.Telephony.AudioGateway1";
const AUDIO_GATEWAY_TRANSPORT: &str = "org.pipewire.Telephony.AudioGatewayTransport1";
pub const VOICE_CALL_MANAGER: &str = "org.ofono.VoiceCallManager";
pub const CALL: &str = "org.pipewire.Telephony.Call1";
const VOICE_CALL: &str = "org.ofono.VoiceCall";

/// `Introspect` on `/org/pipewire/Telephony`, captured from the real service.
const MANAGER_INTROSPECTION: &str = include_str!("../introspection/telephony-manager.xml");

/// `Introspect` on `/org/pipewire/Telephony/ag1`, captured from the real service.
const AUDIO_GATEWAY_INTROSPECTION: &str =
    include_str!("../introspection/telephony-audio-gateway.xml");

/// `Introspect` on `/org/pipewire/Telephony/ag1/call1`, captured from the real service.
const CALL_INTROSPECTION: &str = include_str!("../introspection/telephony-call.xml");

/// The interfaces `InterfacesRemoved` names when an AudioGateway goes.
pub const AUDIO_GATEWAY_INTERFACES: [&str; 2] = [AUDIO_GATEWAY, AUDIO_GATEWAY_TRANSPORT];

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
/// `audio_gateway`, as `callN` with N = `index`, and returns its path and
/// interfaces as `InterfacesAdded` carries them.
pub fn add_incoming_call(
    tree: &mut ObjectTree,
    audio_gateway: &OwnedObjectPath,
    index: u32,
    number: &str,
) -> zbus::Result<(OwnedObjectPath, Interfaces)> {
    let path = OwnedObjectPath::try_from(format!("{audio_gateway}/call{index}"))?;
    let mut call = Properties::new();
    call.insert(
        "LineIdentification".into(),
        OwnedVariant::from(Str::from(number.to_owned())),
    );
    call.insert("IncomingLine".into(), OwnedVariant::from(Str::from("")));
    call.insert("Name".into(), OwnedVariant::from(Str::from("")));
    call.insert("Multiparty".into(), OwnedVariant::from(false));
    call.insert("State".into(), OwnedVariant::from(Str::from("incoming")));
    let interfaces = Interfaces::from([(CALL.to_owned(), call)]);
    tree.insert(
        &path,
        interfaces.clone(),
        vec![VOICE_CALL, PROPERTIES, INTROSPECTABLE],
    );
    tree.capture_introspection(&path, CALL_INTROSPECTION);
    Ok((path, interfaces))
}

/// The Calls on the AudioGateway at `audio_gateway`, by path.
pub fn calls(tree: &ObjectTree, audio_gateway: &str) -> Vec<(OwnedObjectPath, Properties)> {
    tree.objects_below(audio_gateway)
        .into_iter()
        .filter_map(|(path, interfaces)| Some((path, interfaces.get(CALL)?.clone())))
        .collect()
}

/// Answers a method call on the Telephony service.
pub fn answer(
    tree: &ObjectTree,
    audio_gateways: &[OwnedObjectPath],
    call: &MethodCall,
    message: &Message,
) -> Reply {
    match tree.answer(call, message) {
        Reply::Unhandled => {}
        reply => return reply,
    }
    match (
        call.path.as_str(),
        call.interface.as_str(),
        call.member.as_str(),
    ) {
        (MANAGER_PATH, OFONO_MANAGER, "GetModems") => Reply::body(
            audio_gateways
                .iter()
                .map(|path| (path.clone(), HashMap::<String, OwnedVariant>::new()))
                .collect::<HashMap<_, _>>(),
        ),
        (_, VOICE_CALL_MANAGER, "GetCalls")
            if audio_gateways.iter().any(|path| path.as_str() == call.path) =>
        {
            Reply::body(
                calls(tree, &call.path)
                    .into_iter()
                    .map(|(path, properties)| (path, properties.into_iter().collect()))
                    .collect::<HashMap<OwnedObjectPath, HashMap<String, OwnedVariant>>>(),
            )
        }
        _ => Reply::unknown_method(call),
    }
}
