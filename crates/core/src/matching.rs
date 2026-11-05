//! Matching: which Contact a number belongs to, however each is written;
//! whether what the user typed is a number the Phone can dial; and the
//! Dialer's ranked rows for what the user typed.
//!
//! Numbers are compared as libphonenumber reads them, so `07700 900123` in
//! Contacts matches a Call from `+447700900123`. A number written without
//! its country code is read in the default region: the country most of the
//! Contacts' own international numbers belong to, or, if they have none, the
//! region of the Desktop's locale.
//!
//! Names are compared folded: lower case, accents taken off (`José` is
//! `jose`), apostrophes dropped and every other mark a word break.

use std::collections::{HashMap, HashSet};

use phonenumber::country::Id;
use unicode_normalization::UnicodeNormalization as _;
use unicode_normalization::char::is_combining_mark;

use crate::call::{Caller, Timestamp};
use crate::contacts::{
    CallHistory, CallHistoryEntry, CallHistoryKind, Contact, Contacts, NumberLabel,
};

/// Fewer digits than this, and a number matched on its national digits
/// alone, across countries, would match too much.
const MIN_NATIONAL_DIGITS: usize = 7;

/// The longest dial string the Phone accepts.
const MAX_DIAL_LENGTH: usize = 80;

/// The most rows the Dialer lists.
pub const MAX_ROWS: usize = 50;

/// A fuzzy match needs this many letters, or nearly every name would match.
const MIN_FUZZY_LETTERS: usize = 3;

/// Contacts, indexed by number and for search.
#[derive(Default)]
pub struct Directory {
    contacts: Vec<Contact>,
    region: Option<Id>,
    /// `(contact, number)` by the number in E.164 form.
    by_e164: HashMap<String, (usize, usize)>,
    /// `(contact, number)` by the national significant number, where only
    /// one Contact has it.
    by_national: HashMap<String, Option<(usize, usize)>>,
    /// Each Contact's name, folded for search.
    names: Vec<Name>,
    /// Each Contact's numbers, as search compares and shows them.
    numbers: Vec<Vec<IndexedNumber>>,
}

/// The Contact a number belongs to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ContactMatch {
    pub name: String,
    pub label: NumberLabel,
}

impl Directory {
    /// Indexes `contacts`. `locale_region` is the Desktop locale's region,
    /// such as `GB` from `en_GB.UTF-8`, used when the Contacts don't say.
    /// Slow for thousands of Contacts: build it off the main thread.
    #[must_use]
    pub fn new(contacts: Vec<Contact>, locale_region: Option<&str>) -> Self {
        let region = infer_region(&contacts)
            .or_else(|| locale_region.and_then(|region| region.to_ascii_uppercase().parse().ok()));
        let mut by_e164 = HashMap::new();
        let mut by_national: HashMap<String, Option<(usize, usize)>> = HashMap::new();
        let mut numbers = Vec::with_capacity(contacts.len());
        for (contact_index, contact) in contacts.iter().enumerate() {
            let mut indexed = Vec::with_capacity(contact.numbers.len());
            for (number_index, number) in contact.numbers.iter().enumerate() {
                let number = IndexedNumber::new(&number.number, region);
                if let Some(key) = &number.key {
                    by_e164
                        .entry(key.e164.clone())
                        .or_insert((contact_index, number_index));
                    if let Some(national) = &key.national {
                        by_national
                            .entry(national.clone())
                            .and_modify(|found| {
                                if found.is_some_and(|(other, _)| other != contact_index) {
                                    *found = None;
                                }
                            })
                            .or_insert(Some((contact_index, number_index)));
                    }
                }
                indexed.push(number);
            }
            numbers.push(indexed);
        }
        let names = contacts
            .iter()
            .map(|contact| Name::new(&contact.name))
            .collect();
        Self {
            contacts,
            region,
            by_e164,
            by_national,
            names,
            numbers,
        }
    }

