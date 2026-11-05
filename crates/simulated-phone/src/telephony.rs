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
//!   `active`, as the Pixel does. PipeWire replies to `Activate` before the
//!   Phone acts, and tests can make the Phone take a while to act
//!   (`SetActivateDelay`), as the Pixel took 62 to 313 ms.
//! - `Dial`, on `AudioGateway1` or `org.ofono.VoiceCallManager`, checks the
//!   number as PipeWire does (`0`-`9`, `A`-`D`, `#`, `*`, `+` and `,`, 1 to
//!   80 of them, else `InvalidFormat` "Invalid phone number or tones"), and
//!   replies once the Phone answers `ATD`, with no Call yet. The Pixel then
//!   reports the Call straight in `alerting`, and while `RejectSCO` is false
//!   brings the audio up itself, with no `Activate`
//!   (`docs/research/2026-11-04-place-a-call.md`). A Phone error is
//!   `org.pipewire.Telephony.Error.CME` with PipeWire's description of the
//!   code as its message.
//! - `SpeakerVolume` and `MicrophoneVolume` are bytes, and setting one
//!   reports the new value with `PropertiesChanged`. PipeWire reads the
//!   value without checking its type (a wrong one crashed `WirePlumber`), so
//!   anything but a byte is refused here, to catch it.
//! - `SendTones` needs an `active` Call (`InvalidState`) and only `0-9`,
//!   `A-D`, `*` and `#` (`InvalidFormat`), as PipeWire checks. PipeWire sends
//!   the whole string in one `AT+VTS`, and Android accepts exactly one tone
//!   per `AT+VTS`, answering anything longer with `+CME ERROR: 25`, so a
//!   longer string fails with `CME` here too.

use std::collections::{BTreeSet, HashMap};
use std::time::Duration;

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
const INVALID_FORMAT: &str = "org.pipewire.Telephony.Error.InvalidFormat";
const INVALID_FORMAT_MESSAGE: &str = "Invalid phone number or tones";
const CME: &str = "org.pipewire.Telephony.Error.CME";
/// PipeWire's text for `+CME ERROR: 25`.
const CME_25_MESSAGE: &str = "invalid characters in text string";

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
    /// `GetCalls` leaves out a Call that is `dialing` or `alerting`.
    GetCallsEmptyWhileDialling,
    /// `Properties.Set` on an AudioGateway is carried out and recorded at
    /// once, but answered only [`SLOW_REPLY`] later, as a busy `WirePlumber`
    /// may. Tests use it to see what the app shows before the reply.
    SlowPropertyWrites,
    /// `Activate` replies, and the audio stays where it is, every time: a
    /// Phone that keeps the audio, as one with earbuds in may.
    ActivateIgnored,
    /// `Activate` replies, and the audio stays where it is, the next time
    /// only, as the Pixel did once in 7 tries (ADR-0005).
    ActivateIgnoredOnce,
    /// Answering on the Desktop leaves the earbuds the active device, so the
    /// audio stays on them, as an Android call-establish audio policy of
    /// `POLICY_NOT_ALLOWED` would.
    EarbudsKeepAudio,
}

/// How long [`Quirk::SlowPropertyWrites`] holds each reply.
pub const SLOW_REPLY: Duration = Duration::from_secs(1);

impl Quirk {
    /// The quirk with this control-interface name.
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "call1-hangup-missing" => Some(Self::Call1HangupMissing),
            "hangup-too-few-arguments" => Some(Self::HangupTooFewArguments),
            "get-calls-empty-while-dialling" => Some(Self::GetCallsEmptyWhileDialling),
            "slow-property-writes" => Some(Self::SlowPropertyWrites),
            "activate-ignored" => Some(Self::ActivateIgnored),
            "activate-ignored-once" => Some(Self::ActivateIgnoredOnce),
            "earbuds-keep-audio" => Some(Self::EarbudsKeepAudio),
            _ => None,
        }
    }
}

/// How the Phone behaves where Phones differ.
pub struct Behaviour {
    /// The codec id the audio link negotiates: 1 CVSD, 2 mSBC, 3 LC3-SWB.
    pub codec: u8,
    /// The hands-free microphone gain a new AudioGateway reports.
    pub microphone_volume: u8,
    pub quirks: BTreeSet<Quirk>,
    pub earbuds: Earbuds,
    /// How long the Phone takes after `Activate` to act on it. PipeWire
    /// replies at once either way.
    pub activate_delay: Duration,
}

