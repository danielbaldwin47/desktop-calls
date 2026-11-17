//! The Ring Sounds the app makes itself, synthesised here, so no sound file
//! is bundled for them and no licence applies but the app's own. Each is
//! one repeat: soft notes with quick attacks and gentle decays, no bright
//! overtones, at about the same loudness as the others.

use std::f32::consts::{PI, TAU};

use desktop_calls_core::RingSound;

pub(super) const SAMPLE_RATE: u32 = 48_000;

/// One repeat of a built-in Ring Sound: samples from -1 to 1, mono, at
/// [`SAMPLE_RATE`]. `None` for a sound the app doesn't make.
pub(super) fn samples(sound: RingSound) -> Option<Vec<f32>> {
    let (mut samples, peak) = match sound {
        RingSound::Chime => (notes(&CHIME), CHIME.peak),
        RingSound::Bells => (notes(&BELLS), BELLS.peak),
        RingSound::Glass => (notes(&GLASS), GLASS.peak),
        RingSound::Harp => (notes(&HARP), HARP.peak),
        RingSound::Purr => (purr(), PURR_PEAK),
        RingSound::Drops => (drops(), DROPS_PEAK),
        _ => return None,
    };
    fade_tail(&mut samples);
    normalise(&mut samples, peak);
    Some(samples)
}

/// A note: when it starts, its pitch and how loud it is.
struct Note {
    start: f32,
    frequency: f32,
    level: f32,
}

/// An overtone of a note: its pitch as a multiple of the note's, how loud
/// it is beside the note, and how fast it dies away (to about a third
/// every this many seconds), faster than the note so it rings round.
struct Overtone {
    ratio: f32,
    level: f32,
    decay: f32,
}

/// A Ring Sound made of struck notes.
struct Notes {
    played: &'static [Note],
    overtones: &'static [Overtone],
    /// Each note's attack: soft, but quick enough to sound struck.
    attack: f32,
    /// How fast each note dies away: to about a third every this many
    /// seconds.
    decay: f32,
    /// The length of one repeat, in seconds, with the last note's tail.
    length: f32,
    /// The loudest sample, from 0 to 1 of full scale, chosen so each sound
    /// is about as loud as the Chime: about -17.6 LUFS in the file, and
    /// -22 LUFS as played at the default volume of 60 % (measured with
    /// `ffmpeg`'s `ebur128`).
    peak: f32,
}

/// The default: F major, rising and falling back, around middle C: F4, A4,
/// C5, A4, like a marimba.
const CHIME: Notes = Notes {
    played: &[
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
    ],
    // A little of the octave, and of a marimba bar's fourth partial for the
    // strike.
    overtones: &[
        Overtone {
            ratio: 2.0,
            level: 0.15,
            decay: 0.12,
        },
        Overtone {
            ratio: 3.93,
            level: 0.05,
            decay: 0.03,
        },
    ],
    attack: 0.012,
    decay: 0.38,
    length: 1.8,
    // -9 dBFS.
    peak: 0.36,
};

/// Two soft bell strokes, falling a major third: A4, then F4.
const BELLS: Notes = Notes {
    played: &[
        Note {
            start: 0.0,
            frequency: 440.0,
            level: 1.0,
        },
        Note {
            start: 0.5,
            frequency: 349.23,
            level: 0.9,
        },
    ],
    // A bell's partials, not whole multiples, which give it its colour.
    overtones: &[
        Overtone {
            ratio: 2.0,
            level: 0.3,
            decay: 0.5,
        },
        Overtone {
            ratio: 2.76,
            level: 0.15,
            decay: 0.3,
        },
        Overtone {
            ratio: 5.4,
            level: 0.04,
            decay: 0.1,
        },
    ],
    attack: 0.005,
    decay: 0.8,
    length: 2.3,
    peak: 0.39,
};

/// Three clear notes rising, like a glass struck: C5, E5, G5.
const GLASS: Notes = Notes {
    played: &[
        Note {
            start: 0.0,
            frequency: 523.25,
            level: 0.9,
        },
        Note {
            start: 0.18,
            frequency: 659.25,
            level: 0.85,
        },
        Note {
            start: 0.36,
            frequency: 783.99,
            level: 0.8,
        },
    ],
    overtones: &[Overtone {
        ratio: 2.0,
        level: 0.06,
        decay: 0.15,
    }],
    attack: 0.02,
    decay: 0.5,
    length: 1.7,
    peak: 0.4,
};