    /// The Contact `number` belongs to, if any.
    #[must_use]
    pub fn lookup(&self, number: &str) -> Option<ContactMatch> {
        let (contact, number) = self.find(&Key::of(number, self.region)?)?;
        let contact = self.contacts.get(contact)?;
        Some(ContactMatch {
            name: contact.name.clone(),
            label: contact.numbers.get(number)?.label.clone(),
        })
    }

    /// `(contact, number)` for the number with `key`.
    fn find(&self, key: &Key) -> Option<(usize, usize)> {
        self.by_e164.get(&key.e164).copied().or_else(|| {
            key.national
                .as_ref()
                .and_then(|national| self.by_national.get(national).copied().flatten())
        })
    }

    /// The Contacts, as given.
    #[must_use]
    pub fn contacts(&self) -> &[Contact] {
        &self.contacts
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.contacts.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.contacts.is_empty()
    }
}

/// How a number is compared.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Key {
    /// `+` country code, national number; or, where libphonenumber can't
    /// read the number, its digits.
    e164: String,
    /// The national significant number, if long enough to match on alone.
    national: Option<String>,
}

impl Key {
    fn of(number: &str, region: Option<Id>) -> Option<Self> {
        if let Ok(parsed) = phonenumber::parse(region, number) {
            Some(Self::from_parsed(&parsed))
        } else {
            let digits: String = number.chars().filter(char::is_ascii_digit).collect();
            (!digits.is_empty()).then_some(Self {
                e164: digits,
                national: None,
            })
        }
    }

    fn from_parsed(parsed: &phonenumber::PhoneNumber) -> Self {
        let national = format!(
            "{}{}",
            "0".repeat(usize::from(parsed.national().zeros())),
            parsed.national().value()
        );
        Self {
            e164: format!("+{}{national}", parsed.code().value()),
            national: (national.len() >= MIN_NATIONAL_DIGITS).then_some(national),
        }
    }
}

/// A number as search compares and shows it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct IndexedNumber {
    key: Option<Key>,
    /// Its digits as stored, in international form without the `+`, and in
    /// national form with the trunk prefix, such as `447700900123` and
    /// `07700900123`, for digits typed in any of those forms.
    digits: Vec<String>,
    /// The number grouped for reading.
    shown: String,
}

impl IndexedNumber {
    fn new(number: &str, region: Option<Id>) -> Self {
        let stored: String = number.chars().filter(char::is_ascii_digit).collect();
        let mut digits = vec![stored];
        let (key, shown) = match phonenumber::parse(region, number) {
            Ok(parsed) => {
                let key = Key::from_parsed(&parsed);
                digits.push(key.e164.trim_start_matches('+').to_owned());
                let national = parsed
                    .format()
                    .mode(phonenumber::Mode::National)
                    .to_string();
                digits.push(national.chars().filter(char::is_ascii_digit).collect());
                (Some(key), shown(number, &parsed))
            }
            Err(_) => (Key::of(number, region), number.trim().to_owned()),
        };
        digits.retain(|form| !form.is_empty());
        digits.dedup();
        Self { key, digits, shown }
    }

    /// How well `typed`, digits only, matches: equal, a prefix, or
    /// anywhere.
    fn matches(&self, typed: &str) -> Option<Rank> {
        let mut best: Option<Rank> = None;
        for form in &self.digits {
            let found = if form == typed {
                Some(Rank::new(Tier::Whole, 0))
            } else if form.starts_with(typed) {
                Some(Rank::new(Tier::Start, form.len() - typed.len()))
            } else {
                form.find(typed).map(|at| Rank::new(Tier::Inside, at))
            };
            best = best.into_iter().chain(found).min();
        }
        best
    }
}

