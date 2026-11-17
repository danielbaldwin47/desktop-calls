//! The Ringer: plays the Ring Sound on the default output while the Call
//! state machine says the Desktop rings, as the [`RingerSettings`] say:
//! which Ring Sound, at what volume. It plays one once, too, when the
//! Ringer Window asks.
//!
//! It writes the Ring Sound to its folder (see [`directory`]), then plays
//! it with PipeWire's `pw-play`, one repeat at a time: the first faded in
//! and each after a pause, for the sounds the app makes; one after another,
//! for Android's ringtones, which are made to loop. The user's own file that is missing, or that
//! `pw-play` can't play, gives way to the Chime: the Ringer says so to the
//! Call state machine, and logs it. If no sound can be played (no audio
//! server, no player, nowhere to write the sound) it logs why once and
//! stays silent: a ringing failure never stops the app.

mod android;
mod directory;
mod ring_sound;
mod tones;
mod wav;

use std::path::Path;
use std::process::{Command, ExitCode, Stdio};

use desktop_calls_core::{Input, RingSound, RingerSettings};
use futures_channel::mpsc::UnboundedSender;
use futures_channel::oneshot;
use futures_util::future::{Either, select};
use glib::MainContext;

use crate::logging::EventKind;
use ring_sound::{Missing, Playable};

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

/// Plays the Ring Sound while told to.
pub struct Ringer {
    settings: RingerSettings,
    /// Where it tells the Call state machine that a preview ended, or that
    /// the user's own file couldn't be played.
    inputs: UnboundedSender<Input>,
    /// Stops the ringing when sent to, or dropped.
    playing: Option<oneshot::Sender<()>>,
    /// Stops the preview when sent to, or dropped.
    previewing: Option<oneshot::Sender<()>>,
}

impl Ringer {
    pub fn new(settings: RingerSettings, inputs: UnboundedSender<Input>) -> Self {
        Self {
            settings,
            inputs,
            playing: None,
            previewing: None,
        }
    }

    /// Rings with `settings` from the next ring on.
    pub fn keep(&mut self, settings: RingerSettings) {
        self.settings = settings;
    }

