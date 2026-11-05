//! `org.bluez.obex` on the session bus: obexd's client, through which the
//! app reads the Phone's Contacts and Call History over PBAP. Shaped like
//! obexd from BlueZ 5.87 talking to the real Pixel 9a (D-Bus snapshot,
//! `pbap-session.json` and `obex-client.json`), and, where nothing was
//! captured, like obexd's source (`obexd/client/{manager,session,pbap,
//! transfer}.c` at 5.87).
//!
//! - `/org/bluez/obex` has `Client1` and `AgentManager1`, and no
//!   `Properties`. Its `Introspect` reply is captured from the real obexd.
//!   `GetManagedObjects` on `/` lists every interface of each object.
//! - `Client1.CreateSession` to the Phone with `Target` `pbap` asks the
//!   person holding the Phone whether this Desktop may read Contacts, as
//!   Android does. Granted, it opens `/org/bluez/obex/client/sessionN`, with
//!   `Session1` and `PhonebookAccess1`, announced by `InterfacesAdded` on
//!   `/`; the session's `Introspect` reply is captured from the real obexd.
//!   Denied, it fails with `org.bluez.obex.Error.Forbidden`. Ignored, it
//!   fails after obexd's 10 s OBEX timeout with `org.bluez.obex.Error.Failed`
//!   `Timed out waiting for response`, as the real one did on 2026-11-04.
//! - `Session1.Target` is the PBAP server's UUID, not `pbap`, and
//!   `PhonebookAccess1.Folder` appears only after `Select`. Selecting a
//!   folder the Phone lacks fails with `Failed` `Not Found`; selecting a
//!   name that isn't a PBAP folder fails with `InvalidArguments`, as on the
//!   Pixel.
//! - `GetSize` and `PullAll` before `Select` fail with `Forbidden` `Call
//!   Select first of all`. `GetSize` gives how many vCards the folder holds.
//! - `PullAll` queues a transfer, `sessionN/transferM` with `Transfer1`
//!   (captured from the real obexd on 2026-11-05, `obex-transfer.json`), and
//!   replies with its path and properties, `Status` `queued`, without
//!   `Transferred`. The transfer goes `active`, writes the vCards to the
//!   target file (mode 0600), reports `Transferred`, then `Status`
//!   `complete`, and is removed.
//! - `PullAll` honours the `Offset` and `MaxCount` filters, and, like the
//!   Pixel, gives at most 50 vCards from a Call History folder however
//!   many it holds: a client reads the rest page by page.
//! - `RemoveSession` closes a session and its transfers.
//!
//! Each folder holds whatever vCard text a test gives it, served as is
//! whichever format is asked for, a vCard at a time.

use std::collections::{BTreeMap, HashMap};
use std::fs::OpenOptions;
use std::io::Write as _;
use std::os::unix::fs::OpenOptionsExt as _;
use std::path::PathBuf;
use std::time::Duration;

use zbus::{Message, OwnedObjectPath, OwnedVariant, Str};

use crate::objects::{
    INTROSPECTABLE, Interfaces, MethodCall, OBJECT_MANAGER, ObjectTree, PROPERTIES, Properties,
    Reply,
};
use crate::telephony::Signal;

pub const SERVICE: &str = "org.bluez.obex";
const CLIENT_PATH: &str = "/org/bluez/obex";
/// What every session's path starts with.
const SESSION_PREFIX: &str = "/org/bluez/obex/client/session";
const CLIENT: &str = "org.bluez.obex.Client1";
const AGENT_MANAGER: &str = "org.bluez.obex.AgentManager1";
const SESSION: &str = "org.bluez.obex.Session1";
const PHONEBOOK_ACCESS: &str = "org.bluez.obex.PhonebookAccess1";
const TRANSFER: &str = "org.bluez.obex.Transfer1";

