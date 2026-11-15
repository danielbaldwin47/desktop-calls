//! Where Desktop Calls keeps the lasting Missed Call cue for the next run
//! (#236), so the Indicator's badge survives a restart:
//! `$XDG_STATE_HOME/desktop-calls/missed-calls`, one line holding how many
//! Missed Calls there have been since the user last opened the Dialer,
//! absent when there are none. It holds only the count, nothing of who
//! called. The directory is 0700; the file is written whole to a temporary
//! file and renamed, so a crash leaves the old count or the new. A file
//! that doesn't read as a count counts as none, and is logged.

use std::fs::{self, DirBuilder};
use std::io;
use std::os::unix::fs::DirBuilderExt as _;
use std::path::PathBuf;

use crate::logging::{self, EventKind};

fn file() -> Option<PathBuf> {
    Some(logging::state_directory()?.join("missed-calls"))
}

/// How many Missed Calls an earlier run's cue counted, if any, and the
/// file reads as a count.
pub fn load() -> Option<u32> {
    let path = file()?;
    let text = match fs::read_to_string(&path) {
        Ok(text) => text,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return None,
        Err(error) => {
            failed("missed-calls-io", &error.to_string());
            return None;
        }
    };
    let Ok(count) = text.trim().parse::<u32>() else {
        failed("missed-calls-unreadable", "not a count");
        return None;
    };
    tracing::info!(
        event_kind = %EventKind::MissedCallsLoaded,
        count,
        "Picking up the Missed Calls an earlier run counted"
    );
    Some(count).filter(|count| *count > 0)
}

/// Keeps `count` for the next run, replacing what was kept; with 0,
/// removes it.
pub fn save(count: u32) {
    let saved = if count > 0 { write(count) } else { remove() };
    match saved {
        Ok(true) => {}
        Ok(false) => failed(
            "missed-calls-no-state-directory",
            "neither XDG_STATE_HOME nor HOME is an absolute path",
        ),
        Err(error) => failed("missed-calls-io", &error.to_string()),
    }
}

/// Whether the file was written: `false` if there is nowhere to keep it.
fn write(count: u32) -> io::Result<bool> {
    let Some(path) = file() else {
        return Ok(false);
    };
    if let Some(directory) = path.parent() {
        DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(directory)?;
    }
    // One fixed name is enough: the app runs once per user, and only its
    // main loop writes the cue.
    let temporary = path.with_extension("tmp");
    fs::write(&temporary, format!("{count}\n"))?;
    fs::rename(&temporary, &path)?;
    Ok(true)
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

fn failed(error_kind: &str, detail: &str) {
    tracing::warn!(
        event_kind = %EventKind::MissedCallsFailed,
        error_kind,
        detail,
        "Couldn't keep or read the Missed Call cue"
    );
}
