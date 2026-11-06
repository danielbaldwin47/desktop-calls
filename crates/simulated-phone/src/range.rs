//! The Phone coming into the Desktop's range and going out of it, and
//! BlueZ's `Device1.Connect`, as BlueZ 5.87 and PipeWire 1.6.9 show them
//! with the real Pixel 9a (ticket #6's notes; ticket #19):
//!
//! - Hands-free coming up: BlueZ marks the Phone connected, then once the
//!   service level connection is set up the device gains `Battery1`, and
//!   PipeWire registers a new AudioGateway, at the next index, then sends
//!   `ModemAdded`. Calls the Phone carried on while away are announced on
//!   it, in their state.
//! - Going out of range: PipeWire unannounces each Call, without ending
//!   it, then the AudioGateway, then sends `ModemRemoved`; the device loses
//!   `Battery1`, and BlueZ marks the Phone disconnected. The Calls carry on
//!   on the Phone.
//! - `Connect` to a Phone out of range pages for BlueZ's page timeout,
//!   [`PAGE_TIMEOUT`], then fails with `org.bluez.Error.Failed`
//!   `"br-connection-page-timeout"`, as measured on the Desktop. A Phone
//!   that comes into range meanwhile answers the page. A Phone that has
//!   forgotten the pairing fails it for want of the link key:
//!   `"br-connection-key-missing"`, which is inferred from BlueZ's error
//!   table, not seen.

use std::time::{Duration, Instant};

use zbus::{Connection, Message, OwnedObjectPath, OwnedVariant};

use crate::objects::{MethodCall, Properties, Reply};
use crate::telephony::hands_free::{Away, audio_gateway_back, calls_back, drop_hands_free};
use crate::telephony::{self, Signal};
use crate::{SharedWorld, TelephonyConnection, World, bluez, lock};

/// BlueZ's page timeout: 8192 slots of 0.625 ms.
const PAGE_TIMEOUT: Duration = Duration::from_millis(5_120);
/// How long a Phone in range takes to answer the page and set up hands-free.
const CONNECT_TIME: Duration = Duration::from_millis(600);
/// How long a Phone that forgot the pairing takes to fail the connect.
const KEY_MISSING_TIME: Duration = Duration::from_millis(300);
const PAGE_POLL: Duration = Duration::from_millis(20);

/// Whether the Phone is in range, and what it carries on while it isn't.
#[derive(Default)]
pub struct Range {
    /// The Phone would answer a page.
    pub reachable: bool,
    /// The Phone has forgotten the Desktop: it refuses its connects.
    pub forgot_pairing: bool,
    /// BlueZ is answering a `Connect`.
    connecting: bool,
    /// Calls carrying on on the Phone while hands-free is down, by the last
    /// part of their path, with their properties.
    away: Vec<(String, Properties)>,
    /// The device's `Battery1`, while hands-free is down.
    battery: Option<Properties>,
}

impl Range {
    /// The Phone out of range, with its `Battery1` taken from `bluez`'s
    /// device at `device`: BlueZ shows it only with hands-free up.
    pub fn new(bluez: &mut crate::objects::ObjectTree, device: &OwnedObjectPath) -> Self {
        Self {
            battery: bluez::remove_battery(bluez, device),
            ..Self::default()
        }
    }

    /// The Calls carrying on on the Phone while hands-free is down, as
    /// `(path's last part, State)`.
    pub fn calls_away(&self) -> impl Iterator<Item = (String, String)> + '_ {
        self.away
            .iter()
            .map(|(name, properties)| (name.clone(), call_state(properties)))
    }
}

/// A Call's `State` property.
pub fn call_state(properties: &Properties) -> String {
    properties
        .get("State")
        .and_then(|state| String::try_from(state.clone()).ok())
        .unwrap_or_default()
}

/// The signals a change in range sends, in order, each on BlueZ's
/// connection or the Telephony service's.
#[derive(Default)]
pub struct Change {
    signals: Vec<(Service, Signal)>,
}

#[derive(Clone, Copy)]
enum Service {
    Bluez,
    Telephony,
}