const ERROR_FAILED: &str = "org.bluez.obex.Error.Failed";
const ERROR_FORBIDDEN: &str = "org.bluez.obex.Error.Forbidden";
const ERROR_INVALID_ARGUMENTS: &str = "org.bluez.obex.Error.InvalidArguments";

/// The Phone Book Server Equipment's UUID, which `Session1.Target` reports.
const PBAP_SERVER_UUID: &str = "0000112f-0000-1000-8000-00805f9b34fb";

/// The folders obexd lets a session select, in the internal memory.
const SELECTABLE_FOLDERS: [&str; 7] = ["pb", "ich", "och", "mch", "cch", "spd", "fav"];

/// The Call History folders, which the Pixel serves at most
/// [`HISTORY_PULL_CAP`] vCards of per pull.
const HISTORY_FOLDERS: [&str; 4] = ["cch", "ich", "och", "mch"];

/// The most vCards one pull from a Call History folder gives on the Pixel,
/// whatever `MaxCount` asks for (2026-11-05: `GetSize` gave 922 for `cch`,
/// and `PullAll` 50).
const HISTORY_PULL_CAP: usize = 50;

/// How long obexd waits for the Phone to answer an OBEX request.
const OBEX_TIMEOUT: Duration = Duration::from_secs(10);

/// `Introspect` on `/org/bluez/obex`, captured from the real obexd.
const CLIENT_INTROSPECTION: &str = include_str!("../introspection/obex-client.xml");

/// `Introspect` on a PBAP session, captured from the real obexd.
const SESSION_INTROSPECTION: &str = include_str!("../introspection/obex-session.xml");

/// `Introspect` on a transfer, captured from the real obexd.
const TRANSFER_INTROSPECTION: &str = include_str!("../introspection/obex-transfer.xml");

/// How the person holding the Phone answers its prompt to share Contacts.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Prompt {
    #[default]
    Grant,
    Deny,
    Ignore,
}

impl Prompt {
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "grant" => Some(Self::Grant),
            "deny" => Some(Self::Deny),
            "ignore" => Some(Self::Ignore),
            _ => None,
        }
    }
}

/// obexd's objects, the Phone's folders, and how the Phone behaves.
pub struct Obex {
    tree: ObjectTree,
    next_session_index: u32,
    next_transfer_index: u32,
    /// Each folder the Phone has, by name, with its vCard text.
    folders: BTreeMap<String, String>,
    pub prompt: Prompt,
    pub pull_delay: Duration,
    /// Sessions from `CreateSession` until they fail or are removed.
    sessions_open: u32,
    pub most_sessions_at_once: u32,
}

/// What obexd does some time after a method call.
pub enum Later {
    /// The Phone never answered `CreateSession`.
    SessionTimedOut,
    /// A pull finishes.
    Transfer {
        transfer: OwnedObjectPath,
        filename: PathBuf,
        vcards: String,
    },
}

/// obexd's answer to a method call: the reply, the signals that follow it,
/// and what happens later, after how long.
pub struct Answer {
    pub reply: Reply,
    pub signals: Vec<Signal>,
    pub later: Option<(Duration, Later)>,
}

impl From<Reply> for Answer {
    fn from(reply: Reply) -> Self {
        Self {
            reply,
            signals: Vec::new(),
            later: None,
        }
    }
}

/// obexd with no session open, talking to a Phone whose folders are all
/// there and empty.
pub fn new() -> zbus::Result<Obex> {
    let mut tree = ObjectTree::new(true);
    tree.list_every_interface();
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
        next_transfer_index: 0,
        folders: ["pb", "ich", "och", "mch", "cch"]
            .into_iter()
            .map(|name| (name.to_owned(), String::new()))
            .collect(),
        prompt: Prompt::default(),
        pull_delay: Duration::ZERO,
        sessions_open: 0,
        most_sessions_at_once: 0,
    })
}

