//! Pairing the Phone with the Desktop through BlueZ, from either side, and
//! what BlueZ needs for it: discovery, the adapter's discoverable and
//! pairable switches, the Desktop's pairing agent, and the Phone's choice to
//! share Calls. Shaped after BlueZ 5.87's documented API
//! (`org.bluez.Adapter1`, `Device1`, `AgentManager1`, `Agent1`); none of it is
//! measured against the real Pixel 9a yet, so the times are inferred.
//!
//! - With `--unpaired`, the Phone is nearby and not paired. Discovery finds
//!   it [`FOUND_AFTER`] after `StartDiscovery`, with any headphones nearby
//!   (`--headphones`), each as `InterfacesAdded` with `RSSI`.
//! - `Device1.Pair` connects ([`Connected`] goes true), then asks the
//!   registered agent's `RequestConfirmation` with a six-digit passkey. The
//!   Phone's side always accepts. Confirmed: `Paired` and `Bonded` go true
//!   and `Pair` returns. Refused: the connection drops and `Pair` fails with
//!   `org.bluez.Error.AuthenticationRejected`. With no agent it fails with
//!   `org.bluez.Error.AuthenticationFailed`.
//! - The control interface's `PairFromPhone` is the person holding the Phone
//!   tapping the Desktop on its "Pair new device" screen. It needs the
//!   adapter `Discoverable`. The agent is asked the same way; once
//!   confirmed, the Phone connects by itself, as Phones do.
//! - Whether the Phone shares Calls: `shared` brings hands-free up on every
//!   connect; `when-asked` connects media alone, and brings hands-free up
//!   only for `ConnectProfile`; `refused` fails that `ConnectProfile` with
//!   `org.bluez.Error.Failed` `"br-connection-refused"` (inferred).
//! - With `--old-phone`, another Phone is paired too, and out of range:
//!   connects to it page and time out. The control interface's
//!   `ConnectOtherPhone` brings it into range, connected with hands-free
//!   and its own AudioGateway, beside the Phone.
//! - `Adapter1.RemoveDevice` unpairs a device that isn't connected, as the
//!   user does in Bluetooth settings: `InterfacesRemoved`. A connected one
//!   fails `org.bluez.Error.Failed`, because the simulation doesn't model
//!   BlueZ disconnecting it first: the test disconnects it, then removes it.
//!
//! [`Connected`]: https://git.kernel.org/pub/scm/bluetooth/bluez.git/tree/doc/org.bluez.Device.rst

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use zbus::{Connection, Message, OwnedObjectPath, OwnedVariant};

use crate::objects::{Interfaces, MethodCall, PROPERTIES, Reply};
use crate::telephony::Signal;
use crate::{SharedWorld, TelephonyConnection, World, bluez, lock, range};

/// How long discovery takes to find each device nearby.
const FOUND_AFTER: Duration = Duration::from_millis(300);
/// How long the Phone takes to answer the page that starts pairing.
const PAGE_ANSWERED: Duration = Duration::from_millis(200);
/// How long the Phone takes, once paired from its own screen, to connect.
const CONNECT_AFTER_PAIRING: Duration = Duration::from_millis(600);
/// How long the Phone takes to refuse or bring up hands-free when asked.
const HANDS_FREE_ANSWER: Duration = Duration::from_millis(300);

const ADAPTER: &str = "org.bluez.Adapter1";
const AGENT_MANAGER: &str = "org.bluez.AgentManager1";
const AGENT: &str = "org.bluez.Agent1";
const HEADPHONES_ADDRESS: &str = "00:1B:66:00:00:02";
/// The other Phone's address. Its first octet is 00, below any address the
/// harness gives the Phone (locally administered, so at least 02): it sorts
/// first by device path, which is the order the app follows when the user
/// has chosen none.
pub const OLD_PHONE_ADDRESS: &str = "00:A4:93:00:00:03";

/// Who and what is nearby, from the command line.
#[derive(clap::Args)]
pub struct Nearby {
    /// The Phone starts nearby and not paired: a fresh install.
    #[arg(long)]
    unpaired: bool,
    /// Headphones with this name are nearby, not paired.
    #[arg(long)]
    headphones: Option<String>,
    /// Another Phone with this name is paired too, and out of range.
    #[arg(long)]
    old_phone: Option<String>,
}

/// Whether the Phone shares Calls with the Desktop: its "Phone calls"
/// switch for the Desktop, and whether it turns hands-free on by itself.
#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub enum HandsFree {
    #[default]
    Shared,
    WhenAsked,
    Refused,
}