/// `number` grouped for reading: in national form if written that way,
/// otherwise in international form.
fn shown(number: &str, parsed: &phonenumber::PhoneNumber) -> String {
    let written = number.trim_start();
    let mode = if written.starts_with('+') || written.starts_with("00") {
        phonenumber::Mode::International
    } else {
        phonenumber::Mode::National
    };
    parsed.format().mode(mode).to_string()
}

/// The country most of the Contacts' international numbers belong to.
fn infer_region(contacts: &[Contact]) -> Option<Id> {
    let mut counts: HashMap<u16, usize> = HashMap::new();
    for number in contacts.iter().flat_map(|contact| &contact.numbers) {
        let written = number.number.trim_start();
        if !(written.starts_with('+') || written.starts_with("00")) {
            continue;
        }
        if let Ok(parsed) = phonenumber::parse(None, written) {
            *counts.entry(parsed.code().value()).or_default() += 1;
        }
    }
    let (code, _) = counts
        .into_iter()
        .max_by_key(|&(code, count)| (count, std::cmp::Reverse(code)))?;
    let regions = phonenumber::metadata::DATABASE.by_code(&code)?;
    let main = regions
        .iter()
        .find(|metadata| metadata.is_main_country_for_code())
        .or_else(|| regions.first())?;
    main.id().parse().ok()
}

/// The number to dial for `text`, a number as typed or pasted, or a `tel:`
/// link, or `None` if the Phone can't dial it.
///
/// Spaces and the visual separators `-`, `.`, `(` and `)` are dropped, and
/// a `tel:` link loses its scheme and parameters. What is left must be made
/// only of the characters the Phone accepts in a dial string (`0`-`9`, `+`,
/// `*`, `#`, `,` and `A`-`D`), between 1 and 80 of them.
#[must_use]
pub fn dialable(text: &str) -> Option<String> {
    let text = text.trim();
    let number = match strip_tel_scheme(text) {
        Some(link) => {
            let link = link.trim_start_matches('/');
            percent_decoded(link.split(';').next().unwrap_or_default())?
        }
        None => text.to_owned(),
    };
    let number: String = number
        .chars()
        .filter(|character| !is_visual_separator(*character))
        .collect();
    let accepted = number.chars().all(|character| {
        character.is_ascii_digit() || matches!(character, '+' | '*' | '#' | ',' | 'A'..='D')
    });
    (accepted && (1..=MAX_DIAL_LENGTH).contains(&number.len())).then_some(number)
}

/// Whether `text` looks like an attempt at a number, having a digit in it,
/// but isn't one the Phone can dial, so the Dialer can say so before Enter.
#[must_use]
pub fn looks_undialable(text: &str) -> bool {
    text.chars().any(|character| character.is_ascii_digit()) && dialable(text).is_none()
}

/// What follows `tel:`, in any case, if `text` is a `tel:` link.
fn strip_tel_scheme(text: &str) -> Option<&str> {
    let scheme = text.get(..4)?;
    scheme
        .eq_ignore_ascii_case("tel:")
        .then(|| text.get(4..))
        .flatten()
}

fn is_visual_separator(character: char) -> bool {
    character.is_whitespace() || matches!(character, '-' | '.' | '(' | ')')
}

/// `text` with each `%XX` escape decoded, or `None` if one is malformed or
/// decodes to something other than ASCII.
fn percent_decoded(text: &str) -> Option<String> {
    let mut decoded = String::with_capacity(text.len());
    let mut characters = text.chars();
    while let Some(character) = characters.next() {
        if character == '%' {
            let high = characters.next()?.to_digit(16)?;
            let low = characters.next()?.to_digit(16)?;
            let byte = u8::try_from(high * 16 + low).ok()?;
            if !byte.is_ascii() {
                return None;
            }
            decoded.push(char::from(byte));
        } else {
            decoded.push(character);
        }
    }
    Some(decoded)
}

