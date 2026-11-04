//! A tree of D-Bus objects served by hand, message by message.
//!
//! zbus's object server writes its own `Introspect` replies, listing every
//! child node. Real PipeWire lists none, and an app that walked the children
//! would work against the object server and fail on the real Phone. So the
//! Simulated Phone answers every method call itself: `Introspect` exactly as
//! the service it stands in for, `Properties`, `ObjectManager`, and whatever
//! the service adds.

use std::collections::{BTreeMap, HashMap};
use std::fmt::Write as _;

use zbus::message::{Header, Type};
use zbus::{Connection, Message, OwnedObjectPath, OwnedVariant, Structure};

/// The properties of one interface on one object.
pub type Properties = BTreeMap<String, OwnedVariant>;

/// The interfaces of one object, with their properties.
pub type Interfaces = BTreeMap<String, Properties>;

/// What `GetManagedObjects` returns.
type ManagedObjects = HashMap<OwnedObjectPath, HashMap<String, HashMap<String, OwnedVariant>>>;

pub const PROPERTIES: &str = "org.freedesktop.DBus.Properties";
pub const OBJECT_MANAGER: &str = "org.freedesktop.DBus.ObjectManager";
pub const INTROSPECTABLE: &str = "org.freedesktop.DBus.Introspectable";

/// One interface's introspection data: its name and its XML members.
pub struct InterfaceSchema {
    pub name: &'static str,
    pub members: &'static str,
}

/// The objects one service exposes on one bus.
pub struct ObjectTree {
    /// Objects by path.
    objects: BTreeMap<String, Interfaces>,
    /// Interfaces each object has beyond those holding properties (such as
    /// `ObjectManager`), by path.
    extra_interfaces: BTreeMap<String, Vec<&'static str>>,
    /// `Introspect` replies captured from the real service, by path, used
    /// verbatim in place of generated ones.
    captured_introspection: BTreeMap<String, &'static str>,
    schemas: &'static [InterfaceSchema],
    /// Whether `Introspect` lists child nodes. BlueZ does; PipeWire doesn't.
    introspect_lists_children: bool,
}

/// What a method call asked for, as the Simulated Phone records it.
#[derive(Clone, Debug)]
pub struct MethodCall {
    pub path: String,
    pub interface: String,
    pub member: String,
    pub arguments: String,
}

impl MethodCall {
    /// Reads the call out of `message`, or `None` if it isn't a method call.
    pub fn from_message(message: &Message) -> Option<Self> {
        let header = message.header();
        if header.message_type() != Type::MethodCall {
            return None;
        }
        let body = message.body();
        let arguments = if body.is_empty() {
            "()".to_owned()
        } else {
            body.deserialize::<Structure<'_>>().map_or_else(
                |error| format!("<{error}>"),
                |arguments| arguments.to_string(),
            )
        };
        Some(Self {
            path: header.path().map(ToString::to_string).unwrap_or_default(),
            interface: header
                .interface()
                .map(ToString::to_string)
                .unwrap_or_default(),
            member: header.member().map(ToString::to_string).unwrap_or_default(),
            arguments,
        })
    }
}

/// The answer to a method call.
pub enum Reply {
    /// Reply with this body.
    Body(Box<dyn ErasedBody>),
    /// Reply with this D-Bus error name and message.
    Error(&'static str, String),
    /// Not a generic call; the service decides.
    Unhandled,
}

/// A reply body of any serializable type.
pub trait ErasedBody: Send {
    fn reply<'a>(
        &'a self,
        connection: &'a Connection,
        header: &'a Header<'a>,
    ) -> std::pin::Pin<Box<dyn Future<Output = zbus::Result<()>> + Send + 'a>>;
}

impl<B> ErasedBody for B
where
    B: zbus::export::serde::Serialize + zbus::DynamicType + Send + Sync,
{
    fn reply<'a>(
        &'a self,
        connection: &'a Connection,
        header: &'a Header<'a>,
    ) -> std::pin::Pin<Box<dyn Future<Output = zbus::Result<()>> + Send + 'a>> {
        Box::pin(connection.reply(header, self))
    }
}

impl Reply {
    pub fn body<B>(body: B) -> Self
    where
        B: zbus::export::serde::Serialize + zbus::DynamicType + Send + Sync + 'static,
    {
        Self::Body(Box::new(body))
    }

    /// The error the real services give for a method an object lacks.
    pub fn unknown_method(call: &MethodCall) -> Self {
        Self::Error(
            "org.freedesktop.DBus.Error.UnknownMethod",
            format!(
                "Unknown method {} on interface {}",
                call.member, call.interface
            ),
        )
    }

