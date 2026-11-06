//! BlueZ's `Device1.DisconnectProfile` and `ConnectProfile` for the
//! Hands-Free Audio Gateway profile: hands-free dropping and coming back
//! while the Phone stays connected and paired, as the Desktop does to move a
//! Call's audio to the Phone (ADR-0005). Shaped like BlueZ 5.87 with the
//! real Pixel 9a mid-Call (ticket #14's prototype,
//! `docs/research/2026-11-04-handoff-to-phone-prototype.md`), times from
//! `DisconnectProfile`:
//!
//! - `DisconnectProfile` replies at once. Each Call goes, then the
//!   AudioGateway, then the device's `Battery1`. `Device1.Connected` stays
//!   true. No Call goes `disconnected`.
//! - `ConnectProfile` replies [`RECONNECT_REPLY`] later (the Pixel: 0.11 to
//!   0.49 s).
//! - [`AUDIO_GATEWAY_BACK`] after `ConnectProfile` (the Pixel: 0.50 to 0.96 s
//!   after the drop), `Battery1` and the AudioGateway are back, then
//!   [`CALLS_BACK`] later (the Pixel: 30 to 60 ms) each Call, still in its
//!   state, then the volumes.
//! - [`FIRST_AUDIO_LINK_TRY`] after the AudioGateway (the Pixel: 0.4 to
//!   0.7 s), the Phone tries to bring the audio link up, and again
//!   [`NEXT_AUDIO_LINK_TRY`] later (the Pixel: 50 to 250 ms apart).
//!
//! PipeWire's `ModemRemoved` for the drop comes late, once `ConnectProfile`
//! has replied, and its `ModemAdded` follows the AudioGateway's
//! `InterfacesAdded`. With the
//! `"disconnect-profile-refused"` quirk, `DisconnectProfile` fails and
//! nothing changes. With `"connect-profile-fails"`, each `ConnectProfile`
//! after the drop fails [`RECONNECT_REPLY`] later and hands-free stays down,
//! with the Calls carrying on on the Phone; with
//! `"connect-profile-fails-once"`, only the next one does.

use std::time::Duration;

use zbus::{Connection, Message, OwnedObjectPath};

use crate::objects::{MethodCall, Properties, Reply};
use crate::telephony::hands_free::{
    Away, audio_gateway_back, audio_link_tries, calls_back, drop_hands_free,
};
use crate::telephony::{Quirk, Signal};
use crate::{SharedWorld, TelephonyConnection, World, bluez, lock, pairing, range};

/// The Hands-Free Audio Gateway profile: the Phone's side of hands-free.
const HANDS_FREE_AUDIO_GATEWAY_UUID: &str = "0000111f-0000-1000-8000-00805f9b34fb";

const RECONNECT_REPLY: Duration = Duration::from_millis(150);
const AUDIO_GATEWAY_BACK: Duration = Duration::from_millis(600);
const CALLS_BACK: Duration = Duration::from_millis(40);
const FIRST_AUDIO_LINK_TRY: Duration = Duration::from_millis(500);
const NEXT_AUDIO_LINK_TRY: Duration = Duration::from_millis(150);

/// What comes back with hands-free: the AudioGateway, its Calls, and the
/// signals that announce it on BlueZ and on the Telephony service.
struct Returned {
    gateway: OwnedObjectPath,
    away: Away,
    system_signals: Vec<Signal>,
    telephony_signals: Vec<Signal>,
}

/// Hands-free dropped by `DisconnectProfile`, until it is back.
pub struct Dropped {
    away: Away,
    battery: Option<Properties>,
    /// `ConnectProfile` is bringing it back.
    reconnecting: bool,
}

impl Dropped {
    /// What went with hands-free, and the device's `Battery1`.
    pub fn into_parts(self) -> (Away, Option<Properties>) {
        (self.away, self.battery)
    }

    /// The Calls carrying on on the Phone while hands-free is down, as
    /// `(path's last part, State)`.
    pub fn calls_away(&self) -> impl Iterator<Item = (String, String)> + '_ {
        self.away.calls_away()
    }
}

/// Whether `call` is one this module answers.
pub fn handles(call: &MethodCall) -> bool {
    call.interface == bluez::DEVICE
        && matches!(call.member.as_str(), "DisconnectProfile" | "ConnectProfile")
}

/// Answers `DisconnectProfile` or `ConnectProfile` on a thread of its own,
/// so BlueZ keeps answering meanwhile, and acts it out on BlueZ's `system`
/// connection and the Telephony service's.
pub fn answer(
    world: SharedWorld,
    system: Connection,
    telephony: TelephonyConnection,
    call: MethodCall,
    message: Message,
) {
    std::thread::spawn(move || {
        let acted = if call.member == "DisconnectProfile" {
            disconnect_profile(&world, &system, &telephony, &call, &message)
        } else {
            connect_profile(&world, &system, &telephony, &call, &message)
        };
        if let Err(error) = acted {
            eprintln!("simulated-phone: {} failed: {error}", call.member);
        }
    });
}

