//! What the Phone does on its own, as the Simulated Phone's control interface
//! asks: ringing, Calls placed or answered on the Phone, volumes, and
//! `WirePlumber` restarting.

use super::methods::{bring_audio_up, rejects_sco};
use super::tree::{
    announce_call, call_state, end_call, find_call, insert_call, open_audio_link, register_call,
    set_call_state, set_transport, transport_state,
};
use super::{
    AUDIO_GATEWAY, AUDIO_GATEWAY_INTERFACES, Behaviour, CALL, Dialling, MANAGER_PATH, Quirk, Signal,
};
use crate::objects::{ObjectTree, Properties};
use zbus::{OwnedObjectPath, OwnedVariant, Str};

/// The number PipeWire 1.6.9's hands-free backend reads from a `+CLIP` or
/// `+CLCC` caller ID: `"%16[^\"]",%u` in `backend-native.c`, which needs 1
/// to 16 characters between the quotes and a number type after them. When
/// the parse fails, as it does for a withheld caller's `"",129`, PipeWire
/// logs "Failed to parse number" and the Call's number stays empty.
pub fn number_from_caller_id(caller_id: &str) -> String {
    let parsed = caller_id.strip_prefix('"').and_then(|rest| {
        let (number, after) = rest.split_once('"')?;
        let kind = after.strip_prefix(',')?;
        let digits = kind.trim_start();
        let has_kind = digits.starts_with(|first: char| first.is_ascii_digit());
        (has_kind && (1..=16).contains(&number.chars().count())).then(|| number.to_owned())
    });
    parsed.unwrap_or_default()
}