impl HandsFree {
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "shared" => Some(Self::Shared),
            "when-asked" => Some(Self::WhenAsked),
            "refused" => Some(Self::Refused),
            _ => None,
        }
    }
}

/// The Desktop's registered pairing agent.
struct Agent {
    owner: String,
    path: OwnedObjectPath,
}

/// The pairing state of the Simulated BlueZ.
#[derive(Default)]
pub struct Pairing {
    agent: Option<Agent>,
    pub hands_free: HandsFree,
    /// Devices discovery finds, not in BlueZ's objects yet.
    undiscovered: Vec<(OwnedObjectPath, Interfaces)>,
    /// Each pairing's passkey and how it went.
    requests: Vec<(u32, String)>,
    /// Paired Phones other than the Phone, never in range.
    others: Vec<OwnedObjectPath>,
}

impl Pairing {
    /// Sets BlueZ's objects up for `nearby`, given the Phone at `phone`.
    pub fn new(
        tree: &mut crate::objects::ObjectTree,
        phone: &OwnedObjectPath,
        nearby: &Nearby,
        range: &mut range::Range,
    ) -> zbus::Result<Self> {
        let mut pairing = Self::default();
        if nearby.unpaired
            && let Some(mut interfaces) = tree.remove(phone)
        {
            if let Some(device) = interfaces.get_mut(bluez::DEVICE) {
                for name in ["Paired", "Bonded", "Trusted"] {
                    device.insert(name.to_owned(), OwnedVariant::from(false));
                }
                device.insert("RSSI".to_owned(), OwnedVariant::from(-52_i16));
            }
            range.reachable = true;
            pairing.undiscovered.push((phone.clone(), interfaces));
        }
        if let Some(name) = &nearby.headphones {
            let path = bluez::device_path(HEADPHONES_ADDRESS)?;
            // Major class audio, minor class headphones.
            let interfaces = bluez::other_device(name, HEADPHONES_ADDRESS, 0x24_0418, false)?;
            pairing.undiscovered.push((path, interfaces));
        }
        if let Some(name) = &nearby.old_phone {
            let path = bluez::device_path(OLD_PHONE_ADDRESS)?;
            bluez::insert_device(
                tree,
                &path,
                bluez::other_device(name, OLD_PHONE_ADDRESS, 5_915_148, true)?,
            );
            pairing.others.push(path);
        }
        Ok(pairing)
    }

    /// Each pairing's passkey and how it went: `waiting`, `confirmed` or
    /// `rejected`.
    pub fn requests(&self) -> Vec<(u32, String)> {
        self.requests.clone()
    }

    /// Whether `path` is another paired Phone, never in range.
    pub fn is_other_phone(&self, path: &str) -> bool {
        self.others.iter().any(|other| other.as_str() == path)
    }
}

/// Whether `call` is one this module answers.
pub fn handles(call: &MethodCall) -> bool {
    match call.interface.as_str() {
        AGENT_MANAGER => true,
        ADAPTER => matches!(
            call.member.as_str(),
            "StartDiscovery" | "StopDiscovery" | "SetDiscoveryFilter" | "RemoveDevice"
        ),
        bluez::DEVICE => call.member == "Pair",
        PROPERTIES => call.member == "Set",
        _ => false,
    }
}

/// Answers `call` on a thread of its own, so BlueZ keeps answering
/// meanwhile, as the module documentation says.
pub fn answer(world: SharedWorld, system: Connection, call: MethodCall, message: Message) {
    std::thread::spawn(move || {
        let acted = match (call.interface.as_str(), call.member.as_str()) {
            (bluez::DEVICE, "Pair") => pair(&world, &system, &call, &message),
            (ADAPTER, "StartDiscovery") => start_discovery(&world, &system, &message),
            _ => {
                let (reply, signals) = answer_now(&mut lock(&world), &call, &message);
                zbus::block_on(async {
                    reply.send(&system, &message.header()).await?;
                    for signal in signals {
                        signal.emit(&system).await?;
                    }
                    Ok(())
                })
            }
        };
        if let Err(error) = acted {
            eprintln!("simulated-phone: {} failed: {error}", call.member);
        }
    });
}

