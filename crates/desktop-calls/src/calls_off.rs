//! Where Desktop Calls keeps Calls Off for the next run (ADR-0008), so that
//! calls stay off across a restart, and a run stopped halfway through
//! blocking or unblocking the Phone is put right at the next start.
//!
//! It is one JSON file, `$XDG_STATE_HOME/desktop-calls/calls-off.json`,
//! holding the state ("off", "on" while the Phone is being unblocked,
//! "pending" while a turn-off waits for the last Call to end, or
//! "kept-on" while calls stay on through Quiet Hours the user turned them
//! on in), the address of the Phone the app blocked or is to block (none
//! for "kept-on"), and, for calls off until a deadline, `until`: the Unix
//! time in seconds they come back on; for "on" and "kept-on", when the
//! Quiet Hours they stay on through next begin. `"quiet_hours": true`
//! marks calls off, or waiting to go off, for Quiet Hours. It is written
//! before the Phone is blocked or unblocked, and removed once calls are on
//! with the Phone unblocked, or a waiting turn-off is cancelled, with
//! nothing to keep. The directory is 0700 and the file 0600; the file is
//! written whole to a temporary file and renamed over the old one, so a
//! crash leaves one or the other. A file that can't be read, or isn't one
//! this version wrote, counts as none: the app logs it and leaves the
//! Phone as BlueZ has it.

use std::fs::{self, DirBuilder, OpenOptions, Permissions};
use std::io::{self, Write as _};
use std::os::unix::fs::{DirBuilderExt as _, OpenOptionsExt as _, PermissionsExt as _};
use std::path::PathBuf;

use desktop_calls_core::{CallsOffRecord, Deadline, Input, Timestamp};
use futures_channel::mpsc::UnboundedSender;
use glib::MainContext;
use serde_json::{Value, json};
use zbus::Connection;

use crate::logging::{self, EventKind};
use crate::{bluez, clock};

/// Bumped when the file's shape changes; other versions are ignored.
const VERSION: u64 = 1;

fn file() -> Option<PathBuf> {
    Some(logging::state_directory()?.join("calls-off.json"))
}

/// Whether a record that may have the Phone blocked is kept now: any
/// unreadable one, or one that isn't of calls kept on through Quiet Hours,
/// which blocks nothing.
pub fn may_block_phone() -> bool {
    let Some(path) = file() else {
        return false;
    };
    match fs::read(&path) {
        Err(error) => error.kind() != io::ErrorKind::NotFound,
        Ok(text) => !matches!(
            serde_json::from_slice(&text)
                .ok()
                .and_then(|document| from_json(&document)),
            Some(CallsOffRecord::KeptOn { .. })
        ),
    }
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
    let (state, address, until, _) = parts(&record);
    tracing::info!(
        event_kind = %EventKind::CallsOffLoaded,
        phone_address = address.unwrap_or_default(),
        state,
        until,
        "Picking up Calls Off as the run before left it"
    );
    Some(record)
}

/// Logs that the deadline of calls off for the Phone with `address` passed,
/// so calls come back on.
pub fn deadline_passed(address: &str) {
    tracing::info!(
        event_kind = %EventKind::CallsOffDeadlinePassed,
        phone_address = %address,
        "Calls off: the deadline passed, so calls are on"
    );
}

/// Logs that calls were turned off during a Call, for the Phone with
/// `address`: they go off once no Call is left.
pub fn pending(address: &str) {
    tracing::info!(
        event_kind = %EventKind::CallsOffPending,
        phone_address = %address,
        "Calls off: waiting for the last Call to end"
    );
}

/// Logs that a Call was placed while calls were off, so the Phone with
/// `address` is unblocked for it, and calls go off again once it ends.
pub fn placed_while_off(address: &str) {
    tracing::info!(
        event_kind = %EventKind::CallPlacedWhileCallsOff,
        phone_address = %address,
        "Calls off: a Call placed anyway; calls go off again once it ends"
    );
}

/// Logs that the turn-off waiting for the last Call to end was cancelled.
pub fn kept_on(address: &str) {
    tracing::info!(
        event_kind = %EventKind::CallsKeptOn,
        phone_address = %address,
        "Calls kept on: the waiting turn-off is cancelled"
    );
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
    let (state, address, until, quiet_hours) = parts(record);
    let mut document = json!({ "version": VERSION, "state": state });
    if let Some(address) = address {
        document["address"] = json!(address);
    }
    if let Some(until) = until {
        document["until"] = json!(until);
    }
    if quiet_hours {
        document["quiet_hours"] = json!(true);
    }
    // A turn-off waiting for a Call placed while calls were off; a build
    // without the field reads it as any other waiting turn-off.
    if let CallsOffRecord::Pending {
        exception: true, ..
    } = record
    {
        document["exception"] = json!(true);
    }
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

/// The record's state, Phone address, deadline in Unix seconds, and
/// whether it is for Quiet Hours.
fn parts(record: &CallsOffRecord) -> (&'static str, Option<&str>, Option<u64>, bool) {
    let seconds = |until: &Deadline| until.at.0 / 1_000;
    match record {
        CallsOffRecord::Off {
            address,
            until,
            quiet_hours,
        } => (
            "off",
            Some(address),
            until.as_ref().map(seconds),
            *quiet_hours,
        ),
        CallsOffRecord::TurningOn {
            address,
            kept_on_until,
        } => (
            "on",
            Some(address),
            kept_on_until.as_ref().map(seconds),
            false,
        ),
        CallsOffRecord::Pending {
            address,
            until,
            quiet_hours,
            ..
        } => (
            "pending",
            Some(address),
            until.as_ref().map(seconds),
            *quiet_hours,
        ),
        CallsOffRecord::KeptOn { until } => ("kept-on", None, Some(seconds(until)), false),
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
    let until = match document.get("until") {
        None => None,
        Some(until) => Some(clock::deadline(Timestamp(
            until.as_u64()?.checked_mul(1_000)?,
        ))),
    };
    let state = document.get("state")?.as_str()?;
    if state == "kept-on" {
        return Some(CallsOffRecord::KeptOn { until: until? });
    }
    let address = document.get("address")?.as_str()?;
    if !crate::chosen_phone::is_address(address) {
        return None;
    }
    let address = address.to_owned();
    let flag = |name: &str| match document.get(name) {
        None => Some(false),
        Some(flag) => flag.as_bool(),
    };
    let quiet_hours = flag("quiet_hours")?;
    match state {
        "off" => Some(CallsOffRecord::Off {
            address,
            until,
            quiet_hours,
        }),
        "on" => Some(CallsOffRecord::TurningOn {
            address,
            kept_on_until: until,
        }),
        "pending" => Some(CallsOffRecord::Pending {
            address,
            until,
            quiet_hours,
            exception: flag("exception")?,
        }),
        _ => None,
    }
}