impl Obex {
    /// Gives the Phone `folder` with exactly `vcards`.
    pub fn set_pbap_folder(&mut self, folder: &str, vcards: String) {
        self.folders.insert(folder.to_owned(), vcards);
    }

    /// Takes `folder` away from the Phone.
    pub fn remove_pbap_folder(&mut self, folder: &str) {
        self.folders.remove(folder);
    }

    fn session_opened(&mut self) {
        self.sessions_open += 1;
        self.most_sessions_at_once = self.most_sessions_at_once.max(self.sessions_open);
    }

    fn session_closed(&mut self) {
        self.sessions_open = self.sessions_open.saturating_sub(1);
    }
}

/// Answers a method call on obexd.
pub fn answer(
    obex: &mut Obex,
    phone_address: &str,
    call: &MethodCall,
    message: &Message,
) -> Answer {
    match obex.tree.answer(call, message) {
        Reply::Unhandled => {}
        reply => return reply.into(),
    }
    match (call.interface.as_str(), call.member.as_str()) {
        (CLIENT, "CreateSession") if call.path == CLIENT_PATH => {
            create_session(obex, phone_address, message)
        }
        (CLIENT, "RemoveSession") if call.path == CLIENT_PATH => remove_session(obex, message),
        (PHONEBOOK_ACCESS, "Select") if call.path.starts_with(SESSION_PREFIX) => {
            select(obex, call, message).into()
        }
        (PHONEBOOK_ACCESS, "PullAll") if call.path.starts_with(SESSION_PREFIX) => {
            pull_all(obex, call, message)
        }
        (PHONEBOOK_ACCESS, "GetSize") if call.path.starts_with(SESSION_PREFIX) => {
            get_size(obex, call).into()
        }
        _ => Reply::unknown_method(call).into(),
    }
}

/// Does what obexd does `later`, and returns the reply still owed, if any,
/// and the signals to send.
pub fn run_later(obex: &mut Obex, later: Later) -> (Reply, Vec<Signal>) {
    match later {
        Later::SessionTimedOut => {
            obex.session_closed();
            (
                Reply::Error(ERROR_FAILED, "Timed out waiting for response".to_owned()),
                Vec::new(),
            )
        }
        Later::Transfer {
            transfer,
            filename,
            vcards,
        } => (
            Reply::Unhandled,
            finish_transfer(obex, &transfer, &filename, &vcards),
        ),
    }
}

fn create_session(obex: &mut Obex, phone_address: &str, message: &Message) -> Answer {
    let Ok((destination, arguments)) = message
        .body()
        .deserialize::<(String, HashMap<String, OwnedVariant>)>()
    else {
        return invalid_arguments("Invalid arguments in method call").into();
    };
    let target = arguments
        .get("Target")
        .and_then(|target| String::try_from(target.clone()).ok());
    if !target.is_some_and(|target| target.eq_ignore_ascii_case("pbap")) {
        return invalid_arguments("Invalid target").into();
    }
    if !destination.eq_ignore_ascii_case(phone_address) {
        return Reply::Error(ERROR_FAILED, "Unable to connect".to_owned()).into();
    }
    match obex.prompt {
        Prompt::Grant => {}
        Prompt::Deny => {
            return Reply::Error(ERROR_FORBIDDEN, "Forbidden".to_owned()).into();
        }
        Prompt::Ignore => {
            obex.session_opened();
            return Answer {
                reply: Reply::Unhandled,
                signals: Vec::new(),
                later: Some((OBEX_TIMEOUT, Later::SessionTimedOut)),
            };
        }
    }
    let Ok(path) =
        OwnedObjectPath::try_from(format!("{SESSION_PREFIX}{}", obex.next_session_index))
    else {
        return invalid_arguments("Invalid session").into();
    };
    obex.next_session_index += 1;
    obex.session_opened();

    let mut session = Properties::new();
    session.insert("Destination".into(), text(phone_address));
    session.insert("Channel".into(), OwnedVariant::from(0_u8));
    session.insert("PSM".into(), OwnedVariant::from(0_u16));
    session.insert("Target".into(), text(PBAP_SERVER_UUID));
    let zeroes = "0".repeat(32);
    let mut folder = Properties::new();
    folder.insert("DatabaseIdentifier".into(), text(&zeroes));
    folder.insert("PrimaryCounter".into(), text(&zeroes));
    folder.insert("SecondaryCounter".into(), text(&zeroes));
    folder.insert("FixedImageSize".into(), OwnedVariant::from(true));
    let interfaces = Interfaces::from([
        (SESSION.to_owned(), session),
        (PHONEBOOK_ACCESS.to_owned(), folder),
    ]);
    obex.tree
        .insert(&path, interfaces.clone(), vec![PROPERTIES, INTROSPECTABLE]);
    obex.tree
        .capture_introspection(&path, SESSION_INTROSPECTION);
    Answer {
        reply: Reply::body(path.clone()),
        signals: vec![Signal::InterfacesAdded {
            manager: root(),
            object: path,
            interfaces,
        }],
        later: None,
    }
}

