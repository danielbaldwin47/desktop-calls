//! The BlueZ client: which paired Phones exist (ADR-0004).
//!
//! It reports each paired Bluetooth device that offers the Hands-Free Audio
//! Gateway profile, which is what makes it a Phone that can carry Calls.

use std::collections::HashMap;

use desktop_calls_core::{Input, Phone};
use futures_channel::mpsc::{UnboundedSender, unbounded};
use futures_util::StreamExt as _;
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
