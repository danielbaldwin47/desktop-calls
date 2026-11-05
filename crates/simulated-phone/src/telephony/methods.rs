//! Answering method calls on the Telephony service, as PipeWire does.

use super::call_waiting::{
    after_hang_up, hold_and_answer, hung_up_call, release_and_answer, swap_calls,
};
use super::phone::{calls, set_volume};
use super::tree::{
    call_state, end_call, find_call, set_call_state, set_transport, transport_state,
};
use super::{
    AUDIO_GATEWAY, AUDIO_GATEWAY_TRANSPORT, Behaviour, CALL, CME, CME_25_MESSAGE, Dialling,
    INVALID_FORMAT, INVALID_FORMAT_MESSAGE, INVALID_STATE, INVALID_STATE_MESSAGE, MANAGER_PATH,
    OFONO_MANAGER, Quirk, Signal, VOICE_CALL, VOICE_CALL_MANAGER,
};
use crate::objects::{MethodCall, ObjectTree, PROPERTIES, Reply};
use std::collections::HashMap;
use std::time::Duration;
use zbus::{Message, OwnedObjectPath, OwnedVariant, Str};

/// What the service does about a method call: the reply, the signals that
/// follow it, and what the Phone does later.
pub struct Answer {
    pub reply: Reply,
    pub signals: Vec<Signal>,
    pub later: Option<ActivateLater>,
}

/// The Phone acts on an `Activate` to the AudioGateway at `audio_gateway`
/// once `after` has passed: [`activate_later`] does it.
pub struct ActivateLater {
    pub after: Duration,
    pub audio_gateway: OwnedObjectPath,
}

/// Answers a method call on the Telephony service, changing the tree as the
/// real service would. The signals go out after the reply. A quirk that
/// shows only once is used up here, or for `Activate`, when the Phone acts.
pub fn answer(
    tree: &mut ObjectTree,
    audio_gateway: Option<&OwnedObjectPath>,
    behaviour: &mut Behaviour,
    dialling: &mut Dialling,
    call: &MethodCall,
    message: &Message,
) -> Answer {
    let now = |(reply, signals): (Reply, Vec<Signal>)| Answer {
        reply,
        signals,
        later: None,
    };
    match tree.answer(call, message) {
        Reply::Unhandled => {}
        reply => return now((reply, Vec::new())),
    }
    // The AudioGateway the call is addressed to, if it is to the AudioGateway
    // itself or to one of its Calls.
    let to_audio_gateway = audio_gateway.filter(|gateway| gateway.as_str() == call.path);
    let to_call = audio_gateway.filter(|gateway| {
        call.path
            .strip_prefix(gateway.as_str())
            .is_some_and(|rest| rest.starts_with("/call"))
    });
    let answered = match (
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
                    .filter(|(_, properties)| {
                        !(behaviour
                            .quirks
                            .contains(&Quirk::GetCallsEmptyWhileDialling)
                            && matches!(call_state(properties).as_str(), "dialing" | "alerting"))
                    })
                    .map(|(path, properties)| (path, properties.into_iter().collect()))
                    .collect::<HashMap<OwnedObjectPath, HashMap<String, OwnedVariant>>>(),
            ),
            Vec::new(),
        ),
        (PROPERTIES, "Set", Some(_), _) => set_property(tree, call, message),
        (AUDIO_GATEWAY | VOICE_CALL_MANAGER, "Dial", Some(_), _) => {
            let reply = dial(dialling, message);
            if matches!(reply, Reply::Body(_)) {
                behaviour.earbuds.desktop_takes_over(&behaviour.quirks);
            }
            (reply, Vec::new())
        }
        (AUDIO_GATEWAY | VOICE_CALL_MANAGER, "SendTones", Some(gateway), _) => {
            (send_tones(tree, gateway, message), Vec::new())
        }
        (AUDIO_GATEWAY | VOICE_CALL_MANAGER, "HoldAndAnswer", Some(gateway), _) => {
            hold_and_answer(tree, gateway, behaviour)
        }
        (AUDIO_GATEWAY | VOICE_CALL_MANAGER, "ReleaseAndAnswer", Some(gateway), _) => {
            release_and_answer(tree, gateway, behaviour)
        }
        (AUDIO_GATEWAY | VOICE_CALL_MANAGER, "SwapCalls", Some(gateway), _) => {
            swap_calls(tree, gateway, behaviour)
        }
        (AUDIO_GATEWAY_TRANSPORT, "Activate", Some(gateway), _)
            if !behaviour.activate_delay.is_zero() =>
        {
            return Answer {
                reply: Reply::body(()),
                signals: Vec::new(),
                later: Some(ActivateLater {
                    after: behaviour.activate_delay,
                    audio_gateway: gateway.clone(),
                }),
            };
        }
        (AUDIO_GATEWAY_TRANSPORT, "Activate", Some(gateway), _) => {
            (Reply::body(()), activate(tree, gateway, behaviour))
        }
        (CALL | VOICE_CALL, "Answer", _, Some(gateway)) => {
            answer_call(tree, gateway, behaviour, call)
        }
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
    };
    now(answered)
}

