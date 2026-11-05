//! Phone numbers compared without being kept: the runner checks that 7.1's
//! Call goes to the Test Number, and that 2.5 calls back the number that
//! rang, while never storing or printing either number.

use std::hash::{DefaultHasher, Hash as _, Hasher as _};

use phonenumber::country;
use phonenumber::metadata::DATABASE;

/// A phone number's fingerprint: its national number, hashed, and its
/// country code, if it has one. It stays in memory and is never written
/// anywhere, so the number itself isn't kept.
///
/// Two fingerprints are [the same number](Fingerprint::is) when their whole
/// national numbers match and so do their country codes, where both have
/// one: `07700 900999` and `+44 7700 900999` match; `+44 8700 900999` and
/// `+33 7700 900999` don't match `+44 7700 900999`.
#[derive(Clone, Copy, Debug)]
pub struct Fingerprint {
    /// The national number's hash, or `None` for a number with no digits,
    /// such as a withheld caller's.
    national: Option<u64>,
    country: Option<u16>,
}

/// The Test Number has no country code, and the Desktop's locale doesn't
/// say which country the user is in.
#[derive(Debug, PartialEq, Eq)]
pub struct NoCountry;

impl Fingerprint {
    /// The fingerprint of `number` as a Phone reports it, or as written
    /// down: `+44 7700 900999`, `0044 7700 900999`, `+44 (0)7700 900999`,
    /// `(+44) 7700 900999`, `tel:+44-7700-900999`, or, with no country code,
    /// `07700 900999`.
    pub fn of(number: &str) -> Self {
        Self::read(number, None)
    }

    /// The Test Number's fingerprint, written as in [`Fingerprint::of`].
    /// Without a country code it is taken as a number in `locale_region`
    /// (`US` for `en_US.UTF-8`), and the country code that gives is
    /// returned as assumed. `None` for an empty number.
    ///
    /// # Errors
    ///
    /// [`NoCountry`] if the number has no country code and
    /// `locale_region` names no country.
    pub fn of_test_number(
        number: &str,
        locale_region: Option<&str>,
    ) -> Result<Option<(Self, Option<u16>)>, NoCountry> {
        let plain = Self::of(number);
        if plain.national.is_none() {
            return Ok(None);
        }
        if plain.country.is_some() {
            return Ok(Some((plain, None)));
        }
        let (region, code) = locale_region
            .and_then(|region| region.to_ascii_uppercase().parse::<country::Id>().ok())
            .and_then(|region| {
                let code = DATABASE.by_id(region.as_ref())?.country_code();
                Some((region, code))
            })
            .ok_or(NoCountry)?;
        Ok(Some((Self::read(number, Some((region, code))), Some(code))))
    }

    /// Whether the Phone reported a number at all.
    pub fn is_known(&self) -> bool {
        self.national.is_some()
    }

    /// Whether `other` is the same number: the whole national number
    /// matches, and so does the country code where both have one. A number
    /// with no digits is no number, not even itself.
    pub fn is(&self, other: &Self) -> bool {
        self.national.is_some()
            && self.national == other.national
            && match (self.country, other.country) {
                (Some(mine), Some(theirs)) => mine == theirs,
                _ => true,
            }
    }

