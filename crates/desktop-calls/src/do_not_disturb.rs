//! The Desktop's Do Not Disturb, as Omarchy 4.5 records it, turned into
//! inputs for the Call state machine.
//!
//! - Where: `~/.local/state/omarchy/notifications/quiet.json`, under `HOME`
//!   as Omarchy's shell has it (`Service.qml` `stateDir`), holding
//!   `{"dnd":true}` or `{"dnd":false}`. Its migration from 4.0 also writes
//!   other fields beside `dnd`.
//! - What: read as Omarchy's own `Store.js` `parseQuiet` reads it. On only
//!   if it is a JSON object whose `dnd` is `true`; missing, empty, invalid,
//!   or anything else is off. Without Omarchy, it is off.
//! - Live: the shell replaces the file by an atomic rename a moment after
//!   each toggle, so a monitor watches the directory, not the file, and the
//!   file is read again on every change there. A directory that isn't there
//!   yet is watched too: GLib looks for it every few seconds.
//!
//! Omarchy 4.0's `~/.local/state/omarchy/notifications.json` is not read:
//! 4.5's migration deletes it.

use std::cell::Cell;
use std::env;
use std::io;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use desktop_calls_core::Input;
use futures_channel::mpsc::UnboundedSender;
use gtk::gio::{self, prelude::*};

use crate::logging::EventKind;

/// Omarchy 4.5's notification state directory, under the home directory.
const OMARCHY_NOTIFICATIONS: &str = ".local/state/omarchy/notifications";

/// The longest `quiet.json` Omarchy reads (`Store.js` `parseQuiet`). It
/// counts UTF-16 code units, this counts bytes; Omarchy's own files are
/// ASCII and a few bytes long.
const MAX_QUIET_BYTES: usize = 262_144;

/// Follows Do Not Disturb for as long as it lives.
pub struct DoNotDisturb {
    /// Dropping the monitor would stop it.
    _monitor: Option<gio::FileMonitor>,
}

impl DoNotDisturb {
    /// Reports Do Not Disturb to `inputs` now, and on every change from now
    /// on.
    pub fn follow(inputs: UnboundedSender<Input>) -> Self {
        let Some(home) = env::var_os("HOME") else {
            // No home, so no Omarchy: off, which the state machine starts as.
            return Self { _monitor: None };
        };
        let directory = PathBuf::from(home).join(OMARCHY_NOTIFICATIONS);
        let reader = Rc::new(Reader {
            quiet: directory.join("quiet.json"),
            inputs,
            on: Cell::new(None),
        });
        // Watching before the first read, so no change slips between them.
        let monitor = watch(&directory, Rc::clone(&reader));
        reader.read();
        Self { _monitor: monitor }
    }
}

/// Reads `quiet.json` and reports what changed.
struct Reader {
    quiet: PathBuf,
    inputs: UnboundedSender<Input>,
    /// What was last reported.
    on: Cell<Option<bool>>,
}

impl Reader {
    fn read(&self) {
        let on = read_quiet(&self.quiet);
        if self.on.replace(Some(on)) == Some(on) {
            return;
        }
        tracing::info!(
            event_kind = %EventKind::DoNotDisturbChanged,
            do_not_disturb = on,
            "Do Not Disturb is {}",
            if on { "on" } else { "off" }
        );
        // The state machine only stops with the service.
        let _ = self.inputs.unbounded_send(Input::DoNotDisturbChanged(on));
    }
}

/// Watches `directory`, where Omarchy replaces `quiet.json`, and has
/// `reader` read it again on every change there. It need not exist yet.
fn watch(directory: &Path, reader: Rc<Reader>) -> Option<gio::FileMonitor> {
    let monitor = gio::File::for_path(directory)
        .monitor_directory(gio::FileMonitorFlags::WATCH_MOVES, gio::Cancellable::NONE);
    match monitor {
        Ok(monitor) => {
            monitor.connect_changed(move |_, _, _, _| reader.read());
            Some(monitor)
        }
        Err(error) => {
            tracing::warn!(
                event_kind = %EventKind::DoNotDisturbUnwatched,
                error_kind = "do-not-disturb-unwatched",
                detail = %error,
                "Can't watch Omarchy's Do Not Disturb; following only what it was at startup"
            );
            None
        }
    }
}

/// Whether `quiet` says Do Not Disturb is on, as Omarchy reads it.
fn read_quiet(quiet: &Path) -> bool {
    match std::fs::read_to_string(quiet) {
        Ok(text) => parse_quiet(&text),
        // No Omarchy, or not yet.
        Err(error) if error.kind() == io::ErrorKind::NotFound => false,
        Err(error) => {
            tracing::warn!(
                event_kind = %EventKind::DoNotDisturbUnreadable,
                error_kind = "do-not-disturb-unreadable",
                detail = %error,
                "Can't read Omarchy's Do Not Disturb; taking it as off"
            );
            false
        }
    }
}

/// `Store.js` `parseQuiet`: on only for a JSON object whose `dnd` is `true`.
fn parse_quiet(text: &str) -> bool {
    if text.trim().is_empty() || text.len() > MAX_QUIET_BYTES {
        return false;
    }
    match serde_json::from_str::<serde_json::Value>(text) {
        Ok(serde_json::Value::Object(read)) => {
            read.get("dnd") == Some(&serde_json::Value::Bool(true))
        }
        Ok(_) => false,
        Err(error) => {
            tracing::debug!(
                event_kind = %EventKind::DoNotDisturbUnreadable,
                error_kind = "do-not-disturb-malformed",
                detail = %error,
                "Omarchy's Do Not Disturb isn't JSON; taking it as off, as Omarchy does"
            );
            false
        }
    }
}