/// The Call History, indexed for search alongside the Contacts it was
/// read with.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct HistoryIndex {
    /// Each entry's number, as search compares and shows it; `None` if
    /// withheld.
    numbers: Vec<Option<IndexedNumber>>,
    /// Each entry's name from the Phone, folded.
    names: Vec<Name>,
    /// How many Calls, and the latest, by number in E.164 form.
    calls: HashMap<String, Calls>,
}

/// How often, and how lately, the user and a number have called.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Calls {
    count: u32,
    latest: Option<Timestamp>,
}

impl HistoryIndex {
    /// Indexes `entries`, read in the region of `directory`. Slow for
    /// thousands of entries: build it off the main thread.
    pub(crate) fn new(entries: &[CallHistoryEntry], directory: &Directory) -> Self {
        let numbers: Vec<Option<IndexedNumber>> = entries
            .iter()
            .map(|entry| match &entry.caller {
                Caller::Number(number) => Some(IndexedNumber::new(number, directory.region)),
                Caller::Withheld => None,
            })
            .collect();
        let mut calls: HashMap<String, Calls> = HashMap::new();
        for (entry, number) in entries.iter().zip(&numbers) {
            if let Some(key) = number.as_ref().and_then(|number| number.key.as_ref()) {
                let tally = calls.entry(key.e164.clone()).or_default();
                tally.count += 1;
                tally.latest = tally.latest.max(entry.at);
            }
        }
        Self {
            numbers,
            names: entries.iter().map(|entry| Name::new(&entry.name)).collect(),
            calls,
        }
    }

    fn calls(&self, number: &IndexedNumber) -> Calls {
        number
            .key
            .as_ref()
            .and_then(|key| self.calls.get(&key.e164))
            .copied()
            .unwrap_or_default()
    }
}

/// What a Dialer row offers to Call.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DialerRowKind {
    /// One of a Contact's numbers.
    Contact,
    /// A Call in the Call History, of this kind.
    CallHistory(CallHistoryKind),
    /// The number the user typed.
    CallNumber,
}

impl DialerRowKind {
    /// The lower-case name the control interface uses.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Contact => "contact",
            Self::CallHistory(_) => "call-history",
            Self::CallNumber => "call-number",
        }
    }
}

/// One row of the Dialer: someone to Call.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DialerRow {
    pub kind: DialerRowKind,
    /// The Contact's name, or the name the Phone gave; may be empty.
    pub name: String,
    /// The number's label, for a Contact's number.
    pub label: Option<NumberLabel>,
    /// The number as stored or typed, or withheld.
    pub caller: Caller,
    /// The number grouped for reading, or empty if withheld.
    pub shown_number: String,
    /// When the Call was, for a Call History row, if the Phone said.
    pub at: Option<Timestamp>,
}

impl DialerRow {
    /// The number Enter dials, or `None` if it was withheld or isn't one
    /// the Phone can dial.
    #[must_use]
    pub fn dial(&self) -> Option<String> {
        match &self.caller {
            Caller::Number(number) => dialable(number),
            Caller::Withheld => None,
        }
    }
}

