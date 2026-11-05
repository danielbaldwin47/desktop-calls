//! The ringer: loops the sound theme's `phone-incoming-call` on the default
//! output while the Call state machine says the Desktop rings.
//!
//! It plays the file with PipeWire's `pw-play`, one play at a time, and
//! starts the next as each ends. If the sound can't be played (no audio
//! server, no player, no sound file) it logs why once and stays silent: a
//! ringing failure never stops the app.

use std::env;
use std::path::PathBuf;
use std::process::{Command, Stdio};

use futures_channel::oneshot;
use futures_util::future::{Either, select};
use glib::MainContext;

use crate::logging::EventKind;

const PLAYER: &str = "pw-play";
const SOUND: &str = "phone-incoming-call";
/// The freedesktop sound theme, which every other theme falls back to.
const THEME_DIRECTORY: &str = "sounds/freedesktop/stereo";
const EXTENSIONS: [&str; 3] = ["oga", "ogg", "wav"];

/// Plays the ringtone while told to.
#[derive(Default)]
pub struct Ringer {
    /// Stops the ringing when sent to, or dropped.
    playing: Option<oneshot::Sender<()>>,
}

impl Ringer {
    /// Starts ringing, unless already ringing.
    pub fn start(&mut self) {
        if self.playing.is_some() {
            return;
        }
        let (stop, stopped) = oneshot::channel();
        self.playing = Some(stop);
        tracing::debug!(event_kind = %EventKind::RingingStarted, "Ringing");
        MainContext::default().spawn_local(ring(stopped));
    }

    /// Stops ringing.
    pub fn stop(&mut self) {
        if self.playing.take().is_some() {
            tracing::debug!(event_kind = %EventKind::RingingStopped, "Stopped ringing");
        }
    }
}

/// Plays the ringtone over and over until `stopped`.
async fn ring(mut stopped: oneshot::Receiver<()>) {
    let Some(sound) = sound_file() else {
        failed(
            "no-sound-file",
            &format!("no {SOUND} sound in the sound theme"),
        );
        return;
    };
    loop {
        let mut player = match Command::new(PLAYER)
            .arg(&sound)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
        {
            Ok(player) => player,
            Err(error) => {
                failed("player-unavailable", &format!("{PLAYER}: {error}"));
                return;
            }
        };
        let Ok(pid) = i32::try_from(player.id()) else {
            let _ = player.kill();
            return;
        };
        let exited = glib::child_watch_future(glib::Pid(pid));
        match select(exited, &mut stopped).await {
            Either::Left(((_, 0), _)) => {}
            Either::Left(((_, status), _)) => {
                // `pw-play` fails at once when no audio server is reachable.
                failed(
                    "playback-failed",
                    &format!("{PLAYER} exited with wait status {status}"),
                );
                return;
            }
            Either::Right(_) => {
                let _ = player.kill();
                // Reap it, so it doesn't linger as a zombie.
                glib::child_watch_future(glib::Pid(pid)).await;
                return;
            }
        }
    }
}

fn failed(error_kind: &str, detail: &str) {
    tracing::warn!(
        event_kind = %EventKind::RingingFailed,
        error_kind,
        detail,
        "Couldn't play the ringtone; the Call still shows on the Desktop"
    );
}

/// The sound theme's ringtone file, looked up in the XDG data directories.
fn sound_file() -> Option<PathBuf> {
    let directories = env::var("XDG_DATA_DIRS")
        .ok()
        .filter(|directories| !directories.is_empty())
        .unwrap_or_else(|| "/usr/local/share:/usr/share".to_owned());
    let mut candidates = Vec::new();
    if let Some(home) = env::var_os("XDG_DATA_HOME").filter(|home| !home.is_empty()) {
        candidates.push(PathBuf::from(home));
    }
    candidates.extend(env::split_paths(&directories));
    candidates.into_iter().find_map(|directory| {
        EXTENSIONS
            .iter()
            .map(|extension| {
                directory
                    .join(THEME_DIRECTORY)
                    .join(format!("{SOUND}.{extension}"))
            })
            .find(|path| path.is_file())
    })
}
