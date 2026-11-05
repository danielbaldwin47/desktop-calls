//! Registering, announcing, changing and ending Calls and the audio transport
//! in the object tree.

use super::methods::{bring_audio_up, rejects_sco};
use super::phone::{calls, unannounce_call};
use super::{AUDIO_GATEWAY_TRANSPORT, Behaviour, CALL, CALL_INTROSPECTION, Signal, VOICE_CALL};
use crate::objects::{INTROSPECTABLE, Interfaces, ObjectTree, PROPERTIES, Properties};
use std::collections::BTreeSet;
use zbus::{OwnedObjectPath, OwnedVariant, Str};

/// The Call at `path` goes `disconnected` and is unregistered; the audio
/// link drops with the last Call.
pub(super) fn end_call(
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
pub(super) fn open_audio_link(
    tree: &mut ObjectTree,
    audio_gateway: &OwnedObjectPath,
    behaviour: &Behaviour,
) -> Vec<Signal> {
    if rejects_sco(tree, audio_gateway)
        || transport_state(tree, audio_gateway).as_deref() != Some("idle")
    {
        return Vec::new();
    }
    bring_audio_up(
        tree,
        audio_gateway,
        vec![("Codec", OwnedVariant::from(behaviour.codec))],
    )
}

/// Registers a Call at the lowest free `callN` of the AudioGateway.
pub(super) fn register_call(
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

pub(super) fn insert_call(tree: &mut ObjectTree, path: &OwnedObjectPath, properties: Properties) {
    tree.insert(
        path,
        Interfaces::from([(CALL.to_owned(), properties)]),
        vec![VOICE_CALL, PROPERTIES, INTROSPECTABLE],
    );
    tree.capture_introspection(path, CALL_INTROSPECTION);
}

pub(super) fn announce_call(
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

pub(super) fn find_call(
    tree: &ObjectTree,
    audio_gateway: &OwnedObjectPath,
    path: &str,
) -> Option<(OwnedObjectPath, Properties)> {
    calls(tree, audio_gateway.as_str())
        .into_iter()
        .find(|(call, _)| call.as_str() == path)
}

pub(super) fn call_state(properties: &Properties) -> String {
    properties
        .get("State")
        .and_then(|state| String::try_from(state.clone()).ok())
        .unwrap_or_default()
}

pub(super) fn set_call_state(
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

pub(super) fn transport_state(
    tree: &ObjectTree,
    audio_gateway: &OwnedObjectPath,
) -> Option<String> {
    tree.get(audio_gateway, AUDIO_GATEWAY_TRANSPORT, "State")
        .and_then(|state| String::try_from(state.clone()).ok())
}

/// Sets transport properties and returns the one `PropertiesChanged` that
/// reports them together.
pub(super) fn set_transport(
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