/// The Dialer's rows for `query`, best first, at most [`MAX_ROWS`].
///
/// - Nothing typed: the Call History, newest first.
/// - Letters: Contacts and Call History names by word prefix (the first
///   word first), initials, anywhere in a word, then letters in order.
/// - Digits: Contact and Call History numbers, whatever their formatting,
///   and a row to Call the number typed, if the Phone can dial it.
/// - Among equal matches, the number called most often comes first, then
///   the one called most lately.
#[must_use]
pub fn search(query: &str, contacts: &Contacts, call_history: &CallHistory) -> Vec<DialerRow> {
    let directory = contacts.directory();
    let history = call_history.index();
    let entries = call_history.entries();
    if query.trim().is_empty() {
        return entries
            .iter()
            .zip(&history.numbers)
            .take(MAX_ROWS)
            .map(|(entry, number)| history_row(entry, number.as_ref(), directory))
            .collect();
    }
    let typed = Typed::new(query, directory.region);
    let mut found: Vec<Found> = Vec::new();
    let mut exact = false;
    for (contact_index, (contact, name)) in
        directory.contacts.iter().zip(&directory.names).enumerate()
    {
        let name_rank = typed.name_rank(name);
        for (number_index, number) in directory.numbers[contact_index].iter().enumerate() {
            let number_rank = typed.number_rank(number);
            let Some(rank) = name_rank.into_iter().chain(number_rank).min() else {
                continue;
            };
            exact |= typed.is(number);
            let label = contact.numbers[number_index].label.clone();
            found.push(Found {
                rank,
                calls: history.calls(number),
                folded: name.folded.clone(),
                row: DialerRow {
                    kind: DialerRowKind::Contact,
                    name: contact.name.clone(),
                    label: Some(label),
                    caller: Caller::Number(contact.numbers[number_index].number.clone()),
                    shown_number: number.shown.clone(),
                    at: None,
                },
            });
        }
    }
    // Callers who aren't Contacts, once each, as their latest Call.
    let mut seen = HashSet::new();
    for ((entry, number), name) in entries.iter().zip(&history.numbers).zip(&history.names) {
        let Some(number) = number else { continue };
        let known = number
            .key
            .as_ref()
            .is_some_and(|key| directory.find(key).is_some());
        let first = seen.insert(number.digits.first().cloned().unwrap_or_default());
        if known || !first {
            continue;
        }
        let Some(rank) = typed
            .name_rank(name)
            .into_iter()
            .chain(typed.number_rank(number))
            .min()
        else {
            continue;
        };
        exact |= typed.is(number);
        found.push(Found {
            rank,
            calls: history.calls(number),
            folded: name.folded.clone(),
            row: history_row(entry, Some(number), directory),
        });
    }
    // Stable, so a Contact's numbers keep their order.
    found.sort_by(Found::order);
    let mut rows: Vec<DialerRow> = found.into_iter().map(|found| found.row).collect();
    if let Some(call) = typed.call_row().filter(|_| !exact) {
        if typed.is_whole_number() {
            rows.insert(0, call);
        } else {
            rows.push(call);
        }
    }
    rows.truncate(MAX_ROWS);
    rows
}

fn history_row(
    entry: &CallHistoryEntry,
    number: Option<&IndexedNumber>,
    directory: &Directory,
) -> DialerRow {
    let contact = number
        .and_then(|number| number.key.as_ref())
        .and_then(|key| directory.find(key))
        .and_then(|(contact, number)| {
            let contact = directory.contacts.get(contact)?;
            Some((
                contact.name.clone(),
                contact.numbers.get(number)?.label.clone(),
            ))
        });
    let (name, label) = match contact {
        Some((name, label)) if !name.trim().is_empty() => (name, Some(label)),
        Some((_, label)) => (entry.name.clone(), Some(label)),
        None => (entry.name.clone(), None),
    };
    DialerRow {
        kind: DialerRowKind::CallHistory(entry.kind),
        name,
        label,
        caller: entry.caller.clone(),
        shown_number: number
            .map(|number| number.shown.clone())
            .unwrap_or_default(),
        at: entry.at,
    }
}

/// A row that matched, with what it is ranked by.
struct Found {
    rank: Rank,
    calls: Calls,
    folded: String,
    row: DialerRow,
}

impl Found {
    /// Better matches first; among equal ones, an exact word first, then
    /// the most called, the latest called, the closest, and by name.
    fn order(&self, other: &Self) -> std::cmp::Ordering {
        let key = |found: &Self| {
            (
                found.rank.tier,
                found.rank.closeness != 0,
                std::cmp::Reverse(found.calls.count),
                std::cmp::Reverse(found.calls.latest),
                found.rank.closeness,
            )
        };
        key(self)
            .cmp(&key(other))
            .then_with(|| self.folded.cmp(&other.folded))
    }
}

