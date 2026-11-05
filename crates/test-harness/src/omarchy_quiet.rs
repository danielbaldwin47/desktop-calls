//! Omarchy 4.5's rules for whether a notification gets through Do Not
//! Disturb, ported from its notification plugin
//! (`/usr/share/omarchy/shell/plugins/notifications/`, omarchy 4.5.0-2) so
//! the notification server stand-in decides as Omarchy does.
//!
//! Under Do Not Disturb a notification shows only if it comes from Omarchy's
//! own senders (`Delivery.js` `bypassesQuiet`) or if `Detect.js` finds a
//! verification code in its summary and body (`Delivery.js` `quietReason`).
//! Detection is a port of `Detect.js` `codes`: JavaScript regular
//! expressions without the `u` flag, so it works on UTF-16 code units, and
//! `\w`, `\b`, `\d` and case folding are ASCII only. Omarchy lifts a
//! browser's origin anchor off the body before detection; this port treats
//! every sender as not a browser.
//!
//! It also keeps Omarchy 4.5's record of whether Do Not Disturb is on, the
//! way Omarchy's shell keeps it (`Service.qml`'s store, `Store.js`
//! `serialiseQuiet`): `~/.local/state/omarchy/notifications/quiet.json`, in
//! a private directory. The shell creates the file empty at startup, which
//! means off, and on each toggle replaces the whole file by an atomic
//! rename with `{"dnd":true}` or `{"dnd":false}`. Omarchy 4.0's
//! `~/.local/state/omarchy/notifications.json` is gone: 4.5's migration
//! (`/usr/share/omarchy/migrations/1792648436.sh`) deletes it.

use std::fs::{self, DirBuilder, OpenOptions, Permissions};
use std::io::{self, Write as _};
use std::os::unix::fs::{DirBuilderExt as _, OpenOptionsExt as _, PermissionsExt as _};
use std::path::{Path, PathBuf};

/// Omarchy 4.5's notification state directory, under the home directory.
const STATE_DIRECTORY: &str = ".local/state/omarchy/notifications";

/// `Security.js` `MAX_SUMMARY`, `MAX_BODY` and `MAX_CODES`.
const MAX_SUMMARY: usize = 2_048;
const MAX_BODY: usize = 32_768;
const MAX_CODES: usize = 8;
/// `Detect.js` `NEAR_BEFORE` and `NEAR_AFTER`: how far from its keyword a
/// code may sit.
const NEAR_BEFORE: usize = 24;
const NEAR_AFTER: usize = 72;
/// `Delivery.js` `CRITICAL`.
const CRITICAL: u8 = 2;

/// Whether Omarchy keeps a notification off the screen (`quietReason` is
/// `"silenced"`).
pub(crate) fn silenced(
    do_not_disturb: bool,
    app_name: &str,
    urgency: u8,
    codes: &[String],
) -> bool {
    let bypasses =
        app_name == "omarchy-action" || (app_name == "notify-send" && urgency == CRITICAL);
    do_not_disturb && !bypasses && codes.is_empty()
}

/// Prepares Omarchy's notification state under `home` as its shell does at
/// startup: the directory, private, and an empty `quiet.json` (off).
pub(crate) fn prepare_state(home: &Path) -> io::Result<()> {
    let directory = state_directory(home)?;
    write_private(&directory.join("quiet.json"), b"")
}

/// Records Do Not Disturb `on` under `home` as Omarchy's shell does on each
/// toggle: the whole of `quiet.json`, replaced atomically, keeping its mode.
pub(crate) fn write_do_not_disturb(home: &Path, on: bool) -> io::Result<()> {
    // `JSON.stringify({ dnd: !!state.dnd }) + "\n"`.
    replace_quiet_file(home, &format!("{{\"dnd\":{on}}}\n"))
}

/// Replaces `quiet.json` under `home` with `contents`, atomically, as the
/// shell replaces it.
pub(crate) fn replace_quiet_file(home: &Path, contents: &str) -> io::Result<()> {
    let directory = state_directory(home)?;
    let temporary = directory.join("quiet.json.new");
    write_private(&temporary, contents.as_bytes())?;
    fs::rename(&temporary, directory.join("quiet.json"))
}

