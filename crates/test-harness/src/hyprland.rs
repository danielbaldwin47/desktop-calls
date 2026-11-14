//! The Hyprland stand-in: Hyprland's request socket, answering
//! `j/getoption decoration:rounding` with the scenario's corner rounding and
//! `j/getoption general:gaps_out` with its gap, if it has one; and its event
//! socket, which announces `configreloaded` when either changes.

use std::fs;
use std::io::{Read as _, Write as _};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::Path;
use std::sync::{Arc, Mutex, PoisonError};

/// The instance signature the Hyprland stand-in serves under.
pub(crate) const HYPRLAND_SIGNATURE: &str = "desktop-calls-test";

/// What the stand-in's configuration says.
#[derive(Clone, Copy, Default)]
pub(crate) struct Config {
    /// `decoration:rounding`.
    pub rounding: u32,
    /// `general:gaps_out`, the same on all four sides, or `None` for an
    /// answer without it.
    pub gaps_out: Option<u32>,
}

/// The running Hyprland stand-in.
pub(crate) struct Hyprland {
    config: Arc<Mutex<Config>>,
    /// Clients of the event socket.
    listeners: Arc<Mutex<Vec<UnixStream>>>,
}

impl Hyprland {
    /// Changes the configuration with `change` and announces the reload on
    /// the event socket, as Hyprland does when its configuration is
    /// reloaded.
    pub fn reconfigure(&self, change: impl FnOnce(&mut Config)) {
        change(&mut self.config.lock().unwrap_or_else(PoisonError::into_inner));
        self.listeners
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .retain_mut(|listener| listener.write_all(b"configreloaded>>\n").is_ok());
    }
}

/// Serves Hyprland's request and event sockets under `runtime`, for the rest
/// of the test process, with `config`.
pub(crate) fn serve_hyprland(runtime: &Path, config: Config) -> std::io::Result<Hyprland> {
    let directory = runtime.join("hypr").join(HYPRLAND_SIGNATURE);
    fs::create_dir_all(&directory)?;
    let listener = UnixListener::bind(directory.join(".socket.sock"))?;
    let events = UnixListener::bind(directory.join(".socket2.sock"))?;
    let hyprland = Hyprland {
        config: Arc::new(Mutex::new(config)),
        listeners: Arc::default(),
    };
    let config = hyprland.config.clone();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            let mut request = [0; 256];
            let Ok(length) = stream.read(&mut request) else {
                continue;
            };
            let config = *config.lock().unwrap_or_else(PoisonError::into_inner);
            // As Hyprland 0.56 answers `hyprctl getoption -j`.
            let reply = match (&request[..length], config.gaps_out) {
                (b"j/getoption decoration:rounding", _) => format!(
                    "{{\"option\": \"decoration:rounding\", \"int\": {}, \"set\": true }}",
                    config.rounding
                ),
                (b"j/getoption general:gaps_out", Some(gap)) => format!(
                    "{{\"option\": \"general:gaps_out\", \"css\": \"{gap} {gap} {gap} {gap}\", \
                     \"set\": true }}"
                ),
                _ => "no such option".to_owned(),
            };
            let _ = stream.write_all(reply.as_bytes());
        }
    });
    let listeners = hyprland.listeners.clone();
    std::thread::spawn(move || {
        for stream in events.incoming().flatten() {
            listeners
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push(stream);
        }
    });
    Ok(hyprland)
}
