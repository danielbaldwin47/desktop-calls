//! Made-up Contacts and Call History for the Simulated Phone, as the vCard
//! text a Phone serves over PBAP.
//!
//! Every name is invented and every number comes from a range Ofcom keeps
//! for drama, so nothing here is a real person's. Real Contacts never go in
//! the repository.

use std::fmt::Write as _;

/// Ofcom's drama ranges, each with 1,000 numbers: the international prefix
/// and the national one, which the last three digits complete.
const DRAMA_RANGES: [(&str, &str); 8] = [
    ("+447700900", "07700 900"),
    ("+441632960", "01632 960"),
    ("+442079460", "020 7946 0"),
    ("+441134960", "0113 496 0"),
    ("+441144960", "0114 496 0"),
    ("+441154960", "0115 496 0"),
    ("+441164960", "0116 496 0"),
    ("+441174960", "0117 496 0"),
];

/// The `index`th made-up number, in international form, such as
/// `+447700900001`. Distinct for every index below 8,000.
#[must_use]
pub fn number(index: usize) -> String {
    let (international, _) = DRAMA_RANGES[index / 1_000 % DRAMA_RANGES.len()];
    format!("{international}{:03}", index % 1_000)
}

/// The `index`th made-up number as written at home in the UK, with its
/// leading 0, such as `07700 900001`.
#[must_use]
pub fn national_number(index: usize) -> String {
    let (_, national) = DRAMA_RANGES[index / 1_000 % DRAMA_RANGES.len()];
    format!("{national}{:03}", index % 1_000)
}

/// One Contact as a vCard 3.0, written as the Pixel writes it: `N` and `FN`
/// with `CHARSET=UTF-8`, and each number with its types, such as
/// `("CELL;PREF", "07700 900123")`, each type a `TYPE=` of its own.
#[must_use]
pub fn contact(name: &str, numbers: &[(&str, &str)]) -> String {
    let mut vcard = begin_contact(name);
    push_numbers(&mut vcard, numbers);
    vcard.push_str("END:VCARD\r\n");
    vcard
}

/// [`contact`] with what else the Pixel sends: an email address, a postal
/// address, an organisation, a note, a birthday, and a JPEG photo in
/// base64, folded over many lines and followed by a blank one.
#[must_use]
pub fn contact_with_photo(name: &str, numbers: &[(&str, &str)]) -> String {
    let mut vcard = begin_contact(name);
    push_numbers(&mut vcard, numbers);
    vcard.push_str(
        "EMAIL;TYPE=INTERNET;TYPE=HOME:someone@example.com\r\n\
         ADR;TYPE=HOME:;;1 Example Street;Exampleton;;EX1 1EX;United Kingdom\r\n\
         ORG:Example Ltd\r\n\
         NOTE:Met at the example conference\\, 2024\r\n\
         BDAY:1990-01-31\r\n",
    );
    vcard.push_str(&fold(&format!("PHOTO;ENCODING=b;TYPE=JPEG:{}", photo())));
    vcard.push_str("\r\nEND:VCARD\r\n");
    vcard
}

fn begin_contact(name: &str) -> String {
    let (family, given) = name
        .rsplit_once(' ')
        .map_or(("", name), |(given, family)| (family, given));
    format!(
        "BEGIN:VCARD\r\nVERSION:3.0\r\nN;CHARSET=UTF-8:{family};{given};;;\r\n\
         FN;CHARSET=UTF-8:{name}\r\n"
    )
}

fn push_numbers(vcard: &mut String, numbers: &[(&str, &str)]) {
    for (kinds, number) in numbers {
        vcard.push_str("TEL");
        for kind in kinds.split(';') {
            let _ = write!(vcard, ";TYPE={kind}");
        }
        let _ = write!(vcard, ":{number}\r\n");
    }
}

/// About 1 KB of made-up JPEG in base64.
fn photo() -> String {
    let mut photo = String::from("/9j/4AAQSkZJRgABAQAAAQABAAD/2wBDAAgGBgcGBQgHBwcJCQgKDBQNDAsL");
    while photo.len() < 1_024 {
        photo.push_str("DBkSEw8UHRofHh0aHBwgJC4nICIsIxwcKDcpLDAxNDQ0Hyc5PTgyPC4zNDL");
    }
    photo
}

