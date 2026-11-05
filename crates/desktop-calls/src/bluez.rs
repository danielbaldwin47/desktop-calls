//! The BlueZ client: which paired Phones exist and whether each is within
//! reach (ADR-0004), and the writes the Desktop makes to BlueZ: connecting
//! the Phone while it is away (ADR-0004), dropping hands-free and connecting
//! it again, which moves a Call's audio to the Phone (ADR-0005), and
//! connecting hands-free once more if it didn't come back (ticket #101).
//!
//! It reports each paired Bluetooth device that offers the Hands-Free Audio
//! Gateway profile, which is what makes it a Phone that can carry Calls,
//! and whether it has a Bluetooth connection to the Desktop: `Connected`.
//! For the Setup Window it also reports each nearby device that isn't
//! paired and whose `Class` says Phone, by its name, and when BlueZ has
//! listed what it knows at startup. Pairing itself is `pairing.rs`.

use std::collections::HashMap;

use desktop_calls_core::{ConnectFailure, Input, Phone};
use futures_channel::mpsc::{UnboundedSender, unbounded};
use futures_util::StreamExt as _;
use zbus::fdo::ObjectManagerProxy;
use zbus::{Connection, OwnedVariant};

use crate::logging::EventKind;
use crate::object_manager::{self, ObjectEvent, WatchError, Watched};

const SERVICE: &str = "org.bluez";
const ROOT: &str = "/";
const DEVICE: &str = "org.bluez.Device1";

/// The Hands-Free Audio Gateway profile: the Phone's side of hands-free.
const HANDS_FREE_AUDIO_GATEWAY_UUID: &str = "0000111f-0000-1000-8000-00805f9b34fb";

/// Reports paired Phones to `inputs` until the connection closes, and
/// whether each has a Bluetooth connection to the Desktop.
pub async fn run(system: Connection, inputs: UnboundedSender<Input>) -> Result<(), WatchError> {
    let (object_sender, mut objects) = unbounded();
    let watching = async {
        let watched = Watched {
            service: SERVICE,
            root: ROOT,
            nested_manager: None,
        };
        object_manager::watch(&system, watched, &object_sender).await?;
        drop(object_sender);
        Ok::<_, WatchError>(())
    };
    let reporting = async {
        let mut devices = Devices::default();
        while let Some(event) = objects.next().await {
            for input in devices.event(event) {
                inputs
                    .unbounded_send(input)
                    .map_err(|_| WatchError::ListenerGone)?;
            }
        }
        Ok(())
    };
    futures_util::future::try_join(watching, reporting)
        .await
        .map(drop)
}

/// What BlueZ's devices are, as far as Desktop Calls cares.
#[derive(Default)]
struct Devices {
    /// Every `Device1`'s properties so far, by object path.
    properties: HashMap<String, HashMap<String, OwnedVariant>>,
    /// Paired Phones by object path: their addresses.
    phones: HashMap<String, String>,
    /// Nearby Phones not paired, by object path, as last reported.
    nearby: HashMap<String, Phone>,
}

impl Devices {
    /// The inputs a change to BlueZ's objects makes.
    fn event(&mut self, event: ObjectEvent) -> Vec<Input> {
        match event {
            ObjectEvent::Added { path, interfaces } => {
                let Some(device) = interfaces.get(DEVICE) else {
                    return Vec::new();
                };
                self.properties.insert(path.clone(), device.clone());
                let connected = connected(device);
                self.follow(&path, connected)
            }
            ObjectEvent::Changed {
                path,
                interface,
                changed,
            } if interface == DEVICE => {
                let Some(device) = self.properties.get_mut(&path) else {
                    return Vec::new();
                };
                device.extend(changed.iter().map(|(name, value)| {
                    (
                        name.clone(),
                        value.try_clone().unwrap_or_else(|_| value.clone()),
                    )
                }));
                self.follow(&path, connected(&changed))
            }
            ObjectEvent::Removed { path, interfaces } => {
                if !interfaces.iter().any(|interface| interface == DEVICE) {
                    return Vec::new();
                }
                self.properties.remove(&path);
                if let Some(phone) = self.nearby.remove(&path) {
                    return vec![Input::PhoneNotNearby {
                        address: phone.address,
                    }];
                }
                let Some(address) = self.phones.remove(&path) else {
                    return Vec::new();
                };
                tracing::debug!(
                    event_kind = %EventKind::PhoneUnpaired,
                    phone_address = %address,
                    "A paired Phone was removed"
                );
                vec![Input::PhoneUnpaired { address }]
            }
            ObjectEvent::Listed => vec![Input::PairedPhonesListed],
            // Pairings outlive a bluetoothd restart; the objects are
            // listed again when it is back.
            ObjectEvent::ServiceGone | ObjectEvent::Changed { .. } => Vec::new(),
        }
    }

