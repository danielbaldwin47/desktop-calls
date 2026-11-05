//! The Telephony service's side of BlueZ dropping hands-free and connecting
//! it again (`Device1.DisconnectProfile`, then `ConnectProfile`), as watched
//! on the real Pixel 9a mid-Call (ticket #14's prototype,
//! `docs/research/2026-11-04-handoff-to-phone-prototype.md`):
//!
//! - As hands-free drops, each Call is unannounced, without going
//!   `disconnected`, then the AudioGateway.
//! - When it is back, the AudioGateway is announced again at the same path,
//!   idle, codec 0 and rejecting audio; then each Call at its own path,
//!   still in its state; then the AudioGateway's volumes.
//! - A while later the Phone tries to bring the audio link up for the live
//!   Call, first with its own codec, then with CVSD. While `RejectSCO` is true
//!   the Desktop refuses each try: the codec changes and the state stays
//!   idle.

use super::methods::rejects_sco;
use super::tree::{announce_call, insert_call, open_audio_link, set_transport};
use super::{
    AUDIO_GATEWAY, AUDIO_GATEWAY_INTERFACES, Behaviour, MANAGER_PATH, Signal, add_audio_gateway,
    calls, unannounce_call,
};
use crate::objects::{ObjectTree, Properties};
use zbus::{OwnedObjectPath, OwnedVariant};

/// The hands-free codec id the Phone falls back to: CVSD.
const CVSD: u8 = 1;

/// What dropping hands-free took away, to bring back with it.
pub struct Away {
    pub audio_gateway: OwnedObjectPath,
    /// The Calls, by path, with their properties.
    pub calls: Vec<(OwnedObjectPath, Properties)>,
}

/// Drops the AudioGateway at `audio_gateway` and its Calls. Returns what
/// went, and the signals that announce it.
pub fn drop_hands_free(
    tree: &mut ObjectTree,
    audio_gateway: &OwnedObjectPath,
) -> zbus::Result<(Away, Vec<Signal>)> {
    let calls = calls(tree, audio_gateway.as_str());
    let mut signals = Vec::new();
    for (call, _) in &calls {
        tree.remove(call);
        signals.extend(unannounce_call(audio_gateway, call));
    }
    tree.remove(audio_gateway);
    signals.push(Signal::InterfacesRemoved {
        manager: OwnedObjectPath::try_from(MANAGER_PATH)?,
        object: audio_gateway.clone(),
        interfaces: AUDIO_GATEWAY_INTERFACES.to_vec(),
    });
    Ok((
        Away {
            audio_gateway: audio_gateway.clone(),
            calls,
        },
        signals,
    ))
}

/// Registers the AudioGateway again, as new: returns the signals that
/// announce it.
pub fn audio_gateway_back(
    tree: &mut ObjectTree,
    away: &Away,
    address: &str,
    behaviour: &Behaviour,
) -> zbus::Result<Vec<Signal>> {
    let interfaces = add_audio_gateway(tree, &away.audio_gateway, address, behaviour);
    Ok(vec![Signal::InterfacesAdded {
        manager: OwnedObjectPath::try_from(MANAGER_PATH)?,
        object: away.audio_gateway.clone(),
        interfaces,
    }])
}

/// Registers the Calls again, as they were, then reports the
/// AudioGateway's volumes: returns the signals.
pub fn calls_back(tree: &mut ObjectTree, away: Away) -> Vec<Signal> {
    let mut signals = Vec::new();
    for (call, properties) in away.calls {
        insert_call(tree, &call, properties.clone());
        signals.extend(announce_call(&away.audio_gateway, &call, properties));
    }
    let volumes: Properties = ["SpeakerVolume", "MicrophoneVolume"]
        .into_iter()
        .filter_map(|name| {
            let value = tree.get(&away.audio_gateway, AUDIO_GATEWAY, name)?;
            Some((name.to_owned(), value.try_clone().ok()?))
        })
        .collect();
    signals.push(Signal::PropertiesChanged {
        path: away.audio_gateway,
        interface: AUDIO_GATEWAY,
        changed: volumes,
    });
    signals
}

/// The Phone's tries to bring the audio link up for the live Call once
/// hands-free is back: each try's signals, in order. While `RejectSCO` is
/// true the Desktop refuses them, so only the codec changes; otherwise the
/// first try brings the audio to the Desktop.
pub fn audio_link_tries(
    tree: &mut ObjectTree,
    audio_gateway: &OwnedObjectPath,
    behaviour: &Behaviour,
) -> Vec<Vec<Signal>> {
    if calls(tree, audio_gateway.as_str()).is_empty() {
        return Vec::new();
    }
    if !rejects_sco(tree, audio_gateway) {
        return vec![open_audio_link(tree, audio_gateway, behaviour)];
    }
    [behaviour.codec, CVSD]
        .into_iter()
        .map(|codec| {
            set_transport(
                tree,
                audio_gateway,
                vec![("Codec", OwnedVariant::from(codec))],
            )
        })
        .collect()
}