fn disconnect_profile(
    world: &SharedWorld,
    system: &Connection,
    telephony: &TelephonyConnection,
    call: &MethodCall,
    message: &Message,
) -> zbus::Result<()> {
    let (reply, system_signals, telephony_signals) = {
        let mut world = lock(world);
        let world = &mut *world;
        match refusal(world, call, message)? {
            Some(reply) => (reply, Vec::new(), Vec::new()),
            None if world
                .behaviour
                .quirks
                .contains(&Quirk::DisconnectProfileRefused) =>
            {
                (
                    Reply::Error("org.bluez.Error.Failed", "Operation failed".to_owned()),
                    Vec::new(),
                    Vec::new(),
                )
            }
            None => match world.audio_gateway.take() {
                None => (
                    Reply::Error("org.bluez.Error.NotConnected", "Not Connected".to_owned()),
                    Vec::new(),
                    Vec::new(),
                ),
                Some(gateway) => {
                    let (away, telephony_signals) =
                        drop_hands_free(&mut world.telephony, &gateway)?;
                    let device = bluez::device_path(&world.phone_address)?;
                    let battery = bluez::remove_battery(&mut world.bluez, &device);
                    let system_signals = vec![Signal::InterfacesRemoved {
                        manager: OwnedObjectPath::try_from("/")?,
                        object: device,
                        interfaces: vec![bluez::BATTERY],
                    }];
                    world.dropped = Some(Dropped {
                        away,
                        battery,
                        reconnecting: false,
                    });
                    (Reply::body(()), system_signals, telephony_signals)
                }
            },
        }
    };
    zbus::block_on(async {
        reply.send(system, &message.header()).await?;
        emit(telephony, &telephony_signals).await?;
        for signal in &system_signals {
            signal.emit(system).await?;
        }
        Ok(())
    })
}

fn connect_profile(
    world: &SharedWorld,
    system: &Connection,
    telephony: &TelephonyConnection,
    call: &MethodCall,
    message: &Message,
) -> zbus::Result<()> {
    let mut ask = false;
    // BlueZ takes a while to give up, as it does to connect.
    let mut slow = false;
    let reply = {
        let mut world = lock(world);
        let world = &mut *world;
        match refusal(world, call, message)? {
            Some(reply) => Some(reply),
            None if world.audio_gateway.is_some() => Some(Reply::Error(
                "org.bluez.Error.AlreadyConnected",
                "Already Connected".to_owned(),
            )),
            // Connected without hands-free: the Phone is asked for it.
            None if world.dropped.is_none() && pairing::connected_without_hands_free(world) => {
                ask = true;
                None
            }
            None => match world.dropped.as_mut() {
                None => Some(Reply::Error(
                    "org.bluez.Error.Failed",
                    "br-connection-page-timeout".to_owned(),
                )),
                Some(dropped) if dropped.reconnecting => Some(Reply::Error(
                    "org.bluez.Error.InProgress",
                    "In Progress".to_owned(),
                )),
                Some(_) if fails(&mut world.behaviour.quirks) => {
                    slow = true;
                    Some(Reply::Error(
                        "org.bluez.Error.Failed",
                        "br-connection-refused".to_owned(),
                    ))
                }
                Some(dropped) => {
                    dropped.reconnecting = true;
                    None
                }
            },
        }
    };
    if let Some(reply) = reply {
        if slow {
            std::thread::sleep(RECONNECT_REPLY);
        }
        return zbus::block_on(reply.send(system, &message.header()));
    }
    if ask {
        return ask_for_hands_free(world, system, telephony, message);
    }
    std::thread::sleep(RECONNECT_REPLY);
    zbus::block_on(Reply::body(()).send(system, &message.header()))?;
    // PipeWire's `ModemRemoved` for the drop comes this late.
    let modem = lock(world)
        .dropped
        .as_ref()
        .map(|dropped| dropped.away.audio_gateway.clone());
    if let Some(modem) = modem {
        zbus::block_on(emit(telephony, &[Signal::ModemRemoved { modem }]))?;
    }
    std::thread::sleep(AUDIO_GATEWAY_BACK.saturating_sub(RECONNECT_REPLY));
    let Some(Returned {
        gateway,
        away,
        system_signals,
        telephony_signals,
    }) = audio_gateway_returns(world)?
    else {
        // The test connected or disconnected the Phone meanwhile.
        return Ok(());
    };
    zbus::block_on(async {
        for signal in &system_signals {
            signal.emit(system).await?;
        }
        emit(telephony, &telephony_signals).await
    })?;
    let calls_back_after = lock(world).behaviour.calls_back_after(CALLS_BACK);
    std::thread::sleep(calls_back_after);
    let signals = {
        let mut world = lock(world);
        calls_back(&mut world.telephony, away)
    };
    zbus::block_on(emit(telephony, &signals))?;
    std::thread::sleep(FIRST_AUDIO_LINK_TRY.saturating_sub(calls_back_after));
    let tries = {
        let mut world = lock(world);
        let world = &mut *world;
        if world.audio_gateway.as_ref() != Some(&gateway) {
            return Ok(());
        }
        audio_link_tries(&mut world.telephony, &gateway, &world.behaviour)
    };
    for (index, signals) in tries.iter().enumerate() {
        if index > 0 {
            std::thread::sleep(NEXT_AUDIO_LINK_TRY);
        }
        zbus::block_on(emit(telephony, signals))?;
    }
    Ok(())
}