    /// Sends this reply to the method call in `header`.
    pub async fn send(self, connection: &Connection, header: &Header<'_>) -> zbus::Result<()> {
        match self {
            Self::Body(body) => body.reply(connection, header).await,
            Self::Error(name, message) => connection.reply_error(header, name, &message).await,
            Self::Unhandled => Ok(()),
        }
    }
}

impl ObjectTree {
    pub fn new(schemas: &'static [InterfaceSchema], introspect_lists_children: bool) -> Self {
        Self {
            objects: BTreeMap::new(),
            extra_interfaces: BTreeMap::new(),
            captured_introspection: BTreeMap::new(),
            schemas,
            introspect_lists_children,
        }
    }

    /// Adds an object, or replaces it.
    pub fn insert(
        &mut self,
        path: &OwnedObjectPath,
        interfaces: Interfaces,
        extra_interfaces: Vec<&'static str>,
    ) {
        self.objects.insert(path.to_string(), interfaces);
        self.extra_interfaces
            .insert(path.to_string(), extra_interfaces);
    }

    /// Makes `Introspect` on `path` answer exactly `xml`, as captured from the
    /// real service.
    pub fn capture_introspection(&mut self, path: &OwnedObjectPath, xml: &'static str) {
        self.captured_introspection.insert(path.to_string(), xml);
    }

    /// Removes an object and returns its interfaces.
    pub fn remove(&mut self, path: &OwnedObjectPath) -> Option<Interfaces> {
        self.extra_interfaces.remove(path.as_str());
        self.captured_introspection.remove(path.as_str());
        self.objects.remove(path.as_str())
    }

    /// Sets one property and returns whether it changed.
    pub fn set(
        &mut self,
        path: &OwnedObjectPath,
        interface: &str,
        name: &str,
        value: OwnedVariant,
    ) -> bool {
        let Some(properties) = self
            .objects
            .get_mut(path.as_str())
            .and_then(|interfaces| interfaces.get_mut(interface))
        else {
            return false;
        };
        let changed = properties.get(name) != Some(&value);
        properties.insert(name.to_owned(), value);
        changed
    }

    /// Answers the calls every object of the service answers alike:
    /// `Introspectable`, `Properties` and `ObjectManager`.
    pub fn answer(&self, call: &MethodCall, message: &Message) -> Reply {
        let path = call.path.as_str();
        let Some(interfaces) = self.objects.get(path) else {
            return Reply::Error(
                "org.freedesktop.DBus.Error.UnknownObject",
                format!("No such object path '{}'", call.path),
            );
        };
        match (call.interface.as_str(), call.member.as_str()) {
            (INTROSPECTABLE, "Introspect") => Reply::body(self.introspect(path)),
            (PROPERTIES, _) if !self.has_interface(path, PROPERTIES) => Reply::unknown_method(call),
            (PROPERTIES, "GetAll") => match message.body().deserialize::<String>() {
                Ok(interface) => Reply::body(
                    interfaces
                        .get(&interface)
                        .map(|properties| properties.clone().into_iter().collect::<HashMap<_, _>>())
                        .unwrap_or_default(),
                ),
                Err(error) => {
                    Reply::Error("org.freedesktop.DBus.Error.InvalidArgs", error.to_string())
                }
            },
            (PROPERTIES, "Get") => match message.body().deserialize::<(String, String)>() {
                Ok((interface, name)) => match interfaces
                    .get(&interface)
                    .and_then(|properties| properties.get(&name))
                {
                    Some(value) => Reply::body(value.clone()),
                    None => Reply::Error(
                        "org.freedesktop.DBus.Error.InvalidArgs",
                        format!("No such property '{name}'"),
                    ),
                },
                Err(error) => {
                    Reply::Error("org.freedesktop.DBus.Error.InvalidArgs", error.to_string())
                }
            },
            (OBJECT_MANAGER, "GetManagedObjects") if self.has_interface(path, OBJECT_MANAGER) => {
                Reply::body(self.managed_objects_below(path))
            }
            _ => Reply::Unhandled,
        }
    }

    fn has_interface(&self, path: &str, interface: &str) -> bool {
        self.objects
            .get(path)
            .is_some_and(|interfaces| interfaces.contains_key(interface))
            || self
                .extra_interfaces
                .get(path)
                .is_some_and(|extra| extra.contains(&interface))
    }

