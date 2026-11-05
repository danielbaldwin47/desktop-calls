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
//! - A Call that rings during a live one is `waiting`, and `HoldAndAnswer`,
//!   `ReleaseAndAnswer` and `SwapCalls` act on the Calls as PipeWire's
//!   `AT+CHLD` does (`telephony/call_waiting.rs`).

use std::collections::{BTreeSet, HashMap};
use std::time::Duration;

use zbus::{Connection, OwnedObjectPath, OwnedVariant, Str};

use crate::objects::{
    INTROSPECTABLE, Interfaces, OBJECT_MANAGER, ObjectTree, PROPERTIES, Properties,
    interfaces_added_body,
};

mod call_waiting;
pub mod hands_free;
mod methods;
mod phone;
mod tree;

pub use methods::{ActivateLater, activate_later, answer};
pub use phone::{
    Restart, alert, answer_on_phone, callee_answers, calls, dial_on_phone, end_call_on_phone,
    identify_caller, number_from_caller_id, reregister_call, restart, ring, set_microphone_volume,
    unannounce_call,
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
    /// BlueZ refuses `DisconnectProfile` with `org.bluez.Error.Failed`, and
    /// hands-free stays up: the Desktop can't move the audio to the Phone.
    DisconnectProfileRefused,
    /// A ringing Call appears with no `LineIdentification`, and the caller's
    /// number follows [`LATE_LINE_IDENTIFICATION`] later in a
    /// `PropertiesChanged`, as PipeWire 1.6.9 reports a Phone without
    /// `+CLCC`: it registers the Call on `+CIEV: callsetup,1`, and sets the
    /// number from the `+CLIP` that follows the first `RING`, while the Call
    /// is still ringing (`backend-native.c`). A withheld caller's `+CLIP`
    /// fails PipeWire's parse, so that Call never gets a number.
    LateLineIdentification,
    /// `HoldAndAnswer` and `SwapCalls` fail with `InvalidState` whatever the
    /// Calls' states, as a network or Phone without call hold refuses
    /// `AT+CHLD=2`: both Calls stay as they were.
    HoldRefused,
}

/// How long [`Quirk::SlowPropertyWrites`] holds each reply.
pub const SLOW_REPLY: Duration = Duration::from_secs(1);

/// How long after a ringing Call appears [`Quirk::LateLineIdentification`]
/// reports its number. A real Phone's `+CLIP` follows within a `RING` or
/// two; a second makes the change easy to see, and is still short against
/// a Call that rings for 20 s or more.
pub const LATE_LINE_IDENTIFICATION: Duration = Duration::from_secs(1);

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
            "disconnect-profile-refused" => Some(Self::DisconnectProfileRefused),
            "late-line-identification" => Some(Self::LateLineIdentification),
            "hold-refused" => Some(Self::HoldRefused),
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
    /// `org.ofono.Manager.ModemAdded` for the AudioGateway at `modem`, with
    /// no properties, as PipeWire sends it after the AudioGateway's
    /// `InterfacesAdded`.
    ModemAdded { modem: OwnedObjectPath },
    /// `org.ofono.Manager.ModemRemoved` for the AudioGateway at `modem`.
    ModemRemoved { modem: OwnedObjectPath },
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
            Self::ModemAdded { .. } | Self::ModemRemoved { .. } => {
                self.emit_modem(connection).await
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

    /// Emits `ModemAdded` or `ModemRemoved` on the manager; other signals
    /// aren't the manager's own.
    async fn emit_modem(&self, connection: &Connection) -> zbus::Result<()> {
        match self {
            Self::ModemAdded { modem } => {
                let no_properties = HashMap::<&str, OwnedVariant>::new();
                connection
                    .emit_signal(
                        None::<&str>,
                        MANAGER_PATH,
                        OFONO_MANAGER,
                        "ModemAdded",
                        &(modem, no_properties),
                    )
                    .await
            }
            Self::ModemRemoved { modem } => {
                connection
                    .emit_signal(
                        None::<&str>,
                        MANAGER_PATH,
                        OFONO_MANAGER,
                        "ModemRemoved",
                        &(modem,),
                    )
                    .await
            }
            _ => Ok(()),
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
