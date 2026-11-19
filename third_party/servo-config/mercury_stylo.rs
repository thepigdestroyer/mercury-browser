/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

// Mercury: this file is new (K0, ED2). Servo forwards seven Stylo preferences, each by a line in
// `prefs::set`, so turning on another Stylo feature meant editing that function and a new field.
// `Preferences::stylo_prefs` is a list of Stylo preference names instead, applied by
// `prefs::set` through `apply_stylo_prefs`. It has no dependency on a logging crate: the entries
// that could not be applied are returned, and Mercury's tests assert that there are none.

/// Sets Stylo preferences from `list`, comma-separated `name=true` or `name=false` pairs with
/// Stylo's own (Gecko) names, such as `layout.css.nth-child-of.enabled=true`. Spaces around names,
/// values and commas are ignored and empty entries are skipped. When a name appears twice, the
/// last value is the one kept.
///
/// Returns what could not be applied, in the order of the list: the name of a preference that
/// Stylo does not have as a boolean preference, or the whole entry when it is not `name=true` or
/// `name=false`. Every entry that can be applied is applied, whatever the others are.
pub fn apply_stylo_prefs(list: &str) -> Vec<String> {
    let mut rejected = Vec::new();
    for entry in list
        .split(',')
        .map(str::trim)
        .filter(|entry| !entry.is_empty())
    {
        let parsed = entry.split_once('=').and_then(|(name, value)| {
            let value = match value.trim() {
                "true" => true,
                "false" => false,
                _ => return None,
            };
            Some((name.trim(), value)).filter(|(name, _)| !name.is_empty())
        });
        match parsed {
            Some((name, value)) => {
                if !stylo_static_prefs::set_bool(name, value) {
                    rejected.push(name.to_owned());
                }
            },
            None => rejected.push(entry.to_owned()),
        }
    }
    rejected
}

/// The current value of the Stylo boolean preference called `name` (a Gecko name), or `None` when
/// Stylo has no boolean preference of that name.
pub fn stylo_bool_pref(name: &str) -> Option<bool> {
    stylo_static_prefs::get_bool(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    // The Stylo preferences are process-wide and the tests run in parallel: each test uses
    // preferences of its own.

    #[test]
    fn applies_each_name_and_value() {
        let rejected = apply_stylo_prefs(
            "layout.css.anchor-positioning.enabled=true,layout.css.appearance.enabled=true",
        );

        assert!(rejected.is_empty(), "{rejected:?}");
        assert_eq!(
            stylo_bool_pref("layout.css.anchor-positioning.enabled"),
            Some(true)
        );
        assert_eq!(stylo_bool_pref("layout.css.appearance.enabled"), Some(true));
    }

    #[test]
    fn ignores_spaces_and_empty_entries() {
        let rejected = apply_stylo_prefs(
            " , layout.css.at-scope.enabled = true ,, \t layout.css.counters.enabled=true , ",
        );

        assert!(rejected.is_empty(), "{rejected:?}");
        assert_eq!(stylo_bool_pref("layout.css.at-scope.enabled"), Some(true));
        assert_eq!(stylo_bool_pref("layout.css.counters.enabled"), Some(true));
    }

    #[test]
    fn false_turns_a_preference_off() {
        // On in Stylo by default.
        assert_eq!(
            stylo_bool_pref("layout.css.alpha-color-function.enabled"),
            Some(true)
        );

        let rejected = apply_stylo_prefs("layout.css.alpha-color-function.enabled=false");

        assert!(rejected.is_empty(), "{rejected:?}");
        assert_eq!(
            stylo_bool_pref("layout.css.alpha-color-function.enabled"),
            Some(false)
        );
        apply_stylo_prefs("layout.css.alpha-color-function.enabled=true");
    }

    #[test]
    fn the_last_value_of_a_name_wins() {
        assert!(
            apply_stylo_prefs("layout.css.contain.enabled=true,layout.css.contain.enabled=false")
                .is_empty()
        );
        assert_eq!(stylo_bool_pref("layout.css.contain.enabled"), Some(false));

        assert!(
            apply_stylo_prefs(
                "layout.css.corner-shape.enabled=false,layout.css.corner-shape.enabled=true"
            )
            .is_empty()
        );
        assert_eq!(
            stylo_bool_pref("layout.css.corner-shape.enabled"),
            Some(true)
        );

        // Setting the same list twice leaves the same values.
        for _ in 0..2 {
            apply_stylo_prefs("layout.css.cross-fade.enabled=true");
            assert_eq!(stylo_bool_pref("layout.css.cross-fade.enabled"), Some(true));
        }
    }

    #[test]
    fn returns_unknown_names_and_applies_the_rest() {
        let rejected = apply_stylo_prefs(
            "layout.css.no-such-preference.enabled=true,layout.css.custom-media.enabled=true,\
             layout.threads=true,Layout.css.custom-media.enabled=true",
        );

        // `layout.threads` is an integer preference; names match exactly.
        assert_eq!(
            rejected,
            [
                "layout.css.no-such-preference.enabled",
                "layout.threads",
                "Layout.css.custom-media.enabled",
            ]
        );
        assert_eq!(
            stylo_bool_pref("layout.css.custom-media.enabled"),
            Some(true)
        );
    }

    #[test]
    fn returns_entries_that_are_not_a_name_and_a_boolean() {
        let rejected = apply_stylo_prefs(
            "layout.css.display-animations.enabled,layout.css.display-animations.enabled=1,\
             =true,layout.css.display-animations.enabled=True,layout.css.display-animations.enabled=",
        );

        assert_eq!(
            rejected,
            [
                "layout.css.display-animations.enabled",
                "layout.css.display-animations.enabled=1",
                "=true",
                "layout.css.display-animations.enabled=True",
                "layout.css.display-animations.enabled=",
            ]
        );
        // Nothing was applied.
        assert_eq!(
            stylo_bool_pref("layout.css.display-animations.enabled"),
            Some(false)
        );
    }

    #[test]
    fn an_empty_list_changes_nothing() {
        assert!(apply_stylo_prefs("").is_empty());
        assert!(apply_stylo_prefs("  ,  , ").is_empty());
    }

    #[test]
    fn stylo_bool_pref_is_none_for_other_names() {
        assert_eq!(
            stylo_bool_pref("layout.css.no-such-preference.enabled"),
            None
        );
        assert_eq!(stylo_bool_pref("layout.threads"), None);
        assert_eq!(stylo_bool_pref(""), None);
    }

    #[test]
    fn prefs_set_applies_the_stylo_prefs_list() {
        assert_eq!(
            stylo_bool_pref("layout.css.dominant-baseline.enabled"),
            Some(false)
        );
        let mut preferences = crate::prefs::Preferences::default();
        preferences.stylo_prefs = "layout.css.dominant-baseline.enabled=true".to_owned();

        crate::prefs::set(preferences);

        assert_eq!(
            stylo_bool_pref("layout.css.dominant-baseline.enabled"),
            Some(true)
        );
        // Servo's own seven mappings still work beside it.
        assert_eq!(stylo_bool_pref("layout.css.columns.enabled"), Some(false));
    }
}
