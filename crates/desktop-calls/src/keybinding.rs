//! The Desktop Calls keybinding in the Hyprland config, which the Setup
//! Window's finish page offers to add.
//!
//! Only Omarchy's own place for personal keybindings is supported:
//! `$XDG_CONFIG_HOME/hypr/bindings.lua` (`~/.config` by default), whose
//! lines read `o.bind("SUPER + SHIFT + R", "SSH", "alacritty -e ssh …")`.
//! The binding runs this very binary with no arguments: the Dialer, or the
//! ringing or live Call. Nothing is written until the user confirmed the
//! exact line, and then only that line, appended to the end of the file.

use std::fs::OpenOptions;
use std::io::{self, Read as _, Write as _};
use std::path::PathBuf;

use desktop_calls_core::{Input, KeybindingCheck};

use crate::logging::EventKind;

/// Free in Omarchy's own bindings (4.5): C for Calls.
const KEYS: &str = "SUPER + ALT + C";
/// What Omarchy's keybinding menu lists the binding as.
const DESCRIPTION: &str = "Desktop Calls";

/// Why the keybinding couldn't be added.
#[derive(Debug)]
enum KeybindingError {
    Io(io::Error),
    /// The file changed since the line was offered: it binds the keys now.
    Changed,
}

impl KeybindingError {
    /// The kind logs carry as `error_kind`.
    fn as_str(&self) -> &'static str {
        match self {
            Self::Io(_) => "keybinding-io",
            Self::Changed => "keybinding-changed",
        }
    }

    fn detail(&self) -> String {
        match self {
            Self::Io(error) => error.to_string(),
            Self::Changed => String::new(),
        }
    }
}

/// What the Hyprland config can take for the keybinding. Reads a file:
/// call it off the main thread.
pub fn check() -> KeybindingCheck {
    let command = command();
    let Some(file) = bindings_file().filter(|file| file.is_file()) else {
        return KeybindingCheck::NoConfig { command };
    };
    let shown = file.display().to_string();
    let line = line(&command);
    match read(&file) {
        Ok(config) => match binds_keys(&config, &line) {
            Some(true) => KeybindingCheck::Added {
                keys: KEYS.to_owned(),
                file: shown,
            },
            Some(false) => KeybindingCheck::Taken {
                keys: KEYS.to_owned(),
                file: shown,
                command,
            },
            None => KeybindingCheck::Available {
                keys: KEYS.to_owned(),
                file: shown,
                line,
            },
        },
        Err(_) => KeybindingCheck::NoConfig { command },
    }
}

/// Appends `line`, and nothing else, to the end of `file`, keeping what is
/// there; what became of it, for the Call state machine. Writes a file:
/// call it off the main thread.
pub fn add(file: &str, line: &str) -> Input {
    match append(file, line) {
        Ok(()) => {
            tracing::info!(
                event_kind = %EventKind::KeybindingAdded,
                file,
                "Added the Desktop Calls keybinding to {file}"
            );
            Input::KeybindingAdded
        }
        Err(error) => {
            tracing::warn!(
                event_kind = %EventKind::KeybindingNotAdded,
                error_kind = error.as_str(),
                detail = error.detail(),
                file,
                "Couldn't add the Desktop Calls keybinding to {file}"
            );
            Input::KeybindingNotAdded
        }
    }
}

fn append(file: &str, line: &str) -> Result<(), KeybindingError> {
    let config = read(std::path::Path::new(file)).map_err(KeybindingError::Io)?;
    match binds_keys(&config, line) {
        Some(true) => return Ok(()),
        Some(false) => return Err(KeybindingError::Changed),
        None => {}
    }
    let mut added = String::new();
    if !config.is_empty() && !config.ends_with('\n') {
        added.push('\n');
    }
    added.push_str(line);
    added.push('\n');
    // Appending writes through a symlink, as dotfile managers keep them.
    OpenOptions::new()
        .append(true)
        .open(file)
        .and_then(|mut opened| opened.write_all(added.as_bytes()))
        .map_err(KeybindingError::Io)
}

fn read(file: &std::path::Path) -> io::Result<String> {
    let mut config = String::new();
    std::fs::File::open(file)?.read_to_string(&mut config)?;
    Ok(config)
}

/// Whether `config` binds the keys: `Some(true)` with exactly `line`,
/// `Some(false)` to something else, `None` not at all. Comments don't
/// count.
fn binds_keys(config: &str, line: &str) -> Option<bool> {
    let keys = squashed(&format!("\"{KEYS}\""));
    let mut taken = None;
    for candidate in config.lines().map(str::trim) {
        if candidate.starts_with("--") {
            continue;
        }
        if candidate == line {
            return Some(true);
        }
        if squashed(candidate).contains(&keys) {
            taken = Some(false);
        }
    }
    taken
}

/// Lower case, without spaces: `"SUPER + ALT + C"` and `"super+alt+c"` are
/// the same keys.
fn squashed(text: &str) -> String {
    text.chars()
        .filter(|character| !character.is_whitespace())
        .flat_map(char::to_lowercase)
        .collect()
}

/// The line that binds the keys to `command`.
fn line(command: &str) -> String {
    format!(
        "o.bind({}, {}, {})",
        lua_string(KEYS),
        lua_string(DESCRIPTION),
        lua_string(command)
    )
}

/// The command the keybinding runs: this binary, by its full path, as the
/// installer's desktop file and systemd unit run it, quoted for the shell
/// Hyprland runs it with.
fn command() -> String {
    let path = std::env::current_exe().map_or_else(
        |_| "desktop-calls".to_owned(),
        |path| path.display().to_string(),
    );
    // Linux marks a binary replaced while running.
    let path = path.strip_suffix(" (deleted)").unwrap_or(&path);
    shell_word(path)
}

fn shell_word(word: &str) -> String {
    let plain = word
        .chars()
        .all(|character| character.is_ascii_alphanumeric() || "/._-+".contains(character));
    if plain {
        word.to_owned()
    } else {
        format!("'{}'", word.replace('\'', r"'\''"))
    }
}

fn lua_string(text: &str) -> String {
    format!("\"{}\"", text.replace('\\', r"\\").replace('"', "\\\""))
}

/// `$XDG_CONFIG_HOME/hypr/bindings.lua`, with the XDG default of
/// `~/.config`.
fn bindings_file() -> Option<PathBuf> {
    std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .or_else(|| {
            std::env::var_os("HOME")
                .map(PathBuf::from)
                .filter(|home| home.is_absolute())
                .map(|home| home.join(".config"))
        })
        .map(|config| config.join("hypr").join("bindings.lua"))
}
