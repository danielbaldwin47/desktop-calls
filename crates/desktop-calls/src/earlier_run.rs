//! Where one run of the service keeps what the next needs, so that a run
//! started after a crash picks up the Calls as they were (see
//! [`desktop_calls_core::EarlierRun`]).
//!
//! It is one JSON file, `earlier-run.json`, in the run's runtime directory:
//!
//! - The installed service keeps it in the directory systemd makes for it,
//!   which systemd passes as `$RUNTIME_DIRECTORY` (`RuntimeDirectory=` in
//!   the unit, `$XDG_RUNTIME_DIR/desktop-calls-service`). It outlives a crash and
//!   a restart, and goes when the service stops, as do the Calls it
//!   describes.
//! - A run systemd didn't start, such as a developer's `cargo run`, keeps
//!   it in `$XDG_RUNTIME_DIR/desktop-calls-dev`, which goes at logout.
//!
//! So neither picks up Calls the other left: on 2026-11-05 the installed
//! service picked up a Call from a dev run stopped mid-Call (ticket #123,
//! ADR-0006).
//!
//! The directory is 0700 and the file 0600, since it holds the other
//! party's number. The file is written whole to a temporary file and renamed
//! over the old one, so a crash leaves one or the other. With nothing to
//! keep, the file goes.

use std::fs::{self, DirBuilder, OpenOptions, Permissions};
use std::io::{self, Write as _};
use std::os::unix::fs::{DirBuilderExt as _, OpenOptionsExt as _, PermissionsExt as _};
use std::path::PathBuf;

use desktop_calls_core::{Caller, Direction, EarlierCall, EarlierRun, Timestamp};
use serde_json::{Value, json};

use crate::logging::EventKind;

/// Bumped when the file's shape changes; other versions are ignored.
const VERSION: u64 = 1;

/// Where a run systemd didn't start keeps its file, under `XDG_RUNTIME_DIR`.
const UNMANAGED_DIRECTORY: &str = "desktop-calls-dev";

fn file() -> Option<PathBuf> {
    let absolute = |name: &str| {
        std::env::var_os(name)
            .map(PathBuf::from)
            .filter(|path| path.is_absolute())
    };
    absolute("RUNTIME_DIRECTORY")
        .or_else(|| absolute("XDG_RUNTIME_DIR").map(|runtime| runtime.join(UNMANAGED_DIRECTORY)))
        .map(|directory| directory.join("earlier-run.json"))
}

/// What the run before this one kept, if anything.
pub fn load() -> Option<EarlierRun> {
    let path = file()?;
    let text = match fs::read(&path) {
        Ok(text) => text,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return None,
        Err(error) => {
            failed("earlier-run-io", &error.to_string());
            return None;
        }
    };
    let Some(earlier) = serde_json::from_slice(&text)
        .ok()
        .and_then(|document| from_json(&document))
    else {
        failed("earlier-run-unreadable", "not a file this version wrote");
        return None;
    };
    tracing::info!(
        event_kind = %EventKind::EarlierRunLoaded,
        calls = earlier.calls.len(),
        muted = earlier.restore_microphone_volume.is_some(),
        "{} Call(s) from before this run started may be continuing; picking them up if the Phone still has them",
        earlier.calls.len()
    );
    Some(earlier)
}

/// Logs that `calls` Calls from before this run started never showed
/// again, and so were forgotten.
pub fn forgotten(calls: usize) {
    tracing::info!(
        event_kind = %EventKind::EarlierRunForgotten,
        calls,
        "Forgot {calls} Call(s) from before this run started: the Phone never showed them again, so they had ended"
    );
}

/// Keeps `next` for the next run, replacing what was kept.
pub fn save(next: &EarlierRun) {
    if let Err(error) = write(next) {
        failed("earlier-run-io", &error.to_string());
    }
}

fn write(next: &EarlierRun) -> io::Result<()> {
    let Some(path) = file() else {
        return Ok(());
    };
    if next.is_empty() {
        return match fs::remove_file(&path) {
            Err(error) if error.kind() != io::ErrorKind::NotFound => Err(error),
            _ => Ok(()),
        };
    }
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
    writer.write_all(&serde_json::to_vec(&to_json(next)).map_err(io::Error::other)?)?;
    drop(writer);
    fs::rename(&temporary, &path)
}

fn failed(error_kind: &str, detail: &str) {
    tracing::warn!(
        event_kind = %EventKind::EarlierRunFailed,
        error_kind,
        detail,
        "Couldn't keep or read what a run after a crash needs"
    );
}

fn to_json(earlier: &EarlierRun) -> Value {
    json!({
        "version": VERSION,
        "calls": earlier.calls.iter().map(|call| json!({
            "number": match &call.caller {
                Caller::Number(number) => Some(number.as_str()),
                Caller::Withheld => None,
            },
            "direction": call.direction.as_str(),
            "active_since_ms": call.active_since.0,
        })).collect::<Vec<_>>(),
        "restore_microphone_volume": earlier.restore_microphone_volume,
    })
}

fn from_json(document: &Value) -> Option<EarlierRun> {
    if document.get("version")?.as_u64()? != VERSION {
        return None;
    }
    let calls = document
        .get("calls")?
        .as_array()?
        .iter()
        .map(|call| {
            Some(EarlierCall {
                caller: match call.get("number")? {
                    Value::Null => Caller::Withheld,
                    number => Caller::Number(number.as_str()?.to_owned()),
                },
                direction: match call.get("direction")?.as_str()? {
                    "incoming" => Direction::Incoming,
                    "outgoing" => Direction::Outgoing,
                    _ => Direction::Unknown,
                },
                active_since: Timestamp(call.get("active_since_ms")?.as_u64()?),
            })
        })
        .collect::<Option<Vec<_>>>()?;
    let restore_microphone_volume = match document.get("restore_microphone_volume")? {
        Value::Null => None,
        volume => Some(u8::try_from(volume.as_u64()?).ok()?),
    };
    Some(EarlierRun {
        calls,
        restore_microphone_volume,
    })
}