    /// Every object strictly below `path`, as `GetManagedObjects` returns them.
    fn managed_objects_below(&self, path: &str) -> ManagedObjects {
        let prefix = child_prefix(path);
        self.objects
            .iter()
            .filter(|(child, _)| child.starts_with(&prefix))
            .filter_map(|(child, interfaces)| {
                let interfaces = interfaces
                    .iter()
                    .map(|(name, properties)| {
                        (name.clone(), properties.clone().into_iter().collect())
                    })
                    .collect();
                Some((OwnedObjectPath::try_from(child.as_str()).ok()?, interfaces))
            })
            .collect()
    }

    fn introspect(&self, path: &str) -> String {
        if let Some(xml) = self.captured_introspection.get(path) {
            return (*xml).to_owned();
        }
        let mut xml = String::from(
            "<!DOCTYPE node PUBLIC \"-//freedesktop//DTD D-BUS Object Introspection 1.0//EN\"\n\
             \"http://www.freedesktop.org/standards/dbus/1.0/introspect.dtd\">\n<node>\n",
        );
        let mut names: Vec<&str> = self
            .objects
            .get(path)
            .map(|interfaces| interfaces.keys().map(String::as_str).collect())
            .unwrap_or_default();
        if let Some(extra) = self.extra_interfaces.get(path) {
            names.extend(extra.iter().copied());
        }
        let schemas = STANDARD_SCHEMAS.iter().chain(self.schemas);
        for schema in schemas.filter(|schema| names.contains(&schema.name)) {
            let _ = writeln!(
                xml,
                " <interface name=\"{}\">\n{}\n </interface>",
                schema.name, schema.members
            );
        }
        if self.introspect_lists_children {
            for child in self.children(path) {
                let _ = writeln!(xml, " <node name=\"{child}\"/>");
            }
        }
        xml.push_str("</node>\n");
        xml
    }

    /// The names of the nodes directly below `path`.
    fn children(&self, path: &str) -> Vec<String> {
        let prefix = child_prefix(path);
        let mut children: Vec<String> = self
            .objects
            .keys()
            .filter_map(|child| child.strip_prefix(&prefix))
            .filter_map(|rest| rest.split('/').next())
            .filter(|name| !name.is_empty())
            .map(str::to_owned)
            .collect();
        children.dedup();
        children
    }
}

/// What every path below `path` starts with.
fn child_prefix(path: &str) -> String {
    if path == "/" {
        "/".to_owned()
    } else {
        format!("{path}/")
    }
}

/// The introspection data every object of every service has.
pub const STANDARD_SCHEMAS: [InterfaceSchema; 3] = [
    InterfaceSchema {
        name: INTROSPECTABLE,
        members: r#"  <method name="Introspect">
   <arg name="xml" type="s" direction="out"/>
  </method>"#,
    },
    InterfaceSchema {
        name: PROPERTIES,
        members: r#"  <method name="Get">
   <arg name="interface" type="s" direction="in"/>
   <arg name="name" type="s" direction="in"/>
   <arg name="value" type="v" direction="out"/>
  </method>
  <method name="GetAll">
   <arg name="interface" type="s" direction="in"/>
   <arg name="properties" type="a{sv}" direction="out"/>
  </method>
  <method name="Set">
   <arg name="interface" type="s" direction="in"/>
   <arg name="name" type="s" direction="in"/>
   <arg name="value" type="v" direction="in"/>
  </method>
  <signal name="PropertiesChanged">
   <arg name="interface" type="s"/>
   <arg name="changed_properties" type="a{sv}"/>
   <arg name="invalidated_properties" type="as"/>
  </signal>"#,
    },
    InterfaceSchema {
        name: OBJECT_MANAGER,
        members: r#"  <method name="GetManagedObjects">
   <arg name="objects" type="a{oa{sa{sv}}}" direction="out"/>
  </method>
  <signal name="InterfacesAdded">
   <arg name="object" type="o"/>
   <arg name="interfaces" type="a{sa{sv}}"/>
  </signal>
  <signal name="InterfacesRemoved">
   <arg name="object" type="o"/>
   <arg name="interfaces" type="as"/>
  </signal>"#,
    },
];

/// Converts an interface map into the `a{sa{sv}}` that `InterfacesAdded` carries.
pub fn interfaces_added_body(
    interfaces: &Interfaces,
) -> HashMap<String, HashMap<String, OwnedVariant>> {
    interfaces
        .iter()
        .map(|(name, properties)| (name.clone(), properties.clone().into_iter().collect()))
        .collect()
}