/// The calls answered at once.
fn answer_now(world: &mut World, call: &MethodCall, message: &Message) -> (Reply, Vec<Signal>) {
    let sender = message
        .header()
        .sender()
        .map(ToString::to_string)
        .unwrap_or_default();
    let ok = || Reply::body(());
    match call.member.as_str() {
        "RegisterAgent" => match message.body().deserialize::<(OwnedObjectPath, String)>() {
            Ok(_) if world.pairing.agent.is_some() => (
                Reply::Error("org.bluez.Error.AlreadyExists", "Already Exists".to_owned()),
                Vec::new(),
            ),
            Ok((path, _capability)) => {
                world.pairing.agent = Some(Agent {
                    owner: sender,
                    path,
                });
                (ok(), Vec::new())
            }
            Err(error) => (invalid(&error.to_string()), Vec::new()),
        },
        "UnregisterAgent" => {
            if world
                .pairing
                .agent
                .as_ref()
                .is_some_and(|agent| agent.owner == sender)
            {
                world.pairing.agent = None;
                (ok(), Vec::new())
            } else {
                (
                    Reply::Error("org.bluez.Error.DoesNotExist", "Does Not Exist".to_owned()),
                    Vec::new(),
                )
            }
        }
        "RequestDefaultAgent" | "SetDiscoveryFilter" => (ok(), Vec::new()),
        "StopDiscovery" => match set(world, ADAPTER_PATH, ADAPTER, "Discovering", false.into()) {
            Some(signal) => (ok(), vec![signal]),
            None => (
                Reply::Error("org.bluez.Error.Failed", "No discovery started".to_owned()),
                Vec::new(),
            ),
        },
        "RemoveDevice" => remove_device(world, message),
        "Set" => set_property(world, call, message),
        _ => (Reply::unknown_method(call), Vec::new()),
    }
}

const ADAPTER_PATH: &str = "/org/bluez/hci0";

/// `Adapter1.RemoveDevice`: unpairs the device at the path it is given.
fn remove_device(world: &mut World, message: &Message) -> (Reply, Vec<Signal>) {
    let Ok(device) = message.body().deserialize::<OwnedObjectPath>() else {
        return (invalid("RemoveDevice takes (o)"), Vec::new());
    };
    let connected = world
        .bluez
        .get(&device, bluez::DEVICE, "Connected")
        .and_then(|connected| bool::try_from(connected.clone()).ok());
    match connected {
        None => (
            Reply::Error("org.bluez.Error.DoesNotExist", "Does Not Exist".to_owned()),
            Vec::new(),
        ),
        Some(true) => (
            Reply::Error(
                "org.bluez.Error.Failed",
                "The simulation removes only a device that isn't connected".to_owned(),
            ),
            Vec::new(),
        ),
        Some(false) => {
            let removed = world.bluez.remove(&device).unwrap_or_default();
            world.pairing.others.retain(|other| *other != device);
            let interfaces = [
                bluez::DEVICE,
                "org.bluez.Network1",
                "org.bluez.MediaControl1",
                bluez::BATTERY,
            ]
            .into_iter()
            .filter(|name| removed.contains_key(*name))
            .collect();
            let signal = Signal::InterfacesRemoved {
                manager: OwnedObjectPath::try_from("/").unwrap_or_default(),
                object: device,
                interfaces,
            };
            (Reply::body(()), vec![signal])
        }
    }
}

/// `Properties.Set` for what the Desktop sets: the adapter's
/// `Discoverable`, `Pairable`, `DiscoverableTimeout` and `Alias`, and a
/// device's `Trusted`.
fn set_property(world: &mut World, call: &MethodCall, message: &Message) -> (Reply, Vec<Signal>) {
    let Ok((interface, name, value)) = message
        .body()
        .deserialize::<(String, String, OwnedVariant)>()
    else {
        return (invalid("Set takes (s, s, v)"), Vec::new());
    };
    let writable = matches!(
        (interface.as_str(), name.as_str()),
        (
            ADAPTER,
            "Discoverable" | "Pairable" | "DiscoverableTimeout" | "Alias"
        ) | (bluez::DEVICE, "Trusted")
    );
    let Ok(path) = OwnedObjectPath::try_from(call.path.as_str()) else {
        return (invalid("bad path"), Vec::new());
    };
    if !writable || world.bluez.get(&path, &interface, &name).is_none() {
        return (
            Reply::Error(
                "org.freedesktop.DBus.Error.PropertyReadOnly",
                format!("Property '{name}' is not writable"),
            ),
            Vec::new(),
        );
    }
    let interface = if interface == ADAPTER {
        ADAPTER
    } else {
        bluez::DEVICE
    };
    let signals = set(world, call.path.as_str(), interface, &name, value)
        .into_iter()
        .collect();
    (Reply::body(()), signals)
}

