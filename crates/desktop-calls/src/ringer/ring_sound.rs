//! Ring Sounds the app makes itself: synthesised here, so no sound file is
//! bundled and no licence applies but the app's own, and written as WAV
//! files for `pw-play` to play.
//!
//! Each is written twice: as it repeats, and faded in, for the first
//! repeat of a ring.

use std::f32::consts::{PI, TAU};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

const SAMPLE_RATE: u32 = 48_000;
const CHANNELS: u16 = 2;

/// One sound the Ringer can play.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RingSound {
    /// The default: a soft four-note chime, low and round like a marimba,
    /// with no bright overtones.
    Chime,
}

/// A Ring Sound written out, ready to play.
pub struct Written {
    /// The first repeat, faded in.
    pub first: PathBuf,
    /// Every repeat after it.
    pub repeat: PathBuf,
}

impl RingSound {
    fn name(self) -> &'static str {
        match self {
            Self::Chime => "chime",
        }
    }

    /// One repeat: samples from -1 to 1, mono, at [`SAMPLE_RATE`].
    fn samples(self) -> Vec<f32> {
        match self {
            Self::Chime => chime(),
        }
    }

    /// Writes the sound into `directory`, replacing any earlier copy.
    pub fn write(self, directory: &Path) -> io::Result<Written> {
        fs::create_dir_all(directory)?;
        let repeat = self.samples();
        let first = fade_in(&repeat);
        let written = Written {
            first: directory.join(format!("{}-first.wav", self.name())),
            repeat: directory.join(format!("{}.wav", self.name())),
        };
        write_wav(&written.first, &first)?;
        write_wav(&written.repeat, &repeat)?;
        Ok(written)
    }
}

/// A note of the chime: when it starts, its pitch and how loud it is.
struct Note {
    start: f32,
    frequency: f32,
    level: f32,
}

/// F major, rising and falling back, around middle C: F4, A4, C5, A4.
const CHIME: [Note; 4] = [
    Note {
        start: 0.0,
        frequency: 349.23,
        level: 1.0,
    },
    Note {
        start: 0.16,
        frequency: 440.0,
        level: 0.9,
    },
    Note {
        start: 0.32,
        frequency: 523.25,
        level: 0.85,
    },
    Note {
        start: 0.56,
        frequency: 440.0,
        level: 0.8,
    },
];
/// The length of one repeat, in seconds, with the last note's tail.
const CHIME_LENGTH: f32 = 1.8;
/// How long the tail takes to fade to silence, at the end of a repeat.
const TAIL_FADE: f32 = 0.3;
/// Each note's attack: soft, but quick enough to sound struck.
const ATTACK: f32 = 0.012;
/// How fast each note dies away: to about a third every this many seconds.
const DECAY: f32 = 0.38;
/// The loudest sample of a repeat, from 0 to 1 of full scale: -9 dBFS.
/// That puts a repeat at about -17.6 LUFS in the file, and -22 LUFS as
/// played at the default volume of 60 % (measured with `ffmpeg`'s
/// `ebur128`); the old ring, the sound theme's, played at -6.8 LUFS.
const PEAK: f32 = 0.36;

fn chime() -> Vec<f32> {
    let length = seconds_to_samples(CHIME_LENGTH);
    let mut samples = vec![0.0; length];
    for note in &CHIME {
        let start = seconds_to_samples(note.start);
        for (index, sample) in samples.iter_mut().enumerate().skip(start) {
            let time = samples_to_seconds(index - start);
            *sample += note.level * struck(note.frequency, time);
        }
    }
    let tail = seconds_to_samples(TAIL_FADE);
    for (index, sample) in samples.iter_mut().rev().take(tail).enumerate() {
        *sample *= raised_cosine(samples_to_seconds(index) / TAIL_FADE);
    }
    let loudest = samples.iter().copied().map(f32::abs).fold(0.0, f32::max);
    if loudest > 0.0 {
        for sample in &mut samples {
            *sample *= PEAK / loudest;
        }
    }
    samples
}

