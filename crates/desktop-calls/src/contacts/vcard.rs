//! Reads the vCards a Phone sends over PBAP: version 3.0, which the app asks
//! for, and 2.1, which some Phones send anyway.
//!
//! - Folded lines (a line break followed by a space or tab) are unfolded.
//! - 2.1 values in `QUOTED-PRINTABLE`, with their soft line breaks (`=` at
//!   the end of a line), are decoded, then read in their `CHARSET` (UTF-8
//!   unless it says ISO-8859-1).
//! - 3.0 escapes (`\,` `\;` `\\` `\n`) are undone.
//! - A Call History entry's `X-IRMC-CALL-DATETIME` gives which way the Call
//!   went and when, as the Phone's local time unless it says otherwise.

use desktop_calls_core::{CallHistoryKind, NumberLabel};

/// One vCard: a Contact, or a Call History entry.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Card {
    /// `FN`, or else the name made from `N`; may be empty.
    pub name: String,
    /// `TEL`s in order, with what kind each is.
    pub numbers: Vec<(String, NumberLabel)>,
    /// For a Call History entry: `X-IRMC-CALL-DATETIME`.
    pub call: Option<CallStamp>,
}

/// Which way a Call went, and when, as `X-IRMC-CALL-DATETIME` says.
#[derive(Debug, PartialEq, Eq)]
pub struct CallStamp {
    pub kind: Option<CallHistoryKind>,
    pub at: Option<DateTime>,
}

/// A moment as the Phone wrote it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DateTime {
    pub year: i32,
    pub month: i32,
    pub day: i32,
    pub hour: i32,
    pub minute: i32,
    pub second: i32,
    /// The offset from UTC in seconds, if the Phone gave one (`Z` is 0);
    /// otherwise the time is the Phone's local time.
    pub utc_offset: Option<i32>,
}

/// Reads every vCard in `text`. Lines outside a vCard, and properties this
/// app doesn't use, are skipped.
pub fn parse(text: &[u8]) -> Vec<Card> {
    let mut cards = Vec::new();
    let mut card: Option<Card> = None;
    let mut formatted_name = None;
    let mut structured_name = None;
    let mut depth = 0_u32;
    for line in logical_lines(text) {
        let Some(property) = Property::parse(&line) else {
            continue;
        };
        match property.name.as_str() {
            "BEGIN" if property.text().eq_ignore_ascii_case("VCARD") => {
                depth += 1;
                if depth == 1 {
                    card = Some(Card::default());
                    formatted_name = None;
                    structured_name = None;
                }
            }
            "END" if property.text().eq_ignore_ascii_case("VCARD") => {
                depth = depth.saturating_sub(1);
                if depth == 0
                    && let Some(mut done) = card.take()
                {
                    done.name = formatted_name
                        .take()
                        .filter(|name: &String| !name.trim().is_empty())
                        .or_else(|| structured_name.take())
                        .unwrap_or_default();
                    cards.push(done);
                }
            }
            // A vCard nested in another (`AGENT`) isn't one of the Phone's.
            _ if depth != 1 => {}
            "FN" => formatted_name = Some(property.text()),
            "N" => structured_name = Some(name_from_parts(&property.components())),
            "TEL" => {
                if let Some(card) = card.as_mut() {
                    let number = property.text().trim().to_owned();
                    if !number.is_empty() {
                        card.numbers.push((number, property.number_label()));
                    }
                }
            }
            "X-IRMC-CALL-DATETIME" => {
                if let Some(card) = card.as_mut() {
                    card.call = Some(CallStamp {
                        kind: property.call_kind(),
                        at: parse_date_time(property.text().trim()),
                    });
                }
            }
            _ => {}
        }
    }
    cards
}