/// The Phone acts on an `Activate` it was sent earlier, if the AudioGateway
/// it went to is still there. Returns the signals to emit.
pub fn activate_later(
    tree: &mut ObjectTree,
    audio_gateway: Option<&OwnedObjectPath>,
    behaviour: &mut Behaviour,
    later: &ActivateLater,
) -> Vec<Signal> {
    if audio_gateway != Some(&later.audio_gateway) {
        return Vec::new();
    }
    activate(tree, &later.audio_gateway, behaviour)
}

/// `Properties.Set` on an AudioGateway: `RejectSCO`, which emits nothing,
/// and the two volumes, which report the new value.
fn set_property(
    tree: &mut ObjectTree,
    call: &MethodCall,
    message: &Message,
) -> (Reply, Vec<Signal>) {
    let invalid = |text: String| {
        (
            Reply::Error("org.freedesktop.DBus.Error.InvalidArgs", text),
            Vec::new(),
        )
    };
    let Ok((interface, name, value)) = message
        .body()
        .deserialize::<(String, String, OwnedVariant)>()
    else {
        return invalid("Invalid arguments".to_owned());
    };
    let Ok(path) = OwnedObjectPath::try_from(call.path.as_str()) else {
        return (Reply::unknown_method(call), Vec::new());
    };
    match (interface.as_str(), name.as_str()) {
        (AUDIO_GATEWAY_TRANSPORT, "RejectSCO") if bool::try_from(&value).is_ok() => {
            tree.set(&path, AUDIO_GATEWAY_TRANSPORT, "RejectSCO", value);
            (Reply::body(()), Vec::new())
        }
        (AUDIO_GATEWAY, "MicrophoneVolume" | "SpeakerVolume") => match u8::try_from(&value) {
            Ok(volume) => (Reply::body(()), set_volume(tree, &path, &name, volume)),
            Err(_) => invalid(format!(
                "{name} is a byte ('y'), not '{}'; PipeWire would read it unchecked",
                value.value_signature()
            )),
        },
        _ => invalid(format!("No such writable property '{name}'")),
    }
}

/// `SendTones` on an AudioGateway.
fn send_tones(tree: &ObjectTree, audio_gateway: &OwnedObjectPath, message: &Message) -> Reply {
    let Ok(tones) = message.body().deserialize::<String>() else {
        return Reply::Error(
            "org.freedesktop.DBus.Error.InvalidArgs",
            "Invalid arguments".to_owned(),
        );
    };
    if !tones
        .chars()
        .all(|tone| tone.is_ascii_digit() || matches!(tone, 'A'..='D' | '*' | '#'))
    {
        return Reply::Error(INVALID_FORMAT, INVALID_FORMAT_MESSAGE.to_owned());
    }
    let active = calls(tree, audio_gateway.as_str())
        .iter()
        .any(|(_, properties)| call_state(properties) == "active");
    if !active {
        return invalid_state();
    }
    if tones.chars().count() != 1 {
        return Reply::Error(CME, CME_25_MESSAGE.to_owned());
    }
    Reply::body(())
}