impl Change {
    /// This change split before its first Telephony signal: BlueZ marking
    /// the Phone connected, then hands-free coming up.
    fn split_at_hands_free(self) -> (Self, Self) {
        let at = self
            .signals
            .iter()
            .position(|(service, _)| matches!(service, Service::Telephony))
            .unwrap_or(self.signals.len());
        let mut connected = self.signals;
        let hands_free = connected.split_off(at);
        (
            Self { signals: connected },
            Self {
                signals: hands_free,
            },
        )
    }

    fn bluez(&mut self, signal: Signal) {
        self.signals.push((Service::Bluez, signal));
    }

    fn telephony(&mut self, signals: impl IntoIterator<Item = Signal>) {
        self.signals.extend(
            signals
                .into_iter()
                .map(|signal| (Service::Telephony, signal)),
        );
    }
}

/// Brings hands-free up, as the Phone does once connected: see the module
/// documentation. `None` if it is up already.
pub fn hands_free_up(world: &mut World) -> zbus::Result<Option<Change>> {
    if world.audio_gateway.is_some() {
        return Ok(None);
    }
    // A drop for `DisconnectProfile` doesn't come back as well.
    if let Some(dropped) = world.dropped.take() {
        let (away, battery) = dropped.into_parts();
        keep_away(world, away);
        world.range.battery = battery.or(world.range.battery.take());
    }
    let device = bluez::device_path(&world.phone_address)?;
    world.bluez.set(
        &device,
        bluez::DEVICE,
        "Connected",
        OwnedVariant::from(true),
    );
    let mut change = Change::default();
    change.bluez(connected_changed(&device, true));
    if let Some(battery) = world.range.battery.take() {
        let copy = copy_properties(&battery);
        bluez::restore_battery(&mut world.bluez, &device, battery);
        change.bluez(Signal::InterfacesAdded {
            manager: OwnedObjectPath::try_from("/")?,
            object: device,
            interfaces: [(bluez::BATTERY.to_owned(), copy)].into(),
        });
    }
    let gateway = telephony::audio_gateway_path(world.next_audio_gateway_index)?;
    world.next_audio_gateway_index += 1;
    let calls = std::mem::take(&mut world.range.away)
        .into_iter()
        .map(|(name, properties)| {
            Ok((
                OwnedObjectPath::try_from(format!("{}/{name}", gateway.as_str()))?,
                properties,
            ))
        })
        .collect::<zbus::Result<Vec<_>>>()?;
    let away = Away {
        audio_gateway: gateway.clone(),
        calls,
    };
    let address = world.phone_address.clone();
    change.telephony(audio_gateway_back(
        &mut world.telephony,
        &away,
        &address,
        &world.behaviour,
    )?);
    change.telephony([Signal::ModemAdded {
        modem: gateway.clone(),
    }]);
    if !away.calls.is_empty() {
        change.telephony(calls_back(&mut world.telephony, away));
    }
    world.audio_gateway = Some(gateway);
    Ok(Some(change))
}

/// Drops hands-free and the Bluetooth connection, as a Phone going out of
/// range does: see the module documentation. The Calls carry on on the
/// Phone. `None` if it is down already.
pub fn hands_free_down(world: &mut World) -> zbus::Result<Option<Change>> {
    let device = bluez::device_path(&world.phone_address)?;
    let mut change = Change::default();
    if let Some(gateway) = world.audio_gateway.take() {
        let (away, signals) = drop_hands_free(&mut world.telephony, &gateway)?;
        keep_away(world, away);
        change.telephony(signals);
        change.telephony([Signal::ModemRemoved { modem: gateway }]);
    } else if let Some(dropped) = world.dropped.take() {
        let (away, battery) = dropped.into_parts();
        keep_away(world, away);
        world.range.battery = battery.or(world.range.battery.take());
    } else {
        return Ok(None);
    }
    if let Some(battery) = bluez::remove_battery(&mut world.bluez, &device) {
        world.range.battery = Some(battery);
        change.bluez(Signal::InterfacesRemoved {
            manager: OwnedObjectPath::try_from("/")?,
            object: device.clone(),
            interfaces: vec![bluez::BATTERY],
        });
    }
    world.bluez.set(
        &device,
        bluez::DEVICE,
        "Connected",
        OwnedVariant::from(false),
    );
    change.bluez(connected_changed(&device, false));
    Ok(Some(change))
}