/// The state directory under `home`, created private if it isn't there.
fn state_directory(home: &Path) -> io::Result<PathBuf> {
    let directory = home.join(STATE_DIRECTORY);
    DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(&directory)?;
    // `mkdir -p` leaves an existing directory's mode; the shell then sets it.
    fs::set_permissions(&directory, Permissions::from_mode(0o700))?;
    Ok(directory)
}

/// Writes `contents` to `path` as a new file only its owner can read.
fn write_private(path: &Path, contents: &[u8]) -> io::Result<()> {
    let mut file = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(path)?;
    file.write_all(contents)
}

/// The verification codes Omarchy finds in a notification and offers to
/// copy (`Detect.js` `scan(summary, body).codes`).
pub(crate) fn verification_codes(summary: &str, body: &str) -> Vec<String> {
    let mut text: Vec<u16> = summary.encode_utf16().take(MAX_SUMMARY).collect();
    text.push(u16::from(b'\n'));
    text.extend(body.encode_utf16().take(MAX_BODY));
    text.truncate(MAX_BODY);
    codes(&text)
}

fn codes(text: &[u16]) -> Vec<String> {
    let spans = windows(text);
    let mut seen = Vec::new();
    for &(start, end) in &spans {
        harvest(
            &text[start..end.min(text.len())],
            code_shape,
            &mut seen,
            |raw| {
                let raw = match raw {
                    [letter, b'-', rest @ ..] if letter.is_ascii_alphabetic() => rest,
                    _ => raw,
                };
                let value: Vec<u8> = raw.iter().copied().filter(u8::is_ascii_digit).collect();
                let year =
                    value.len() == 4 && (value.starts_with(b"19") || value.starts_with(b"20"));
                ((4..=8).contains(&value.len()) && !year).then_some(value)
            },
        );
    }
    if seen.is_empty() {
        for &(start, end) in &spans {
            harvest(
                &text[start..end.min(text.len())],
                code_alnum,
                &mut seen,
                |raw| {
                    let letter = raw.iter().any(u8::is_ascii_uppercase);
                    let digit = raw.iter().any(u8::is_ascii_digit);
                    (letter && digit).then(|| raw.to_vec())
                },
            );
        }
    }
    seen.truncate(MAX_CODES);
    seen.into_iter()
        .map(|code| String::from_utf8_lossy(&code).into_owned())
        .collect()
}

/// A regular expression's capturing group: whether it matches at an index,
/// and where the match ends.
type Group = fn(&[u16], usize) -> Option<usize>;

/// Each match of `shape` in `text`, as `keep` makes it, once.
fn harvest(
    text: &[u16],
    shape: Group,
    seen: &mut Vec<Vec<u8>>,
    keep: impl Fn(&[u8]) -> Option<Vec<u8>>,
) {
    let mut from = 0;
    while from < text.len() {
        let Some((start, end)) = find(text, from, shape) else {
            return;
        };
        // A match is ASCII, so its code units are its bytes.
        let raw: Vec<u8> = text[start..end].iter().map(|&unit| ascii(unit)).collect();
        if let Some(value) = keep(&raw)
            && !seen.contains(&value)
        {
            seen.push(value);
        }
        from = end;
    }
}

/// The first match of `(?:^|[^\w-])(group)(?![\w-])` at or after `from`, as
/// the span of `group`.
fn find(text: &[u16], from: usize, group: Group) -> Option<(usize, usize)> {
    (from..text.len()).find_map(|at| {
        let anchored = (at == 0)
            .then(|| group(text, 0).map(|end| (0, end)))
            .flatten();
        anchored.or_else(|| {
            (!is_word(text[at]) && text[at] != u16::from(b'-'))
                .then(|| group(text, at + 1).map(|end| (at + 1, end)))
                .flatten()
        })
    })
}