/// `Dial`: checks the number as PipeWire does, then fails as a test asked,
/// or accepts it.
pub(super) fn dial(dialling: &mut Dialling, message: &Message) -> Reply {
    let Ok((number,)) = message.body().deserialize::<(String,)>() else {
        return Reply::Error(
            "org.freedesktop.DBus.Error.InvalidArgs",
            "Invalid arguments".to_owned(),
        );
    };
    let accepted = number.chars().all(|character| {
        character.is_ascii_digit() || matches!(character, 'A'..='D' | '#' | '*' | '+' | ',')
    });
    if !accepted || !(1..=80).contains(&number.len()) {
        return Reply::Error(INVALID_FORMAT, INVALID_FORMAT_MESSAGE.to_owned());
    }
    if let Some((name, message)) = dialling.refusal.take() {
        return Reply::Error(name, message);
    }
    dialling.number = Some(number);
    Reply::body(())
}

pub(super) fn rejects_sco(tree: &ObjectTree, audio_gateway: &OwnedObjectPath) -> bool {
    tree.get(audio_gateway, AUDIO_GATEWAY_TRANSPORT, "RejectSCO")
        .and_then(|value| bool::try_from(value).ok())
        .unwrap_or(true)
}

/// The transport reports `first` (its codec), then `pending`, then `active`.
pub(super) fn bring_audio_up(
    tree: &mut ObjectTree,
    audio_gateway: &OwnedObjectPath,
    first: Vec<(&str, OwnedVariant)>,
) -> Vec<Signal> {
    let mut signals = set_transport(tree, audio_gateway, first);
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

/// `Activate`: brings the Call audio to the Desktop, unless SCO is rejected,
/// no Call is live, the earbuds are the active device, or the Phone ignores
/// it as the Pixel once did. PipeWire replies before the Phone acts, so a
/// refusal looks the same as nothing happening.
pub(super) fn activate(
    tree: &mut ObjectTree,
    audio_gateway: &OwnedObjectPath,
    behaviour: &mut Behaviour,
) -> Vec<Signal> {
    if behaviour.earbuds.active
        || behaviour.quirks.contains(&Quirk::ActivateIgnored)
        || behaviour.quirks.remove(&Quirk::ActivateIgnoredOnce)
    {
        return Vec::new();
    }
    let rejected = rejects_sco(tree, audio_gateway);
    let live = calls(tree, audio_gateway.as_str())
        .iter()
        .any(|(_, properties)| call_state(properties) != "disconnected");
    if rejected || !live || transport_state(tree, audio_gateway).as_deref() != Some("idle") {
        return Vec::new();
    }
    bring_audio_up(
        tree,
        audio_gateway,
        vec![
            ("Codec", OwnedVariant::from(behaviour.codec)),
            ("RejectSCO", OwnedVariant::from(false)),
        ],
    )
}

fn answer_call(
    tree: &mut ObjectTree,
    audio_gateway: &OwnedObjectPath,
    behaviour: &mut Behaviour,
    call: &MethodCall,
) -> (Reply, Vec<Signal>) {
    let Some((path, properties)) = find_call(tree, audio_gateway, &call.path) else {
        return (Reply::unknown_method(call), Vec::new());
    };
    if call_state(&properties) != "incoming" {
        return (invalid_state(), Vec::new());
    }
    behaviour.earbuds.desktop_takes_over(&behaviour.quirks);
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
    let state = call_state(&properties);
    if !matches!(
        state.as_str(),
        "incoming" | "waiting" | "active" | "dialing" | "alerting" | "held"
    ) {
        return (invalid_state(), Vec::new());
    }
    if behaviour.quirks.contains(&Quirk::HangupIgnored) {
        return (Reply::body(()), Vec::new());
    }
    let ended = hung_up_call(tree, audio_gateway, path, &state);
    let mut signals = end_call(tree, audio_gateway, &ended);
    signals.extend(after_hang_up(tree, audio_gateway, &state));
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

fn invalid_state() -> Reply {
    Reply::Error(INVALID_STATE, INVALID_STATE_MESSAGE.to_owned())
}