    /// What the device at `path` is now, as inputs: newly a paired Phone,
    /// its presence if `connected` changed, or a nearby Phone found,
    /// renamed or paired.
    fn follow(&mut self, path: &str, connected: Option<bool>) -> Vec<Input> {
        let Some(device) = self.properties.get(path) else {
            return Vec::new();
        };
        let mut inputs = Vec::new();
        if let Some(address) = self.phones.get(path) {
            inputs.extend(connected.map(|connected| presence(address, connected)));
        } else if let Some(phone) = paired_phone(device) {
            tracing::debug!(
                event_kind = %EventKind::PhonePaired,
                phone_address = %phone.address,
                "Found a paired Phone"
            );
            if self.nearby.remove(path).is_some() {
                inputs.push(Input::PhoneNotNearby {
                    address: phone.address.clone(),
                });
            }
            self.phones.insert(path.to_owned(), phone.address.clone());
            let presence = self::connected(device).map(|now| presence(&phone.address, now));
            inputs.push(Input::PhonePaired(phone));
            inputs.extend(presence);
        } else if let Some(phone) = nearby_phone(device)
            && self.nearby.get(path) != Some(&phone)
        {
            tracing::debug!(
                event_kind = %EventKind::PhoneNearby,
                phone_address = %phone.address,
                "Found a Phone nearby that isn't paired"
            );
            self.nearby.insert(path.to_owned(), phone.clone());
            inputs.push(Input::PhoneNearby(phone));
        }
        inputs
    }
}

/// The Phone a `Device1` describes, if it isn't paired, says it is a Phone
/// by its `Class`, and has a name to list it by.
fn nearby_phone(device: &HashMap<String, OwnedVariant>) -> Option<Phone> {
    let paired = device
        .get("Paired")
        .and_then(|paired| bool::try_from(paired.clone()).ok())
        .unwrap_or(false);
    // The major device class, bits 8 to 12 of the Class of Device: 2 is
    // Phone (Bluetooth Assigned Numbers, 2.8.2).
    let is_phone = device
        .get("Class")
        .and_then(|class| u32::try_from(class.clone()).ok())
        .is_some_and(|class| (class >> 8) & 0x1f == 2);
    if paired || !is_phone {
        return None;
    }
    let text = |name: &str| {
        device
            .get(name)
            .and_then(|value| String::try_from(value.clone()).ok())
    };
    // Without `Name`, BlueZ's `Alias` is the address: nothing to list it by.
    text("Name")?;
    Some(Phone {
        name: text("Alias").or_else(|| text("Name"))?,
        address: text("Address")?,
    })
}

/// A `Device1`'s `Connected`, if it says.
fn connected(device: &HashMap<String, OwnedVariant>) -> Option<bool> {
    device
        .get("Connected")
        .and_then(|connected| bool::try_from(connected.clone()).ok())
}

/// Whether the Phone at `address` has a Bluetooth connection to the
/// Desktop, as an input.
fn presence(address: &str, connected: bool) -> Input {
    let address = address.to_owned();
    if connected {
        // At info, as its absence is: the Real-Phone Checklist times the
        // reconnect from it (step 6.1), and reads only info and above.
        tracing::info!(
            event_kind = %EventKind::PhonePresent,
            phone_address = %address,
            "The Phone is connected over Bluetooth"
        );
        Input::PhonePresent { address }
    } else {
        tracing::info!(
            event_kind = %EventKind::PhoneAbsent,
            phone_address = %address,
            "The Phone isn't connected over Bluetooth: out of range, or Bluetooth is off"
        );
        Input::PhoneAbsent { address }
    }
}