/// Registers an incoming Call from `number` on the AudioGateway at
/// `audio_gateway`, as the Phone ringing does, and returns its path and the
/// signals that announce it. While no audio link is up, the Phone's attempt
/// to bring one up is refused, and only the codec shows it. With
/// [`Quirk::LateLineIdentification`] the Call appears with no number, which
/// [`identify_caller`] gives it later.
pub fn ring(
    tree: &mut ObjectTree,
    audio_gateway: &OwnedObjectPath,
    behaviour: &Behaviour,
    number: &str,
) -> zbus::Result<(OwnedObjectPath, Vec<Signal>)> {
    let shown = if behaviour.quirks.contains(&Quirk::LateLineIdentification) {
        ""
    } else {
        number
    };
    let mut call = Properties::new();
    call.insert(
        "LineIdentification".into(),
        OwnedVariant::from(Str::from(shown.to_owned())),
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

/// Gives the Call at `call` the caller's `number`, as PipeWire does with the
/// `+CLIP` that follows a ringing Call's `RING`, and returns the signals that
/// report it: `PropertiesChanged` with `LineIdentification` alone, then
/// `org.ofono.VoiceCall.PropertyChanged`. PipeWire takes a `+CLIP` only while
/// the Call is `incoming`, and only with a number it can parse, so nothing
/// changes for a Call that is no longer ringing, already has this number, or
/// has a withheld caller's empty one.
pub fn identify_caller(tree: &mut ObjectTree, call: &OwnedObjectPath, number: &str) -> Vec<Signal> {
    let ringing = tree
        .get(call, CALL, "State")
        .and_then(|state| String::try_from(state.clone()).ok())
        .is_some_and(|state| state == "incoming");
    let value = OwnedVariant::from(Str::from(number.to_owned()));
    if number.is_empty() || !ringing || !tree.set(call, CALL, "LineIdentification", value.clone()) {
        return Vec::new();
    }
    vec![
        Signal::PropertiesChanged {
            path: call.clone(),
            interface: CALL,
            changed: Properties::from([("LineIdentification".to_owned(), value.clone())]),
        },
        Signal::VoiceCallPropertyChanged {
            path: call.clone(),
            name: "LineIdentification",
            value,
        },
    ]
}

/// Registers the Call the Desktop dialled on the AudioGateway at
/// `audio_gateway`, `alerting`, as the Pixel reports it once the other end
/// rings, and returns its path and the signals that announce it. While
/// `RejectSCO` is false the Phone brings the audio to the Desktop at once;
/// otherwise its refused attempt only flips the codec. `None` if nothing was
/// dialled.
pub fn alert(
    tree: &mut ObjectTree,
    audio_gateway: &OwnedObjectPath,
    behaviour: &Behaviour,
    dialling: &mut Dialling,
) -> zbus::Result<Option<(OwnedObjectPath, Vec<Signal>)>> {
    let Some(number) = dialling.number.take() else {
        return Ok(None);
    };
    outgoing_call(tree, audio_gateway, behaviour, number, "alerting").map(Some)
}

/// Registers a Call to `number` placed on the Phone itself, `dialing`, as
/// the person holding it does, and returns its path and the signals that
/// announce it. The Desktop set nothing, so `RejectSCO` is still true and
/// the Phone's attempt to bring the audio to the Desktop only flips the
/// codec: the audio stays on the Phone.
pub fn dial_on_phone(
    tree: &mut ObjectTree,
    audio_gateway: &OwnedObjectPath,
    behaviour: &Behaviour,
    number: &str,
) -> zbus::Result<(OwnedObjectPath, Vec<Signal>)> {
    outgoing_call(tree, audio_gateway, behaviour, number.to_owned(), "dialing")
}

/// Registers an outgoing Call to `number` in `state`, and brings the audio
/// to the Desktop if `RejectSCO` allows, as the Pixel does.
fn outgoing_call(
    tree: &mut ObjectTree,
    audio_gateway: &OwnedObjectPath,
    behaviour: &Behaviour,
    number: String,
    state: &str,
) -> zbus::Result<(OwnedObjectPath, Vec<Signal>)> {
    let mut call = Properties::new();
    call.insert(
        "LineIdentification".into(),
        OwnedVariant::from(Str::from(number)),
    );
    call.insert("IncomingLine".into(), OwnedVariant::from(Str::from("")));
    call.insert("Name".into(), OwnedVariant::from(Str::from("")));
    call.insert("Multiparty".into(), OwnedVariant::from(false));
    call.insert(
        "State".into(),
        OwnedVariant::from(Str::from(state.to_owned())),
    );
    let path = register_call(tree, audio_gateway, call.clone())?;
    let mut signals = announce_call(audio_gateway, &path, call);
    if transport_state(tree, audio_gateway).as_deref() == Some("idle") {
        let codec = vec![("Codec", OwnedVariant::from(behaviour.codec))];
        if rejects_sco(tree, audio_gateway) || behaviour.earbuds.active {
            signals.extend(set_transport(tree, audio_gateway, codec));
        } else {
            signals.extend(bring_audio_up(tree, audio_gateway, codec));
        }
    }
    Ok((path, signals))
}

/// The person called answers: the dialled Call goes `active`. Returns the
/// signals, or `None` if no Call is `dialing` or `alerting`.
pub fn callee_answers(
    tree: &mut ObjectTree,
    audio_gateway: &OwnedObjectPath,
) -> Option<Vec<Signal>> {
    let (path, _) = calls(tree, audio_gateway.as_str())
        .into_iter()
        .find(|(_, properties)| {
            matches!(call_state(properties).as_str(), "dialing" | "alerting")
        })?;
    Some(set_call_state(tree, &path, "active"))
}

/// Sets the AudioGateway's `MicrophoneVolume`, as the Phone's `+VGM` or a
/// `Properties.Set` does, and returns the signal that reports it.
pub fn set_microphone_volume(
    tree: &mut ObjectTree,
    audio_gateway: &OwnedObjectPath,
    volume: u8,
) -> Vec<Signal> {
    set_volume(tree, audio_gateway, "MicrophoneVolume", volume)
}

pub(super) fn set_volume(
    tree: &mut ObjectTree,
    audio_gateway: &OwnedObjectPath,
    name: &str,
    volume: u8,
) -> Vec<Signal> {
    tree.set(
        audio_gateway,
        AUDIO_GATEWAY,
        name,
        OwnedVariant::from(volume),
    );
    vec![Signal::PropertiesChanged {
        path: audio_gateway.clone(),
        interface: AUDIO_GATEWAY,
        changed: Properties::from([(name.to_owned(), OwnedVariant::from(volume))]),
    }]
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

/// What the Telephony service announces when `WirePlumber` restarts with the
/// AudioGateway at `audio_gateway` connected, as watched on the real Pixel:
/// the old service unannounces each Call, then the AudioGateway, then leaves
/// the bus; the new one announces the AudioGateway again at the same path,
/// then, 40 ms later, each Call at its own path.
pub struct Restart {
    pub gone: Vec<Signal>,
    pub audio_gateway_back: Vec<Signal>,
    pub calls_back: Vec<Signal>,
}

/// Restarts the Telephony service's view of the AudioGateway at
/// `audio_gateway`. Its Calls and volumes stay; its audio link drops with
/// the old service, so the new one starts idle and rejecting audio, as for a
/// new AudioGateway. That the link drops is inferred, not observed.
pub fn restart(tree: &mut ObjectTree, audio_gateway: &OwnedObjectPath) -> zbus::Result<Restart> {
    let mut gone = Vec::new();
    let mut calls_back = Vec::new();
    for (call, properties) in calls(tree, audio_gateway.as_str()) {
        gone.extend(unannounce_call(audio_gateway, &call));
        calls_back.extend(announce_call(audio_gateway, &call, properties));
    }
    let manager = OwnedObjectPath::try_from(MANAGER_PATH)?;
    gone.push(Signal::InterfacesRemoved {
        manager: manager.clone(),
        object: audio_gateway.clone(),
        interfaces: AUDIO_GATEWAY_INTERFACES.to_vec(),
    });
    set_transport(
        tree,
        audio_gateway,
        vec![
            ("State", OwnedVariant::from(Str::from("idle"))),
            ("Codec", OwnedVariant::from(0_u8)),
            ("RejectSCO", OwnedVariant::from(true)),
        ],
    );
    let interfaces = tree
        .objects_below(MANAGER_PATH)
        .into_iter()
        .find(|(path, _)| path == audio_gateway)
        .map(|(_, interfaces)| interfaces.clone())
        .unwrap_or_default();
    Ok(Restart {
        gone,
        audio_gateway_back: vec![Signal::InterfacesAdded {
            manager,
            object: audio_gateway.clone(),
            interfaces,
        }],
        calls_back,
    })
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

/// Answers the ringing Call at `call` on the Phone itself, as its holder
/// does: the Call goes `active`, and the Phone tries to bring its audio link
/// up, which the Desktop accepts only while `RejectSCO` is false. Returns the
/// signals, or why the Phone can't.
pub fn answer_on_phone(
    tree: &mut ObjectTree,
    audio_gateway: &OwnedObjectPath,
    behaviour: &mut Behaviour,
    call: &str,
) -> Result<Vec<Signal>, String> {
    let (path, properties) = find_call(tree, audio_gateway, call)
        .ok_or_else(|| format!("there is no Call at {call}"))?;
    let state = call_state(&properties);
    if state != "incoming" {
        return Err(format!("the Call at {call} is {state}, not ringing"));
    }
    let mut signals = set_call_state(tree, &path, "active");
    if behaviour.earbuds.connected {
        // Answered on the earbuds: they are the active device, and the
        // audio goes to them.
        behaviour.earbuds.active = true;
    } else {
        signals.extend(open_audio_link(tree, audio_gateway, behaviour));
    }
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
