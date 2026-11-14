//! Where Desktop Calls keeps Calls Off for the next run (ADR-0008), so that
//! calls stay off across a restart, and a run stopped halfway through
//! blocking or unblocking the Phone is put right at the next start.
//!
//! It is one JSON file, `$XDG_STATE_HOME/desktop-calls/calls-off.json`,
//! holding the state ("off", or "on" while the Phone is being unblocked)
//! and the address of the Phone the app blocked. It is written before the
//! Phone is blocked or unblocked, and removed once calls are on with the
//! Phone unblocked. The directory is 0700 and the file 0600; the file is
//! written whole to a temporary file and renamed over the old one, so a
//! crash leaves one or the other. A file that can't be read, or isn't one
//! this version wrote, counts as none: the app logs it and leaves the
//! Phone as BlueZ has it.

use std::fs::{self, DirBuilder, OpenOptions, Permissions};
use std::io::{self, Write as _};
use std::os::unix::fs::{DirBuilderExt as _, OpenOptionsExt as _, PermissionsExt as _};
use std::path::PathBuf;

use desktop_calls_core::{CallsOffRecord, Input};
use futures_channel::mpsc::UnboundedSender;
use glib::MainContext;
use serde_json::{Value, json};
use zbus::Connection;

use crate::bluez;
use crate::logging::{self, EventKind};

/// Bumped when the file's shape changes; other versions are ignored.
const VERSION: u64 = 1;

fn file() -> Option<PathBuf> {
    Some(logging::state_directory()?.join("calls-off.json"))
}

/// Whether a record is kept now, readable or not.
pub fn is_kept() -> bool {
    file().is_some_and(|path| path.exists())
}

/// What an earlier run kept, if anything, and the file can be read.
pub fn load() -> Option<CallsOffRecord> {
    let path = file()?;
    let text = match fs::read(&path) {
        Ok(text) => text,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return None,
        Err(error) => {
            failed("calls-off-io", &error.to_string());
            return None;
        }
    };
    let Some(record) = serde_json::from_slice(&text)
        .ok()
        .and_then(|document| from_json(&document))
    else {
        failed("calls-off-unreadable", "not a file this version wrote");
        return None;
    };
    let (state, address) = parts(&record);
    tracing::info!(
        event_kind = %EventKind::CallsOffLoaded,
        phone_address = %address,
        state,
        "Picking up Calls Off as the run before left it"
    );
    Some(record)
}

/// Keeps `record` for the next run, replacing what was kept; with `None`,
/// removes it.
pub fn save(record: Option<&CallsOffRecord>) {
    let written = match record {
        Some(record) => write(record),
        None => remove(),
    };
    match written {
        Ok(true) => {}
        Ok(false) => failed(
            "calls-off-no-state-directory",
            "neither XDG_STATE_HOME nor HOME is an absolute path",
        ),
        Err(error) => failed("calls-off-io", &error.to_string()),
    }
}

fn remove() -> io::Result<bool> {
    let Some(path) = file() else {
        return Ok(false);
    };
    match fs::remove_file(path) {
        Err(error) if error.kind() != io::ErrorKind::NotFound => Err(error),
        _ => Ok(true),
    }
}

/// Whether the record was written: `false` if there is nowhere to keep it.
fn write(record: &CallsOffRecord) -> io::Result<bool> {
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
    let (state, address) = parts(record);
    let document = json!({ "version": VERSION, "state": state, "address": address });
    writer.write_all(&serde_json::to_vec(&document).map_err(io::Error::other)?)?;
    writer.sync_all()?;
    drop(writer);
    fs::rename(&temporary, &path)?;
    Ok(true)
}

/// Has BlueZ block (`blocked`) or unblock the Phone with `address`, and
/// reports how it went to `inputs`. The Desktop carries on meanwhile.
pub fn block(system: Connection, inputs: UnboundedSender<Input>, address: String, blocked: bool) {
    MainContext::default().spawn_local(async move {
        let input = match bluez::set_blocked(&system, &address, blocked).await {
            Ok(()) => {
                if blocked {
                    tracing::info!(
                        event_kind = %EventKind::CallsTurnedOff,
                        phone_address = %address,
                        "Calls off: the Phone is blocked"
                    );
                } else {
                    tracing::info!(
                        event_kind = %EventKind::CallsTurnedOn,
                        phone_address = %address,
                        "Calls on: the Phone is unblocked"
                    );
                }
                Input::PhoneBlocked { address, blocked }
            }
            Err(error) => {
                if error == bluez::BlockError::PhoneNotFound {
                    tracing::warn!(
                        event_kind = %EventKind::PhoneBlockFailed,
                        phone_address = %address,
                        blocked,
                        error_kind = error.as_str(),
                        "BlueZ has no device for the Phone to block or unblock"
                    );
                }
                Input::PhoneBlockFailed { address, blocked }
            }
        };
        let _ = inputs.unbounded_send(input);
    });
}

fn parts(record: &CallsOffRecord) -> (&'static str, &str) {
    match record {
        CallsOffRecord::Off { address } => ("off", address),
        CallsOffRecord::TurningOn { address } => ("on", address),
    }
}

fn failed(error_kind: &str, detail: &str) {
    tracing::warn!(
        event_kind = %EventKind::CallsOffFailed,
        error_kind,
        detail,
        "Couldn't keep or read Calls Off"
    );
}

fn from_json(document: &Value) -> Option<CallsOffRecord> {
    if document.get("version")?.as_u64()? != VERSION {
        return None;
    }
    let address = document.get("address")?.as_str()?;
    if !crate::chosen_phone::is_address(address) {
        return None;
    }
    let address = address.to_owned();
    match document.get("state")?.as_str()? {
        "off" => Some(CallsOffRecord::Off { address }),
        "on" => Some(CallsOffRecord::TurningOn { address }),
        _ => None,
    }
}
