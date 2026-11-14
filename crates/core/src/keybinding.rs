//! The Desktop Calls keybinding in Omarchy's Hyprland config: whether the
//! config already binds the keys, and the file with the line added.
//!
//! The shell reads the files and reports what they hold
//! ([`HyprlandConfig`]); everything decided from that is here. Only
//! Omarchy's own place for personal keybindings is written to:
//! `bindings.lua`, whose lines read
//! `o.bind("SUPER + SHIFT + R", "SSH", "alacritty -e ssh …")`. Its lines are
//! read in order, with the files it requires read where it requires them.
//! An `unbind` frees the keys again, and comments don't count. A binding
//! inside an `if` counts as taking the keys, whether or not it applies.

/// Free in Omarchy's own bindings (4.5): C for Calls.
pub const KEYS: &str = "SUPER + ALT + C";
/// What Omarchy's keybinding menu lists the binding as.
const DESCRIPTION: &str = "Desktop Calls";
/// The binary a binding of Desktop Calls runs, wherever it is installed.
const PROGRAM: &str = "desktop-calls";

/// What the shell found of the Hyprland config: only what the files hold.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HyprlandConfig {
    /// The command a keybinding runs: this binary, by its full path, quoted
    /// for the shell Hyprland runs it with.
    pub command: String,
    /// Omarchy's file for personal keybindings, if there is one.
    pub bindings: Option<ConfigFile>,
}

/// `bindings.lua`, and the files it requires.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConfigFile {
    pub path: String,
    /// What it holds; `None` if it couldn't be read.
    pub text: Option<String>,
    /// The files it requires that could be read, by the name it requires
    /// them by (see [`required`]).
    pub required: Vec<RequiredFile>,
}

/// A file `bindings.lua` requires, with what it holds.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RequiredFile {
    /// The name as `bindings.lua` writes it: `hypr.more-bindings`, or a path.
    pub name: String,
    pub path: String,
    pub text: String,
}

/// What the Hyprland config can take for the Desktop Calls keybinding,
/// which runs `desktop-calls`: the Dialer, or the ringing or live Call.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum KeybindingCheck {
    /// `line` binds `keys` once `file`, which holds `seen`, holds `added`
    /// instead: `seen` with `line` at the end, or before a final `return`.
    Available {
        keys: String,
        file: String,
        line: String,
        before_return: bool,
        seen: String,
        added: String,
    },
    /// `file` already binds `keys` to Desktop Calls.
    Added { keys: String, file: String },
    /// `keys` already does something else in `file`; the user can bind
    /// another key to `command`.
    Taken {
        keys: String,
        file: String,
        command: String,
    },
    /// `file` is there but couldn't be read; the user can bind a key to
    /// `command`.
    Unreadable { file: String, command: String },
    /// No Hyprland config Desktop Calls knows how to add to; the user can
    /// bind a key to `command`.
    NoConfig { command: String },
}

impl KeybindingCheck {
    /// What `config` can take.
    #[must_use]
    pub fn of(config: &HyprlandConfig) -> Self {
        let command = config.command.clone();
        let Some(bindings) = &config.bindings else {
            return Self::NoConfig { command };
        };
        let Some(text) = &bindings.text else {
            return Self::Unreadable {
                file: bindings.path.clone(),
                command,
            };
        };
        let keys = KEYS.to_owned();
        match binding_of(&keyset(KEYS), bindings, text) {
            Some(bound) if bound.ours => Self::Added {
                keys,
                file: bound.file,
            },
            Some(bound) => Self::Taken {
                keys,
                file: bound.file,
                command,
            },
            None => {
                let line = line(&command);
                let (added, before_return) = with_line(text, &line);
                Self::Available {
                    keys,
                    file: bindings.path.clone(),
                    line,
                    before_return,
                    seen: text.clone(),
                    added,
                }
            }
        }
    }
}