/// `CODE_SHAPE`'s group at `at`, then `(?![\w-])`: `(?:[A-Za-z]-)?\d{2,4}[\s-]\d{2,5}`
/// or `(?:[A-Za-z]-)?\d{4,8}`, each greedy and backtracking.
fn code_shape(text: &[u16], at: usize) -> Option<usize> {
    let prefixed = |start: usize| -> Vec<usize> {
        let letter_dash = text
            .get(start)
            .is_some_and(|&unit| ascii(unit).is_ascii_alphabetic())
            && text.get(start + 1) == Some(&u16::from(b'-'));
        if letter_dash {
            vec![start + 2, start]
        } else {
            vec![start]
        }
    };
    let digits = |start: usize, lengths: std::ops::RangeInclusive<usize>| -> Vec<usize> {
        let run = text[start.min(text.len())..]
            .iter()
            .take_while(|&&unit| ascii(unit).is_ascii_digit())
            .count();
        lengths
            .rev()
            .filter(|&length| length <= run)
            .map(|length| start + length)
            .collect()
    };
    let paired = prefixed(at).into_iter().find_map(|start| {
        digits(start, 2..=4).into_iter().find_map(|middle| {
            let separator = text
                .get(middle)
                .is_some_and(|&unit| is_space(unit) || unit == u16::from(b'-'));
            if !separator {
                return None;
            }
            digits(middle + 1, 2..=5)
                .into_iter()
                .find(|&end| ends(text, end))
        })
    });
    paired.or_else(|| {
        prefixed(at).into_iter().find_map(|start| {
            digits(start, 4..=8)
                .into_iter()
                .find(|&end| ends(text, end))
        })
    })
}

/// `CODE_ALNUM`'s group at `at`, then `(?![\w-])`: `[A-Z0-9]{5,8}`, greedy.
fn code_alnum(text: &[u16], at: usize) -> Option<usize> {
    let run = text[at.min(text.len())..]
        .iter()
        .take_while(|&&unit| {
            let byte = ascii(unit);
            byte.is_ascii_uppercase() || byte.is_ascii_digit()
        })
        .count();
    (5..=8)
        .rev()
        .filter(|&length| length <= run)
        .map(|length| at + length)
        .find(|&end| ends(text, end))
}

/// `(?![\w-])` at `at`.
fn ends(text: &[u16], at: usize) -> bool {
    text.get(at)
        .is_none_or(|&unit| !is_word(unit) && unit != u16::from(b'-'))
}

/// `Detect.js` `windows`: around each keyword that isn't part of a product
/// name, the span a code may sit in.
fn windows(text: &[u16]) -> Vec<(usize, usize)> {
    let mut spans = Vec::new();
    let mut from = 0;
    while let Some((at, end)) = code_word(text, from) {
        if !is_product_name(text, at, end) {
            spans.push((at.saturating_sub(NEAR_BEFORE), end + NEAR_AFTER));
        }
        from = end;
    }
    spans.truncate(MAX_CODES);
    spans
}

/// `CODE_WORDS`, `/\b(code|otp|one[\s-]?time|verification|verify|passcode|pin|2fa|two[\s-]factor|security key|confirm(?:ation)?)\b/i`:
/// the first keyword at or after `from`, as its span.
fn code_word(text: &[u16], from: usize) -> Option<(usize, usize)> {
    (from..text.len()).find_map(|at| {
        if at > 0 && is_word(text[at - 1]) == is_word(text[at]) {
            return None;
        }
        keyword_end(text, at).map(|end| (at, end))
    })
}

