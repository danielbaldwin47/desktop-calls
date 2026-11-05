//! The Desktop Calls keybinding in the Hyprland config, which the Setup
//! Window's finish page offers to add.
//!
//! This reads and writes files, and decides nothing: it reports what
//! Omarchy's `$XDG_CONFIG_HOME/hypr/bindings.lua` (`~/.config` by default)
//! and the files it requires hold, and the core decides whether the keys
//! are free and what the file holds with the line added
//! (`desktop_calls_core::keybinding`). Nothing is written until the user
//! confirmed the exact line, and then only if the file still holds what it
//! held when the line was offered.

use std::io::{self, Read as _};
use std::path::{Path, PathBuf};

use desktop_calls_core::keybinding::required;
use desktop_calls_core::{ConfigFile, HyprlandConfig, Input, RequiredFile};

use crate::logging::EventKind;

/// Why the keybinding couldn't be added.
#[derive(Debug)]
enum KeybindingError {
    Io(io::Error),
    /// The file changed since the line was offered.
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

/// What the Hyprland config holds. Reads files: call it off the main
/// thread.
pub fn check() -> HyprlandConfig {
    check_in(config_home().as_deref(), command())
}

/// What the Hyprland config under `config_home` holds, for a keybinding
/// that runs `command`.
fn check_in(config_home: Option<&Path>, command: String) -> HyprlandConfig {
    let bindings = config_home.and_then(|home| {
        let file = home.join("hypr").join("bindings.lua");
        // A directory, or a file that can't be read, is there all the same.
        file.exists().then(|| config_file(&file, home))
    });
    HyprlandConfig { command, bindings }
}

fn config_file(file: &Path, config_home: &Path) -> ConfigFile {
    let text = read(file).ok();
    let required = text
        .as_deref()
        .map(required)
        .unwrap_or_default()
        .into_iter()
        .filter_map(|name| {
            let path = resolve(&name, config_home)?;
            let text = read(&path).ok()?;
            Some(RequiredFile {
                name,
                path: path.display().to_string(),
                text,
            })
        })
        .collect();
    ConfigFile {
        path: file.display().to_string(),
        text,
        required,
    }
}

/// The file a `require` or `dofile` names: a path, or a Lua module under
/// the config directory or Omarchy's own, as Omarchy's `package.path` has
/// them.
fn resolve(name: &str, config_home: &Path) -> Option<PathBuf> {
    let path = Path::new(name);
    if path.is_absolute() {
        return path.is_file().then(|| path.to_owned());
    }
    let module = format!(
        "{}.lua",
        name.strip_suffix(".lua").unwrap_or(name).replace('.', "/")
    );
    let omarchy = std::env::var_os("OMARCHY_PATH")
        .map_or_else(|| PathBuf::from("/usr/share/omarchy"), PathBuf::from);
    [config_home.to_owned(), omarchy]
        .into_iter()
        .map(|base| base.join(&module))
        .find(|candidate| candidate.is_file())
}

/// Writes `added` to `file` if it still holds `seen`: what became of it,
/// for the Call state machine. Writes a file: call it off the main thread.
pub fn add(file: &str, seen: &str, added: &str) -> Input {
    match write(Path::new(file), seen, added) {
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
            match error {
                // What the same config holds now: `file` is
                // `<config home>/hypr/bindings.lua`.
                KeybindingError::Changed => Input::KeybindingConfigChanged(check_in(
                    Path::new(file).parent().and_then(Path::parent),
                    command(),
                )),
                KeybindingError::Io(_) => Input::KeybindingNotAdded,
            }
        }
    }
}

fn write(file: &Path, seen: &str, added: &str) -> Result<(), KeybindingError> {
    if read(file).map_err(KeybindingError::Io)? != seen {
        return Err(KeybindingError::Changed);
    }
    // Written in place, so a symlink, as dotfile managers keep them, and
    // the file's permissions stay.
    std::fs::write(file, added).map_err(KeybindingError::Io)
}