/// Earbuds connected to the Phone beside the Desktop: a second hands-free
/// unit. Android sends Call audio only to its one active hands-free device.
/// AOSP's `HeadsetService.isScoAcceptable` refuses audio for any other
/// device, and `HeadsetSystemInterface.answerCall` makes the device that
/// answers the active one. Newly connected earbuds becoming active is
/// inferred, not read in the source.
#[derive(Default)]
pub struct Earbuds {
    pub connected: bool,
    /// The earbuds are the active device, so the Phone keeps the Call's
    /// audio on them and refuses it to the Desktop.
    pub active: bool,
}

impl Earbuds {
    /// The earbuds connect, and become the active device, or disconnect.
    pub fn set_connected(&mut self, connected: bool) {
        self.connected = connected;
        self.active = connected;
    }

    /// The Desktop answers or dials: it becomes the active device, unless
    /// the Phone keeps the earbuds active.
    fn desktop_takes_over(&mut self, quirks: &BTreeSet<Quirk>) {
        if !quirks.contains(&Quirk::EarbudsKeepAudio) {
            self.active = false;
        }
    }
}

impl Default for Behaviour {
    /// The Pixel 9a: LC3-SWB, no quirks.
    fn default() -> Self {
        Self {
            codec: 3,
            microphone_volume: 15,
            quirks: BTreeSet::new(),
            earbuds: Earbuds::default(),
            activate_delay: Duration::ZERO,
        }
    }
}

/// What the Phone does with the Desktop's `Dial`.
#[derive(Default)]
pub struct Dialling {
    /// The Telephony error, name and message, the next `Dial` fails with.
    pub refusal: Option<(&'static str, String)>,
    /// The number of the last `Dial` the Phone accepted, until its Call
    /// appears.
    pub number: Option<String>,
}

/// The Telephony error PipeWire replies to `Dial` with for `name`:
/// `"invalid-format"`, `"invalid-state"`, `"not-supported"`, `"in-progress"`,
/// `"failed"`, or `"cme:N"` for the Phone's `+CME ERROR: N`.
pub fn dial_refusal(name: &str) -> Option<(&'static str, String)> {
    let refusal = match name {
        "invalid-format" => (INVALID_FORMAT, INVALID_FORMAT_MESSAGE),
        "invalid-state" => (INVALID_STATE, INVALID_STATE_MESSAGE),
        "not-supported" => (
            "org.pipewire.Telephony.Error.NotSupported",
            "Method is not supported on this Audio Gateway",
        ),
        "in-progress" => (
            "org.pipewire.Telephony.Error.InProgress",
            "Command already in progress",
        ),
        "failed" => ("org.pipewire.Telephony.Error.Failed", "Method call failed"),
        _ => {
            let code: u8 = name.strip_prefix("cme:")?.parse().ok()?;
            ("org.pipewire.Telephony.Error.CME", cme_description(code))
        }
    };
    Some((refusal.0, refusal.1.to_owned()))
}

/// PipeWire 1.6.9's description of `+CME ERROR: code`, which is all a
/// client sees of it.
fn cme_description(code: u8) -> &'static str {
    match code {
        0 => "AG failure",
        1 => "no connection to phone",
        3 => "operation not allowed",
        4 => "operation not supported",
        5 => "PH-SIM PIN required",
        10 => "SIM not inserted",
        11 => "SIM PIN required",
        12 => "SIM PUK required",
        13 => "SIM failure",
        14 => "SIM busy",
        16 => "incorrect password",
        17 => "SIM PIN2 required",
        18 => "SIM PUK2 required",
        20 => "memory full",
        21 => "invalid index",
        23 => "memory failure",
        24 => "text string too long",
        25 => "invalid characters in text string",
        26 => "dial string too long",
        27 => "invalid characters in dial string",
        30 => "no network service",
        31 => "network Timeout",
        32 => "network not allowed - Emergency calls only",
        _ => "Unknown CME error",
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
    behaviour: &Behaviour,
) -> Interfaces {
    let mut gateway = Properties::new();
    gateway.insert(
        "Address".into(),
        OwnedVariant::from(Str::from(address.to_owned())),
    );
    gateway.insert("SpeakerVolume".into(), OwnedVariant::from(15_u8));
    gateway.insert(
        "MicrophoneVolume".into(),
        OwnedVariant::from(behaviour.microphone_volume),
    );
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

fn set_volume(
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
fn dial(dialling: &mut Dialling, message: &Message) -> Reply {
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

fn rejects_sco(tree: &ObjectTree, audio_gateway: &OwnedObjectPath) -> bool {
    tree.get(audio_gateway, AUDIO_GATEWAY_TRANSPORT, "RejectSCO")
        .and_then(|value| bool::try_from(value).ok())
        .unwrap_or(true)
}

/// The transport reports `first` (its codec), then `pending`, then `active`.
fn bring_audio_up(
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
fn activate(
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
