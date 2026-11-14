//! BlueZ's `Device1.Blocked` on the Phone, held as strict as BlueZ 5.87
//! was with the real Pixel 9a (ticket #202; the probe note
//! `docs/research/2026-11-13-calls-off-mechanism-probe.md`):
//!
//! - Setting it `true` drops the Phone's link and every profile at once:
//!   hands-free goes as when the Phone leaves, and BlueZ marks it
//!   disconnected. It keeps its value across an app restart, as BlueZ does.
//! - While it is `true`, the Phone's own pages are refused
//!   ([`page_refused`]), and a `Device1.Connect` from the Desktop is not:
//!   BlueZ pages the Phone, then fails
//!   `"br-connection-profile-unavailable"`, on the Pixel after 25 s or more,
//!   here after [`BLOCKED_CONNECT_TIME`]. A `ConnectProfile` fails "No more
//!   profiles to connect to". Each is counted, so a scenario can assert the
//!   Desktop made none while calls were off.
//! - With the quirk `connect-busy-after-unblock`, the first `Connect` after
//!   the Phone is unblocked fails `"br-connection-busy"`.

use std::time::Duration;

use zbus::{Connection, Message, OwnedObjectPath, OwnedVariant};

use crate::objects::{MethodCall, PROPERTIES, Reply};
use crate::telephony::{self, Signal};
use crate::{SharedWorld, TelephonyConnection, World, bluez, lock, range};

/// How long a `Connect` on the blocked Phone takes to fail: short, as a
/// test can't wait the Pixel's 25 s or more.
const BLOCKED_CONNECT_TIME: Duration = Duration::from_millis(1_500);

/// What the Desktop asked of the Phone while it was blocked.
#[derive(Default)]
pub struct Blocking {
    /// Every `Connect` and `ConnectProfile` for the Phone while blocked.
    pub connects: u32,
    /// The Phone's own pages refused while blocked.
    pub pages_refused: u32,
    /// The next `Connect` fails busy: the Phone was just unblocked with
    /// the quirk `connect-busy-after-unblock`.
    busy_next: bool,
}

/// Whether the Phone is blocked now.
pub fn blocked(world: &World) -> bool {
    bluez::device_path(&world.phone_address)
        .ok()
        .and_then(|device| world.bluez.get(&device, bluez::DEVICE, "Blocked").cloned())
        .and_then(|blocked| bool::try_from(blocked).ok())
        .unwrap_or(false)
}

/// Whether the Phone's own page reaches the Desktop: not while blocked,
/// which counts it as refused.
pub fn page_refused(world: &mut World) -> bool {
    let refused = blocked(world);
    if refused {
        world.blocking.pages_refused += 1;
    }
    refused
}

/// Whether `call` is one this module answers: `Blocked` written, or a
/// `Connect` or `ConnectProfile` for the blocked Phone.
pub fn handles(world: &World, call: &MethodCall) -> bool {
    let on_phone =
        bluez::device_path(&world.phone_address).is_ok_and(|device| call.path == device.as_str());
    if !on_phone {
        return false;
    }
    match (call.interface.as_str(), call.member.as_str()) {
        (PROPERTIES, "Set") => true,
        (bluez::DEVICE, "Connect" | "ConnectProfile") => blocked(world),
        _ => false,
    }
}

/// Answers `call` on a thread of its own, so BlueZ keeps answering
/// meanwhile.
pub fn answer(
    world: SharedWorld,
    system: Connection,
    telephony: TelephonyConnection,
    call: MethodCall,
    message: Message,
) {
    std::thread::spawn(move || {
        let acted = if call.member == "Set" {
            set(&world, &system, &telephony, &call, &message)
        } else {
            refuse_connect(&world, &system, &call, &message)
        };
        if let Err(error) = acted {
            eprintln!("simulated-phone: {} failed: {error}", call.member);
        }
    });
}

