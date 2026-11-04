//! Follows the objects another service exposes through
//! `org.freedesktop.DBus.ObjectManager`.
//!
//! Objects are learned only from `GetManagedObjects` and the
//! `InterfacesAdded` and `InterfacesRemoved` signals, never by walking
//! `Introspect`: PipeWire's Telephony service lists no child nodes there. When
//! the service restarts, every object is reported gone, then listed afresh.

use std::collections::HashMap;

use futures_channel::mpsc::UnboundedSender;
use futures_util::StreamExt as _;
use futures_util::stream;
use zbus::fdo::{InterfacesAdded, InterfacesRemoved, ObjectManagerProxy};
use zbus::names::UniqueName;
use zbus::{Connection, OwnedVariant};

use crate::logging::EventKind;

/// An object's interfaces, each with its properties.
pub type Interfaces = HashMap<String, HashMap<String, OwnedVariant>>;

/// A change to the service's objects.
pub enum ObjectEvent {
    /// The object at `path` exists with, or gained, these interfaces.
    Added {
        path: String,
        interfaces: Interfaces,
    },
    /// The object at `path` lost these interfaces.
    Removed {
        path: String,
        interfaces: Vec<String>,
    },
    /// The service left the bus: every object it had is gone.
    ServiceGone,
}

/// Why following a service stopped.
#[derive(Clone, Copy, Debug)]
pub enum WatchError {
    /// The bus refused to deliver the service's signals.
    SignalsUnavailable,
    /// Whoever listens to the events has stopped.
    ListenerGone,
}

impl WatchError {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::SignalsUnavailable => "signals-unavailable",
            Self::ListenerGone => "listener-gone",
        }
    }
}

enum Change {
    Added(InterfacesAdded),
    Removed(InterfacesRemoved),
    Owner(Option<UniqueName<'static>>),
}

/// Sends every change to the objects below `root` of `service` to `events`,
/// starting with the objects that exist now. Runs until the connection closes.
pub async fn watch(
    connection: &Connection,
    service: &'static str,
    root: &'static str,
    events: &UnboundedSender<ObjectEvent>,
) -> Result<(), WatchError> {
    let subscribed = async {
        let manager = ObjectManagerProxy::builder(connection)
            .destination(service)
            .path(root)
            .build()
            .await?;
        // Subscribe before listing, so nothing slips between the two.
        let added = manager.receive_interfaces_added().await?;
        let removed = manager.receive_interfaces_removed().await?;
        let owners = manager.inner().receive_owner_changed().await?;
        Ok::<_, zbus::Error>((manager, added, removed, owners))
    }
    .await;
    let (manager, added, removed, owners) = subscribed.map_err(|error| {
        tracing::error!(
            event_kind = %EventKind::WatchFailed,
            error_kind = WatchError::SignalsUnavailable.as_str(),
            service,
            detail = %error,
            "Can't follow {service}'s objects"
        );
        WatchError::SignalsUnavailable
    })?;

    let mut changes = stream::select(
        stream::select(added.map(Change::Added), removed.map(Change::Removed)),
        owners.map(Change::Owner),
    );

    list(&manager, service, events).await?;
    while let Some(change) = changes.next().await {
        match change {
            Change::Added(signal) => match signal.args() {
                Ok(args) => send(
                    events,
                    ObjectEvent::Added {
                        path: args.object_path().to_string(),
                        interfaces: own_interfaces(args.interfaces_and_properties()),
                    },
                )?,
                Err(error) => malformed(service, "InterfacesAdded", &error),
            },
            Change::Removed(signal) => match signal.args() {
                Ok(args) => send(
                    events,
                    ObjectEvent::Removed {
                        path: args.object_path().to_string(),
                        interfaces: args.interfaces().iter().map(ToString::to_string).collect(),
                    },
                )?,
                Err(error) => malformed(service, "InterfacesRemoved", &error),
            },
            Change::Owner(owner) => {
                tracing::debug!(
                    event_kind = %EventKind::ServiceOwnerChanged,
                    service,
                    owner = owner.as_ref().map_or("", |owner| owner.as_str()),
                    "{service} changed owner"
                );
                send(events, ObjectEvent::ServiceGone)?;
                if owner.is_some() {
                    list(&manager, service, events).await?;
                }
            }
        }
    }
    Ok(())
}

/// Reports every object the service has now. A service that isn't on the bus
/// yet has none; its arrival is an owner change.
async fn list(
    manager: &ObjectManagerProxy<'_>,
    service: &str,
    events: &UnboundedSender<ObjectEvent>,
) -> Result<(), WatchError> {
    match manager.get_managed_objects().await {
        Ok(objects) => {
            for (path, interfaces) in objects {
                let interfaces = interfaces
                    .into_iter()
                    .map(|(name, properties)| (name.to_string(), properties))
                    .collect();
                send(
                    events,
                    ObjectEvent::Added {
                        path: path.to_string(),
                        interfaces,
                    },
                )?;
            }
        }
        Err(error) => tracing::debug!(
            event_kind = %EventKind::ServiceUnavailable,
            service,
            detail = %error,
            "{service} isn't answering yet"
        ),
    }
    Ok(())
}

fn send(events: &UnboundedSender<ObjectEvent>, event: ObjectEvent) -> Result<(), WatchError> {
    events
        .unbounded_send(event)
        .map_err(|_| WatchError::ListenerGone)
}

fn own_interfaces(
    interfaces: &HashMap<zbus::names::InterfaceName<'_>, HashMap<&str, zbus::Variant<'_>>>,
) -> Interfaces {
    interfaces
        .iter()
        .map(|(name, properties)| {
            let properties = properties
                .iter()
                .filter_map(|(property, value)| {
                    Some(((*property).to_owned(), OwnedVariant::try_from(value).ok()?))
                })
                .collect();
            (name.to_string(), properties)
        })
        .collect()
}

fn malformed(service: &str, signal: &str, error: &zbus::Error) {
    tracing::warn!(
        event_kind = %EventKind::MalformedSignal,
        error_kind = "malformed-signal",
        service,
        signal,
        detail = %error,
        "Ignored a malformed {signal} from {service}"
    );
}