/// A quick plucked arpeggio up D major: D4, F♯4, A4, D5.
const HARP: Notes = Notes {
    played: &[
        Note {
            start: 0.0,
            frequency: 293.66,
            level: 1.0,
        },
        Note {
            start: 0.08,
            frequency: 369.99,
            level: 0.9,
        },
        Note {
            start: 0.16,
            frequency: 440.0,
            level: 0.85,
        },
        Note {
            start: 0.24,
            frequency: 587.33,
            level: 0.8,
        },
    ],
    overtones: &[
        Overtone {
            ratio: 2.0,
            level: 0.4,
            decay: 0.25,
        },
        Overtone {
            ratio: 3.0,
            level: 0.15,
            decay: 0.12,
        },
        Overtone {
            ratio: 4.0,
            level: 0.05,
            decay: 0.06,
        },
    ],
    attack: 0.003,
    decay: 0.7,
    length: 1.7,
    peak: 0.49,
};

fn notes(sound: &Notes) -> Vec<f32> {
    let mut samples = vec![0.0; seconds_to_samples(sound.length)];
    for note in sound.played {
        let start = seconds_to_samples(note.start);
        for (index, sample) in samples.iter_mut().enumerate().skip(start) {
            let time = samples_to_seconds(index - start);
            *sample += note.level * struck(sound, note.frequency, time);
        }
    }
    samples
}

/// One struck note `time` seconds in: a pure tone and its overtones.
fn struck(sound: &Notes, frequency: f32, time: f32) -> f32 {
    let attack = if time < sound.attack {
        raised_cosine(time / sound.attack)
    } else {
        1.0
    };
    let overtones: f32 = sound
        .overtones
        .iter()
        .map(|overtone| {
            overtone.level
                * (TAU * overtone.ratio * frequency * time).sin()
                * (-time / overtone.decay).exp()
        })
        .sum();
    attack * (-time / sound.decay).exp() * ((TAU * frequency * time).sin() + overtones)
}

/// A low, soft double trill: a major third, E4 and G♯4, swelling and
/// fading at 16 times a second, twice, like an old phone heard through a
/// wall.
fn purr() -> Vec<f32> {
    const BURSTS: [(f32, f32); 2] = [(0.0, 0.5), (0.7, 1.2)];
    const EDGE: f32 = 0.04;
    const TREMOLO: f32 = 16.0;
    let mut samples = vec![0.0; seconds_to_samples(1.4)];
    for (index, sample) in samples.iter_mut().enumerate() {
        let time = samples_to_seconds(index);
        let Some(&(from, to)) = BURSTS
            .iter()
            .find(|(from, to)| (*from..*to).contains(&time))
        else {
            continue;
        };
        let edge = raised_cosine(((time - from) / EDGE).min(1.0))
            * raised_cosine(((to - time) / EDGE).min(1.0));
        let swell = 0.65 + 0.35 * (TAU * TREMOLO * time).sin();
        let tone = (TAU * 329.63 * time).sin()
            + 0.8 * (TAU * 415.3 * time).sin()
            + 0.1 * (TAU * 659.26 * time).sin();
        *sample = edge * swell * tone;
    }
    samples
}
const PURR_PEAK: f32 = 0.36;

/// Water drops: four short notes, each gliding up a little as it starts.
fn drops() -> Vec<f32> {
    const DROPS: [(f32, f32); 4] = [(0.0, 600.0), (0.16, 800.0), (0.27, 700.0), (0.48, 900.0)];
    let mut samples = vec![0.0; seconds_to_samples(1.1)];
    for (start, frequency) in DROPS {
        let mut phase = 0.0_f32;
        for (index, sample) in samples
            .iter_mut()
            .enumerate()
            .skip(seconds_to_samples(start))
        {
            let time = samples_to_seconds(index) - start;
            let glide = 1.0 + 0.5 * (1.0 - (-time / 0.025).exp());
            phase += TAU * frequency * glide * samples_to_seconds(1);
            let attack = raised_cosine((time / 0.003).min(1.0));
            *sample += attack * (-time / 0.09).exp() * phase.sin();
        }
    }
    samples
}
const DROPS_PEAK: f32 = 0.36;

/// How long a repeat's tail takes to fade to silence, at its end.
const TAIL_FADE: f32 = 0.3;

fn fade_tail(samples: &mut [f32]) {
    let tail = seconds_to_samples(TAIL_FADE);
    for (index, sample) in samples.iter_mut().rev().take(tail).enumerate() {
        *sample *= raised_cosine(samples_to_seconds(index) / TAIL_FADE);
    }
}

/// Scales `samples` so the loudest is `peak`.
fn normalise(samples: &mut [f32], peak: f32) {
    let loudest = samples.iter().copied().map(f32::abs).fold(0.0, f32::max);
    if loudest > 0.0 {
        for sample in samples {
            *sample *= peak / loudest;
        }
    }
}

/// How long the first repeat of a ring takes to fade in, in seconds.
const FADE_IN: f32 = 1.0;
/// The level the first repeat fades in from: a fifth of its own.
const FADE_FROM: f32 = 0.2;

/// The first repeat of a ring, rising from [`FADE_FROM`] of its level to
/// all of it over its first [`FADE_IN`] seconds.
pub(super) fn fade_in(samples: &[f32]) -> Vec<f32> {
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
