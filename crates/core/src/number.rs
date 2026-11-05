//! Phone numbers, formatted for reading by libphonenumber's rules.

/// `number` grouped for reading, such as `+44 7700 900123` for
/// `+447700900123`. A number without a country code can't be grouped
/// without knowing the region, so it is returned as it came; the Matching
/// module will infer the region from the Contacts.
#[must_use]
pub fn format(number: &str) -> String {
    match phonenumber::parse(None, number) {
        Ok(parsed) => parsed
            .format()
            .mode(phonenumber::Mode::International)
            .to_string(),
        Err(_) => number.to_owned(),
    }
}
