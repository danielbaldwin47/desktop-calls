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

/// The region of the Desktop's locale for telephone numbers, such as `GB`
/// for `en_GB.UTF-8`, from the environment variables `LC_ALL`,
/// `LC_TELEPHONE` and `LANG`, as the caller read them. The first that is
/// set and not empty decides, as the locale rules say (`LC_ALL` overrides
/// the categories, and `LC_TELEPHONE` overrides `LANG`), so `LC_ALL=C` means
/// no region even if `LANG` has one. `None` for a locale with no region,
/// such as `C.UTF-8`.
///
/// The app's Contacts and the Real-Phone Checklist both read the locale
/// through this one rule.
#[must_use]
pub fn locale_region(
    lc_all: Option<&str>,
    lc_telephone: Option<&str>,
    lang: Option<&str>,
) -> Option<String> {
    let locale = [lc_all, lc_telephone, lang]
        .into_iter()
        .flatten()
        .find(|value| !value.is_empty())?;
    let (_, rest) = locale.split_once('_')?;
    let region: String = rest.chars().take_while(char::is_ascii_alphabetic).collect();
    (region.len() == 2).then_some(region)
}

/// Loads libphonenumber's metadata for every region, which the first
/// [`format`] otherwise does. On the Desktop that takes about 40 ms in a
/// release build and 0.27 s in a debug build.
pub fn prepare() {
    // Dereferencing the lazily built database builds it.
    let _: &phonenumber::metadata::Database = &phonenumber::metadata::DATABASE;
}