/// `RemoveSession(session)`: closes the session and its transfers.
fn remove_session(obex: &mut Obex, message: &Message) -> Answer {
    let Ok(session) = message.body().deserialize::<OwnedObjectPath>() else {
        return invalid_arguments("").into();
    };
    if !session.as_str().starts_with(SESSION_PREFIX) || obex.tree.remove(&session).is_none() {
        return invalid_arguments("").into();
    }
    let mut signals = Vec::new();
    let transfers: Vec<OwnedObjectPath> = obex
        .tree
        .objects_below(session.as_str())
        .into_iter()
        .map(|(transfer, _)| transfer)
        .collect();
    for transfer in transfers {
        obex.tree.remove(&transfer);
        signals.push(Signal::InterfacesRemoved {
            manager: root(),
            object: transfer,
            interfaces: vec![TRANSFER],
        });
    }
    obex.session_closed();
    signals.push(Signal::InterfacesRemoved {
        manager: root(),
        object: session,
        interfaces: vec![SESSION, PHONEBOOK_ACCESS],
    });
    Answer {
        reply: Reply::body(()),
        signals,
        later: None,
    }
}

/// `Select(location, folder)`: picks the folder later pulls read from.
fn select(obex: &mut Obex, call: &MethodCall, message: &Message) -> Reply {
    let Ok((location, folder)) = message.body().deserialize::<(String, String)>() else {
        return invalid_arguments("Invalid arguments in method call");
    };
    let root = match location.to_ascii_lowercase().as_str() {
        "int" | "internal" => "",
        "sim1" => "/SIM1",
        _ => return invalid_arguments("Invalid location"),
    };
    let folder = folder.to_ascii_lowercase();
    if !SELECTABLE_FOLDERS.contains(&folder.as_str()) {
        return invalid_arguments("Invalid path");
    }
    let Ok(path) = OwnedObjectPath::try_from(call.path.as_str()) else {
        return invalid_arguments("Invalid session");
    };
    // Only sessions have PhonebookAccess1, and the tree has already answered
    // for paths with no object. obexd keeps the new folder even when the
    // Phone refuses it.
    let folder_path = format!("{root}/telecom/{folder}");
    obex.tree
        .set(&path, PHONEBOOK_ACCESS, "Folder", text(&folder_path));
    if root.is_empty() && obex.folders.contains_key(&folder) {
        Reply::body(())
    } else {
        Reply::Error(ERROR_FAILED, "Not Found".to_owned())
    }
}