/// Connects the Phone without hands-free, as a Phone that doesn't share
/// Calls by itself does: media alone.
pub fn connected_without_hands_free(world: &mut World) -> zbus::Result<Change> {
    let device = bluez::device_path(&world.phone_address)?;
    let mut change = Change::default();
    if world.bluez.set(
        &device,
        bluez::DEVICE,
        "Connected",
        OwnedVariant::from(true),
    ) {
        change.bluez(connected_changed(&device, true));
    }
    Ok(change)
}

/// Brings the other paired Phone into range, connected with hands-free:
/// BlueZ marks it connected, and PipeWire registers an AudioGateway for its
/// address at the next index, then sends `ModemAdded`. `None` if it is
/// connected already.
///
/// # Errors
///
/// There is no other Phone: the Simulated Phone started without
/// `--old-phone`, or it was unpaired.
pub fn other_phone_up(world: &mut World) -> zbus::Result<Option<Change>> {
    let device = bluez::device_path(crate::pairing::OLD_PHONE_ADDRESS)?;
    if !world.pairing.is_other_phone(device.as_str()) {
        return Err(zbus::Error::Failure("there is no other Phone".to_owned()));
    }
    if !world.bluez.set(
        &device,
        bluez::DEVICE,
        "Connected",
        OwnedVariant::from(true),
    ) {
        return Ok(None);
    }
    let mut change = Change::default();
    change.bluez(connected_changed(&device, true));
    let gateway = telephony::audio_gateway_path(world.next_audio_gateway_index)?;
    world.next_audio_gateway_index += 1;
    let interfaces = telephony::add_audio_gateway(
        &mut world.telephony,
        &gateway,
        crate::pairing::OLD_PHONE_ADDRESS,
        &world.behaviour,
    );
    change.telephony([
        Signal::InterfacesAdded {
            manager: OwnedObjectPath::try_from(telephony::MANAGER_PATH)?,
            object: gateway.clone(),
            interfaces,
        },
        Signal::ModemAdded {
            modem: gateway.clone(),
        },
    ]);
    world.pairing.other_gateway = Some(gateway);
    Ok(Some(change))
}

fn keep_away(world: &mut World, away: Away) {
    for (call, properties) in away.calls {
        let name = call
            .as_str()
            .rsplit('/')
            .next()
            .unwrap_or_default()
            .to_owned();
        world.range.away.push((name, properties));
    }
}

fn connected_changed(device: &OwnedObjectPath, connected: bool) -> Signal {
    Signal::PropertiesChanged {
        path: device.clone(),
        interface: bluez::DEVICE,
        changed: [("Connected".to_owned(), OwnedVariant::from(connected))].into(),
    }
}

fn copy_properties(properties: &Properties) -> Properties {
    properties
        .iter()
        .filter_map(|(name, value)| Some((name.clone(), value.try_clone().ok()?)))
        .collect()
}

/// Sends `change`'s signals.
pub async fn emit(
    system: &Connection,
    telephony: &TelephonyConnection,
    change: Change,
) -> zbus::Result<()> {
    let telephony = lock(telephony).clone();
    for (service, signal) in &change.signals {
        match (service, &telephony) {
            (Service::Bluez, _) => signal.emit(system).await?,
            (Service::Telephony, Some(telephony)) => signal.emit(telephony).await?,
            // The Telephony service is restarting: it announces what it has
            // once back.
            (Service::Telephony, None) => {}
        }
    }
    Ok(())
}

/// Whether `call` is BlueZ's `Device1.Connect`, which this module answers.
pub fn handles(call: &MethodCall) -> bool {
    call.interface == bluez::DEVICE && call.member == "Connect"
}

/// Answers `Device1.Connect` on a thread of its own, so BlueZ keeps
/// answering meanwhile: see the module documentation.
pub fn answer_connect(
    world: SharedWorld,
    system: Connection,
    telephony: TelephonyConnection,
    call: MethodCall,
    message: Message,
) {
    std::thread::spawn(move || {
        if let Err(error) = connect(&world, &system, &telephony, &call, &message) {
            eprintln!("simulated-phone: Connect failed: {error}");
        }
    });
}

