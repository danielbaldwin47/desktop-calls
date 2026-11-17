//! Where Desktop Calls keeps the Ringer's settings (issue #432):
//! `$XDG_CONFIG_HOME/desktop-calls/ringer`, lines such as `sound = chime`,
//! `volume = 60` and `file = /path/to/sound.ogg`, absent until the user
//! changes one. It is configuration, so the user may write it by hand too;
//! the app reads it at startup. A file or a line that doesn't read leaves
//! the default in its place, the Chime at 60 %, and is logged: settings
//! never silence a Call.

use std::fs::{self, DirBuilder};
use std::io;
use std::os::unix::fs::DirBuilderExt as _;
use std::path::PathBuf;

use desktop_calls_core::RingerSettings;

use crate::logging::EventKind;

/// `$XDG_CONFIG_HOME/desktop-calls/ringer`, with the XDG default of
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
    Some(config_home.join("desktop-calls/ringer"))
}

/// The Ringer's settings as the user left them, or the default.
pub fn load() -> RingerSettings {
    let Some(path) = file() else {
        return RingerSettings::default();
    };
    let text = match fs::read_to_string(&path) {
        Ok(text) => text,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return RingerSettings::default();
        }
        Err(error) => {
            failed("ringer-io", &error.to_string());
            return RingerSettings::default();
        }
    };
    let (settings, problems) = RingerSettings::parse(&text);
    for problem in &problems {
        failed(problem.kind(), problem.detail());
    }
    tracing::info!(
        event_kind = %EventKind::RingerLoaded,
        sound = settings.sound().key(),
        volume = settings.volume(),
        "Picking up the Ringer's settings"
    );
    settings
}

/// Keeps `settings` for the next run, replacing what was kept.
pub fn save(settings: &RingerSettings) {
    match write(settings) {
        Ok(true) => tracing::info!(
            event_kind = %EventKind::RingerSet,
            sound = settings.sound().key(),
            volume = settings.volume(),
            "Ringer set"
        ),
        Ok(false) => failed(
            "ringer-no-config-directory",
            "neither XDG_CONFIG_HOME nor HOME is an absolute path",
        ),
        Err(error) => failed("ringer-io", &error.to_string()),
    }
}

/// Whether the file was written: `false` if there is nowhere to keep it.
/// Written whole to a temporary file and renamed, so a crash leaves the
/// old settings or the new.
fn write(settings: &RingerSettings) -> io::Result<bool> {
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
    let written =
        fs::write(&temporary, settings.to_string()).and_then(|()| fs::rename(&temporary, &path));
    if written.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    written.map(|()| true)
}

fn failed(error_kind: &str, detail: &str) {
    tracing::warn!(
        event_kind = %EventKind::RingerFailed,
        error_kind,
        detail,
        "Couldn't keep or read the Ringer's settings; the Chime rings where they don't read"
    );
}
