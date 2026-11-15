//! Where Desktop Calls keeps the user's Quiet Hours (ticket #205):
//! `$XDG_CONFIG_HOME/desktop-calls/quiet-hours`, one line such as
//! `22:00-07:00`, absent when Quiet Hours are off. It is configuration, so
//! the user may write it by hand too; the app reads it at startup. A file
//! that doesn't read as Quiet Hours counts as none, and is logged.

use std::fs::{self, DirBuilder};
use std::io;
use std::os::unix::fs::DirBuilderExt as _;
use std::path::PathBuf;

use desktop_calls_core::QuietHours;

use crate::logging::EventKind;

/// `$XDG_CONFIG_HOME/desktop-calls/quiet-hours`, with the XDG default of
/// `~/.config`.
fn file() -> Option<PathBuf> {
    let config_home = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .or_else(|| {
            std::env::var_os("HOME")
                .map(PathBuf::from)
                .filter(|home| home.is_absolute())
                .map(|home| home.join(".config"))
        })?;
    Some(config_home.join("desktop-calls/quiet-hours"))
}

/// The Quiet Hours the user set, if any, and the file reads as them.
pub fn load() -> Option<QuietHours> {
    let path = file()?;
    let text = match fs::read_to_string(&path) {
        Ok(text) => text,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return None,
        Err(error) => {
            failed("quiet-hours-io", &error.to_string());
            return None;
        }
    };
    let Some(hours) = QuietHours::parse(&text) else {
        failed("quiet-hours-unreadable", "not HH:MM-HH:MM");
        return None;
    };
    tracing::info!(
        event_kind = %EventKind::QuietHoursLoaded,
        quiet_hours = %hours,
        "Picking up the Quiet Hours the user set"
    );
    Some(hours)
}

/// Keeps `hours` for the next run, replacing what was kept; with `None`,
/// removes them.
pub fn save(hours: Option<QuietHours>) {
    let saved = match hours {
        Some(hours) => write(hours),
        None => remove(),
    };
    match saved {
        Ok(true) => {
            if let Some(hours) = hours {
                tracing::info!(
                    event_kind = %EventKind::QuietHoursSet,
                    quiet_hours = %hours,
                    "Quiet Hours set"
                );
            } else {
                tracing::info!(
                    event_kind = %EventKind::QuietHoursCleared,
                    "Quiet Hours turned off"
                );
            }
        }
        Ok(false) => failed(
            "quiet-hours-no-config-directory",
            "neither XDG_CONFIG_HOME nor HOME is an absolute path",
        ),
        Err(error) => failed("quiet-hours-io", &error.to_string()),
    }
}

/// Whether the file was written: `false` if there is nowhere to keep it.
/// Written whole to a temporary file and renamed, so a crash leaves the
/// old Quiet Hours or the new.
fn write(hours: QuietHours) -> io::Result<bool> {
    let Some(path) = file() else {
        return Ok(false);
    };
    if let Some(directory) = path.parent() {
        DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(directory)?;
    }
    let temporary = path.with_extension("tmp");
    fs::write(&temporary, format!("{hours}\n"))?;
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
        event_kind = %EventKind::QuietHoursFailed,
        error_kind,
        detail,
        "Couldn't keep or read the Quiet Hours"
    );
}
