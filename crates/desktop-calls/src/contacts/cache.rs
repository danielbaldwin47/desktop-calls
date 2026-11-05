//! The on-disk copy of each Phone's Contacts and Call History, so callers
//! are named even before the Phone is in range.
//!
//! One JSON file per Phone, keyed by its Bluetooth address, in
//! `$XDG_CACHE_HOME/desktop-calls/` (`~/.cache` by default). Only the user
//! may read it: the directory is 0700 and each file 0600. A file is written
//! whole to a temporary file, flushed to disk, then renamed over the old one,
//! so a crash leaves the old copy or the new one, never half of either.

use std::fs::{self, DirBuilder, OpenOptions, Permissions};
use std::io::{self, Write as _};
use std::os::unix::fs::{DirBuilderExt as _, OpenOptionsExt as _, PermissionsExt as _};
use std::path::{Path, PathBuf};

use desktop_calls_core::{
    CallHistoryEntry, CallHistoryKind, Caller, Contact, ContactNumber, NumberLabel, Timestamp,
};
use serde_json::{Value, json};

/// Bumped when the file's shape changes; other versions are ignored.
const VERSION: u64 = 1;

/// What is cached for one Phone.
#[derive(Default)]
pub struct Cached {
    pub contacts: Vec<Contact>,
    pub call_history: Vec<CallHistoryEntry>,
}

/// Why the cache couldn't be used.
#[derive(Debug)]
pub enum CacheError {
    /// Neither `XDG_CACHE_HOME` nor `HOME` is an absolute path.
    NoCacheDirectory,
    Io(io::Error),
    /// The file isn't one this version wrote.
    Unreadable,
}

impl CacheError {
    /// The kind logs carry as `error_kind`.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::NoCacheDirectory => "no-cache-directory",
            Self::Io(_) => "cache-io",
            Self::Unreadable => "cache-unreadable",
        }
    }
}

impl From<io::Error> for CacheError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

/// `$XDG_CACHE_HOME/desktop-calls`, with the XDG default of `~/.cache`.
pub fn directory() -> Result<PathBuf, CacheError> {
    std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .or_else(|| {
            std::env::var_os("HOME")
                .map(PathBuf::from)
                .filter(|home| home.is_absolute())
                .map(|home| home.join(".cache"))
        })
        .map(|cache| cache.join("desktop-calls"))
        .ok_or(CacheError::NoCacheDirectory)
}

fn file(address: &str) -> Result<PathBuf, CacheError> {
    let name: String = address
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() {
                character.to_ascii_uppercase()
            } else {
                '_'
            }
        })
        .collect();
    Ok(directory()?.join(format!("contacts-{name}.json")))
}

/// Creates the cache directory if need be, and makes it the user's alone.
pub fn private_directory() -> Result<PathBuf, CacheError> {
    let directory = directory()?;
    DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(&directory)?;
    fs::set_permissions(&directory, Permissions::from_mode(0o700))?;
    Ok(directory)
}

/// What is cached for the Phone at `address`; nothing if there is no file.
pub fn load(address: &str) -> Result<Option<Cached>, CacheError> {
    let text = match fs::read(file(address)?) {
        Ok(text) => text,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    let document: Value = serde_json::from_slice(&text).map_err(|_| CacheError::Unreadable)?;
    if document["version"].as_u64() != Some(VERSION) {
        return Err(CacheError::Unreadable);
    }
    let contacts = document["contacts"]
        .as_array()
        .ok_or(CacheError::Unreadable)?
        .iter()
        .map(contact_from_json)
        .collect();
    let call_history = document["call_history"]
        .as_array()
        .ok_or(CacheError::Unreadable)?
        .iter()
        .filter_map(entry_from_json)
        .collect();
    Ok(Some(Cached {
        contacts,
        call_history,
    }))
}

/// Replaces what is cached for the Phone at `address`.
pub fn save(address: &str, cached: &Cached) -> Result<(), CacheError> {
    let directory = private_directory()?;
    let path = file(address)?;
    let document = json!({
        "version": VERSION,
        "address": address,
        "contacts": cached.contacts.iter().map(contact_to_json).collect::<Vec<_>>(),
        "call_history": cached.call_history.iter().map(entry_to_json).collect::<Vec<_>>(),
    });
    let temporary = path.with_extension("json.tmp");
    let mut writer = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(&temporary)?;
    // A temporary file left by an older version may have other permissions.
    writer.set_permissions(Permissions::from_mode(0o600))?;
    writer.write_all(&serde_json::to_vec(&document).map_err(io::Error::other)?)?;
    writer.sync_all()?;
    drop(writer);
    fs::rename(&temporary, &path)?;
    sync_directory(&directory)
}

/// Deletes what is cached for the Phone at `address`.
pub fn forget(address: &str) -> Result<(), CacheError> {
    match fs::remove_file(file(address)?) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}

/// Flushes the rename to disk.
fn sync_directory(directory: &Path) -> Result<(), CacheError> {
    fs::File::open(directory)?.sync_all()?;
    Ok(())
}

fn contact_to_json(contact: &Contact) -> Value {
    json!({
        "name": contact.name,
        "numbers": contact
            .numbers
            .iter()
            .map(|number| json!([number.number, number.label.as_str()]))
            .collect::<Vec<_>>(),
    })
}

fn contact_from_json(value: &Value) -> Contact {
    Contact {
        name: value["name"].as_str().unwrap_or_default().to_owned(),
        numbers: value["numbers"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|number| {
                Some(ContactNumber {
                    number: number[0].as_str()?.to_owned(),
                    label: NumberLabel::from_name(number[1].as_str().unwrap_or("other")),
                })
            })
            .collect(),
    }
}

fn entry_to_json(entry: &CallHistoryEntry) -> Value {
    json!({
        "kind": entry.kind.as_str(),
        "number": match &entry.caller {
            Caller::Number(number) => Value::from(number.as_str()),
            Caller::Withheld => Value::Null,
        },
        "at": entry.at.map(|at| at.0),
        "name": entry.name,
    })
}

fn entry_from_json(value: &Value) -> Option<CallHistoryEntry> {
    Some(CallHistoryEntry {
        kind: CallHistoryKind::from_name(value["kind"].as_str()?)?,
        caller: value["number"]
            .as_str()
            .map_or(Caller::Withheld, |number| Caller::Number(number.to_owned())),
        at: value["at"].as_u64().map(Timestamp),
        name: value["name"].as_str().unwrap_or_default().to_owned(),
    })
}
