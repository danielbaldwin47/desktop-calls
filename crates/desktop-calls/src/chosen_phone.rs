//! Where Desktop Calls keeps which Phone the user chose last in the Setup
//! Window, so that with two Phones paired a restart follows that one and
//! not whichever BlueZ lists first (ticket #121).
//!
//! It is one JSON file, `$XDG_STATE_HOME/desktop-calls/chosen-phone.json`,
//! holding the Phone's Bluetooth address, and its name, so the Setup Window
//! can name it once it is gone (ticket #150; a file from before has no
//! name, and still counts): the app's own state, never the
//! user's configuration. The directory is 0700 and the file 0600. The file is
//! written whole to a temporary file and renamed over the old one, so a crash
//! leaves one or the other. A file that can't be read, or isn't one this
//! version wrote, counts as no choice: the app logs it and follows the first
//! Phone BlueZ lists, as it did before there was a choice. The next choice
//! replaces it.

use std::fs::{self, DirBuilder, OpenOptions, Permissions};
use std::io::{self, Write as _};
use std::os::unix::fs::{DirBuilderExt as _, OpenOptionsExt as _, PermissionsExt as _};
use std::path::PathBuf;

use serde_json::{Value, json};

use desktop_calls_core::Phone;

use crate::logging::{self, EventKind};

/// Bumped when the file's shape changes; other versions are ignored.
const VERSION: u64 = 1;

fn file() -> Option<PathBuf> {
    Some(logging::state_directory()?.join("chosen-phone.json"))
}

/// The address of the Phone the user chose last, and its name if kept, if
/// one was kept and the file can be read.
pub fn load() -> Option<(String, Option<String>)> {
    let path = file()?;
    let text = match fs::read(&path) {
        Ok(text) => text,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return None,
        Err(error) => {
            failed("chosen-phone-io", &error.to_string());
            return None;
        }
    };
    let Some((address, name)) = serde_json::from_slice(&text)
        .ok()
        .and_then(|document| from_json(&document))
    else {
        failed("chosen-phone-unreadable", "not a file this version wrote");
        return None;
    };
    tracing::info!(
        event_kind = %EventKind::ChosenPhoneLoaded,
        phone_address = %address,
        "Following the Phone chosen last"
    );
    Some((address, name))
}

/// Keeps `phone` as the Phone chosen last, replacing what was kept.
pub fn save(phone: &Phone) {
    match write(phone) {
        Ok(true) => tracing::info!(
            event_kind = %EventKind::ChosenPhoneKept,
            phone_address = %phone.address,
            "Keeping the Phone chosen, for the next run"
        ),
        Ok(false) => failed(
            "chosen-phone-no-state-directory",
            "neither XDG_STATE_HOME nor HOME is an absolute path",
        ),
        Err(error) => failed("chosen-phone-io", &error.to_string()),
    }
}

/// Whether the choice was written: `false` if there is nowhere to keep it.
fn write(phone: &Phone) -> io::Result<bool> {
    let Some(path) = file() else {
        return Ok(false);
    };
    if let Some(directory) = path.parent() {
        DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(directory)?;
        fs::set_permissions(directory, Permissions::from_mode(0o700))?;
    }
    let temporary = path.with_extension("json.tmp");
    let mut writer = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(&temporary)?;
    writer.set_permissions(Permissions::from_mode(0o600))?;
    let document = json!({ "version": VERSION, "address": phone.address, "name": phone.name });
    writer.write_all(&serde_json::to_vec(&document).map_err(io::Error::other)?)?;
    writer.sync_all()?;
    drop(writer);
    fs::rename(&temporary, &path)?;
    Ok(true)
}

fn failed(error_kind: &str, detail: &str) {
    tracing::warn!(
        event_kind = %EventKind::ChosenPhoneFailed,
        error_kind,
        detail,
        "Couldn't keep or read which Phone was chosen last"
    );
}

/// The address and, if kept, the name. A name that isn't a string counts
/// as none: the address alone still says which Phone to follow.
fn from_json(document: &Value) -> Option<(String, Option<String>)> {
    if document.get("version")?.as_u64()? != VERSION {
        return None;
    }
    let address = document.get("address")?.as_str()?;
    let name = document
        .get("name")
        .and_then(Value::as_str)
        .filter(|name| !name.trim().is_empty())
        .map(str::to_owned);
    is_address(address).then(|| (address.to_owned(), name))
}

/// Whether `text` is a Bluetooth address: six pairs of hexadecimal digits
/// separated by colons.
pub(crate) fn is_address(text: &str) -> bool {
    let pairs: Vec<&str> = text.split(':').collect();
    pairs.len() == 6
        && pairs
            .iter()
            .all(|pair| pair.len() == 2 && pair.bytes().all(|byte| byte.is_ascii_hexdigit()))
}
