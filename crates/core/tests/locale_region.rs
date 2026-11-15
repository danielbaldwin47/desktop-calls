//! The region of the Desktop's locale for telephone numbers: one rule for
//! the app's Contacts and for the Real-Phone Checklist's step 7.1.

use desktop_calls_core::locale_region;

#[test]
fn the_region_is_the_one_in_the_locale_name() {
    assert_eq!(
        locale_region(None, None, Some("en_GB.UTF-8")).as_deref(),
        Some("GB")
    );
    assert_eq!(
        locale_region(None, None, Some("de_AT@euro")).as_deref(),
        Some("AT")
    );
    assert_eq!(
        locale_region(None, None, Some("fr_CA")).as_deref(),
        Some("CA")
    );
}

#[test]
fn a_locale_with_no_region_gives_none() {
    for locale in ["C", "C.UTF-8", "POSIX", "en", "en.UTF-8", "en_", "en_USA"] {
        assert_eq!(locale_region(None, None, Some(locale)), None, "{locale}");
    }
    assert_eq!(locale_region(None, None, None), None);
}

#[test]
fn lc_all_overrides_lc_telephone_which_overrides_lang_as_the_locale_rules_say() {
    assert_eq!(
        locale_region(Some("en_GB.UTF-8"), Some("en_US.UTF-8"), Some("fr_CA")).as_deref(),
        Some("GB")
    );
    assert_eq!(
        locale_region(None, Some("en_US.UTF-8"), Some("fr_CA")).as_deref(),
        Some("US")
    );
    assert_eq!(
        locale_region(None, None, Some("fr_CA")).as_deref(),
        Some("CA")
    );
}

#[test]
fn an_empty_variable_is_unset_and_a_set_one_without_a_region_is_not_passed_over() {
    assert_eq!(
        locale_region(Some(""), Some(""), Some("en_US.UTF-8")).as_deref(),
        Some("US")
    );
    // LC_ALL=C makes every category C, whatever LC_TELEPHONE says.
    assert_eq!(
        locale_region(Some("C.UTF-8"), Some("en_GB.UTF-8"), Some("en_GB.UTF-8")),
        None
    );
}
