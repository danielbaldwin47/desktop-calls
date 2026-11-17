//! The player stand-in: `pw-play` as the app's Ringer finds it on `PATH`,
//! and what it recorded of each play (`src/bin/player-stand-in.rs`).

use std::fs;
use std::os::unix::fs::symlink;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde_json::Value;

use super::Harness;

/// The stand-in's binary, among the workspace's.
pub(super) const BINARY: &str = "player-stand-in";
/// What the stand-in takes for PipeWire's socket, in `XDG_RUNTIME_DIR`.
const SOCKET: &str = "pipewire-0";
/// How often a wait reads the record again.
const POLL: Duration = Duration::from_millis(50);

/// Whether the Desktop has an audio server for `pw-play` to reach.
#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub(super) enum AudioServer {
    #[default]
    Present,
    /// The player stand-in fails as `pw-play` does without PipeWire.
    Absent,
}

/// Puts the stand-in first on `environment`'s `PATH`, as `pw-play`, from a
/// folder of its own beside the scenario's `XDG_RUNTIME_DIR`. With an
/// `audio_server`, its socket is in `XDG_RUNTIME_DIR`, as PipeWire's is.
pub(super) fn put_on_path(
    bin_directory: &Path,
    audio_server: AudioServer,
    environment: &mut [(&'static str, String)],
) {
    let runtime = environment
        .iter()
        .find(|(name, _)| *name == "XDG_RUNTIME_DIR")
        .map(|(_, runtime)| PathBuf::from(runtime))
        .expect("the scenario has an XDG_RUNTIME_DIR");
    if audio_server == AudioServer::Present {
        fs::write(runtime.join(SOCKET), "").expect("put the audio server's socket in place");
    }
    let stand_ins = runtime.with_file_name("stand-in-bin");
    fs::create_dir_all(&stand_ins).expect("create the stand-in programs' folder");
    symlink(bin_directory.join(BINARY), stand_ins.join("pw-play"))
        .expect("put the player stand-in in place as pw-play");
    for (name, value) in environment.iter_mut() {
        if *name == "PATH" {
            *value = format!("{}:{value}", stand_ins.display());
        }
    }
}

/// One play of a sound file the stand-in was asked for.
#[derive(Clone, Debug)]
pub struct Play {
    /// The player's process ID.
    pub pid: u32,
    /// The file it was given.
    pub file: PathBuf,
    /// `--volume`, if given.
    pub volume: Option<f64>,
    /// `--media-role`, if given.
    pub media_role: Option<String>,
    /// `--properties` (`-P`), if given.
    pub properties: Option<String>,
    pub started: SystemTime,
    /// When it played to the end; never if it was stopped early.
    pub finished: Option<SystemTime>,
}

impl Play {
    /// The loudest sample of the file's first `window`, from 0 to 1 of full scale.
    ///
    /// # Panics
    ///
    /// If the file isn't a 16-bit PCM WAV file.
    #[must_use]
    pub fn peak_within(&self, window: Duration) -> f64 {
        let (rate, channels, samples) = read_wav(&self.file);
        let frames = window.as_secs_f64() * f64::from(rate);
        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "a few seconds of samples, never negative"
        )]
        let count = frames as usize * usize::from(channels);
        samples
            .iter()
            .take(count)
            .map(|sample| f64::from(sample.unsigned_abs()) / 32_768.0)
            .fold(0.0, f64::max)
    }

    /// The loudest sample of the whole file.
    #[must_use]
    pub fn peak(&self) -> f64 {
        self.peak_within(Duration::from_secs(3_600))
    }

    /// How loud the file sounds while it sounds, in dB of full scale: its
    /// mean power over the 400 ms blocks that aren't near silence, gated
    /// as EBU R 128 gates them (blocks under -70 dB, then those 10 dB under
    /// the rest), but without its K-weighting. Near the integrated loudness
    /// `ffmpeg`'s `ebur128` gives, for sounds in the middle of the range of
    /// hearing.
    ///
    /// # Panics
    ///
    /// If the file isn't a 16-bit PCM WAV file.
    #[must_use]
    pub fn loudness(&self) -> f64 {
        let (rate, channels, samples) = read_wav(&self.file);
        let block = usize::try_from(rate / 10 * 4).unwrap_or(1) * usize::from(channels);
        let powers: Vec<f64> = samples
            .chunks(block)
            .filter(|chunk| chunk.len() == block)
            .map(|chunk| {
                let sum: f64 = chunk
                    .iter()
                    .map(|sample| (f64::from(*sample) / 32_768.0).powi(2))
                    .sum();
                #[expect(clippy::cast_precision_loss, reason = "a block of samples")]
                let mean = sum / chunk.len() as f64;
                mean
            })
            .collect();
        let gated = |powers: &[f64], gate: f64| -> Vec<f64> {
            powers
                .iter()
                .copied()
                .filter(|power| decibels(*power) > gate)
                .collect()
        };
        let audible = gated(&powers, -70.0);
        let relative = decibels(mean(&audible)) - 10.0;
        decibels(mean(&gated(&audible, relative)))
    }
}