/// Sets one property; the signal saying so, if it changed.
fn set(
    world: &mut World,
    path: &str,
    interface: &'static str,
    name: &str,
    value: OwnedVariant,
) -> Option<Signal> {
    let path = OwnedObjectPath::try_from(path).ok()?;
    let copy = value.try_clone().ok()?;
    world
        .bluez
        .set(&path, interface, name, value)
        .then(|| Signal::PropertiesChanged {
            path,
            interface,
            changed: [(name.to_owned(), copy)].into(),
        })
}

fn invalid(message: &str) -> Reply {
    Reply::Error("org.bluez.Error.InvalidArguments", message.to_owned())
}

/// `StartDiscovery`: replies, then finds each nearby device in turn.
fn start_discovery(
    world: &SharedWorld,
    system: &Connection,
    message: &Message,
) -> zbus::Result<()> {
    let signal = set(
        &mut lock(world),
        ADAPTER_PATH,
        ADAPTER,
        "Discovering",
        true.into(),
    );
    zbus::block_on(async {
        Reply::body(()).send(system, &message.header()).await?;
        if let Some(signal) = signal {
            signal.emit(system).await?;
        }
        Ok::<_, zbus::Error>(())
    })?;
    loop {
        std::thread::sleep(FOUND_AFTER);
        let found = {
            let mut world = lock(world);
            if world.pairing.undiscovered.is_empty() {
                return Ok(());
            }
            let (path, interfaces) = world.pairing.undiscovered.remove(0);
            let signal = Signal::InterfacesAdded {
                manager: OwnedObjectPath::try_from("/")?,
                object: path.clone(),
                interfaces: copy_interfaces(&interfaces),
            };
            bluez::insert_device(&mut world.bluez, &path, interfaces);
            signal
        };
        zbus::block_on(found.emit(system))?;
    }
}

fn copy_interfaces(interfaces: &Interfaces) -> Interfaces {
    interfaces
        .iter()
        .map(|(name, properties)| {
            (
                name.clone(),
                properties
                    .iter()
                    .filter_map(|(key, value)| Some((key.clone(), value.try_clone().ok()?)))
                    .collect(),
            )
        })
        .collect()
}

/// `Device1.Pair` from the Desktop.
fn pair(
    world: &SharedWorld,
    system: &Connection,
    call: &MethodCall,
    message: &Message,
) -> zbus::Result<()> {
    let refusal = {
        let world = lock(world);
        let phone = bluez::device_path(&world.phone_address)?;
        if call.path != phone.as_str() {
            Some(Reply::Error(
                "org.bluez.Error.Failed",
                "br-connection-page-timeout".to_owned(),
            ))
        } else if paired(&world) && !world.range.forgot_pairing {
            Some(Reply::Error(
                "org.bluez.Error.AlreadyExists",
                "Already Exists".to_owned(),
            ))
        } else {
            None
        }
    };
    if let Some(reply) = refusal {
        return zbus::block_on(reply.send(system, &message.header()));
    }
    std::thread::sleep(PAGE_ANSWERED);
    let reply = match ask_agent(world, system)? {
        Some(true) => Reply::body(()),
        Some(false) => Reply::Error(
            "org.bluez.Error.AuthenticationRejected",
            "Authentication Rejected".to_owned(),
        ),
        None => Reply::Error(
            "org.bluez.Error.AuthenticationFailed",
            "Authentication Failed".to_owned(),
        ),
    };
    zbus::block_on(reply.send(system, &message.header()))
}

/// The person holding the Phone taps the Desktop on its "Pair new device"
/// screen: the agent is asked on a thread of its own. Returns the passkey.
pub fn pair_from_phone(
    world: &SharedWorld,
    system: &Connection,
    telephony: &TelephonyConnection,
) -> Result<u32, String> {
    {
        let world = lock(world);
        let discoverable = bluez::device_path(&world.phone_address)
            .ok()
            .and_then(|_| {
                let adapter = OwnedObjectPath::try_from(ADAPTER_PATH).ok()?;
                let value = world.bluez.get(&adapter, ADAPTER, "Discoverable")?;
                bool::try_from(value.clone()).ok()
            })
            .unwrap_or(false);
        if !discoverable {
            return Err("the Desktop isn't discoverable, so the Phone can't find it".to_owned());
        }
        if world.pairing.agent.is_none() {
            return Err("the Desktop has no pairing agent, so nothing would ask".to_owned());
        }
    }
    let code = passkey();
    let (world, system, telephony) = (world.clone(), system.clone(), telephony.clone());
    std::thread::spawn(move || {
        let connected = (|| {
            if ask_agent_with(&world, &system, code)? != Some(true) {
                return Ok(());
            }
            std::thread::sleep(CONNECT_AFTER_PAIRING);
            let change = connected_by_phone(&mut lock(&world))?;
            zbus::block_on(range::emit(&system, &telephony, change))
        })();
        if let Err(error) = connected {
            eprintln!("simulated-phone: pairing from the Phone failed: {error}");
        }
    });
    Ok(code)
}