/// `given additional family`, from `N`'s `family;given;additional;prefix;
/// suffix`, with the prefix and suffix around them.
fn name_from_parts(parts: &[String]) -> String {
    let part = |index: usize| parts.get(index).map_or("", |part| part.trim());
    [part(3), part(1), part(2), part(0), part(4)]
        .into_iter()
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

/// The lines of `text` with folded lines joined, and quoted-printable soft
/// line breaks joined too.
fn logical_lines(text: &[u8]) -> Vec<Vec<u8>> {
    let mut lines: Vec<Vec<u8>> = Vec::new();
    // Whether the current line is quoted-printable and ends in a soft break.
    let mut soft_break = false;
    for physical in text.split(|&byte| byte == b'\n') {
        let physical = physical.strip_suffix(b"\r").unwrap_or(physical);
        if let Some(current) = lines.last_mut() {
            if soft_break {
                current.pop();
                current.extend_from_slice(physical);
                soft_break = current.ends_with(b"=");
                continue;
            }
            if let Some(rest) = physical
                .strip_prefix(b" ")
                .or_else(|| physical.strip_prefix(b"\t"))
            {
                current.extend_from_slice(rest);
                soft_break = is_quoted_printable(current) && current.ends_with(b"=");
                continue;
            }
        }
        if physical.is_empty() {
            continue;
        }
        lines.push(physical.to_vec());
        soft_break = is_quoted_printable(physical) && physical.ends_with(b"=");
    }
    lines
}

/// Whether the property on `line` says its value is quoted-printable.
fn is_quoted_printable(line: &[u8]) -> bool {
    let header = line
        .iter()
        .position(|&byte| byte == b':')
        .map_or(line, |colon| &line[..colon]);
    header
        .to_ascii_uppercase()
        .windows(b"QUOTED-PRINTABLE".len())
        .any(|window| window == b"QUOTED-PRINTABLE")
}

/// One property: `NAME;PARAM=VALUE;PARAM:value`.
struct Property {
    /// Upper case, without any group.
    name: String,
    /// Each parameter's name (upper case) and value; a 2.1 bare parameter,
    /// such as `CELL`, has no name.
    parameters: Vec<(Option<String>, String)>,
    value: Vec<u8>,
}

impl Property {
    fn parse(line: &[u8]) -> Option<Self> {
        let colon = line.iter().position(|&byte| byte == b':')?;
        let header = String::from_utf8_lossy(&line[..colon]);
        let mut parts = header.split(';');
        let name = parts.next()?;
        let name = name
            .rsplit('.')
            .next()
            .unwrap_or(name)
            .trim()
            .to_ascii_uppercase();
        let parameters = parts
            .map(|parameter| match parameter.split_once('=') {
                Some((name, value)) => (
                    Some(name.trim().to_ascii_uppercase()),
                    value.trim().trim_matches('"').to_owned(),
                ),
                None => (None, parameter.trim().to_owned()),
            })
            .collect();
        Some(Self {
            name,
            parameters,
            value: line[colon + 1..].to_vec(),
        })
    }

    fn parameter(&self, name: &str) -> Option<&str> {
        self.parameters
            .iter()
            .find(|(key, _)| key.as_deref() == Some(name))
            .map(|(_, value)| value.as_str())
    }

    /// Every type the property has: `TYPE=` values, comma-separated or
    /// repeated, and 2.1 bare parameters.
    fn types(&self) -> Vec<String> {
        self.parameters
            .iter()
            .filter(|(key, _)| key.is_none() || key.as_deref() == Some("TYPE"))
            .flat_map(|(_, value)| value.split(','))
            .map(|kind| kind.trim().to_owned())
            .filter(|kind| !kind.is_empty())
            .collect()
    }

    /// The value as text: quoted-printable decoded, read in its charset.
    fn raw_text(&self) -> String {
        let bytes = if self
            .parameter("ENCODING")
            .is_some_and(|encoding| encoding.eq_ignore_ascii_case("QUOTED-PRINTABLE"))
            || self
                .types()
                .iter()
                .any(|kind| kind.eq_ignore_ascii_case("QUOTED-PRINTABLE"))
        {
            decode_quoted_printable(&self.value)
        } else {
            self.value.clone()
        };
        match self
            .parameter("CHARSET")
            .map(str::to_ascii_uppercase)
            .as_deref()
        {
            Some("ISO-8859-1" | "LATIN1" | "ISO8859-1") => {
                bytes.iter().map(|&byte| char::from(byte)).collect()
            }
            _ => String::from_utf8_lossy(&bytes).into_owned(),
        }
    }

    /// The value as one piece of text, escapes undone.
    fn text(&self) -> String {
        unescape(&self.raw_text())
    }

    /// The value's `;`-separated parts, each with escapes undone.
    fn components(&self) -> Vec<String> {
        let raw = self.raw_text();
        let mut parts = Vec::new();
        let mut current = String::new();
        let mut chars = raw.chars();
        while let Some(character) = chars.next() {
            match character {
                '\\' => {
                    current.push('\\');
                    if let Some(next) = chars.next() {
                        current.push(next);
                    }
                }
                ';' => parts.push(unescape(&std::mem::take(&mut current))),
                other => current.push(other),
            }
        }
        parts.push(unescape(&current));
        parts
    }

    fn number_label(&self) -> NumberLabel {
        let types: Vec<String> = self.types();
        let has = |name: &str| types.iter().any(|kind| kind.eq_ignore_ascii_case(name));
        if let Some(custom) = types
            .iter()
            .find_map(|kind| kind.strip_prefix("X-").or_else(|| kind.strip_prefix("x-")))
            .filter(|custom| !custom.is_empty())
        {
            NumberLabel::Custom(custom.to_owned())
        } else if has("CELL") {
            NumberLabel::Mobile
        } else if has("WORK") {
            NumberLabel::Work
        } else if has("HOME") {
            NumberLabel::Home
        } else if has("MAIN") {
            NumberLabel::Main
        } else {
            NumberLabel::Other
        }
    }

    /// `RECEIVED`, `DIALED` or `MISSED`, as a bare 2.1 parameter or a 3.0
    /// `TYPE`.
    fn call_kind(&self) -> Option<CallHistoryKind> {
        self.types()
            .iter()
            .find_map(|kind| match kind.to_ascii_uppercase().as_str() {
                "RECEIVED" => Some(CallHistoryKind::Incoming),
                "DIALED" => Some(CallHistoryKind::Outgoing),
                "MISSED" => Some(CallHistoryKind::Missed),
                _ => None,
            })
    }
}

fn decode_quoted_printable(value: &[u8]) -> Vec<u8> {
    let hex = |byte: u8| char::from(byte).to_digit(16);
    let mut decoded = Vec::with_capacity(value.len());
    let mut index = 0;
    while index < value.len() {
        if value[index] == b'='
            && let (Some(&high), Some(&low)) = (value.get(index + 1), value.get(index + 2))
            && let (Some(high), Some(low)) = (hex(high), hex(low))
            && let Ok(byte) = u8::try_from(high * 16 + low)
        {
            decoded.push(byte);
            index += 3;
        } else {
            decoded.push(value[index]);
            index += 1;
        }
    }
    decoded
}

/// Undoes vCard 3.0 escapes.
fn unescape(text: &str) -> String {
    let mut unescaped = String::with_capacity(text.len());
    let mut chars = text.chars();
    while let Some(character) = chars.next() {
        if character != '\\' {
            unescaped.push(character);
            continue;
        }
        match chars.next() {
            Some('n' | 'N') => unescaped.push('\n'),
            Some(other) => unescaped.push(other),
            None => unescaped.push('\\'),
        }
    }
    unescaped
}

/// `YYYYMMDDTHHMMSS`, then optionally `Z` or `±HHMM` (or `±HH:MM`). Dashes
/// and colons in the date and time are tolerated.
fn parse_date_time(text: &str) -> Option<DateTime> {
    let (date, time) = text.split_once(['T', 't'])?;
    let date: String = date.chars().filter(char::is_ascii_digit).collect();
    let zone_start = time.find(['Z', 'z', '+', '-']).unwrap_or(time.len());
    let (clock, zone) = time.split_at(zone_start);
    let clock: String = clock.chars().filter(char::is_ascii_digit).collect();
    if date.len() != 8 || clock.len() < 4 {
        return None;
    }
    let number = |text: &str| text.parse::<i32>().ok();
    let utc_offset = match zone.chars().next() {
        None => None,
        Some('Z' | 'z') => Some(0),
        Some(sign) => {
            let digits: String = zone.chars().filter(char::is_ascii_digit).collect();
            let hours = number(digits.get(0..2)?)?;
            let minutes = digits.get(2..4).and_then(number).unwrap_or(0);
            let offset = hours * 3_600 + minutes * 60;
            Some(if sign == '-' { -offset } else { offset })
        }
    };
    Some(DateTime {
        year: number(&date[0..4])?,
        month: number(&date[4..6])?,
        day: number(&date[6..8])?,
        hour: number(&clock[0..2])?,
        minute: number(&clock[2..4])?,
        second: clock.get(4..6).and_then(number).unwrap_or(0),
        utc_offset,
    })
}
