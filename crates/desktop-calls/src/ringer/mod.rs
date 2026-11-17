//! The Ringer: plays a Ring Sound on the default output while the Call
//! state machine says the Desktop rings, as [`RingerSettings`] say: which
//! Ring Sound, at what volume, and the pause between repeats.
//!
//! It writes the Ring Sound to the cache, then plays it with PipeWire's
//! `pw-play`, one repeat at a time: the first faded in, each after a pause.
//! If the sound can't be played (no audio server, no player, nowhere to
//! write the sound) it logs why once and stays silent: a ringing failure
//! never stops the app.

mod ring_sound;

use std::process::{Command, ExitCode, Stdio};
use std::time::Duration;

use futures_channel::oneshot;
use futures_util::future::{Either, select};
use glib::MainContext;

use crate::contacts::cache;
use crate::logging::EventKind;
pub use ring_sound::RingSound;

const PLAYER: &str = "pw-play";
/// The ring's stream, as `pw-play` asks PipeWire for it. Named, so mixers
/// show what it is. Its media role is `Notification`, so the Desktop's
/// audio rules treat it as an alert. `WirePlumber` neither restores nor saves
/// its volume (`state.restore-props`): it is the Ringer's own setting,
/// under the output's volume, and a stored one would replace it. Both read
/// in `WirePlumber` 0.5's `state-stream.lua`; that `pw-play` hands these
/// properties to the stream as given is from its `--help`, not yet heard.
const STREAM_PROPERTIES: &str = r#"{ application.name = "Desktop Calls" media.name = "Desktop Calls ringer" state.restore-props = false }"#;
const MEDIA_ROLE: &str = "Notification";

/// What the Ringer plays, and how.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RingerSettings {
    pub sound: RingSound,
    /// The ring stream's volume, from 0 to 1, under the output's own.
    pub volume: f64,
    /// The silence after each repeat.
    pub pause: Duration,
}

impl Default for RingerSettings {
    /// The soft chime at 60 % volume, every 2 s.
    fn default() -> Self {
        Self {
            sound: RingSound::Chime,
            volume: 0.6,
            pause: Duration::from_secs(2),
        }
    }
}

/// Plays the Ring Sound while told to.
pub struct Ringer {
    settings: RingerSettings,
    /// Stops the ringing when sent to, or dropped.
    playing: Option<oneshot::Sender<()>>,
}

impl Ringer {
    pub fn new(settings: RingerSettings) -> Self {
        Self {
            settings,
            playing: None,
        }
    }

    /// Starts ringing, unless already ringing.
    pub fn start(&mut self) {
        if self.playing.is_some() {
            return;
        }
        let (stop, stopped) = oneshot::channel();
        self.playing = Some(stop);
        tracing::debug!(event_kind = %EventKind::RingingStarted, "Ringing");
        let settings = self.settings;
        MainContext::default().spawn_local(async move {
            if let Err(failure) = ring(settings, stopped).await {
                failed(&failure);
            }
        });
    }

    /// Stops ringing.
    pub fn stop(&mut self) {
        if self.playing.take().is_some() {
            tracing::debug!(event_kind = %EventKind::RingingStopped, "Stopped ringing");
        }
    }
}

/// Rings as an incoming Call does, until SIGINT (Ctrl-C) or SIGTERM: for
/// `desktop-calls ring`, to hear the Ringer without a Call. A failure is
/// printed on standard error.
pub fn ring_until_interrupted() -> ExitCode {
    let (stop, stopped) = oneshot::channel::<()>();
    let context = MainContext::default();
    context.spawn_local(async move {
        select(
            glib_unix::signal_future(SIGINT),
            glib_unix::signal_future(SIGTERM),
        )
        .await;
        drop(stop);
    });
    match context.block_on(ring(RingerSettings::default(), stopped)) {
        Ok(()) => ExitCode::SUCCESS,
        Err(failure) => {
            eprintln!("{}: {}", failure.kind, failure.detail);
            ExitCode::FAILURE
        }
    }
}

const SIGINT: i32 = 2;
const SIGTERM: i32 = 15;

/// Why the Ring Sound couldn't be played.
struct Failure {
    /// The kind logs carry as `error_kind`.
    kind: &'static str,
    detail: String,
}

impl Failure {
    fn new(kind: &'static str, detail: impl Into<String>) -> Self {
        Self {
            kind,
            detail: detail.into(),
        }
    }
}

/// Plays the Ring Sound over and over, with a pause after each, until
/// `stopped`.
async fn ring(settings: RingerSettings, mut stopped: oneshot::Receiver<()>) -> Result<(), Failure> {
    let directory = cache::directory()
        .map_err(|error| Failure::new("no-cache-directory", error.as_str()))?
        .join("ringer");
    let written = settings
        .sound
        .write(&directory)
        .map_err(|error| Failure::new("ring-sound-unwritten", error.to_string()))?;
    let volume = format!("{:.3}", settings.volume.clamp(0.0, 1.0));
    let mut sound = &written.first;
    loop {
        let mut player = Command::new(PLAYER)
            .args(["--volume", &volume, "--media-role", MEDIA_ROLE])
            .args(["-P", STREAM_PROPERTIES])
            .arg(sound)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|error| Failure::new("player-unavailable", format!("{PLAYER}: {error}")))?;
        let Ok(pid) = i32::try_from(player.id()) else {
            let _ = player.kill();
            return Ok(());
        };
        let exited = glib::child_watch_future(glib::Pid(pid));
        match select(exited, &mut stopped).await {
            Either::Left(((_, 0), _)) => {}
            Either::Left(((_, status), _)) => {
                // `pw-play` fails at once when no audio server is reachable.
                return Err(Failure::new(
                    "playback-failed",
                    format!("{PLAYER} exited with wait status {status}"),
                ));
            }
            Either::Right((_, exited)) => {
                let _ = player.kill();
                // Reap it with the same watch, so it doesn't linger as a
                // zombie. A second watch on it would find it already reaped
                // by the first, and GLib would warn "waitid ... No child
                // processes".
                exited.await;
                return Ok(());
            }
        }
        sound = &written.repeat;
        let pause = glib::timeout_future(settings.pause);
        if let Either::Right(_) = select(pause, &mut stopped).await {
            return Ok(());
        }
    }
}

fn failed(failure: &Failure) {
    tracing::warn!(
        event_kind = %EventKind::RingingFailed,
        error_kind = failure.kind,
        detail = failure.detail,
        "Couldn't play the Ring Sound; the Call still shows on the Desktop"
    );
}
