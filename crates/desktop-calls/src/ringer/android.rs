//! Android's ringtones, from the Android Open Source Project under the
//! Apache License 2.0, built into the binary: six of about 750 KB in all.
//! Their notice and licence are beside them in `data/ringtones/android/`,
//! and `just install` installs both. `pw-play` plays them as they are.

use desktop_calls_core::RingSound;

/// One ringtone: its Ogg Vorbis file, and the stream volume that brings it
/// to the Chime's loudness, about -17.6 LUFS in the file, so no sound
/// rings much louder than another at the same Ringer volume.
pub(super) struct Ringtone {
    pub(super) ogg: &'static [u8],
    pub(super) gain: f64,
}

/// The ringtone `sound` names, if it is one of Android's.
///
/// Gains are 10 ^ ((-17.6 - loudness) / 20), at most 1, from each file's
/// integrated loudness by `ffmpeg`'s `ebur128`: Atria -16.9 LUFS, Ganymede
/// -19.4, Luna -16.3, Oberon -14.1, Triton -15.9, Umbriel -15.1.
pub(super) fn ringtone(sound: RingSound) -> Option<Ringtone> {
    let (ogg, gain): (&'static [u8], f64) = match sound {
        RingSound::Atria => (ATRIA, 0.92),
        RingSound::Ganymede => (GANYMEDE, 1.0),
        RingSound::Luna => (LUNA, 0.86),
        RingSound::Oberon => (OBERON, 0.67),
        RingSound::Triton => (TRITON, 0.82),
        RingSound::Umbriel => (UMBRIEL, 0.75),
        _ => return None,
    };
    Some(Ringtone { ogg, gain })
}

const ATRIA: &[u8] = include_bytes!("../../../../data/ringtones/android/Atria.ogg");
const GANYMEDE: &[u8] = include_bytes!("../../../../data/ringtones/android/Ganymede.ogg");
const LUNA: &[u8] = include_bytes!("../../../../data/ringtones/android/Luna.ogg");
const OBERON: &[u8] = include_bytes!("../../../../data/ringtones/android/Oberon.ogg");
const TRITON: &[u8] = include_bytes!("../../../../data/ringtones/android/Triton.ogg");
const UMBRIEL: &[u8] = include_bytes!("../../../../data/ringtones/android/Umbriel.ogg");
