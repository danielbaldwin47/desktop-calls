//! The Hyprland stand-in: Hyprland's request socket, answering `j/layers`
//! with an `omarchy-notifications` layer as wide as the scenario asks, the
//! way `hyprctl layers -j` reports Omarchy's notification-toast column, and
//! `j/getoption decoration:rounding` with the scenario's corner rounding;
//! and its event socket, which announces `configreloaded` when the rounding
//! changes.

use std::fs;
use std::io::{Read as _, Write as _};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::Path;
use std::sync::{Arc, Mutex, PoisonError};

/// The instance signature the Hyprland stand-in serves under.
pub(crate) const HYPRLAND_SIGNATURE: &str = "desktop-calls-test";

/// How wide Omarchy 4.5's notification-toast column is, as Hyprland reports
/// it on the user's Desktop.
pub(crate) const OMARCHY_TOAST_COLUMN_WIDTH: u32 = 394;

/// The running Hyprland stand-in.
pub(crate) struct Hyprland {
    rounding: Arc<Mutex<u32>>,
    /// Clients of the event socket.
    listeners: Arc<Mutex<Vec<UnixStream>>>,
}

impl Hyprland {
    /// Changes `decoration:rounding` and announces the reload on the event
    /// socket, as Hyprland does when its configuration is reloaded.
    pub fn set_rounding(&self, rounding: u32) {
        *self.rounding.lock().unwrap_or_else(PoisonError::into_inner) = rounding;
        self.listeners
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .retain_mut(|listener| listener.write_all(b"configreloaded>>\n").is_ok());
    }
}

/// Serves Hyprland's request and event sockets under `runtime`, for the rest
/// of the test process. The request socket reports an
/// `omarchy-notifications` layer `toast_width` wide at the top-right of the
/// headless output, and `decoration:rounding` as `rounding`.
pub(crate) fn serve_hyprland(
    runtime: &Path,
    toast_width: u32,
    rounding: u32,
) -> std::io::Result<Hyprland> {
    let directory = runtime.join("hypr").join(HYPRLAND_SIGNATURE);
    fs::create_dir_all(&directory)?;
    let listener = UnixListener::bind(directory.join(".socket.sock"))?;
    let events = UnixListener::bind(directory.join(".socket2.sock"))?;
    let (width, height) = crate::SCREEN_SIZE;
    // `hyprctl layers -j` reports layout coordinates: at scale 1.6 the
    // 3840x2160 output is 2400x1350.
    let (width, height) = (width * 10 / 16, height * 10 / 16);
    let layers = format!(
        "{{\"HEADLESS-1\": {{\"levels\": {{\"0\": [], \"1\": [], \"2\": [], \"3\": [\
         {{\"address\": \"0x1\", \"x\": {x}, \"y\": 0, \"w\": {toast_width}, \
         \"h\": {height}, \"namespace\": \"omarchy-notifications\", \"pid\": 1, \
         \"alpha\": 1.0}}]}}}}}}",
        x = width.saturating_sub(toast_width),
    );
    let hyprland = Hyprland {
        rounding: Arc::new(Mutex::new(rounding)),
        listeners: Arc::default(),
    };
    let rounding = hyprland.rounding.clone();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            let mut request = [0; 256];
            let Ok(length) = stream.read(&mut request) else {
                continue;
            };
            let reply = match &request[..length] {
                b"j/layers" => layers.clone(),
                // As Hyprland 0.56 answers `hyprctl getoption -j`.
                b"j/getoption decoration:rounding" => format!(
                    "{{\"option\": \"decoration:rounding\", \"int\": {}, \"set\": true }}",
                    rounding.lock().unwrap_or_else(PoisonError::into_inner)
                ),
                _ => "unknown request".to_owned(),
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