/// The keys of the binding that runs Desktop Calls once every line has run,
/// as written, one key each: `SUPER + ALT + C` is `["SUPER", "ALT", "C"]`.
/// `None` if nothing binds Desktop Calls, or what did was unbound or its
/// keys taken over by a later line. The Banner names them.
#[must_use]
pub fn bound_keys(config: &HyprlandConfig) -> Option<Vec<String>> {
    let bindings = config.bindings.as_ref()?;
    let text = bindings.text.as_ref()?;
    let mut ours: Option<String> = None;
    let mut read = |code: &str| {
        let (unbinding, arguments) = if let Some(arguments) = call(code, ".unbind(") {
            (true, arguments)
        } else if let Some(arguments) = call(code, ".bind(") {
            (false, arguments)
        } else {
            return;
        };
        let Some(keys) = arguments.first().and_then(Option::as_deref) else {
            return;
        };
        let same = ours
            .as_deref()
            .is_some_and(|bound| keyset(bound) == keyset(keys));
        if unbinding {
            if same {
                ours = None;
            }
            return;
        }
        let description = arguments.get(1).and_then(Option::as_deref);
        let runs = arguments.get(2).and_then(Option::as_deref);
        if description == Some(DESCRIPTION) || runs.is_some_and(runs_desktop_calls) {
            ours = Some(keys.to_owned());
        } else if same {
            ours = None;
        }
    };
    for line in text.lines() {
        let in_line = code(line);
        if let Some(name) = required_by(in_line)
            && let Some(file) = bindings.required.iter().find(|file| file.name == name)
        {
            for required_line in file.text.lines() {
                read(code(required_line));
            }
        }
        read(in_line);
    }
    ours.map(|keys| {
        keys.split(|character: char| character == '+' || character.is_whitespace())
            .filter(|key| !key.is_empty())
            .map(str::to_owned)
            .collect()
    })
}

/// The names `text` requires other files by, in order: `require("x.y")`,
/// `require "x.y"`, `pcall(require, "x.y")` and `dofile("/a/b.lua")`, outside
/// comments. The shell finds and reads them.
#[must_use]
pub fn required(text: &str) -> Vec<String> {
    text.lines()
        .filter_map(|line| required_by(code(line)))
        .collect()
}

fn required_by(code: &str) -> Option<String> {
    let at = ["require", "dofile"]
        .iter()
        .filter_map(|word| code.find(word).map(|at| at + word.len()))
        .min()?;
    strings(&code[at..]).into_iter().next()
}

/// Who binds the keys once every line has run, if anyone.
struct Bound {
    file: String,
    ours: bool,
}

fn binding_of(keys: &[String], bindings: &ConfigFile, text: &str) -> Option<Bound> {
    let mut bound = None;
    for line in text.lines() {
        let in_line = code(line);
        if let Some(name) = required_by(in_line)
            && let Some(file) = bindings.required.iter().find(|file| file.name == name)
        {
            for required_line in file.text.lines() {
                read_line(code(required_line), keys, &file.path, &mut bound);
            }
        }
        read_line(in_line, keys, &bindings.path, &mut bound);
    }
    bound
}

/// What one line does to the keys: binds them, or unbinds them.
fn read_line(code: &str, keys: &[String], file: &str, bound: &mut Option<Bound>) {
    if let Some(arguments) = call(code, ".unbind(") {
        if arguments
            .first()
            .and_then(Option::as_deref)
            .map(keyset)
            .as_deref()
            == Some(keys)
        {
            *bound = None;
        }
        return;
    }
    let Some(arguments) = call(code, ".bind(") else {
        return;
    };
    if arguments
        .first()
        .and_then(Option::as_deref)
        .map(keyset)
        .as_deref()
        != Some(keys)
    {
        return;
    }
    let description = arguments.get(1).and_then(Option::as_deref);
    let runs = arguments.get(2).and_then(Option::as_deref);
    *bound = Some(Bound {
        file: file.to_owned(),
        ours: description == Some(DESCRIPTION) || runs.is_some_and(runs_desktop_calls),
    });
}

/// Whether a binding's command runs Desktop Calls, from wherever it was
/// installed then.
fn runs_desktop_calls(command: &str) -> bool {
    let program = command
        .trim()
        .trim_start_matches(['\'', '"'])
        .split(['\'', '"', ' '])
        .next()
        .unwrap_or_default();
    program.rsplit('/').next() == Some(PROGRAM)
}