/// How well something matched, better first.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct Rank {
    tier: Tier,
    /// How much of the match wasn't typed, or where it was; less is closer.
    closeness: usize,
}

impl Rank {
    fn new(tier: Tier, closeness: usize) -> Self {
        Self { tier, closeness }
    }
}

/// Kinds of match, best first.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Tier {
    /// The whole number, or each typed word starting a name's words in
    /// order from its first.
    Whole,
    /// The start of a number, or each typed word starting a name's words in
    /// order, from a later one.
    Start,
    /// Each typed word starting one of a name's words, in another order.
    WordsAnyOrder,
    /// The initials of the name's words.
    Initials,
    /// Each typed word, or the digits, anywhere.
    Inside,
    /// The typed letters, in order, anywhere in the name.
    Fuzzy,
}

/// A name, folded for search.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Name {
    words: Vec<String>,
    /// The words joined by single spaces.
    folded: String,
    /// The words' first letters.
    initials: String,
    /// The words run together.
    compact: String,
}

impl Name {
    fn new(name: &str) -> Self {
        let folded = fold(name);
        let words: Vec<String> = folded.split_whitespace().map(str::to_owned).collect();
        Self {
            initials: words
                .iter()
                .filter_map(|word| word.chars().next())
                .collect(),
            compact: words.concat(),
            folded: words.join(" "),
            words,
        }
    }
}

/// `text` in lower case without accents or apostrophes, every other mark
/// a space.
fn fold(text: &str) -> String {
    let mut folded = String::with_capacity(text.len());
    for character in text.nfd() {
        match character {
            _ if is_combining_mark(character) => {}
            '\'' | '\u{2019}' | '\u{02bc}' => {}
            'ß' => folded.push_str("ss"),
            'æ' | 'Æ' => folded.push_str("ae"),
            'œ' | 'Œ' => folded.push_str("oe"),
            'þ' | 'Þ' => folded.push_str("th"),
            'ø' | 'Ø' => folded.push('o'),
            'ł' | 'Ł' => folded.push('l'),
            'đ' | 'Đ' | 'ð' | 'Ð' => folded.push('d'),
            'ı' => folded.push('i'),
            _ if character.is_alphanumeric() => folded.extend(character.to_lowercase()),
            _ => folded.push(' '),
        }
    }
    folded
}

/// What the user typed, ready to compare.
struct Typed {
    words: Vec<String>,
    compact: String,
    /// The number to dial, if the Phone can dial what was typed and it has
    /// a digit.
    dial: Option<String>,
    /// Its digits alone, to compare with numbers' digits.
    digits: Option<String>,
    key: Option<Key>,
    parsed: Option<phonenumber::PhoneNumber>,
    region: Option<Id>,
}

impl Typed {
    fn new(query: &str, region: Option<Id>) -> Self {
        let name = Name::new(query);
        let dial = dialable(query).filter(|number| number.chars().any(|c| c.is_ascii_digit()));
        let digits = dial.as_ref().map(|number| {
            let digits: String = number.chars().filter(char::is_ascii_digit).collect();
            // 00 is the international prefix in most of the world.
            match digits.strip_prefix("00") {
                Some(international) if number.starts_with("00") && !international.is_empty() => {
                    international.to_owned()
                }
                _ => digits,
            }
        });
        let parsed = dial
            .as_ref()
            .and_then(|number| phonenumber::parse(region, number).ok());
        Self {
            words: name.words,
            compact: name.compact,
            key: parsed.as_ref().map(Key::from_parsed),
            dial,
            digits,
            parsed,
            region,
        }
    }

