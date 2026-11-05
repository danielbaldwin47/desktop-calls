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

/// Loads libphonenumber's metadata for every region, which the first
/// [`format`] otherwise does. On the Desktop that takes about 40 ms in a
/// release build and 0.27 s in a debug build.
pub fn prepare() {
    // Dereferencing the lazily built database builds it.
    let _: &phonenumber::metadata::Database = &phonenumber::metadata::DATABASE;
}