/// The end of the keyword alternative that matches at `at` and is followed
/// by a word boundary, trying the alternatives in order.
fn keyword_end(text: &[u16], at: usize) -> Option<usize> {
    let literal = |start: usize, word: &str| -> Option<usize> {
        let end = start + word.len();
        let units = text.get(start..end)?;
        units
            .iter()
            .zip(word.bytes())
            .all(|(&unit, byte)| ascii(unit).eq_ignore_ascii_case(&byte))
            .then_some(end)
    };
    let separated = |first: &str, second: &str, optional: bool| -> Vec<usize> {
        let Some(middle) = literal(at, first) else {
            return Vec::new();
        };
        let separator = text
            .get(middle)
            .is_some_and(|&unit| is_space(unit) || unit == u16::from(b'-'));
        let mut ends = Vec::new();
        if separator {
            ends.extend(literal(middle + 1, second));
        }
        if optional {
            ends.extend(literal(middle, second));
        }
        ends
    };
    let boundary = |end: usize| {
        text.get(end)
            .is_none_or(|&unit| !is_word(unit) || !is_word(text[end - 1]))
    };
    let mut candidates = Vec::new();
    candidates.extend(literal(at, "code"));
    candidates.extend(literal(at, "otp"));
    candidates.extend(separated("one", "time", true));
    candidates.extend(literal(at, "verification"));
    candidates.extend(literal(at, "verify"));
    candidates.extend(literal(at, "passcode"));
    candidates.extend(literal(at, "pin"));
    candidates.extend(literal(at, "2fa"));
    candidates.extend(separated("two", "factor", false));
    candidates.extend(literal(at, "security key"));
    candidates.extend(literal(at, "confirmation"));
    candidates.extend(literal(at, "confirm"));
    candidates.into_iter().find(|&end| boundary(end))
}

/// `Detect.js` `isProductName`: a capitalised "Code" after a capitalised
/// word ("Claude Code", "VS Code") that isn't itself a keyword.
fn is_product_name(text: &[u16], at: usize, end: usize) -> bool {
    let word = &text[at..end];
    let capital_code = word.len() == 4
        && word
            .iter()
            .zip(b"code")
            .all(|(&unit, byte)| ascii(unit).eq_ignore_ascii_case(byte))
        && ascii(word[0]).is_ascii_uppercase();
    if !capital_code {
        return false;
    }
    // `before.match(/([A-Za-z][\w.]*)\s+$/)` over the 20 code units before.
    let before = &text[at.saturating_sub(20)..at];
    let spaces = before
        .iter()
        .rev()
        .take_while(|&&unit| is_space(unit))
        .count();
    if spaces == 0 {
        return false;
    }
    let token = &before[..before.len() - spaces];
    let length = token
        .iter()
        .rev()
        .take_while(|&&unit| is_word(unit) || unit == u16::from(b'.'))
        .count();
    let token = &token[token.len() - length..];
    let Some(first_letter) = token
        .iter()
        .position(|&unit| ascii(unit).is_ascii_alphabetic())
    else {
        return false;
    };
    let previous: Vec<u8> = token[first_letter..]
        .iter()
        .map(|&unit| ascii(unit))
        .collect();
    // `/^([A-Z][a-z]+|[A-Z]{2,})$/`
    let capitalised = previous.len() >= 2
        && (previous.iter().all(u8::is_ascii_uppercase)
            || (previous[0].is_ascii_uppercase()
                && previous[1..].iter().all(u8::is_ascii_lowercase)));
    let units: Vec<u16> = previous.iter().map(|&byte| u16::from(byte)).collect();
    capitalised && code_word(&units, 0).is_none()
}

/// The code unit as an ASCII byte, or NUL if it isn't ASCII.
fn ascii(unit: u16) -> u8 {
    u8::try_from(unit).ok().filter(u8::is_ascii).unwrap_or(0)
}

/// JavaScript's `\w`: `[A-Za-z0-9_]`.
fn is_word(unit: u16) -> bool {
    let byte = ascii(unit);
    byte.is_ascii_alphanumeric() || byte == b'_'
}

/// JavaScript's `\s`: ASCII and Unicode white space, and line terminators.
fn is_space(unit: u16) -> bool {
    matches!(
        unit,
        0x09..=0x0d
            | 0x20
            | 0xa0
            | 0x1680
            | 0x2000..=0x200a
            | 0x2028
            | 0x2029
            | 0x202f
            | 0x205f
            | 0x3000
            | 0xfeff
    )
}