/// `PullAll(targetfile, filters)`: queues a transfer of the selected
/// folder into `targetfile`.
fn pull_all(obex: &mut Obex, call: &MethodCall, message: &Message) -> Answer {
    let Ok((targetfile, filters)) = message
        .body()
        .deserialize::<(String, HashMap<String, OwnedVariant>)>()
    else {
        return invalid_arguments("").into();
    };
    let (session, folder, held) = match selected(obex, call) {
        Ok(selected) => selected,
        Err(reply) => return reply.into(),
    };
    if !filters_are_valid(&filters) {
        return invalid_arguments("").into();
    }
    let filter = |name: &str| {
        filters
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(name))
            .and_then(|(_, value)| u16::try_from(value.clone()).ok())
            .map(usize::from)
    };
    let mut most = filter("MaxCount").unwrap_or(usize::from(u16::MAX));
    if HISTORY_FOLDERS.contains(&folder.as_str()) {
        most = most.min(HISTORY_PULL_CAP);
    }
    let vcards: String = vcards_in(&held)
        .into_iter()
        .skip(filter("Offset").unwrap_or(0))
        .take(most)
        .collect();
    let folder = format!("/telecom/{folder}");
    let filename = if targetfile.is_empty() {
        std::env::temp_dir().join(format!("obex-client{}", obex.next_transfer_index))
    } else {
        PathBuf::from(&targetfile)
    };
    let Ok(transfer) =
        OwnedObjectPath::try_from(format!("{session}/transfer{}", obex.next_transfer_index))
    else {
        return invalid_arguments("Invalid session").into();
    };
    obex.next_transfer_index += 1;

    let mut properties = Properties::new();
    properties.insert("Status".into(), text("queued"));
    properties.insert(
        "Name".into(),
        text(&format!("{}.vcf", folder.trim_start_matches('/'))),
    );
    properties.insert("Size".into(), OwnedVariant::from(0_u64));
    properties.insert("Filename".into(), text(&filename.to_string_lossy()));
    let Ok(session_value) = OwnedVariant::try_from(zbus::Variant::from(session.clone())) else {
        return invalid_arguments("Invalid session").into();
    };
    properties.insert("Session".into(), session_value);
    let interfaces = Interfaces::from([(TRANSFER.to_owned(), properties.clone())]);
    obex.tree.insert(
        &transfer,
        interfaces.clone(),
        vec![PROPERTIES, INTROSPECTABLE],
    );
    obex.tree
        .capture_introspection(&transfer, TRANSFER_INTROSPECTION);
    // The session is idle, so the transfer starts at once.
    obex.tree.set(&transfer, TRANSFER, "Status", text("active"));
    obex.tree.set(
        &transfer,
        TRANSFER,
        "Transferred",
        OwnedVariant::from(0_u64),
    );
    Answer {
        reply: Reply::body((
            transfer.clone(),
            properties.into_iter().collect::<HashMap<_, _>>(),
        )),
        signals: vec![
            Signal::InterfacesAdded {
                manager: root(),
                object: transfer.clone(),
                interfaces,
            },
            Signal::PropertiesChanged {
                path: transfer.clone(),
                interface: TRANSFER,
                changed: Properties::from([
                    ("Status".to_owned(), text("active")),
                    ("Transferred".to_owned(), OwnedVariant::from(0_u64)),
                ]),
            },
        ],
        later: Some((
            obex.pull_delay,
            Later::Transfer {
                transfer,
                filename,
                vcards,
            },
        )),
    }
}

/// `GetSize()`: how many vCards the selected folder holds.
fn get_size(obex: &Obex, call: &MethodCall) -> Reply {
    match selected(obex, call) {
        Ok((_, _, held)) => Reply::body(u16::try_from(vcards_in(&held).len()).unwrap_or(u16::MAX)),
        Err(reply) => reply,
    }
}