/// The Phone, connected without hands-free, is asked for it: it brings it
/// up, or refuses, as its "Phone calls" switch says.
fn ask_for_hands_free(
    world: &SharedWorld,
    system: &Connection,
    telephony: &TelephonyConnection,
    message: &Message,
) -> zbus::Result<()> {
    std::thread::sleep(pairing::HANDS_FREE_ANSWER_TIME);
    let (reply, change) = pairing::connect_hands_free(&mut lock(world))?;
    zbus::block_on(async {
        if let Some(change) = change {
            range::emit(system, telephony, change).await?;
        }
        reply.send(system, &message.header()).await
    })
}

/// Whether this `ConnectProfile` fails, by the quirks: every time, or this
/// time only.
fn fails(quirks: &mut std::collections::BTreeSet<Quirk>) -> bool {
    quirks.contains(&Quirk::ConnectProfileFails) || quirks.remove(&Quirk::ConnectProfileFailsOnce)
}

/// Brings `Battery1` and the AudioGateway back, if hands-free is still
/// dropped.
fn audio_gateway_returns(world: &SharedWorld) -> zbus::Result<Option<Returned>> {
    let mut world = lock(world);
    let world = &mut *world;
    let Some(dropped) = world.dropped.take() else {
        return Ok(None);
    };
    let device = bluez::device_path(&world.phone_address)?;
    let mut system_signals = Vec::new();
    if let Some(battery) = dropped.battery {
        let copy = battery
            .iter()
            .filter_map(|(name, value)| Some((name.clone(), value.try_clone().ok()?)))
            .collect::<Properties>();
        bluez::restore_battery(&mut world.bluez, &device, battery);
        system_signals.push(Signal::InterfacesAdded {
            manager: OwnedObjectPath::try_from("/")?,
            object: device,
            interfaces: [(bluez::BATTERY.to_owned(), copy)].into(),
        });
    }
    let address = world.phone_address.clone();
    let mut telephony_signals = audio_gateway_back(
        &mut world.telephony,
        &dropped.away,
        &address,
        &world.behaviour,
    )?;
    let gateway = dropped.away.audio_gateway.clone();
    telephony_signals.push(Signal::ModemAdded {
        modem: gateway.clone(),
    });
    world.audio_gateway = Some(gateway.clone());
    Ok(Some(Returned {
        gateway,
        away: dropped.away,
        system_signals,
        telephony_signals,
    }))
}

/// Why BlueZ refuses `call` before acting on it, if it does: the wrong
/// object, arguments or profile.
fn refusal(world: &World, call: &MethodCall, message: &Message) -> zbus::Result<Option<Reply>> {
    if world.pairing.is_other_phone(&call.path) {
        // The other paired Phone: connected (`ConnectOtherPhone`) has
        // hands-free up already; away, it never answers the page.
        let connected = zbus::OwnedObjectPath::try_from(call.path.as_str())
            .ok()
            .and_then(|path| world.bluez.get(&path, bluez::DEVICE, "Connected").cloned())
            .and_then(|connected| bool::try_from(connected).ok())
            .unwrap_or(false);
        return Ok(Some(if connected {
            Reply::Error(
                "org.bluez.Error.AlreadyConnected",
                "Already Connected".to_owned(),
            )
        } else {
            Reply::Error(
                "org.bluez.Error.Failed",
                "br-connection-page-timeout".to_owned(),
            )
        }));
    }
    if call.path != bluez::device_path(&world.phone_address)?.as_str() {
        return Ok(Some(Reply::unknown_method(call)));
    }
    let Ok((uuid,)) = message.body().deserialize::<(String,)>() else {
        return Ok(Some(Reply::Error(
            "org.bluez.Error.InvalidArguments",
            "Invalid arguments in method call".to_owned(),
        )));
    };
    if !uuid.eq_ignore_ascii_case(HANDS_FREE_AUDIO_GATEWAY_UUID) {
        // Only the hands-free profile is modelled.
        return Ok(Some(Reply::Error(
            "org.bluez.Error.NotSupported",
            "Operation is not supported".to_owned(),
        )));
    }
    Ok(None)
}

async fn emit(telephony: &TelephonyConnection, signals: &[Signal]) -> zbus::Result<()> {
    let Some(connection) = lock(telephony).clone() else {
        return Ok(());
    };
    for signal in signals {
        signal.emit(&connection).await?;
    }
    Ok(())
}
