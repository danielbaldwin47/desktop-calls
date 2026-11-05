//! The obexd client: PBAP sessions to the Phone, and pulls from them.
//!
//! - A pull is complete when its transfer's `Status` goes `complete`, which
//!   obexd reports by `PropertiesChanged` and then removes the transfer. So
//!   the signals are subscribed to before `PullAll` is called.
//! - Errors are sorted by what they mean for the app: the Phone refusing
//!   (`Forbidden`, as an error name or an OBEX response), the Phone not
//!   answering in time, a folder the Phone lacks, or obexd missing.

use std::collections::HashMap;
use std::path::Path;
use std::time::Duration;

use futures_util::StreamExt as _;
use futures_util::future::{Either, select};
use zbus::message::Type;
use zbus::{Connection, MatchRule, MessageStream, OwnedObjectPath, OwnedVariant, Variant};

const SERVICE: &str = "org.bluez.obex";
const CLIENT_PATH: &str = "/org/bluez/obex";
const CLIENT: &str = "org.bluez.obex.Client1";
const PHONEBOOK_ACCESS: &str = "org.bluez.obex.PhonebookAccess1";
const TRANSFER: &str = "org.bluez.obex.Transfer1";
const PROPERTIES: &str = "org.freedesktop.DBus.Properties";

/// How long a pull may go without obexd reporting progress.
const PULL_SILENCE: Duration = Duration::from_secs(60);

/// Why an obexd call failed.
#[derive(Debug)]
pub enum ObexError {
    /// The Phone refuses to share Contacts.
    Forbidden,
    /// The Phone didn't answer in time, as when its prompt is still up.
    TimedOut,
    /// The Phone lacks the folder.
    NotFound,
    /// obexd isn't on the session bus.
    NoService,
    /// The transfer stopped with `Status` `error`.
    TransferFailed,
    /// obexd reported no progress for [`PULL_SILENCE`].
    TransferStalled,
    /// Anything else, with obexd's error.
    Other(String),
}

impl ObexError {
    /// The kind logs carry as `error_kind`.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Forbidden => "forbidden",
            Self::TimedOut => "timed-out",
            Self::NotFound => "not-found",
            Self::NoService => "no-contacts-service",
            Self::TransferFailed => "transfer-failed",
            Self::TransferStalled => "transfer-stalled",
            Self::Other(_) => "obex-error",
        }
    }

    /// The error as obexd gave it, for logs.
    pub fn detail(&self) -> &str {
        match self {
            Self::Other(detail) => detail,
            _ => "",
        }
    }
}

impl From<zbus::Error> for ObexError {
    fn from(error: zbus::Error) -> Self {
        let zbus::Error::MethodError(name, message, _) = &error else {
            return Self::Other(error.to_string());
        };
        let message = message.as_deref().unwrap_or_default();
        match name.as_str() {
            "org.freedesktop.DBus.Error.ServiceUnknown"
            | "org.freedesktop.DBus.Error.NameHasNoOwner" => Self::NoService,
            "org.bluez.obex.Error.Forbidden" => Self::Forbidden,
            _ if is_forbidden_response(message) => Self::Forbidden,
            _ if message.eq_ignore_ascii_case("Not Found") => Self::NotFound,
            _ if message.contains("Timed out") => Self::TimedOut,
            _ => Self::Other(format!("{name}: {message}")),
        }
    }
}

/// Whether obexd's message relays the Phone's OBEX `Forbidden` (0x43, 0xC3
/// with the final bit) or `Unauthorized` (0x41, 0xC1).
fn is_forbidden_response(message: &str) -> bool {
    let lower = message.to_ascii_lowercase();
    lower.contains("forbidden")
        || lower.contains("unauthorized")
        || ["0x43", "0xc3", "0x41", "0xc1"]
            .iter()
            .any(|code| lower.ends_with(code))
}

