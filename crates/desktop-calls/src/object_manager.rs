//! Follows the objects another service exposes through
//! `org.freedesktop.DBus.ObjectManager`, and their property changes.
//!
//! Objects are learned only from `GetManagedObjects` and the
//! `InterfacesAdded` and `InterfacesRemoved` signals, never by walking
//! `Introspect`: PipeWire's Telephony service lists no child nodes there. A
//! service can nest managers (each PipeWire AudioGateway manages its own
//! Calls), so every signal below the root is followed, and a nested manager's
//! objects are listed as soon as it appears. When the service restarts, every
//! object is reported gone, then listed afresh.

use std::collections::HashMap;

use futures_channel::mpsc::UnboundedSender;
use futures_util::StreamExt as _;
use futures_util::{future, stream};
use zbus::fdo::ObjectManagerProxy;
use zbus::message::Type;
use zbus::names::UniqueName;
use zbus::{Connection, MatchRule, Message, MessageStream, OwnedObjectPath, OwnedVariant};

use crate::logging::EventKind;

/// An object's interfaces, each with its properties.
pub type Interfaces = HashMap<String, HashMap<String, OwnedVariant>>;

const OBJECT_MANAGER: &str = "org.freedesktop.DBus.ObjectManager";
const PROPERTIES: &str = "org.freedesktop.DBus.Properties";

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
    /// Properties of one interface of the object at `path` changed.
    Changed {
        path: String,
        interface: String,
        changed: HashMap<String, OwnedVariant>,
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
    Signal(Message),
    Owner(Option<UniqueName<'static>>),
}

/// What to follow.
pub struct Watched {
    pub service: &'static str,
    /// The service's root `ObjectManager`.
    pub root: &'static str,
    /// An interface whose objects are `ObjectManager`s too, listed as soon as
    /// they appear.
    pub nested_manager: Option<&'static str>,
}

/// Sends every change to the objects below `watched.root` to `events`,
/// starting with the objects that exist now. Runs until the connection closes.
pub async fn watch(
    connection: &Connection,
    watched: Watched,
    events: &UnboundedSender<ObjectEvent>,
) -> Result<(), WatchError> {
    let service = watched.service;
    let subscribed = async {
        let manager = ObjectManagerProxy::builder(connection)
            .destination(service)
            .path(watched.root)
            .build()
            .await?;
        // The bus delivers only the service's signals: it resolves the
        // well-known sender itself. Subscribe before listing, so nothing slips
        // between the two.
        let rule = MatchRule::builder()
            .msg_type(Type::Signal)
            .sender(service)
            .path_namespace(watched.root)
            .build()?;
        let signals = MessageStream::for_match_rule(rule, connection, None).await?;
        let owners = manager.inner().receive_owner_changed().await?;
        Ok::<_, zbus::Error>((manager, signals, owners))
    }
    .await;
    let (manager, signals, owners) = subscribed.map_err(|error| {
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
        signals.filter_map(|message| future::ready(message.ok().map(Change::Signal))),
        owners.map(Change::Owner),
    );
    let lister = Lister {
        connection,
        watched: &watched,
        events,
    };

    lister.list(&manager).await?;
    while let Some(change) = changes.next().await {
        match change {
            Change::Signal(message) => lister.signal(&message).await?,
            Change::Owner(owner) => {
                tracing::debug!(
                    event_kind = %EventKind::ServiceOwnerChanged,
                    service,
                    owner = owner.as_ref().map_or("", |owner| owner.as_str()),
                    "{service} changed owner"
                );
                send(events, ObjectEvent::ServiceGone)?;
                if owner.is_some() {
                    lister.list(&manager).await?;
                }
            }
        }
    }
    Ok(())
}

struct Lister<'a> {
    connection: &'a Connection,
    watched: &'a Watched,
    events: &'a UnboundedSender<ObjectEvent>,
}

impl Lister<'_> {
    async fn signal(&self, message: &Message) -> Result<(), WatchError> {
        let header = message.header();
        let (Some(path), Some(interface), Some(member)) =
            (header.path(), header.interface(), header.member())
        else {
            return Ok(());
        };
        match (interface.as_str(), member.as_str()) {
            (OBJECT_MANAGER, "InterfacesAdded") => {
                match message
                    .body()
                    .deserialize::<(OwnedObjectPath, Interfaces)>()
                {
                    Ok((object, interfaces)) => self.added(object.to_string(), interfaces).await?,
                    Err(error) => malformed(self.watched.service, "InterfacesAdded", &error),
                }
            }
            (OBJECT_MANAGER, "InterfacesRemoved") => {
                match message
                    .body()
                    .deserialize::<(OwnedObjectPath, Vec<String>)>()
                {
                    Ok((object, interfaces)) => send(
                        self.events,
                        ObjectEvent::Removed {
                            path: object.to_string(),
                            interfaces,
                        },
                    )?,
                    Err(error) => malformed(self.watched.service, "InterfacesRemoved", &error),
                }
            }
            (PROPERTIES, "PropertiesChanged") => {
                match message
                    .body()
                    .deserialize::<(String, HashMap<String, OwnedVariant>, Vec<String>)>()
                {
                    Ok((interface, changed, _)) => send(
                        self.events,
                        ObjectEvent::Changed {
                            path: path.to_string(),
                            interface,
                            changed,
                        },
                    )?,
                    Err(error) => malformed(self.watched.service, "PropertiesChanged", &error),
                }
            }
            _ => {}
        }
        Ok(())
    }

    /// Reports an object, and if it is a nested manager, its objects too.
    async fn added(&self, path: String, interfaces: Interfaces) -> Result<(), WatchError> {
        let nested = self
            .watched
            .nested_manager
            .is_some_and(|nested| interfaces.contains_key(nested));
        send(
            self.events,
            ObjectEvent::Added {
                path: path.clone(),
                interfaces,
            },
        )?;
        if nested {
            let manager = ObjectManagerProxy::builder(self.connection)
                .destination(self.watched.service)
                .path(path)
                .build()
                .await;
            match manager {
                Ok(manager) => self.list(&manager).await?,
                Err(error) => unavailable(self.watched.service, &error),
            }
        }
        Ok(())
    }

    /// Reports every object the service has now. A service that isn't on the
    /// bus yet has none; its arrival is an owner change.
    async fn list(&self, manager: &ObjectManagerProxy<'_>) -> Result<(), WatchError> {
        match manager.get_managed_objects().await {
            Ok(objects) => {
                for (path, interfaces) in objects {
                    let interfaces = interfaces
                        .into_iter()
                        .map(|(name, properties)| (name.to_string(), properties))
                        .collect();
                    Box::pin(self.added(path.to_string(), interfaces)).await?;
                }
            }
            Err(error) => unavailable(self.watched.service, &error.into()),
        }
        Ok(())
    }
}

fn send(events: &UnboundedSender<ObjectEvent>, event: ObjectEvent) -> Result<(), WatchError> {
    events
        .unbounded_send(event)
        .map_err(|_| WatchError::ListenerGone)
}

fn unavailable(service: &str, error: &zbus::Error) {
    tracing::debug!(
        event_kind = %EventKind::ServiceUnavailable,
        service,
        detail = %error,
        "{service} isn't answering yet"
    );
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