/// One struck note `time` seconds in: a pure tone, with a little of its
/// octave and of a marimba bar's fourth partial for the strike, both dying
/// away faster than the tone, so it rings round rather than bright.
fn struck(frequency: f32, time: f32) -> f32 {
    let attack = if time < ATTACK {
        raised_cosine(time / ATTACK)
    } else {
        1.0
    };
    let tone = (TAU * frequency * time).sin();
    let octave = 0.15 * (TAU * 2.0 * frequency * time).sin() * (-time / 0.12).exp();
    let strike = 0.05 * (TAU * 3.93 * frequency * time).sin() * (-time / 0.03).exp();
    attack * (-time / DECAY).exp() * (tone + octave + strike)
}

/// How long the first repeat of a ring takes to fade in, in seconds.
const FADE_IN: f32 = 1.0;
/// The level the first repeat fades in from: a fifth of its own.
const FADE_FROM: f32 = 0.2;

/// The first repeat of a ring, rising from [`FADE_FROM`] of its level to
/// all of it over its first [`FADE_IN`] seconds.
fn fade_in(samples: &[f32]) -> Vec<f32> {
    samples
        .iter()
        .enumerate()
        .map(|(index, sample)| {
            let progress = (samples_to_seconds(index) / FADE_IN).min(1.0);
            sample * (FADE_FROM + (1.0 - FADE_FROM) * raised_cosine(progress))
        })
        .collect()
}

/// From 0 at `progress` 0 to 1 at 1, smoothly.
fn raised_cosine(progress: f32) -> f32 {
    0.5 - 0.5 * (PI * progress).cos()
}

#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss,
    reason = "a few seconds at 48 kHz: well within f32 and usize, never negative"
)]
fn seconds_to_samples(seconds: f32) -> usize {
    (seconds * SAMPLE_RATE as f32).round() as usize
}

#[expect(
    clippy::cast_precision_loss,
    reason = "a few seconds of samples, exact in f32"
)]
fn samples_to_seconds(samples: usize) -> f32 {
    samples as f32 / SAMPLE_RATE as f32
}

/// Writes `samples` to `path` as a 16-bit PCM WAV file, the same on both
/// channels: first beside it, then into place, so a player never reads
/// half a file.
fn write_wav(path: &Path, samples: &[f32]) -> io::Result<()> {
    let too_long = || io::Error::other("the sound is too long for a WAV file");
    let data_size =
        u32::try_from(samples.len() * usize::from(CHANNELS) * 2).map_err(|_| too_long())?;
    let block = CHANNELS * 2;
    let mut bytes = Vec::with_capacity(44 + data_size as usize);
    bytes.extend_from_slice(b"RIFF");
    bytes.extend_from_slice(&(36 + data_size).to_le_bytes());
    bytes.extend_from_slice(b"WAVEfmt ");
    bytes.extend_from_slice(&16_u32.to_le_bytes());
    bytes.extend_from_slice(&1_u16.to_le_bytes());
    bytes.extend_from_slice(&CHANNELS.to_le_bytes());
    bytes.extend_from_slice(&SAMPLE_RATE.to_le_bytes());
    bytes.extend_from_slice(&(SAMPLE_RATE * u32::from(block)).to_le_bytes());
    bytes.extend_from_slice(&block.to_le_bytes());
    bytes.extend_from_slice(&16_u16.to_le_bytes());
    bytes.extend_from_slice(b"data");
    bytes.extend_from_slice(&data_size.to_le_bytes());
    for sample in samples {
        #[expect(
            clippy::cast_possible_truncation,
            reason = "clamped to the range of i16 first"
        )]
        let value = (sample.clamp(-1.0, 1.0) * f32::from(i16::MAX)).round() as i16;
        for _ in 0..CHANNELS {
            bytes.extend_from_slice(&value.to_le_bytes());
        }
    }
    let partial = path.with_extension(format!("wav.{}", std::process::id()));
    fs::write(&partial, bytes)?;
    fs::rename(partial, path)
}