/// Opens a PBAP session to the Phone at `address`.
pub async fn create_session(
    connection: &Connection,
    address: &str,
) -> Result<OwnedObjectPath, ObexError> {
    let arguments = HashMap::from([("Target", Variant::from("pbap"))]);
    let reply = connection
        .call_method(
            Some(SERVICE),
            CLIENT_PATH,
            Some(CLIENT),
            "CreateSession",
            &(address, arguments),
        )
        .await?;
    Ok(reply.body().deserialize()?)
}

/// Closes the session.
pub async fn remove_session(
    connection: &Connection,
    session: &OwnedObjectPath,
) -> Result<(), ObexError> {
    connection
        .call_method(
            Some(SERVICE),
            CLIENT_PATH,
            Some(CLIENT),
            "RemoveSession",
            &(session,),
        )
        .await?;
    Ok(())
}

/// Selects the Phone's internal folder `folder` (`pb`, `cch`, ...), which
/// later calls on the session read.
pub async fn select_folder(
    connection: &Connection,
    session: &OwnedObjectPath,
    folder: &str,
) -> Result<(), ObexError> {
    connection
        .call_method(
            Some(SERVICE),
            session.as_str(),
            Some(PHONEBOOK_ACCESS),
            "Select",
            &("int", folder),
        )
        .await?;
    Ok(())
}

/// How many vCards the selected folder holds, as the Phone counts them.
pub async fn size(connection: &Connection, session: &OwnedObjectPath) -> Result<u16, ObexError> {
    let reply = connection
        .call_method(
            Some(SERVICE),
            session.as_str(),
            Some(PHONEBOOK_ACCESS),
            "GetSize",
            &(),
        )
        .await?;
    Ok(reply.body().deserialize()?)
}

/// Pulls the selected folder's vCards, from the `offset`th on, into
/// `target` as vCard 3.0, and returns once obexd has written them. The
/// Phone may give fewer than the folder holds: the Pixel gives at most 50
/// Call History entries a pull.
pub async fn pull(
    connection: &Connection,
    session: &OwnedObjectPath,
    offset: u16,
    target: &Path,
) -> Result<(), ObexError> {
    let rule = MatchRule::builder()
        .msg_type(Type::Signal)
        .sender(SERVICE)
        .interface(PROPERTIES)
        .member("PropertiesChanged")
        .path_namespace(session.as_str())
        .build()?;
    let mut signals = MessageStream::for_match_rule(rule, connection, None).await?;
    let mut filters = HashMap::from([("Format", Variant::from("vcard30"))]);
    if offset > 0 {
        filters.insert("Offset", Variant::from(offset));
    }
    let reply = connection
        .call_method(
            Some(SERVICE),
            session.as_str(),
            Some(PHONEBOOK_ACCESS),
            "PullAll",
            &(target.to_string_lossy().as_ref(), filters),
        )
        .await?;
    let (transfer, properties): (OwnedObjectPath, HashMap<String, OwnedVariant>) =
        reply.body().deserialize()?;
    if let Some(done) = finished(&properties) {
        return done;
    }
    loop {
        let next = signals.next();
        let silence = glib::timeout_future(PULL_SILENCE);
        let message = match select(next, Box::pin(silence)).await {
            Either::Left((Some(message), _)) => message?,
            Either::Left((None, _)) => {
                return Err(ObexError::Other("the session bus closed".to_owned()));
            }
            Either::Right(_) => return Err(ObexError::TransferStalled),
        };
        if message
            .header()
            .path()
            .is_none_or(|path| path.as_str() != transfer.as_str())
        {
            continue;
        }
        let Ok((interface, changed, _)) =
            message
                .body()
                .deserialize::<(String, HashMap<String, OwnedVariant>, Vec<String>)>()
        else {
            continue;
        };
        if interface != TRANSFER {
            continue;
        }
        if let Some(done) = finished(&changed) {
            return done;
        }
    }
}

/// The transfer's outcome, if `properties` say it is over.
fn finished(properties: &HashMap<String, OwnedVariant>) -> Option<Result<(), ObexError>> {
    let status = properties
        .get("Status")
        .and_then(|status| String::try_from(status.clone()).ok())?;
    match status.as_str() {
        "complete" => Some(Ok(())),
        "error" => Some(Err(ObexError::TransferFailed)),
        _ => None,
    }
}