/// Asks BlueZ to connect the Phone at `address`: `Device1.Connect`, which
/// pages it and connects the profiles it offers (ADR-0004). A Phone out of
/// range fails it after BlueZ's page timeout, about 5 s.
///
/// # Errors
///
/// Why the Phone didn't connect; the raw BlueZ error is logged here.
pub async fn connect_phone(system: &Connection, address: &str) -> Result<(), ConnectFailure> {
    tracing::debug!(
        event_kind = %EventKind::PhoneConnectRequested,
        phone_address = %address,
        "Asking BlueZ to connect the Phone"
    );
    let device = match find_device(system, address).await {
        Ok(Some(device)) => device,
        Ok(None) => {
            connect_failed(
                address,
                ConnectFailure::Unreachable,
                "BlueZ has no device for the Phone",
            );
            return Err(ConnectFailure::Unreachable);
        }
        Err(error) => {
            connect_failed(address, ConnectFailure::Unreachable, &error.to_string());
            return Err(ConnectFailure::Unreachable);
        }
    };
    let failure = match system
        .call_method(Some(SERVICE), device.as_str(), Some(DEVICE), "Connect", &())
        .await
    {
        Ok(_) => return Ok(()),
        Err(zbus::Error::MethodError(name, message, _)) => {
            match connect_failure(name.as_str(), message.as_deref().unwrap_or_default()) {
                None => return Ok(()),
                Some(failure) => (failure, format!("{name}: {}", message.unwrap_or_default())),
            }
        }
        Err(error) => (ConnectFailure::Unreachable, error.to_string()),
    };
    connect_failed(address, failure.0, &failure.1);
    Err(failure.0)
}

/// What a BlueZ error from `Device1.Connect` means, or `None` if the Phone
/// is connected after all.
fn connect_failure(name: &str, message: &str) -> Option<ConnectFailure> {
    match name {
        "org.bluez.Error.AlreadyConnected" => None,
        // The Phone answered the page and had no link key for the Desktop.
        "org.bluez.Error.AuthenticationFailed" | "org.bluez.Error.AuthenticationRejected" => {
            Some(ConnectFailure::PairingLost)
        }
        "org.bluez.Error.Failed" if message.contains("key-missing") => {
            Some(ConnectFailure::PairingLost)
        }
        // `br-connection-page-timeout` when out of range, `InProgress`
        // while BlueZ is connecting it already, and the rest: worth trying
        // again.
        _ => Some(ConnectFailure::Unreachable),
    }
}

fn connect_failed(address: &str, failure: ConnectFailure, detail: &str) {
    tracing::info!(
        event_kind = %EventKind::PhoneConnectFailed,
        error_kind = match failure {
            ConnectFailure::Unreachable => "phone-unreachable",
            ConnectFailure::PairingLost => "pairing-lost",
        },
        phone_address = %address,
        detail = %detail,
        "BlueZ couldn't connect the Phone"
    );
}

/// Why hands-free couldn't be dropped and connected again.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BounceError {
    /// BlueZ has no device with the Phone's address.
    PhoneNotFound,
    /// BlueZ didn't drop hands-free: it is still up, with the audio where
    /// it was.
    NotDropped,
    /// Hands-free dropped, and BlueZ didn't connect it again.
    NotReconnected,
}

impl BounceError {
    /// The kind, for the log's `error_kind` field.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::PhoneNotFound => "phone-not-found",
            Self::NotDropped => "hands-free-not-dropped",
            Self::NotReconnected => "hands-free-not-reconnected",
        }
    }
}

/// Drops hands-free to the Phone at `address` and connects it again at once:
/// `DisconnectProfile`, then `ConnectProfile`, both for the Hands-Free Audio
/// Gateway profile only (ADR-0005). The Phone takes back a live Call's audio
/// as hands-free drops, and the Call carries on. The device stays connected
/// and paired: nothing here calls `Disconnect` or `RemoveDevice`.
///
/// # Errors
///
/// Which step failed; the raw BlueZ error is logged here.
pub async fn bounce_hands_free(system: &Connection, address: &str) -> Result<(), BounceError> {
    let device = device_path(system, address).await?;
    let profile = (HANDS_FREE_AUDIO_GATEWAY_UUID,);
    if let Err(error) = system
        .call_method(
            Some(SERVICE),
            device.as_str(),
            Some(DEVICE),
            "DisconnectProfile",
            &profile,
        )
        .await
    {
        return Err(bounce_failed(BounceError::NotDropped, &error));
    }
    connect_profile(system, &device)
        .await
        .map_err(|error| bounce_failed(BounceError::NotReconnected, &error))
}

