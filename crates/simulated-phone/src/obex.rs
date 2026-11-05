//! `org.bluez.obex` on the session bus: obexd's client, through which the
//! app reads the Phone's Contacts and Call History over PBAP. Shaped like
//! obexd from BlueZ 5.87 talking to the real Pixel 9a (D-Bus snapshot,
//! `pbap-session.json`).
//!
//! - `/org/bluez/obex` has `Client1` and `AgentManager1`, and no
//!   `Properties`. Its `Introspect` reply is captured from the real obexd.
//! - `Client1.CreateSession` to the Phone with `Target` `pbap` opens
//!   `/org/bluez/obex/client/sessionN`, with `Session1` and
//!   `PhonebookAccess1`, announced by `InterfacesAdded` on `/`. The session's
//!   `Introspect` reply is captured from the real obexd.
//! - `Session1.Target` is the PBAP server's UUID, not `pbap`, and
//!   `PhonebookAccess1.Folder` appears only after `Select`.
//!
//! So far the Phone grants every session at once, and no phonebook can be
//! pulled yet.

use zbus::{Message, OwnedObjectPath, OwnedVariant, Str};

use crate::objects::{
    INTROSPECTABLE, Interfaces, MethodCall, OBJECT_MANAGER, ObjectTree, PROPERTIES, Properties,
    Reply,
};

pub const SERVICE: &str = "org.bluez.obex";
const CLIENT_PATH: &str = "/org/bluez/obex";
/// What every session's path starts with.
const SESSION_PREFIX: &str = "/org/bluez/obex/client/session";
const CLIENT: &str = "org.bluez.obex.Client1";
const AGENT_MANAGER: &str = "org.bluez.obex.AgentManager1";
const SESSION: &str = "org.bluez.obex.Session1";
const PHONEBOOK_ACCESS: &str = "org.bluez.obex.PhonebookAccess1";

/// The Phone Book Server Equipment's UUID, which `Session1.Target` reports.
const PBAP_SERVER_UUID: &str = "0000112f-0000-1000-8000-00805f9b34fb";

/// `Introspect` on `/org/bluez/obex`, captured from the real obexd.
const CLIENT_INTROSPECTION: &str = include_str!("../introspection/obex-client.xml");

/// `Introspect` on a PBAP session, captured from the real obexd.
const SESSION_INTROSPECTION: &str = include_str!("../introspection/obex-session.xml");

/// obexd's objects, and the number of the next session.
pub struct Obex {
    tree: ObjectTree,
    next_session_index: u32,
}

/// obexd with no session open.
pub fn new() -> zbus::Result<Obex> {
    let mut tree = ObjectTree::new(true);
    tree.insert(
        &OwnedObjectPath::try_from("/")?,
        Interfaces::new(),
        vec![OBJECT_MANAGER, INTROSPECTABLE],
    );
    for path in ["/org", "/org/bluez"] {
        tree.insert(
            &OwnedObjectPath::try_from(path)?,
            Interfaces::new(),
            vec![INTROSPECTABLE],
        );
    }
    let client = OwnedObjectPath::try_from(CLIENT_PATH)?;
    tree.insert(
        &client,
        Interfaces::new(),
        vec![CLIENT, AGENT_MANAGER, INTROSPECTABLE],
    );
    tree.capture_introspection(&client, CLIENT_INTROSPECTION);
    Ok(Obex {
        tree,
        next_session_index: 0,
    })
}

/// Answers a method call on obexd. A new session comes back too, with its
/// interfaces, for `InterfacesAdded`.
pub fn answer(
    obex: &mut Obex,
    phone_address: &str,
    call: &MethodCall,
    message: &Message,
) -> (Reply, Option<(OwnedObjectPath, Interfaces)>) {
    match obex.tree.answer(call, message) {
        Reply::Unhandled => {}
        reply => return (reply, None),
    }
    match (call.interface.as_str(), call.member.as_str()) {
        (CLIENT, "CreateSession") if call.path == CLIENT_PATH => {
            create_session(obex, phone_address, message)
        }
        (PHONEBOOK_ACCESS, "Select") if call.path.starts_with(SESSION_PREFIX) => {
            (select(obex, call, message), None)
        }
        _ => (Reply::unknown_method(call), None),
    }
}