    /// `number`'s fingerprint; one without a country code is read in
    /// `region`, if given, which has the country code given with it.
    fn read(number: &str, region: Option<(country::Id, u16)>) -> Self {
        let written = number.trim();
        let written = written
            .get(..4)
            .filter(|scheme| scheme.eq_ignore_ascii_case("tel:"))
            .map_or(written, |_| &written[4..]);
        // `tel:` parameters, such as `;phone-context=`, aren't the number.
        let written = written.split(';').next().unwrap_or_default();
        // `+44 (0)7700 …`: the trunk prefix, shown but not dialled.
        let written = written.replace("(0)", "");
        let digits: String = written.chars().filter(char::is_ascii_digit).collect();
        if digits.is_empty() {
            return Self {
                national: None,
                country: None,
            };
        }
        let plus = written
            .trim_start_matches(|first: char| first == '(' || first.is_whitespace())
            .starts_with('+');
        let international = if plus {
            Some(digits.as_str())
        } else {
            digits.strip_prefix("00")
        };
        let parsed = match (international, region) {
            (Some(international), _) => phonenumber::parse(None, format!("+{international}")).ok(),
            (None, Some((region, _))) => phonenumber::parse(Some(region), &digits).ok(),
            (None, None) => None,
        };
        let (national, country) = match parsed {
            Some(parsed) => (
                format!(
                    "{}{}",
                    "0".repeat(usize::from(parsed.national().zeros())),
                    parsed.national().value()
                ),
                Some(parsed.code().value()),
            ),
            // Unreadable with its country code: never the same as a number
            // libphonenumber could read.
            None if international.is_some() => (digits, None),
            None => (
                digits.strip_prefix('0').unwrap_or(&digits).to_owned(),
                region.map(|(_, code)| code),
            ),
        };
        let mut hasher = DefaultHasher::new();
        national.hash(&mut hasher);
        Self {
            national: Some(hasher.finish()),
            country,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Fingerprint, NoCountry};

    /// The Test Number's fingerprint, and the country code assumed for it.
    fn test_number(number: &str, region: Option<&str>) -> (Fingerprint, Option<u16>) {
        Fingerprint::of_test_number(number, region)
            .expect("a country")
            .expect("a number")
    }

    #[test]
    fn a_national_test_number_takes_the_country_of_the_locale() {
        let (gb, assumed) = test_number("07700 900999", Some("GB"));
        assert_eq!(assumed, Some(44));
        assert!(gb.is(&Fingerprint::of("+447700900999")));
        assert!(
            gb.is(&Fingerprint::of("07700900999")),
            "a Call with no country code"
        );
        assert!(!gb.is(&Fingerprint::of("+337700900999")));
        assert!(
            !gb.is(&Fingerprint::of("+448700900999")),
            "the first digit slipped"
        );

        let (us, assumed) = test_number("(201) 555-0123", Some("us"));
        assert_eq!(assumed, Some(1));
        assert!(us.is(&Fingerprint::of("+1 201-555-0123")));
        assert!(
            us.is(&Fingerprint::of("2015550123")),
            "a Call with no country code"
        );
        assert!(!us.is(&Fingerprint::of("+33 2015550123")));
        assert!(!us.is(&Fingerprint::of("+44 2015550123")));
    }

    #[test]
    fn a_test_number_with_its_country_code_ignores_the_locale() {
        let (number, assumed) = test_number("+44 7700 900999", Some("US"));
        assert_eq!(assumed, None);
        assert!(number.is(&Fingerprint::of("+447700900999")));
        assert!(!number.is(&Fingerprint::of("+17700900999")));
    }

    #[test]
    fn without_a_country_code_or_a_locale_region_the_test_number_is_refused() {
        assert_eq!(
            Fingerprint::of_test_number("07700 900999", None).err(),
            Some(NoCountry)
        );
        assert_eq!(
            Fingerprint::of_test_number("07700 900999", Some("ZZ")).err(),
            Some(NoCountry)
        );
        assert!(matches!(Fingerprint::of_test_number("", None), Ok(None)));
    }

    #[test]
    fn every_international_form_is_the_same_number() {
        let plain = Fingerprint::of("+447700900999");
        for written in [
            "+44 7700 900999",
            "0044 7700 900999",
            "+44 (0)7700 900999",
            "(+44) 7700 900999",
            "tel:+44-7700-900999",
            "TEL:+447700900999;phone-context=+44",
        ] {
            let number = Fingerprint::of(written);
            assert!(number.is(&plain), "{written}");
            assert!(plain.is(&number), "{written}");
        }
    }

    #[test]
    fn an_empty_number_is_no_number() {
        let empty = Fingerprint::of("");
        assert!(!empty.is_known());
        assert!(!empty.is(&Fingerprint::of("")));
        assert!(!empty.is(&Fingerprint::of("+447700900999")));
        assert!(!Fingerprint::of("+447700900999").is(&empty));
        assert!(Fingerprint::of("+447700900999").is_known());
    }
}
