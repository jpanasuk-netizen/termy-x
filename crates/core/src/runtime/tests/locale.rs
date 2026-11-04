use crate::locale::{
    DEFAULT_UTF8_LOCALE, Utf8LocaleOverridePlan, preferred_utf8_locale, utf8_locale_override_plan,
};

#[test]
fn locale_override_plan_forces_lc_ctype_when_no_utf8_and_no_lc_all() {
    assert_eq!(
        utf8_locale_override_plan(None, Some("C"), Some("")),
        Utf8LocaleOverridePlan::LcCtypeOnly
    );
}

#[test]
fn locale_override_plan_forces_lc_all_when_lc_all_is_non_utf8() {
    assert_eq!(
        utf8_locale_override_plan(Some("C"), Some("C"), Some("")),
        Utf8LocaleOverridePlan::LcAllAndLcCtype
    );
}

#[test]
fn locale_override_plan_skips_when_utf8_present() {
    assert_eq!(
        utf8_locale_override_plan(Some("en_US.UTF-8"), Some("C"), Some("")),
        Utf8LocaleOverridePlan::None
    );
}

#[test]
fn locale_override_plan_prefers_lc_all_over_lang() {
    assert_eq!(
        utf8_locale_override_plan(Some("fr_FR.ISO8859-1"), Some("C"), Some("en_US.UTF-8")),
        Utf8LocaleOverridePlan::LcAllAndLcCtype
    );
}

#[test]
fn locale_override_plan_does_not_skip_for_utf8_substring_false_positive() {
    assert_eq!(
        utf8_locale_override_plan(Some("en_US.fakeutf8"), Some("C"), Some("")),
        Utf8LocaleOverridePlan::LcAllAndLcCtype
    );
}

#[test]
fn locale_override_plan_skips_for_utf8_with_modifier() {
    assert_eq!(
        utf8_locale_override_plan(Some("en_US.UTF-8@variant"), Some("C"), Some("")),
        Utf8LocaleOverridePlan::None
    );
}

#[test]
fn preferred_utf8_locale_preserves_lang_region_from_lc_all() {
    assert_eq!(
        preferred_utf8_locale(Some("fr_FR.ISO8859-1"), Some("C"), Some("en_US.ISO8859-1")),
        "fr_FR.UTF-8"
    );
}

#[test]
fn preferred_utf8_locale_preserves_locale_modifier() {
    assert_eq!(
        preferred_utf8_locale(None, Some("sr_RS@latin"), Some("")),
        "sr_RS.UTF-8@latin"
    );
}

#[test]
fn preferred_utf8_locale_falls_back_for_c_or_posix() {
    assert_eq!(
        preferred_utf8_locale(Some("C"), Some("POSIX"), Some("")),
        DEFAULT_UTF8_LOCALE
    );
}