/// The arguments of the call to `function` on the line, if any: each one
/// a string literal, or `None` for anything else.
fn call(code: &str, function: &str) -> Option<Vec<Option<String>>> {
    let at = code.find(function)? + function.len();
    let mut arguments = Vec::new();
    let mut argument = Argument::Empty;
    let mut depth = 0_u32;
    let mut characters = code[at..].chars();
    while let Some(character) = characters.next() {
        match character {
            ',' if depth == 0 => arguments.push(std::mem::take(&mut argument).string()),
            ')' if depth == 0 => break,
            '"' | '\'' => {
                let body = string_body(&mut characters, character);
                argument = match argument {
                    Argument::Empty if depth == 0 => Argument::String(body),
                    _ => Argument::Other,
                };
            }
            '(' | '{' | '[' => {
                depth += 1;
                argument = Argument::Other;
            }
            ')' | '}' | ']' => depth = depth.saturating_sub(1),
            character if character.is_whitespace() => {}
            _ => argument = Argument::Other,
        }
    }
    arguments.push(argument.string());
    Some(arguments)
}

/// One argument of a call, as read so far.
#[derive(Default)]
enum Argument {
    #[default]
    Empty,
    String(String),
    Other,
}

impl Argument {
    fn string(self) -> Option<String> {
        match self {
            Self::String(string) => Some(string),
            Self::Empty | Self::Other => None,
        }
    }
}

/// The rest of a string literal opened by `quote`, unescaped.
fn string_body(characters: &mut std::str::Chars<'_>, quote: char) -> String {
    let mut body = String::new();
    while let Some(character) = characters.next() {
        match character {
            '\\' => body.extend(characters.next()),
            character if character == quote => break,
            character => body.push(character),
        }
    }
    body
}

/// Every string literal in `code`, in order.
fn strings(code: &str) -> Vec<String> {
    let mut found = Vec::new();
    let mut characters = code.chars();
    while let Some(character) = characters.next() {
        if character == '"' || character == '\'' {
            found.push(string_body(&mut characters, character));
        }
    }
    found
}

/// The line without its comment: from `--` outside a string.
fn code(line: &str) -> &str {
    let mut quote = None;
    let mut escaped = false;
    let mut previous_dash = false;
    for (at, character) in line.char_indices() {
        match (quote, character) {
            (Some(_), '\\') if !escaped => {
                escaped = true;
                continue;
            }
            (Some(open), character) if character == open && !escaped => quote = None,
            (None, '"' | '\'') => quote = Some(character),
            (None, '-') if previous_dash => return &line[..at - 1],
            _ => {}
        }
        escaped = false;
        previous_dash = quote.is_none() && character == '-';
    }
    line
}

/// Keys as a set: any order, any case, and Hyprland's other names for the
/// modifiers, so `ALT + SUPER + c` is `SUPER + ALT + C`.
fn keyset(keys: &str) -> Vec<String> {
    let mut set: Vec<String> = keys
        .split(|character: char| character == '+' || character.is_whitespace())
        .filter(|key| !key.is_empty())
        .map(|key| {
            let key = key.to_ascii_uppercase();
            match key.as_str() {
                "WIN" | "MOD4" | "LOGO" | "META" => "SUPER".to_owned(),
                "MOD1" => "ALT".to_owned(),
                "CONTROL" => "CTRL".to_owned(),
                _ => key,
            }
        })
        .collect();
    set.sort();
    set.dedup();
    set
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

fn lua_string(text: &str) -> String {
    format!("\"{}\"", text.replace('\\', r"\\").replace('"', "\\\""))
}

/// `text` with `line` added: at the end, or before a final top-level
/// `return`, after which Lua runs nothing. Whether it went before one.
fn with_line(text: &str, line: &str) -> (String, bool) {
    let lines: Vec<&str> = text.split_inclusive('\n').collect();
    let last_code = lines
        .iter()
        .rposition(|candidate| !code(candidate).trim().is_empty());
    if let Some(at) = last_code
        && let Some(last) = lines.get(at)
        && is_return(last)
    {
        let mut added: String = lines[..at].concat();
        added.push_str(line);
        added.push('\n');
        added.push_str(&lines[at..].concat());
        return (added, true);
    }
    let mut added = text.to_owned();
    if !added.is_empty() && !added.ends_with('\n') {
        added.push('\n');
    }
    added.push_str(line);
    added.push('\n');
    (added, false)
}

/// Whether the line is a `return` at the top level, not inside a function.
fn is_return(line: &str) -> bool {
    let rest = line.strip_prefix("return").unwrap_or("x");
    !line.starts_with(char::is_whitespace)
        && rest
            .chars()
            .next()
            .is_none_or(|next| !next.is_alphanumeric() && next != '_')
}