/// How a `Connect` ended, and what it changed: the failure BlueZ answers
/// with, if any; the signals of the Phone connecting; and, if the service
/// level connection is slow, the signals of hands-free coming up after it.
type Ended = (Option<&'static str>, Option<Change>, Option<Change>);

/// Ends the `Connect` that was under way. `failure` is how it fails by what
/// the Phone was when it began.
///
/// The Phone can forget the pairing while a connect to it is under way: the
/// link key is checked as the link comes up, so that connect fails too, and
/// never brings the Phone back (ticket #171).
fn end_connect(world: &SharedWorld, failure: Option<&'static str>) -> zbus::Result<Ended> {
    let mut world = lock(world);
    world.range.connecting = false;
    let failure = failure.or_else(|| {
        world
            .range
            .forgot_pairing
            .then_some("br-connection-key-missing")
    });
    if failure.is_some() {
        return Ok((failure, None, None));
    }
    let slow = world
        .behaviour
        .quirks
        .contains(&telephony::Quirk::SlowServiceLevel);
    let change = crate::pairing::connected_by_phone(&mut world)?;
    Ok(if slow {
        let (connected, hands_free) = change.split_at_hands_free();
        (None, Some(connected), Some(hands_free))
    } else {
        (None, Some(change), None)
    })
}

fn connect(
    world: &SharedWorld,
    system: &Connection,
    telephony: &TelephonyConnection,
    call: &MethodCall,
    message: &Message,
) -> zbus::Result<()> {
    if !crate::desktop::powered(&lock(world)) {
        return zbus::block_on(crate::desktop::not_ready().send(system, &message.header()));
    }
    let other = {
        let world = lock(world);
        world.pairing.is_other_phone(&call.path).then(|| {
            OwnedObjectPath::try_from(call.path.as_str())
                .ok()
                .and_then(|path| world.bluez.get(&path, bluez::DEVICE, "Connected").cloned())
                .and_then(|connected| bool::try_from(connected).ok())
                .unwrap_or(false)
        })
    };
    if other == Some(true) {
        // The other Phone is connected already (`ConnectOtherPhone`).
        return zbus::block_on(Reply::body(()).send(system, &message.header()));
    }
    if other == Some(false) {
        // Another paired Phone, out of range: pages and times out, with
        // BlueZ answering everything else meanwhile.
        std::thread::sleep(PAGE_TIMEOUT);
        let reply = Reply::Error(
            "org.bluez.Error.Failed",
            "br-connection-page-timeout".to_owned(),
        );
        return zbus::block_on(reply.send(system, &message.header()));
    }
    let refused = {
        let mut world = lock(world);
        if call.path != bluez::device_path(&world.phone_address)?.as_str() {
            Some(Reply::unknown_method(call))
        } else if world.audio_gateway.is_some() {
            Some(Reply::Error(
                "org.bluez.Error.AlreadyConnected",
                "Already Connected".to_owned(),
            ))
        } else if world.range.connecting {
            Some(Reply::Error(
                "org.bluez.Error.InProgress",
                "In Progress".to_owned(),
            ))
        } else {
            world.range.connecting = true;
            None
        }
    };
    if let Some(reply) = refused {
        return zbus::block_on(reply.send(system, &message.header()));
    }
    let paging = Instant::now();
    while !lock(world).range.reachable && paging.elapsed() < PAGE_TIMEOUT {
        std::thread::sleep(PAGE_POLL);
    }
    let (reachable, forgot_pairing) = {
        let world = lock(world);
        (world.range.reachable, world.range.forgot_pairing)
    };
    let failure = if !reachable {
        Some("br-connection-page-timeout")
    } else if forgot_pairing {
        std::thread::sleep(KEY_MISSING_TIME);
        Some("br-connection-key-missing")
    } else {
        std::thread::sleep(CONNECT_TIME);
        None
    };
    let (failure, change, service_level) = end_connect(world, failure)?;
    if let Some(change) = change {
        zbus::block_on(emit(system, telephony, change))?;
    }
    if let Some(hands_free) = service_level {
        std::thread::sleep(telephony::SERVICE_LEVEL_TIME);
        zbus::block_on(emit(system, telephony, hands_free))?;
    }
    zbus::block_on(async {
        let reply = match failure {
            Some(failure) => Reply::Error("org.bluez.Error.Failed", failure.to_owned()),
            None => Reply::body(()),
        };
        reply.send(system, &message.header()).await
    })
}
