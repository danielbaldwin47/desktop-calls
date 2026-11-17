//! `player-stand-in`: a stand-in for PipeWire's `pw-play`, which the app's
//! Ringer runs. The harness puts it first on every scenario's `PATH` under
//! the name `pw-play`, so a ringing Call plays here rather than failing for
//! want of an audio server.
//!
//! Like `pw-play`, it fails at once, with exit status 1, when there is no
//! audio server: no `pipewire-0` in `$XDG_RUNTIME_DIR`, which the harness
//! puts there unless a scenario asks for none.
//!
//! It takes the options of `pw-play` the app may pass, and refuses any
//! other, as `pw-play` does. It plays only 16-bit PCM WAV files: it reads
//! the file's length from its header and takes as long as the real player
//! would, then exits 0. A file it can't read fails it at once, with exit
//! status 1.
//!
//! Each play is recorded as JSON lines appended to
//! `$XDG_RUNTIME_DIR/pw-play-stand-in.jsonl`, where the real player would
//! find its audio server: one `started` line before it plays, with the
//! options and file it was given, and one `finished` line once it has
//! played to the end. A play stopped early (killed) has no `finished` line.

use std::env;
use std::fs::{self, OpenOptions};
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde_json::{Value, json};

/// Options that take a value, as `pw-play --help` lists them.
const VALUE_OPTIONS: [&str; 7] = [
    "--volume",
    "--media-role",
    "--media-category",
    "--media-type",
    "--target",
    "--latency",
    "--properties",
];

fn main() -> ExitCode {
    let Some(play) = parse(env::args().skip(1)) else {
        eprintln!("pw-play: unknown option or missing file");
        return ExitCode::FAILURE;
    };
    let server = env::var_os("XDG_RUNTIME_DIR")
        .is_some_and(|runtime| Path::new(&runtime).join("pipewire-0").exists());
    if !server {
        eprintln!("pw-play: can't connect: Host is down");
        return ExitCode::FAILURE;
    }
    let Some(length) = wav_length(&play.file) else {
        eprintln!("pw-play: can't open {}", play.file.display());
        return ExitCode::FAILURE;
    };
    let pid = std::process::id();
    let mut started = json!({
        "event": "started",
        "pid": pid,
        "file": play.file,
        "started_ms": now_ms(),
        "length_ms": length.as_millis(),
    });
    for (option, value) in &play.options {
        started[option.trim_start_matches("--")] = Value::from(value.as_str());
    }
    if !record(&started) {
        return ExitCode::FAILURE;
    }
    thread::sleep(length);
    if !record(&json!({ "event": "finished", "pid": pid, "finished_ms": now_ms() })) {
        return ExitCode::FAILURE;
    }
    ExitCode::SUCCESS
}

struct Play {
    options: Vec<(String, String)>,
    file: PathBuf,
}

/// The options and file `pw-play` was given; nothing for an option it
/// doesn't know, or no file.
fn parse(mut arguments: impl Iterator<Item = String>) -> Option<Play> {
    let mut options = Vec::new();
    let mut file = None;
    while let Some(argument) = arguments.next() {
        let argument = if argument == "-P" {
            "--properties".to_owned()
        } else {
            argument
        };
        if let Some((option, value)) = argument.split_once('=')
            && VALUE_OPTIONS.contains(&option)
        {
            options.push((option.to_owned(), value.to_owned()));
        } else if VALUE_OPTIONS.contains(&argument.as_str()) {
            options.push((argument, arguments.next()?));
        } else if argument.starts_with('-') || file.is_some() {
            return None;
        } else {
            file = Some(PathBuf::from(argument));
        }
    }
    Some(Play {
        options,
        file: file?,
    })
}

/// How long the 16-bit PCM WAV file at `path` plays for.
fn wav_length(path: &Path) -> Option<Duration> {
    let bytes = fs::read(path).ok()?;
    if bytes.get(0..4)? != b"RIFF" || bytes.get(8..12)? != b"WAVE" {
        return None;
    }
    let mut format = None;
    let mut at = 12;
    while let Some(header) = bytes.get(at..at + 8) {
        let size = usize::try_from(u32::from_le_bytes(header[4..8].try_into().ok()?)).ok()?;
        let body = bytes.get(at + 8..at + 8 + size)?;
        match &header[0..4] {
            b"fmt " => {
                let word = |offset: usize| u16::from_le_bytes([body[offset], body[offset + 1]]);
                let pcm = word(0) == 1 && word(14) == 16;
                let rate = u32::from_le_bytes(body.get(4..8)?.try_into().ok()?);
                format = pcm.then_some((word(2), rate));
            }
            b"data" => {
                let (channels, rate) = format?;
                let frames = size / (2 * usize::from(channels));
                return Some(Duration::from_secs_f64(
                    f64::from(u32::try_from(frames).ok()?) / f64::from(rate),
                ));
            }
            _ => {}
        }
        at += 8 + size + size % 2;
    }
    None
}

/// Appends `line` to the record, saying so on standard error if it can't.
fn record(line: &Value) -> bool {
    let Some(runtime) = env::var_os("XDG_RUNTIME_DIR") else {
        eprintln!("pw-play: no XDG_RUNTIME_DIR");
        return false;
    };
    let written = OpenOptions::new()
        .create(true)
        .append(true)
        .open(Path::new(&runtime).join("pw-play-stand-in.jsonl"))
        .and_then(|mut file| file.write_all(format!("{line}\n").as_bytes()));
    if let Err(error) = written {
        eprintln!("pw-play: can't record the play: {error}");
        return false;
    }
    true
}

fn now_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| since.as_millis())
}