/// Connects hands-free to the Phone at `address` again, after it dropped to
/// move a Call's audio to the Phone and didn't come back: `ConnectProfile`
/// for the Hands-Free Audio Gateway profile only, as the drop's own second
/// step (ADR-0005). The Call comes back with hands-free.
///
/// # Errors
///
/// Why it didn't connect; the raw BlueZ error is logged here.
pub async fn connect_hands_free(system: &Connection, address: &str) -> Result<(), BounceError> {
    let device = device_path(system, address).await?;
    connect_profile(system, &device).await.map_err(|error| {
        tracing::info!(
            event_kind = %EventKind::PhoneCommandFailed,
            error_kind = BounceError::NotReconnected.as_str(),
            phone_address = %address,
            detail = %error,
            "BlueZ couldn't connect hands-free again"
        );
        BounceError::NotReconnected
    })
}

/// `ConnectProfile` for the Hands-Free Audio Gateway profile on BlueZ's
/// `device`. `AlreadyConnected` and `InProgress` count as connected: the
/// Phone connected hands-free itself, first.
async fn connect_profile(system: &Connection, device: &str) -> zbus::Result<()> {
    match system
        .call_method(
            Some(SERVICE),
            device,
            Some(DEVICE),
            "ConnectProfile",
            &(HANDS_FREE_AUDIO_GATEWAY_UUID,),
        )
        .await
    {
        Ok(_) => Ok(()),
        Err(zbus::Error::MethodError(name, _, _))
            if matches!(
                name.as_str(),
                "org.bluez.Error.AlreadyConnected" | "org.bluez.Error.InProgress"
            ) =>
        {
            Ok(())
        }
        Err(error) => Err(error),
    }
}

/// The object path of BlueZ's device with `address`.
async fn device_path(system: &Connection, address: &str) -> Result<String, BounceError> {
    find_device(system, address)
        .await
        .map_err(|error| bounce_failed(BounceError::PhoneNotFound, &error))?
        .ok_or_else(|| {
            tracing::warn!(
                event_kind = %EventKind::PhoneCommandFailed,
                error_kind = BounceError::PhoneNotFound.as_str(),
                phone_address = %address,
                "BlueZ has no device for the Phone"
            );
            BounceError::PhoneNotFound
        })
}

/// The object path of BlueZ's device with `address`, if it has one.
pub(crate) async fn find_device(
    system: &Connection,
    address: &str,
) -> zbus::Result<Option<String>> {
    let objects = ObjectManagerProxy::builder(system)
        .destination(SERVICE)
        .path(ROOT)
        .build()
        .await?
        .get_managed_objects()
        .await?;
    Ok(objects
        .into_iter()
        .find(|(_, interfaces)| {
            interfaces
                .iter()
                .find(|(name, _)| name.as_str() == DEVICE)
                .and_then(|(_, device)| device.get("Address"))
                .and_then(|value| String::try_from(value.clone()).ok())
                .is_some_and(|found| found.eq_ignore_ascii_case(address))
        })
        .map(|(path, _)| path.to_string()))
}

fn bounce_failed(kind: BounceError, error: &zbus::Error) -> BounceError {
    tracing::warn!(
        event_kind = %EventKind::PhoneCommandFailed,
        error_kind = kind.as_str(),
        detail = %error,
        "Couldn't drop hands-free and connect it again"
    );
    kind
}

/// The Phone a `Device1` describes, if it is paired and offers hands-free.
fn paired_phone(device: &HashMap<String, OwnedVariant>) -> Option<Phone> {
    let paired = device
        .get("Paired")
        .and_then(|paired| bool::try_from(paired.clone()).ok())
        .unwrap_or(false);
    let offers_hands_free = device
        .get("UUIDs")
        .and_then(|uuids| Vec::<String>::try_from(uuids.clone()).ok())
        .is_some_and(|uuids| {
            uuids
                .iter()
                .any(|uuid| uuid.eq_ignore_ascii_case(HANDS_FREE_AUDIO_GATEWAY_UUID))
        });
    if !paired || !offers_hands_free {
        return None;
    }
    let text = |name: &str| {
        device
            .get(name)
            .and_then(|value| String::try_from(value.clone()).ok())
    };
    let address = text("Address")?;
    // `Alias` is the name the user sees in Bluetooth settings; BlueZ always
    // sets it, falling back to `Name`, then the address.
    let name = text("Alias")
        .or_else(|| text("Name"))
        .unwrap_or_else(|| address.clone());
    Some(Phone { name, address })
}
