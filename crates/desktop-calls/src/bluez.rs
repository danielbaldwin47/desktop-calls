//! The BlueZ client: which paired Phones exist (ADR-0004), and the one write
//! the Desktop makes to BlueZ: dropping hands-free and connecting it again,
//! which moves a Call's audio to the Phone (ADR-0005).
//!
//! It reports each paired Bluetooth device that offers the Hands-Free Audio
//! Gateway profile, which is what makes it a Phone that can carry Calls.

use std::collections::HashMap;

use desktop_calls_core::{Input, Phone};
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

/// Reports paired Phones to `inputs` until the connection closes.
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
        // Paired Phones by object path: their addresses.
        let mut phones: HashMap<String, String> = HashMap::new();
        while let Some(event) = objects.next().await {
            let input = match event {
                ObjectEvent::Added { path, interfaces } => {
                    let Some(phone) = interfaces.get(DEVICE).and_then(paired_phone) else {
                        continue;
                    };
                    tracing::debug!(
                        event_kind = %EventKind::PhonePaired,
                        phone_address = %phone.address,
                        "Found a paired Phone"
                    );
                    phones.insert(path, phone.address.clone());
                    Input::PhonePaired(phone)
                }
                ObjectEvent::Removed { path, interfaces } => {
                    if !interfaces.iter().any(|interface| interface == DEVICE) {
                        continue;
                    }
                    let Some(address) = phones.remove(&path) else {
                        continue;
                    };
                    tracing::debug!(
                        event_kind = %EventKind::PhoneUnpaired,
                        phone_address = %address,
                        "A paired Phone was removed"
                    );
                    Input::PhoneUnpaired { address }
                }
                // Pairings outlive a bluetoothd restart; the objects are
                // listed again when it is back.
                ObjectEvent::ServiceGone | ObjectEvent::Changed { .. } => continue,
            };
            inputs
                .unbounded_send(input)
                .map_err(|_| WatchError::ListenerGone)?;
        }
        Ok(())
    };
    futures_util::future::try_join(watching, reporting)
        .await
        .map(drop)
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
    match system
        .call_method(
            Some(SERVICE),
            device.as_str(),
            Some(DEVICE),
            "ConnectProfile",
            &profile,
        )
        .await
    {
        Ok(_) => Ok(()),
        // The Phone connected hands-free again itself, first.
        Err(zbus::Error::MethodError(name, _, _))
            if matches!(
                name.as_str(),
                "org.bluez.Error.AlreadyConnected" | "org.bluez.Error.InProgress"
            ) =>
        {
            Ok(())
        }
        Err(error) => Err(bounce_failed(BounceError::NotReconnected, &error)),
    }
}

/// The object path of BlueZ's device with `address`.
async fn device_path(system: &Connection, address: &str) -> Result<String, BounceError> {
    let objects = async {
        ObjectManagerProxy::builder(system)
            .destination(SERVICE)
            .path(ROOT)
            .build()
            .await?
            .get_managed_objects()
            .await
            .map_err(zbus::Error::from)
    }
    .await
    .map_err(|error| bounce_failed(BounceError::PhoneNotFound, &error))?;
    objects
        .into_iter()
        .find(|(_, interfaces)| {
            interfaces
                .iter()
                .find(|(name, _)| name.as_str() == DEVICE)
                .and_then(|(_, device)| device.get("Address"))
                .and_then(|value| String::try_from(value.clone()).ok())
                .is_some_and(|found| found.eq_ignore_ascii_case(address))
        })
        .map(|(path, _)| path.to_string())
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
