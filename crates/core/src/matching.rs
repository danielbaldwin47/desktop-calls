//! Matching: which Contact a number belongs to, however each is written.
//!
//! Numbers are compared as libphonenumber reads them, so `07700 900123` in
//! Contacts matches a Call from `+447700900123`. A number written without
//! its country code is read in the default region: the country most of the
//! Contacts' own international numbers belong to, or, if they have none, the
//! region of the Desktop's locale.

use std::collections::HashMap;

use phonenumber::country::Id;

use crate::contacts::{Contact, NumberLabel};

/// Fewer digits than this, and a number matched on its national digits
/// alone, across countries, would match too much.
const MIN_NATIONAL_DIGITS: usize = 7;

/// Contacts, indexed by number.
#[derive(Default)]
pub struct Directory {
    contacts: Vec<Contact>,
    region: Option<Id>,
    /// `(contact, number)` by the number in E.164 form.
    by_e164: HashMap<String, (usize, usize)>,
    /// `(contact, number)` by the national significant number, where only
    /// one Contact has it.
    by_national: HashMap<String, Option<(usize, usize)>>,
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
        for (contact_index, contact) in contacts.iter().enumerate() {
            for (number_index, number) in contact.numbers.iter().enumerate() {
                let Some(key) = Key::of(&number.number, region) else {
                    continue;
                };
                by_e164
                    .entry(key.e164)
                    .or_insert((contact_index, number_index));
                if let Some(national) = key.national {
                    by_national
                        .entry(national)
                        .and_modify(|found| {
                            if found.is_some_and(|(other, _)| other != contact_index) {
                                *found = None;
                            }
                        })
                        .or_insert(Some((contact_index, number_index)));
                }
            }
        }
        Self {
            contacts,
            region,
            by_e164,
            by_national,
        }
    }

    /// The Contact `number` belongs to, if any.
    #[must_use]
    pub fn lookup(&self, number: &str) -> Option<ContactMatch> {
        let key = Key::of(number, self.region)?;
        let (contact, number) = self.by_e164.get(&key.e164).copied().or_else(|| {
            key.national
                .and_then(|national| self.by_national.get(&national).copied().flatten())
        })?;
        let contact = self.contacts.get(contact)?;
        Some(ContactMatch {
            name: contact.name.clone(),
            label: contact.numbers.get(number)?.label.clone(),
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
            let national = format!(
                "{}{}",
                "0".repeat(usize::from(parsed.national().zeros())),
                parsed.national().value()
            );
            Some(Self {
                e164: format!("+{}{national}", parsed.code().value()),
                national: (national.len() >= MIN_NATIONAL_DIGITS).then_some(national),
            })
        } else {
            let digits: String = number.chars().filter(char::is_ascii_digit).collect();
            (!digits.is_empty()).then_some(Self {
                e164: digits,
                national: None,
            })
        }
    }
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
