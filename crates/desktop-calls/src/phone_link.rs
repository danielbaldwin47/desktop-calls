//! The Phone link: the PipeWire Telephony client.
//!
//! It is attached to a Phone by Bluetooth address and then reports when that
//! Phone connects and disconnects for Calls. What it hides:
//!
//! - AudioGateway objects are found only through `GetManagedObjects` and
//!   `InterfacesAdded`/`InterfacesRemoved` on `/org/pipewire/Telephony`, because
//!   PipeWire's `Introspect` lists no child nodes and `GetModems` carries no
//!   properties.
//! - Their index is not `ag0` (the Pixel's is `ag1`) and changes on every
//!   reconnect, so they are matched by their `Address` property.
//! - An AudioGateway exists exactly while hands-free is fully up, so its
//!   lifetime is the Phone's connection.

use std::collections::HashMap;

use desktop_calls_core::Input;
use futures_channel::mpsc::{UnboundedReceiver, UnboundedSender, unbounded};
use futures_util::StreamExt as _;
use futures_util::stream;
use zbus::Connection;

use crate::logging::EventKind;
use crate::object_manager::{self, ObjectEvent, WatchError};

const SERVICE: &str = "org.pipewire.Telephony";
const MANAGER_PATH: &str = "/org/pipewire/Telephony";
const AUDIO_GATEWAY: &str = "org.pipewire.Telephony.AudioGateway1";

/// What the Phone link can be told to do.
pub enum Command {
    /// Follow the Phone with this Bluetooth address.
    Attach { address: String },
    /// Stop following any Phone.
    Detach,
}

enum Event {
    Command(Command),
    Object(ObjectEvent),
}

/// Runs the Phone link until the connection closes, taking `commands` and
/// reporting to `inputs`.
pub async fn run(
    session: Connection,
    commands: UnboundedReceiver<Command>,
    inputs: UnboundedSender<Input>,
) -> Result<(), WatchError> {
    let (object_sender, objects) = unbounded();
    let watching = async {
        object_manager::watch(&session, SERVICE, MANAGER_PATH, &object_sender).await?;
        drop(object_sender);
        Ok::<_, WatchError>(())
    };
    let following = async {
        let mut events = stream::select(commands.map(Event::Command), objects.map(Event::Object));
        let mut link = PhoneLink::default();
        while let Some(event) = events.next().await {
            for input in link.handle(event) {
                inputs
                    .unbounded_send(input)
                    .map_err(|_| WatchError::ListenerGone)?;
            }
        }
        Ok(())
    };
    futures_util::future::try_join(watching, following)
        .await
        .map(drop)
}

#[derive(Default)]
struct PhoneLink {
    /// Every AudioGateway the service has, by object path: its address.
    audio_gateways: HashMap<String, String>,
    /// The address of the Phone being followed.
    attached: Option<String>,
    /// The AudioGateway of the followed Phone, while it is connected.
    connected: Option<String>,
}

impl PhoneLink {
    fn handle(&mut self, event: Event) -> Vec<Input> {
        match event {
            Event::Command(Command::Attach { address }) => {
                self.attached = Some(address);
                self.connected = None;
                self.adopt_matching_gateway().into_iter().collect()
            }
            Event::Command(Command::Detach) => {
                self.attached = None;
                self.connected = None;
                Vec::new()
            }
            Event::Object(ObjectEvent::Added { path, interfaces }) => {
                let Some(address) = interfaces
                    .get(AUDIO_GATEWAY)
                    .and_then(|properties| properties.get("Address"))
                    .and_then(|address| String::try_from(address.clone()).ok())
                else {
                    return Vec::new();
                };
                tracing::debug!(
                    event_kind = %EventKind::AudioGatewayAdded,
                    phone_address = %address,
                    path = %path,
                    "An AudioGateway appeared"
                );
                self.audio_gateways.insert(path, address);
                self.adopt_matching_gateway().into_iter().collect()
            }
            Event::Object(ObjectEvent::Removed { path, interfaces }) => {
                if !interfaces
                    .iter()
                    .any(|interface| interface == AUDIO_GATEWAY)
                {
                    return Vec::new();
                }
                self.forget(&path).into_iter().collect()
            }
            Event::Object(ObjectEvent::ServiceGone) => {
                let paths: Vec<String> = self.audio_gateways.keys().cloned().collect();
                paths.iter().filter_map(|path| self.forget(path)).collect()
            }
        }
    }

    /// If the followed Phone isn't connected yet but has an AudioGateway,
    /// it is now connected.
    fn adopt_matching_gateway(&mut self) -> Option<Input> {
        if self.connected.is_some() {
            return None;
        }
        let attached = self.attached.as_ref()?;
        let (path, address) = self
            .audio_gateways
            .iter()
            .find(|(_, address)| address.eq_ignore_ascii_case(attached))?;
        self.connected = Some(path.clone());
        Some(Input::PhoneConnected {
            address: address.clone(),
        })
    }

    /// The AudioGateway at `path` is gone. If it was the followed Phone's, the
    /// Phone disconnected, unless another of its AudioGateways remains.
    fn forget(&mut self, path: &str) -> Option<Input> {
        let address = self.audio_gateways.remove(path)?;
        tracing::debug!(
            event_kind = %EventKind::AudioGatewayRemoved,
            phone_address = %address,
            path,
            "An AudioGateway disappeared"
        );
        if self.connected.as_deref() != Some(path) {
            return None;
        }
        self.connected = None;
        match self.adopt_matching_gateway() {
            Some(_) => None,
            None => Some(Input::PhoneDisconnected { address }),
        }
    }
}
