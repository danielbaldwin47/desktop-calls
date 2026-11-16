//! Text with its Bluetooth addresses and phone numbers hidden, for a line
//! an agent reads: the Real-Phone Checklist's Bluetooth D-Bus captures
//! (#422, #424) and the app's debug lines of each BlueZ method call.

use std::collections::HashMap;
use std::hash::{BuildHasher, DefaultHasher, Hash, Hasher};

/// Each `AA:BB:CC:DD:EE:FF` or `AA_BB_CC_DD_EE_FF` in `line` as `dev-` and a
/// short hash. `stand_ins` keeps each address's stand-in, so one Phone reads
/// the same all through.
#[must_use]
pub fn hide_addresses<S: BuildHasher>(
    line: &str,
    stand_ins: &mut HashMap<String, String, S>,
) -> String {
    let bytes = line.as_bytes();
    let mut out = String::with_capacity(line.len());
    let mut at = 0;
    while at < bytes.len() {
        if let Some(address) = address_at(&line[at..]) {
            let key = address.to_ascii_uppercase().replace('_', ":");
            let stand_in = stand_ins.entry(key.clone()).or_insert_with(|| {
                let mut hasher = DefaultHasher::new();
                key.hash(&mut hasher);
                format!("dev-{:04x}", hasher.finish() & 0xffff)
            });
            out.push_str(stand_in);
            at += address.len();
        } else {
            let next = line[at..].chars().next().map_or(1, char::len_utf8);
            out.push_str(&line[at..at + next]);
            at += next;
        }
    }
    out
}

/// The Bluetooth address `text` starts with, if any.
fn address_at(text: &str) -> Option<&str> {
    let candidate = text.get(..17)?;
    let bytes = candidate.as_bytes();
    let separator = bytes[2];
    if separator != b':' && separator != b'_' {
        return None;
    }
    let shaped = bytes.iter().enumerate().all(|(index, byte)| {
        if index % 3 == 2 {
            *byte == separator
        } else {
            byte.is_ascii_hexdigit()
        }
    });
    shaped.then_some(candidate)
}

/// Each phone number in `line` as `<number>` (see [`number_at`]), but
/// none inside a UUID, such as a Bluetooth profile's
/// `0000111f-0000-1000-8000-00805f9b34fb`.
#[must_use]
pub fn hide_numbers(line: &str) -> String {
    let characters: Vec<char> = line.chars().collect();
    let mut out = String::with_capacity(line.len());
    let mut at = 0;
    while at < characters.len() {
        if is_uuid(&characters[at..]) {
            out.extend(&characters[at..at + UUID_SHAPE.len()]);
            at += UUID_SHAPE.len();
        } else if let Some(length) = number_at(&characters[at..]) {
            out.push_str("<number>");
            at += length;
        } else {
            out.push(characters[at]);
            at += 1;
        }
    }
    out
}

/// How long the phone number `text` starts with is, if it starts with
/// one: seven digits or more, however grouped or punctuated, as in
/// `+44 7700 900123`, `(555) 123-4567` or `555.123.4567`. A `+` or `(`
/// before the first digit, and a `)` after the last, are part of it.
/// Redacting a date or a long count too is the price of never missing a
/// number.
fn number_at(text: &[char]) -> Option<usize> {
    let mut at = 0;
    while at < text.len() && "+(".contains(text[at]) && at < 2 {
        at += 1;
    }
    if !text.get(at).is_some_and(char::is_ascii_digit) {
        return None;
    }
    let (mut digits, mut end, mut gap) = (0, at, 0);
    while let Some(&character) = text.get(at) {
        if character.is_ascii_digit() {
            digits += 1;
            end = at + 1;
            gap = 0;
        } else if NUMBER_SEPARATORS.contains(character) && gap < 4 {
            gap += 1;
        } else {
            break;
        }
        at += 1;
    }
    if text.get(end) == Some(&')') {
        end += 1;
    }
    (digits >= 7).then_some(end)
}

/// What may sit between a phone number's digits, at most four together
/// (`) - `): ASCII punctuation, and the no-break space, en dash and
/// no-break hyphen a copied number can carry.
const NUMBER_SEPARATORS: &str = " -.()/_\u{a0}\u{2013}\u{2011}";

/// A UUID's shape: `h` a hex digit.
const UUID_SHAPE: &str = "hhhhhhhh-hhhh-hhhh-hhhh-hhhhhhhhhhhh";

/// Whether `text` starts with a UUID.
fn is_uuid(text: &[char]) -> bool {
    text.len() >= UUID_SHAPE.len()
        && UUID_SHAPE
            .chars()
            .zip(text)
            .all(|(shape, character)| match shape {
                'h' => character.is_ascii_hexdigit(),
                _ => *character == shape,
            })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn addresses_become_one_stand_in_each() {
        let mut stand_ins = HashMap::new();
        let line = "/org/bluez/hci0/dev_FC_41_16_00_00_01 Address: \"fc:41:16:00:00:01\"";
        let redacted = hide_addresses(line, &mut stand_ins);
        assert!(!redacted.contains("41"), "{redacted}");
        assert_eq!(stand_ins.len(), 1, "{redacted}");
    }

    #[test]
    fn numbers_are_hidden_but_uuids_and_short_counts() {
        assert_eq!(
            hide_numbers("call +1 (555) 123-4567 at 12:30"),
            "call <number> at 12:30"
        );
        let uuid = "0000111f-0000-1000-8000-00805f9b34fb";
        assert_eq!(hide_numbers(uuid), uuid);
        assert_eq!(hide_numbers("BlueZ 5.87, page 12"), "BlueZ 5.87, page 12");
    }
}
