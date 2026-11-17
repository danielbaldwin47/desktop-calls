//! Writing a Ring Sound the app makes as a WAV file for `pw-play`.

use std::io;

use super::tones::SAMPLE_RATE;

const CHANNELS: u16 = 2;

/// `samples` (mono, from -1 to 1) as a 16-bit PCM WAV file, the same on
/// both channels.
pub(super) fn encode(samples: &[f32]) -> io::Result<Vec<u8>> {
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
    Ok(bytes)
}