fn read(file: &Path) -> io::Result<String> {
    let mut config = String::new();
    std::fs::File::open(file)?.read_to_string(&mut config)?;
    Ok(config)
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

/// `$XDG_CONFIG_HOME`, with the XDG default of `~/.config`.
fn config_home() -> Option<PathBuf> {
    std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .or_else(|| {
            std::env::var_os("HOME")
                .map(PathBuf::from)
                .filter(|home| home.is_absolute())
                .map(|home| home.join(".config"))
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use desktop_calls_core::KeybindingCheck;

    const COMMAND: &str = "/usr/bin/desktop-calls";

    fn bindings(home: &Path) -> PathBuf {
        home.join("hypr").join("bindings.lua")
    }

    fn with_bindings(text: &str) -> tempfile::TempDir {
        let home = tempfile::tempdir().expect("a temporary config directory");
        std::fs::create_dir_all(home.path().join("hypr")).expect("the hypr directory");
        std::fs::write(bindings(home.path()), text).expect("write bindings.lua");
        home
    }

    fn checked(home: &Path) -> KeybindingCheck {
        KeybindingCheck::of(&check_in(Some(home), COMMAND.to_owned()))
    }

    /// Offers the line, and confirms it: what `add` reports.
    fn confirm(home: &Path) -> (KeybindingCheck, Input) {
        let check = checked(home);
        let KeybindingCheck::Available {
            file, seen, added, ..
        } = &check
        else {
            panic!("the line isn't offered: {check:?}");
        };
        let input = add(file, seen, added);
        (check, input)
    }

    #[test]
    fn with_no_bindings_file_there_is_no_config_to_add_to() {
        let home = tempfile::tempdir().expect("a temporary config directory");
        assert_eq!(
            checked(home.path()),
            KeybindingCheck::NoConfig {
                command: COMMAND.to_owned()
            }
        );
    }

    #[test]
    fn a_bindings_file_that_cant_be_read_is_named_and_left_alone() {
        let home = tempfile::tempdir().expect("a temporary config directory");
        // A directory where the file should be: no reading it, as root too.
        std::fs::create_dir_all(bindings(home.path())).expect("the directory");
        assert_eq!(
            checked(home.path()),
            KeybindingCheck::Unreadable {
                file: bindings(home.path()).display().to_string(),
                command: COMMAND.to_owned()
            }
        );
    }

    #[test]
    fn a_file_without_a_final_newline_gets_the_line_on_a_line_of_its_own() {
        let home = with_bindings("-- mine\no.bind(\"SUPER + B\", \"Browser\", \"firefox\")");
        let (check, input) = confirm(home.path());
        assert_eq!(input, Input::KeybindingAdded);
        let KeybindingCheck::Available { line, .. } = check else {
            unreachable!()
        };
        assert_eq!(
            read(&bindings(home.path())).expect("read bindings.lua"),
            format!("-- mine\no.bind(\"SUPER + B\", \"Browser\", \"firefox\")\n{line}\n")
        );
    }

    #[test]
    fn adding_twice_writes_the_line_once_and_then_finds_it_added() {
        let home = with_bindings("-- mine\n");
        let (check, first) = confirm(home.path());
        assert_eq!(first, Input::KeybindingAdded);
        let KeybindingCheck::Available {
            file, seen, added, ..
        } = &check
        else {
            unreachable!()
        };
        let second = add(file, seen, added);
        let Input::KeybindingConfigChanged(config) = second else {
            panic!("written twice: {second:?}");
        };
        assert!(matches!(
            KeybindingCheck::of(&config),
            KeybindingCheck::Added { .. }
        ));
        assert_eq!(read(Path::new(file)).expect("read bindings.lua"), *added);
    }

    #[test]
    fn a_file_changed_between_the_offer_and_confirm_is_left_as_it_is_now() {
        let home = with_bindings("-- mine\n");
        let check = checked(home.path());
        let KeybindingCheck::Available {
            file, seen, added, ..
        } = &check
        else {
            panic!("the line isn't offered: {check:?}");
        };
        let now = "-- mine\no.bind(\"SUPER + ALT + C\", \"Calculator\", \"gnome-calculator\")\n";
        std::fs::write(file, now).expect("change bindings.lua");
        let input = add(file, seen, added);
        let Input::KeybindingConfigChanged(config) = input else {
            panic!("written over a change: {input:?}");
        };
        assert!(matches!(
            KeybindingCheck::of(&config),
            KeybindingCheck::Taken { .. }
        ));
        assert_eq!(read(Path::new(file)).expect("read bindings.lua"), now);
    }

    #[test]
    fn a_trailing_return_keeps_the_line_before_it() {
        let home = with_bindings("o.bind(\"SUPER + B\", \"Browser\", \"firefox\")\nreturn M\n");
        let (check, input) = confirm(home.path());
        assert_eq!(input, Input::KeybindingAdded);
        let KeybindingCheck::Available { line, .. } = check else {
            unreachable!()
        };
        assert_eq!(
            read(&bindings(home.path())).expect("read bindings.lua"),
            format!("o.bind(\"SUPER + B\", \"Browser\", \"firefox\")\n{line}\nreturn M\n")
        );
    }

    #[test]
    fn keys_bound_in_a_required_file_are_taken_there() {
        let home = with_bindings("require(\"hypr.more\")\n");
        std::fs::write(
            home.path().join("hypr").join("more.lua"),
            "o.bind(\"ALT + SUPER + c\", \"Calculator\", \"gnome-calculator\")\n",
        )
        .expect("write more.lua");
        assert!(
            matches!(
                checked(home.path()),
                KeybindingCheck::Taken { ref file, .. } if file.ends_with("more.lua")
            ),
            "{:?}",
            checked(home.path())
        );
    }
}
