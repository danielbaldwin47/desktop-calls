//! What each Ring Sound plays from, written out for `pw-play`: the sounds
//! the app makes as WAV files, twice (as each repeat plays, and faded in,
//! for a ring's first repeat); Android's ringtones as the Ogg files built
//! in; the user's own file where it is.

use std::path::{Path, PathBuf};
use std::time::Duration;

use desktop_calls_core::{RingSound, RingerSettings};

use super::{Failure, android, directory, tones, wav};

/// A Ring Sound, ready to play.
pub(super) struct Playable {
    /// The first repeat of a ring: faded in, for the sounds the app makes.
    pub(super) first: PathBuf,
    /// Every repeat after it, and a preview.
    pub(super) repeat: PathBuf,
    /// The stream volume that brings it to the others' loudness.
    pub(super) gain: f64,
    /// The silence after each repeat of a ring: none for a sound made to
    /// loop, as Android's ringtones are.
    pub(super) pause: Duration,
    /// The user's own file, which may yet fail to play.
    pub(super) own_file: Option<String>,
    /// The least time a play of it takes: one that ends sooner had next to
    /// no sound in it.
    pub(super) shortest: Duration,
}

/// The silence after each repeat of a sound the app makes, or the user's
/// own.
const PAUSE: Duration = Duration::from_secs(2);

/// The least time the user's own file plays for: a file that decodes to
/// nothing, or to a click, isn't a ring.
const SHORTEST: Duration = Duration::from_millis(500);

/// Why the user's own file can't play.
pub(super) struct Missing(pub(super) String);

/// The Ring Sound `settings` name, ready to play; for the user's own file,
/// [`Missing`] if it isn't a readable file.
pub(super) fn playable(settings: &RingerSettings) -> Result<Result<Playable, Missing>, Failure> {
    let sound = settings.sound();
    if let (RingSound::File, Some(file)) = (sound, settings.file()) {
        let path = Path::new(file);
        if !path.is_file() || std::fs::File::open(path).is_err() {
            return Ok(Err(Missing(file.to_owned())));
        }
        return Ok(Ok(Playable {
            first: path.to_owned(),
            repeat: path.to_owned(),
            gain: 1.0,
            pause: PAUSE,
            own_file: Some(file.to_owned()),
            shortest: SHORTEST,
        }));
    }
    built_in(sound).map(Ok)
}

/// One of the Ring Sounds built into the app, written out; the Chime for
/// the user's own file.
pub(super) fn built_in(sound: RingSound) -> Result<Playable, Failure> {
    let unwritten = |error: std::io::Error| Failure::new("ring-sound-unwritten", error.to_string());
    if let Some(ringtone) = android::ringtone(sound) {
        let name = format!("{}.ogg", sound.key());
        let directory = directory::write_all(&[(&name, ringtone.ogg)]).map_err(unwritten)?;
        let path = directory.join(name);
        return Ok(Playable {
            first: path.clone(),
            repeat: path,
            gain: ringtone.gain,
            pause: Duration::ZERO,
            own_file: None,
            shortest: Duration::ZERO,
        });
    }
    let (sound, repeat) = match tones::samples(sound) {
        Some(samples) => (sound, samples),
        None => (
            RingSound::Chime,
            tones::samples(RingSound::Chime).unwrap_or_default(),
        ),
    };
    let first = wav::encode(&tones::fade_in(&repeat)).map_err(unwritten)?;
    let repeat = wav::encode(&repeat).map_err(unwritten)?;
    let (first_name, repeat_name) = (
        format!("{}-first.wav", sound.key()),
        format!("{}.wav", sound.key()),
    );
    let directory = directory::write_all(&[(&first_name, &first), (&repeat_name, &repeat)])
        .map_err(unwritten)?;
    Ok(Playable {
        first: directory.join(first_name),
        repeat: directory.join(repeat_name),
        gain: 1.0,
        pause: PAUSE,
        own_file: None,
        shortest: Duration::ZERO,
    })
}