/// The first `Connect` after the Phone was unblocked, with the quirk
/// `connect-busy-after-unblock`: BlueZ's answer, once.
pub fn busy(world: &mut World) -> Option<Reply> {
    std::mem::take(&mut world.blocking.busy_next)
        .then(|| Reply::Error("org.bluez.Error.Failed", "br-connection-busy".to_owned()))
}

/// A `Connect` or `ConnectProfile` for the blocked Phone: counted, and
/// failed as BlueZ fails it.
fn refuse_connect(
    world: &SharedWorld,
    system: &Connection,
    call: &MethodCall,
    message: &Message,
) -> zbus::Result<()> {
    lock(world).blocking.connects += 1;
    let reply = if call.member == "Connect" {
        // BlueZ pages the Phone and browses it before giving up.
        std::thread::sleep(BLOCKED_CONNECT_TIME);
        Reply::Error(
            "org.bluez.Error.Failed",
            "br-connection-profile-unavailable".to_owned(),
        )
    } else {
        Reply::Error(
            "org.bluez.Error.Failed",
            "No more profiles to connect to".to_owned(),
        )
    };
    zbus::block_on(reply.send(system, &message.header()))
}

/// `Properties.Set` on the Phone: `Blocked`, which drops the
/// Phone when set, or `Trusted`, as before. Anything else is read-only.
fn set(
    world: &SharedWorld,
    system: &Connection,
    telephony: &TelephonyConnection,
    call: &MethodCall,
    message: &Message,
) -> zbus::Result<()> {
    let Ok((interface, name, value)) = message
        .body()
        .deserialize::<(String, String, OwnedVariant)>()
    else {
        return zbus::block_on(
            Reply::Error(
                "org.freedesktop.DBus.Error.InvalidArgs",
                "Set takes (s, s, v)".to_owned(),
            )
            .send(system, &message.header()),
        );
    };
    let writable = interface == bluez::DEVICE && matches!(name.as_str(), "Blocked" | "Trusted");
    let Ok(new) = bool::try_from(value) else {
        return zbus::block_on(read_only(&name).send(system, &message.header()));
    };
    if !writable {
        return zbus::block_on(read_only(&name).send(system, &message.header()));
    }
    let device = OwnedObjectPath::try_from(call.path.as_str())?;
    let (changed, dropped) = {
        let mut world = lock(world);
        let world = &mut *world;
        let was = blocked(world);
        let changed = world
            .bluez
            .set(&device, bluez::DEVICE, &name, OwnedVariant::from(new));
        let mut dropped = None;
        if name == "Blocked" && new && !was {
            dropped = Some(drop_link(world)?);
        }
        if name == "Blocked"
            && !new
            && was
            && world
                .behaviour
                .quirks
                .contains(&telephony::Quirk::ConnectBusyAfterUnblock)
        {
            world.blocking.busy_next = true;
        }
        (changed, dropped)
    };
    zbus::block_on(async {
        Reply::body(()).send(system, &message.header()).await?;
        if changed {
            Signal::PropertiesChanged {
                path: device.clone(),
                interface: bluez::DEVICE,
                changed: [(name.clone(), OwnedVariant::from(new))].into(),
            }
            .emit(system)
            .await?;
        }
        if let Some(dropped) = dropped {
            range::emit(system, telephony, dropped).await?;
        }
        Ok(())
    })
}

/// Drops the Phone's link and every profile, as blocking it does: as for a
/// Phone leaving, though it stays in range.
fn drop_link(world: &mut World) -> zbus::Result<range::Change> {
    world.obex.link_lost();
    match range::hands_free_down(world)? {
        Some(change) => Ok(change),
        // Connected without hands-free, or not at all.
        None => range::link_down(world),
    }
}

fn read_only(name: &str) -> Reply {
    Reply::Error(
        "org.freedesktop.DBus.Error.PropertyReadOnly",
        format!("Property '{name}' is not writable"),
    )
}