    fn name_rank(&self, name: &Name) -> Option<Rank> {
        if self.words.is_empty() || name.words.is_empty() {
            return None;
        }
        if let Some((first, closeness)) = prefixes_in_order(&self.words, &name.words) {
            let tier = if first == 0 { Tier::Whole } else { Tier::Start };
            return Some(Rank::new(tier, closeness));
        }
        if let Some(closeness) = prefixes_any_order(&self.words, &name.words) {
            return Some(Rank::new(Tier::WordsAnyOrder, closeness));
        }
        if let [word] = self.words.as_slice()
            && word.chars().count() >= 2
            && name.initials.starts_with(word.as_str())
        {
            return Some(Rank::new(Tier::Initials, name.initials.len() - word.len()));
        }
        if let Some(at) = self
            .words
            .iter()
            .map(|word| name.folded.find(word.as_str()))
            .collect::<Option<Vec<usize>>>()
        {
            return Some(Rank::new(Tier::Inside, at.into_iter().sum()));
        }
        if self.compact.chars().count() >= MIN_FUZZY_LETTERS {
            return in_order(&self.compact, &name.compact).map(|span| Rank::new(Tier::Fuzzy, span));
        }
        None
    }

    fn number_rank(&self, number: &IndexedNumber) -> Option<Rank> {
        if self.is(number) {
            return Some(Rank::new(Tier::Whole, 0));
        }
        number.matches(self.digits.as_deref()?)
    }

    /// Whether what was typed is `number`, however each is written.
    fn is(&self, number: &IndexedNumber) -> bool {
        match (&self.key, &number.key) {
            (Some(typed), Some(number)) => {
                typed.e164 == number.e164
                    || typed.national.is_some() && typed.national == number.national
            }
            _ => false,
        }
    }

    /// Whether what was typed is a whole number, by libphonenumber's rules.
    fn is_whole_number(&self) -> bool {
        self.parsed.as_ref().is_some_and(phonenumber::is_valid)
    }

    /// The row that Calls the number typed, if the Phone can dial it.
    fn call_row(&self) -> Option<DialerRow> {
        let dial = self.dial.clone()?;
        let shown = match &self.parsed {
            Some(parsed) if self.region.is_some() || dial.starts_with('+') => shown(&dial, parsed),
            _ => dial.clone(),
        };
        Some(DialerRow {
            kind: DialerRowKind::CallNumber,
            name: String::new(),
            label: None,
            caller: Caller::Number(dial),
            shown_number: shown,
            at: None,
        })
    }
}

/// Where each of `typed` starts one of `words`, in order: the first word
/// it starts, and how many letters of those words weren't typed.
fn prefixes_in_order(typed: &[String], words: &[String]) -> Option<(usize, usize)> {
    let mut next = 0;
    let mut first = None;
    let mut closeness = 0;
    for part in typed {
        let at = next
            + words
                .get(next..)?
                .iter()
                .position(|word| word.starts_with(part.as_str()))?;
        first.get_or_insert(at);
        closeness += words[at].len() - part.len();
        next = at + 1;
    }
    Some((first?, closeness))
}

/// Whether each of `typed` starts a different one of `words`, in any order,
/// and how many letters of those words weren't typed.
fn prefixes_any_order(typed: &[String], words: &[String]) -> Option<usize> {
    let mut used = vec![false; words.len()];
    let mut closeness = 0;
    for part in typed {
        let at = words
            .iter()
            .enumerate()
            .position(|(index, word)| !used[index] && word.starts_with(part.as_str()))?;
        used[at] = true;
        closeness += words[at].len() - part.len();
    }
    Some(closeness)
}

/// Whether `letters` appear in `text` in order, and how far apart: the
/// span they cover, less their own length.
fn in_order(letters: &str, text: &str) -> Option<usize> {
    let mut letters_left = letters.chars().peekable();
    let mut start = None;
    let mut end = 0;
    for (at, character) in text.char_indices() {
        if letters_left.peek() == Some(&character) {
            start.get_or_insert(at);
            letters_left.next();
            end = at + character.len_utf8();
            if letters_left.peek().is_none() {
                break;
            }
        }
    }
    if letters_left.peek().is_some() {
        return None;
    }
    Some((end - start?).saturating_sub(letters.len()))
}