/// Whether the Phone is paired, as BlueZ sees it.
fn paired(world: &World) -> bool {
    bluez::device_path(&world.phone_address)
        .ok()
        .and_then(|device| world.bluez.get(&device, bluez::DEVICE, "Paired").cloned())
        .and_then(|value| bool::try_from(value).ok())
        .unwrap_or(false)
}

fn ask_agent(world: &SharedWorld, system: &Connection) -> zbus::Result<Option<bool>> {
    ask_agent_with(world, system, passkey())
}

/// Connects the Phone and asks the agent whether `code` shows on both
/// screens: `None` with no agent. Confirmed, the Phone is paired;
/// otherwise the connection drops.
fn ask_agent_with(
    world: &SharedWorld,
    system: &Connection,
    code: u32,
) -> zbus::Result<Option<bool>> {
    let (agent, device, signal) = {
        let mut world = lock(world);
        let device = bluez::device_path(&world.phone_address)?;
        let Some(agent) = world
            .pairing
            .agent
            .as_ref()
            .map(|agent| (agent.owner.clone(), agent.path.clone()))
        else {
            return Ok(None);
        };
        world.pairing.requests.push((code, "waiting".to_owned()));
        let signal = set(
            &mut world,
            device.as_str(),
            bluez::DEVICE,
            "Connected",
            true.into(),
        );
        (agent, device, signal)
    };
    if let Some(signal) = signal {
        zbus::block_on(signal.emit(system))?;
    }
    let confirmed = zbus::block_on(system.call_method(
        Some(agent.0.as_str()),
        agent.1.as_str(),
        Some(AGENT),
        "RequestConfirmation",
        &(&device, code),
    ))
    .is_ok();
    let signals = {
        let mut world = lock(world);
        let world = &mut *world;
        if let Some(request) = world
            .pairing
            .requests
            .iter_mut()
            .rev()
            .find(|(passkey, _)| *passkey == code)
        {
            if confirmed { "confirmed" } else { "rejected" }.clone_into(&mut request.1);
        }
        if confirmed {
            world.range.forgot_pairing = false;
            world.range.reachable = true;
            ["Paired", "Bonded"]
                .into_iter()
                .filter_map(|name| set(world, device.as_str(), bluez::DEVICE, name, true.into()))
                .collect::<Vec<_>>()
        } else {
            set(
                world,
                device.as_str(),
                bluez::DEVICE,
                "Connected",
                false.into(),
            )
            .into_iter()
            .collect()
        }
    };
    zbus::block_on(async {
        for signal in signals {
            signal.emit(system).await?;
        }
        Ok::<_, zbus::Error>(())
    })?;
    Ok(Some(confirmed))
}

/// A six-digit passkey, different each time.
fn passkey() -> u32 {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| since.subsec_nanos());
    nanos % 1_000_000
}

/// The Phone connected by itself, or for the Desktop's `Connect`: with
/// hands-free if it shares Calls by itself, otherwise media alone.
pub fn connected_by_phone(world: &mut World) -> zbus::Result<range::Change> {
    if world.pairing.hands_free == HandsFree::Shared {
        return Ok(range::hands_free_up(world)?.unwrap_or_default());
    }
    range::connected_without_hands_free(world)
}

/// `ConnectProfile` for hands-free while the Phone is connected without it:
/// the reply, and the change if hands-free comes up.
pub fn connect_hands_free(world: &mut World) -> zbus::Result<(Reply, Option<range::Change>)> {
    if world.pairing.hands_free == HandsFree::Refused {
        return Ok((
            Reply::Error("org.bluez.Error.Failed", "br-connection-refused".to_owned()),
            None,
        ));
    }
    Ok((Reply::body(()), range::hands_free_up(world)?))
}

/// How long the Phone takes to answer `ConnectProfile` while connected
/// without hands-free.
pub const HANDS_FREE_ANSWER_TIME: Duration = HANDS_FREE_ANSWER;

/// Whether the Phone is connected without hands-free: media alone.
pub fn connected_without_hands_free(world: &World) -> bool {
    world.audio_gateway.is_none()
        && bluez::device_path(&world.phone_address)
            .ok()
            .and_then(|device| {
                world
                    .bluez
                    .get(&device, bluez::DEVICE, "Connected")
                    .cloned()
            })
            .and_then(|value| bool::try_from(value).ok())
            .unwrap_or(false)
}