/// `line` folded as vCard 3.0 folds long lines: at most 75 characters a
/// line, each continuation starting with a space.
fn fold(line: &str) -> String {
    let mut folded = String::new();
    let mut rest = line;
    let mut width = 75;
    while rest.len() > width {
        let (head, tail) = rest.split_at(width);
        folded.push_str(head);
        folded.push_str("\r\n ");
        rest = tail;
        width = 74;
    }
    folded.push_str(rest);
    folded.push_str("\r\n");
    folded
}

/// `count` made-up Contacts shaped like the Pixel's, each with one mobile
/// number: Contact `i` is "Contact `i`" with [`number`]`(i + offset)`, and
/// one in four has a photo, as on the Pixel (73 of 319). Skip the numbers a
/// test gives its own Contacts with `offset`.
#[must_use]
pub fn contacts(count: usize, offset: usize) -> String {
    (0..count)
        .map(|index| {
            let name = format!("Contact {index}");
            let numbers = [("CELL;PREF", number(index + offset))];
            let numbers: Vec<(&str, &str)> = numbers
                .iter()
                .map(|(kinds, number)| (*kinds, number.as_str()))
                .collect();
            if index % 4 == 3 {
                contact_with_photo(&name, &numbers)
            } else {
                contact(&name, &numbers)
            }
        })
        .collect()
}

/// Which way a Call in the Call History went, as the Phone's
/// `X-IRMC-CALL-DATETIME` says.
#[derive(Clone, Copy, Debug)]
pub enum CallKind {
    Received,
    Dialed,
    Missed,
}

impl CallKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::Received => "RECEIVED",
            Self::Dialed => "DIALED",
            Self::Missed => "MISSED",
        }
    }
}

/// One Call History entry as a vCard 3.0, as the Pixel writes it for a
/// number with no Contact: `FN` and `N` empty, the number (none if
/// withheld) with `TYPE=0`, and when, in the Phone's local time with no
/// time zone, such as `20261104T091500`.
#[must_use]
pub fn call(kind: CallKind, number: Option<&str>, at: &str) -> String {
    call_entry(kind, None, number, at)
}

/// [`call`] for a number the Phone has a Contact for: the Pixel names the
/// Contact in `FN` and `N`, each with `CHARSET=UTF-8`.
#[must_use]
pub fn named_call(kind: CallKind, name: &str, number: &str, at: &str) -> String {
    call_entry(kind, Some(name), Some(number), at)
}

fn call_entry(kind: CallKind, name: Option<&str>, number: Option<&str>, at: &str) -> String {
    let mut vcard = String::from("BEGIN:VCARD\r\nVERSION:3.0\r\n");
    match name {
        Some(name) => {
            let _ = write!(
                vcard,
                "FN;CHARSET=UTF-8:{name}\r\nN;CHARSET=UTF-8:{name}\r\n"
            );
        }
        None => vcard.push_str("FN:\r\nN:\r\n"),
    }
    if let Some(number) = number {
        let _ = write!(vcard, "TEL;TYPE=0:{number}\r\n");
    }
    let _ = write!(
        vcard,
        "X-IRMC-CALL-DATETIME;TYPE={}:{at}\r\nEND:VCARD\r\n",
        kind.as_str()
    );
    vcard
}

/// `count` made-up Call History entries (at most 720), newest first, one a
/// minute back from 2026-11-04 12:00, cycling through received, dialed and
/// missed. As on the Pixel, some name a Contact ("Contact `i`") and the
/// rest are unnamed.
#[must_use]
pub fn call_history(count: usize) -> String {
    (0..count.min(720))
        .map(|index| {
            let kind = [CallKind::Received, CallKind::Dialed, CallKind::Missed][index % 3];
            let minutes = 12 * 60 - index;
            let at = format!("20261104T{:02}{:02}00", minutes / 60, minutes % 60);
            if index % 4 == 1 {
                named_call(kind, &format!("Contact {index}"), &number(index), &at)
            } else {
                call(kind, Some(&number(index)), &at)
            }
        })
        .collect()
}