fn mean(values: &[f64]) -> f64 {
    #[expect(clippy::cast_precision_loss, reason = "a few blocks")]
    let count = values.len().max(1) as f64;
    values.iter().sum::<f64>() / count
}

fn decibels(power: f64) -> f64 {
    10.0 * power.max(1e-12).log10()
}

impl Harness {
    /// Every play the player stand-in has started, in order.
    #[must_use]
    pub fn plays(&self) -> Vec<Play> {
        let record = self.directory.join("runtime/pw-play-stand-in.jsonl");
        let text = fs::read_to_string(record).unwrap_or_default();
        let lines: Vec<Value> = text
            .lines()
            .filter_map(|line| serde_json::from_str(line).ok())
            .collect();
        let time = |line: &Value, field: &str| {
            line[field]
                .as_u64()
                .map(|ms| UNIX_EPOCH + Duration::from_millis(ms))
        };
        let text_of = |line: &Value, field: &str| line[field].as_str().map(str::to_owned);
        lines
            .iter()
            .filter(|line| line["event"] == "started")
            .map(|line| {
                let pid = line["pid"].as_u64().unwrap_or_default();
                Play {
                    pid: u32::try_from(pid).unwrap_or_default(),
                    file: PathBuf::from(line["file"].as_str().unwrap_or_default()),
                    volume: text_of(line, "volume").and_then(|volume| volume.parse().ok()),
                    media_role: text_of(line, "media-role"),
                    properties: text_of(line, "properties"),
                    started: time(line, "started_ms").unwrap_or(UNIX_EPOCH),
                    finished: lines
                        .iter()
                        .find(|end| end["event"] == "finished" && end["pid"] == pid)
                        .and_then(|end| time(end, "finished_ms")),
                }
            })
            .collect()
    }

    /// Waits until the player stand-in has started `count` plays, and
    /// returns them all.
    ///
    /// # Panics
    ///
    /// If fewer have started by `deadline`.
    pub fn wait_for_plays(&self, count: usize, deadline: Duration) -> Vec<Play> {
        self.wait_for_player(&format!("{count} plays"), deadline, |plays| {
            plays.len() >= count
        })
    }

    /// Waits until no play is still running: every player has played to
    /// the end, or been stopped and reaped, so no process of it is left.
    ///
    /// # Panics
    ///
    /// If one is still running by `deadline`.
    pub fn wait_until_nothing_plays(&self, deadline: Duration) -> Vec<Play> {
        self.wait_for_player("nothing playing", deadline, |plays| {
            plays.iter().all(|play| {
                play.finished.is_some() || !Path::new(&format!("/proc/{}", play.pid)).exists()
            })
        })
    }

    fn wait_for_player(
        &self,
        description: &str,
        deadline: Duration,
        condition: impl Fn(&[Play]) -> bool,
    ) -> Vec<Play> {
        let until = Instant::now() + deadline;
        loop {
            let plays = self.plays();
            if condition(&plays) {
                return plays;
            }
            assert!(
                Instant::now() < until,
                "the player stand-in didn't reach {description} within {deadline:?}: {plays:#?}{}",
                self.timeline.render()
            );
            thread::sleep(POLL);
        }
    }

    /// What the app has written to its standard error so far.
    #[must_use]
    pub fn app_stderr(&self) -> String {
        fs::read_to_string(self.directory.join("desktop-calls.stderr")).unwrap_or_default()
    }
}

/// The sample rate, channel count and interleaved samples of the 16-bit
/// PCM WAV file at `path`.
fn read_wav(path: &Path) -> (u32, u16, Vec<i16>) {
    let bytes = fs::read(path).expect("read the played file");
    assert!(
        bytes.starts_with(b"RIFF") && bytes.get(8..12) == Some(b"WAVE".as_slice()),
        "{} isn't a WAV file",
        path.display()
    );
    let mut format = None;
    let mut at = 12;
    while let Some(header) = bytes.get(at..at + 8) {
        let size = u32::from_le_bytes([header[4], header[5], header[6], header[7]]) as usize;
        let body = &bytes[at + 8..(at + 8 + size).min(bytes.len())];
        match &header[0..4] {
            b"fmt " => {
                let channels = u16::from_le_bytes([body[2], body[3]]);
                let rate = u32::from_le_bytes([body[4], body[5], body[6], body[7]]);
                assert_eq!(
                    u16::from_le_bytes([body[14], body[15]]),
                    16,
                    "16-bit samples"
                );
                format = Some((rate, channels));
            }
            b"data" => {
                let (rate, channels) = format.expect("a fmt chunk before the data");
                let samples = body
                    .as_chunks::<2>()
                    .0
                    .iter()
                    .map(|pair| i16::from_le_bytes(*pair))
                    .collect();
                return (rate, channels, samples);
            }
            _ => {}
        }
        at += 8 + size + size % 2;
    }
    panic!("{} has no data", path.display());
}