    /// Starts ringing, unless already ringing.
    pub fn start(&mut self) {
        if self.playing.is_some() {
            return;
        }
        let (stop, stopped) = oneshot::channel();
        self.playing = Some(stop);
        tracing::debug!(event_kind = %EventKind::RingingStarted, "Ringing");
        let settings = self.settings.clone();
        let inputs = self.inputs.clone();
        MainContext::default().spawn_local(async move {
            if let Err(failure) = ring(&settings, Some(&inputs), stopped).await {
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

    /// Plays `settings`' Ring Sound once, at their volume, stopping one
    /// already playing; then says the preview ended.
    pub fn preview(&mut self, settings: RingerSettings) {
        self.stop_preview();
        let (stop, stopped) = oneshot::channel();
        self.previewing = Some(stop);
        tracing::debug!(
            event_kind = %EventKind::PreviewStarted,
            sound = settings.sound().key(),
            volume = settings.volume(),
            "Playing the Ring Sound once"
        );
        let inputs = self.inputs.clone();
        MainContext::default().spawn_local(async move {
            match preview(&settings, &inputs, stopped).await {
                Ok(Ended::Stopped) => return,
                Ok(Ended::Finished) => {}
                Err(failure) => failed(&failure),
            }
            let _ = inputs.unbounded_send(Input::PreviewEnded);
        });
    }

    /// Stops the preview playing, if one is.
    pub fn stop_preview(&mut self) {
        if self.previewing.take().is_some() {
            tracing::debug!(event_kind = %EventKind::PreviewStopped, "Stopped the Ring Sound");
        }
    }
}

/// Rings with `settings` as an incoming Call does, until SIGINT (Ctrl-C)
/// or SIGTERM: for `desktop-calls ring`, to hear the Ringer without a Call.
/// A failure is printed on standard error.
pub fn ring_until_interrupted(settings: &RingerSettings) -> ExitCode {
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
    match context.block_on(ring(settings, None, stopped)) {
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

/// How a play ended without failing.
enum Ended {
    Finished,
    Stopped,
}

/// The Ring Sound `settings` name, ready to play: the Chime in place of
/// the user's own file when it is missing, said and logged.
fn ready(
    settings: &RingerSettings,
    inputs: Option<&UnboundedSender<Input>>,
) -> Result<Playable, Failure> {
    match ring_sound::playable(settings)? {
        Ok(playable) => Ok(playable),
        Err(Missing(file)) => {
            unplayable(&file, "the file is missing or can't be read", inputs);
            ring_sound::built_in(RingSound::Chime)
        }
    }
}

/// Plays the Ring Sound over and over, with a pause after each, until
/// `stopped`.
async fn ring(
    settings: &RingerSettings,
    inputs: Option<&UnboundedSender<Input>>,
    mut stopped: oneshot::Receiver<()>,
) -> Result<(), Failure> {
    let mut playable = ready(settings, inputs)?;
    let mut first = true;
    loop {
        let sound = if first {
            &playable.first
        } else {
            &playable.repeat
        };
        let volume = f64::from(settings.volume()) / 100.0 * playable.gain;
        match play(sound, volume, &mut stopped).await {
            Ok(Ended::Finished) => {}
            Ok(Ended::Stopped) => return Ok(()),
            Err(failure) => {
                // The user's own file may be one `pw-play` can't play: the
                // Chime rings in its place, at once, if it can.
                let Some(file) = playable.own_file.take() else {
                    return Err(failure);
                };
                playable = ring_sound::built_in(RingSound::Chime)?;
                match play(&playable.first, volume, &mut stopped).await {
                    Err(failure) => return Err(failure),
                    Ok(ended) => {
                        unplayable(&file, &failure.detail, inputs);
                        if let Ended::Stopped = ended {
                            return Ok(());
                        }
                    }
                }
            }
        }
        first = false;
        let pause = glib::timeout_future(playable.pause);
        if let Either::Right(_) = select(pause, &mut stopped).await {
            return Ok(());
        }
    }
}

/// Plays the Ring Sound once, as each repeat of a ring plays, until it
/// ends or is `stopped`.
async fn preview(
    settings: &RingerSettings,
    inputs: &UnboundedSender<Input>,
    mut stopped: oneshot::Receiver<()>,
) -> Result<Ended, Failure> {
    let playable = ready(settings, Some(inputs))?;
    let volume = f64::from(settings.volume()) / 100.0 * playable.gain;
    match play(&playable.repeat, volume, &mut stopped).await {
        Err(failure) => {
            let Some(file) = playable.own_file else {
                return Err(failure);
            };
            let chime = ring_sound::built_in(RingSound::Chime)?;
            let ended = play(&chime.repeat, volume, &mut stopped).await?;
            unplayable(&file, &failure.detail, Some(inputs));
            Ok(ended)
        }
        ended => ended,
    }
}

/// Plays `sound` once at `volume` (from 0 to 1, under the output's own),
/// until it ends or is `stopped`.
async fn play(
    sound: &Path,
    volume: f64,
    stopped: &mut oneshot::Receiver<()>,
) -> Result<Ended, Failure> {
    let volume = format!("{:.3}", volume.clamp(0.0, 1.0));
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
        return Ok(Ended::Stopped);
    };
    let exited = glib::child_watch_future(glib::Pid(pid));
    match select(exited, stopped).await {
        Either::Left(((_, 0), _)) => Ok(Ended::Finished),
        // `pw-play` fails at once when no audio server is reachable, or
        // when it can't read the file.
        Either::Left(((_, status), _)) => Err(Failure::new(
            "playback-failed",
            format!("{PLAYER} exited with wait status {status}"),
        )),
        Either::Right((_, exited)) => {
            let _ = player.kill();
            // Reap it with the same watch, so it doesn't linger as a
            // zombie. A second watch on it would find it already reaped by
            // the first, and GLib would warn "waitid ... No child
            // processes".
            exited.await;
            Ok(Ended::Stopped)
        }
    }
}

/// Logs that the user's own `file` couldn't be played, and says so to the
/// Call state machine, so the Ringer Window shows it.
fn unplayable(file: &str, detail: &str, inputs: Option<&UnboundedSender<Input>>) {
    tracing::warn!(
        event_kind = %EventKind::RingSoundUnplayable,
        detail,
        "Couldn't play the chosen file; the Chime rings instead"
    );
    if let Some(inputs) = inputs {
        let _ = inputs.unbounded_send(Input::RingSoundUnplayable {
            file: file.to_owned(),
        });
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