/// The session `call` is on, the folder it has selected, and that folder's
/// vCard text; or the error obexd gives when there is none.
fn selected(obex: &Obex, call: &MethodCall) -> Result<(OwnedObjectPath, String, String), Reply> {
    let Ok(session) = OwnedObjectPath::try_from(call.path.as_str()) else {
        return Err(invalid_arguments("Invalid session"));
    };
    let Some(path) = obex
        .tree
        .get(&session, PHONEBOOK_ACCESS, "Folder")
        .and_then(|folder| String::try_from(folder.clone()).ok())
    else {
        return Err(Reply::Error(
            ERROR_FORBIDDEN,
            "Call Select first of all".to_owned(),
        ));
    };
    let Some((folder, held)) = path
        .strip_prefix("/telecom/")
        .and_then(|folder| obex.folders.get_key_value(folder))
    else {
        return Err(Reply::Error(ERROR_FAILED, "Not Found".to_owned()));
    };
    Ok((session, folder.clone(), held.clone()))
}

/// Each top-level vCard in `text`, as written, with its line breaks. Lines
/// outside any vCard are dropped.
fn vcards_in(text: &str) -> Vec<&str> {
    let mut vcards = Vec::new();
    let mut depth = 0_u32;
    let mut start = 0;
    let mut offset = 0;
    for line in text.split_inclusive('\n') {
        let content = line.trim_end();
        if content.eq_ignore_ascii_case("BEGIN:VCARD") {
            if depth == 0 {
                start = offset;
            }
            depth += 1;
        } else if content.eq_ignore_ascii_case("END:VCARD") && depth > 0 {
            depth -= 1;
            if depth == 0 {
                vcards.push(&text[start..offset + line.len()]);
            }
        }
        offset += line.len();
    }
    vcards
}

/// The filters obexd accepts: `Format`, `Order`, `Offset`, `MaxCount`,
/// `Fields` and `FilterAll`, each of its own type.
fn filters_are_valid(filters: &HashMap<String, OwnedVariant>) -> bool {
    filters.iter().all(|(name, value)| {
        let signature = value.value_signature().to_string();
        match name.to_ascii_lowercase().as_str() {
            "format" => String::try_from(value.clone()).is_ok_and(|format| {
                matches!(format.to_ascii_lowercase().as_str(), "vcard21" | "vcard30")
            }),
            "order" => signature == "s",
            "offset" | "maxcount" => signature == "q",
            "fields" => signature == "as",
            "filterall" => signature == "b",
            _ => false,
        }
    })
}

/// Writes the pulled folder as obexd does, then reports the transfer
/// complete and removes it. A transfer whose session has gone writes
/// nothing.
fn finish_transfer(
    obex: &mut Obex,
    transfer: &OwnedObjectPath,
    filename: &PathBuf,
    vcards: &str,
) -> Vec<Signal> {
    if obex.tree.remove(transfer).is_none() {
        return Vec::new();
    }
    let written = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(filename)
        .and_then(|mut file| file.write_all(vcards.as_bytes()));
    let size = u64::try_from(vcards.len()).unwrap_or(u64::MAX);
    let status = if written.is_ok() { "complete" } else { "error" };
    let mut signals = Vec::new();
    if written.is_ok() {
        signals.push(Signal::PropertiesChanged {
            path: transfer.clone(),
            interface: TRANSFER,
            changed: Properties::from([
                ("Size".to_owned(), OwnedVariant::from(size)),
                ("Transferred".to_owned(), OwnedVariant::from(size)),
            ]),
        });
    }
    signals.push(Signal::PropertiesChanged {
        path: transfer.clone(),
        interface: TRANSFER,
        changed: Properties::from([("Status".to_owned(), text(status))]),
    });
    signals.push(Signal::InterfacesRemoved {
        manager: root(),
        object: transfer.clone(),
        interfaces: vec![TRANSFER],
    });
    signals
}

fn root() -> OwnedObjectPath {
    OwnedObjectPath::try_from("/").unwrap_or_default()
}

fn invalid_arguments(message: &str) -> Reply {
    Reply::Error(ERROR_INVALID_ARGUMENTS, message.to_owned())
}

fn text(value: &str) -> OwnedVariant {
    OwnedVariant::from(Str::from(value.to_owned()))
}