fn create_session(
    obex: &mut Obex,
    phone_address: &str,
    message: &Message,
) -> (Reply, Option<(OwnedObjectPath, Interfaces)>) {
    let Ok((destination, arguments)) = message
        .body()
        .deserialize::<(String, std::collections::HashMap<String, OwnedVariant>)>()
    else {
        return (invalid_arguments("Invalid arguments in method call"), None);
    };
    let target = arguments
        .get("Target")
        .and_then(|target| String::try_from(target.clone()).ok());
    if !target.is_some_and(|target| target.eq_ignore_ascii_case("pbap")) {
        return (invalid_arguments("Invalid target"), None);
    }
    if !destination.eq_ignore_ascii_case(phone_address) {
        return (
            Reply::Error(
                "org.bluez.obex.Error.Failed",
                "Unable to connect".to_owned(),
            ),
            None,
        );
    }
    let Ok(path) =
        OwnedObjectPath::try_from(format!("{SESSION_PREFIX}{}", obex.next_session_index))
    else {
        return (invalid_arguments("Invalid session"), None);
    };
    obex.next_session_index += 1;

    let mut session = Properties::new();
    session.insert("Destination".into(), text(phone_address));
    session.insert("Channel".into(), OwnedVariant::from(0_u8));
    session.insert("PSM".into(), OwnedVariant::from(0_u16));
    session.insert("Target".into(), text(PBAP_SERVER_UUID));
    let zeroes = "0".repeat(32);
    let mut phonebook = Properties::new();
    phonebook.insert("DatabaseIdentifier".into(), text(&zeroes));
    phonebook.insert("PrimaryCounter".into(), text(&zeroes));
    phonebook.insert("SecondaryCounter".into(), text(&zeroes));
    phonebook.insert("FixedImageSize".into(), OwnedVariant::from(true));
    let interfaces = Interfaces::from([
        (SESSION.to_owned(), session),
        (PHONEBOOK_ACCESS.to_owned(), phonebook),
    ]);
    obex.tree
        .insert(&path, interfaces.clone(), vec![PROPERTIES, INTROSPECTABLE]);
    obex.tree
        .capture_introspection(&path, SESSION_INTROSPECTION);
    (Reply::body(path.clone()), Some((path, interfaces)))
}

/// `Select(location, phonebook)`: picks the folder later pulls read from.
fn select(obex: &mut Obex, call: &MethodCall, message: &Message) -> Reply {
    let Ok((location, phonebook)) = message.body().deserialize::<(String, String)>() else {
        return invalid_arguments("Invalid arguments in method call");
    };
    let root = match location.to_ascii_lowercase().as_str() {
        "int" | "internal" => "",
        "sim1" => "/SIM1",
        _ => return invalid_arguments("Invalid location"),
    };
    let Ok(path) = OwnedObjectPath::try_from(call.path.as_str()) else {
        return invalid_arguments("Invalid session");
    };
    // Only sessions have PhonebookAccess1, and the tree has already answered
    // for paths with no object.
    let folder = format!("{root}/telecom/{}", phonebook.to_ascii_lowercase());
    obex.tree
        .set(&path, PHONEBOOK_ACCESS, "Folder", text(&folder));
    Reply::body(())
}

fn invalid_arguments(message: &str) -> Reply {
    Reply::Error("org.bluez.obex.Error.InvalidArguments", message.to_owned())
}

fn text(value: &str) -> OwnedVariant {
    OwnedVariant::from(Str::from(value.to_owned()))
}
