// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! The engine preferences Mercury sets for the web platform, in one place.
//!
//! Servo's own defaults (`third_party/servo-config/prefs.rs`) reproduce Servo 0.8; the values
//! Mercury ships are chosen here, and the lane that makes a feature work flips its line here and
//! nowhere in the Servo crates. [`apply`] runs for every launch: nothing in it depends on
//! `RuntimeOptions::experimental_web_features`, so `--disable-experimental-web-features` keeps
//! these values. A preference that is listed with no consumer yet changes nothing.
//!
//! Every preference named here must exist: `Preferences::set_value` panics on an unknown name,
//! which the tests below catch.

use servo::{PrefValue, Preferences};

/// Stylo (style system) preferences, by their Gecko names, that Mercury sets. They are joined into
/// `Preferences::stylo_prefs`, which `servo::prefs::set` applies to Stylo (Servo forwards only
/// seven of them by itself). A Stylo preference is turned on by the lane that implements the
/// layout it needs, not before.
pub(crate) const STYLO_PREFS: &[(&str, bool)] = &[
    // `:nth-child(An+B of S)`: Servo's selector parser knows it and Stylo matches it.
    ("layout.css.nth-child-of.enabled", true),
    // CSS counters (ED5, lane txt-1): `counter-reset` and `counter-increment`. The counter pass
    // is in `third_party/servo-layout/counters.rs`; without this line Stylo drops both properties.
    ("layout.css.counters.enabled", true),
    // `text-overflow: clip | ellipsis | <string>` (ED7, lane txt-1), laid out by
    // `third_party/servo-layout/ellipsis.rs` and `flow/inline/line.rs`.
    ("layout.css.text-overflow.enabled", true),
];

/// `STYLO_PREFS` as the comma-separated list `Preferences::stylo_prefs` holds.
fn stylo_prefs_list() -> String {
    STYLO_PREFS
        .iter()
        .map(|(name, value)| format!("{name}={value}"))
        .collect::<Vec<_>>()
        .join(",")
}

/// Feature switches that are off for now and stay pinned off, so that neither a change of Servo's
/// default nor the experimental-features list can turn one on by accident. The lane that makes a
/// feature work removes its line here (and turns the preference on in [`apply`]).
const PINNED_OFF: &[&str] = &[
    // Media (R14a section 3.1), by the lane that implements each.
    "dom_media_source_enabled",
    "dom_media_capabilities_enabled",
    "dom_eme_enabled",
    "dom_picture_in_picture_enabled",
    "dom_video_frame_callback_enabled",
    "dom_webcodecs_enabled",
    "dom_display_capture_enabled",
    "dom_audio_output_enabled",
    "dom_audioworklet_enabled",
    "dom_media_recorder_enabled",
    "dom_webrtc_peerconnection_enabled",
    "media_eme_clearkey_enabled",
    // Network and storage. `cookieStore` reads and writes the cookie jar directly, past the
    // third-party cookie rule and Global Privacy Control, until the network lane routes it.
    "network_cookie_samesite_lax_default",
    "dom_web_locks_enabled",
    "dom_cookiestore_enabled",
    // Interfaces that are stubs in Servo 0.8 stay hidden, so that feature detection does not
    // report what is not there, until their items land.
    "dom_serviceworker_enabled",
    "dom_web_animations_enabled",
    // The tree integrity checks walk the whole tree on every update: for Servo's own tests only.
    // (`accessibility_enabled` itself is on, in [`apply`]: the tree is still built only for the tab
    // whose page assistive technology reads, see `accessibility.rs`.)
    "expensive_accessibility_test_assertions_enabled",
];

/// Sets Mercury's engine preferences on `preferences`.
pub(crate) fn apply(preferences: &mut Preferences) {
    for name in PINNED_OFF {
        preferences.set_value(name, PrefValue::Bool(false));
    }
    // Global Privacy Control (owner decision O-18): on for every site. Nothing reads it before the
    // network lane's header and `navigator.globalPrivacyControl` (EC25), which then need no edit
    // of the Servo crates.
    preferences.privacy_gpc_enabled = true;
    // Autoplay (owner decision): media with sound waits for the user. The consumer is EA3.
    preferences.media_autoplay_policy = "block-audible".to_owned();
    preferences.media_autoplay_allow_origins.clear();
    preferences.media_autoplay_block_origins.clear();
    preferences.media_webrtc_ip_policy.clear();
    preferences.stylo_prefs = stylo_prefs_list();
    // Web compatibility (R14d ED1). Sites and frameworks feature-detect these, so each is on only
    // because it works:
    // - composition events: Servo already dispatches them; with the preference off,
    //   `'CompositionEvent' in window` is false and React's input-method path falls back;
    // - adopted style sheets: constructable style sheets in documents and shadow roots (Lit);
    // - the visual viewport: read-only `window.visualViewport`. A trial (decision D2) that stays only
    //   while `tests/pages/compat-features.html` shows sane values and no run crashes; the K0 window
    //   run showed 1000 x 658 @ scale 1 in a 1000 x 800 window with no crash, and the page checks the
    //   numbers against `innerWidth` and `innerHeight`. To turn it off, move its name to `PINNED_OFF`.
    // Container queries are off, in `runtime.rs` (`EXCLUDED_EXPERIMENTAL_PREFS`): Servo parses
    // `@container` blocks and drops them, and `@supports` would report support that is not there.
    preferences.dom_composition_event_enabled = true;
    preferences.dom_adoptedstylesheet_enabled = true;
    preferences.dom_visual_viewport_enabled = true;
    // Page accessibility (lane a11y-1): lets a web view build an accessibility tree when asked
    // (`WebView::set_accessibility_active`). Nothing is built until assistive technology attaches
    // to the window and the tab is the shown one: activation is lazy per document, and a document
    // that was never activated records no accessibility damage.
    preferences.accessibility_enabled = true;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn applied() -> Preferences {
        let mut preferences = Preferences::default();
        apply(&mut preferences);
        preferences
    }

    #[test]
    fn every_pinned_name_is_a_boolean_preference_that_stays_off() {
        let preferences = applied();
        for name in PINNED_OFF {
            assert!(
                Preferences::exists(name),
                "{name} is not a Servo preference"
            );
            assert_eq!(
                preferences.get_value(name),
                PrefValue::Bool(false),
                "{name}"
            );
        }
    }

    #[test]
    fn a_pin_wins_over_a_value_set_before() {
        let mut preferences = Preferences::default();
        for name in PINNED_OFF {
            preferences.set_value(name, PrefValue::Bool(true));
        }
        apply(&mut preferences);
        for name in PINNED_OFF {
            assert_eq!(
                preferences.get_value(name),
                PrefValue::Bool(false),
                "{name}"
            );
        }
    }

    #[test]
    fn product_values_are_set() {
        let preferences = applied();
        assert!(preferences.privacy_gpc_enabled);
        assert_eq!(preferences.media_autoplay_policy, "block-audible");
        assert_eq!(preferences.media_autoplay_allow_origins, "");
        assert_eq!(preferences.media_autoplay_block_origins, "");
        assert_eq!(preferences.media_webrtc_ip_policy, "");
    }

    #[test]
    fn the_web_compatibility_preferences_are_on() {
        let preferences = applied();
        assert!(preferences.dom_composition_event_enabled);
        assert!(preferences.dom_adoptedstylesheet_enabled);
        assert!(preferences.dom_visual_viewport_enabled);
    }

    #[test]
    fn page_accessibility_is_on_and_the_expensive_checks_stay_off() {
        let preferences = applied();
        assert!(preferences.accessibility_enabled);
        assert!(!preferences.expensive_accessibility_test_assertions_enabled);
        // A value set before does not matter: the product's value wins.
        let mut before = Preferences::default();
        before.set_value("accessibility_enabled", PrefValue::Bool(false));
        before.set_value(
            "expensive_accessibility_test_assertions_enabled",
            PrefValue::Bool(true),
        );
        apply(&mut before);
        assert!(before.accessibility_enabled);
        assert!(!before.expensive_accessibility_test_assertions_enabled);
        // It is not a pinned-off name any more, but the other one is.
        assert!(!PINNED_OFF.contains(&"accessibility_enabled"));
        assert!(PINNED_OFF.contains(&"expensive_accessibility_test_assertions_enabled"));
    }

    #[test]
    fn the_stylo_list_is_joined_into_the_preferences() {
        assert_eq!(
            applied().stylo_prefs,
            "layout.css.nth-child-of.enabled=true,layout.css.counters.enabled=true,\
             layout.css.text-overflow.enabled=true"
        );
    }

    #[test]
    fn every_stylo_name_exists_and_is_applied_by_servo() {
        let list = applied().stylo_prefs;

        let unknown = servo::prefs::apply_stylo_prefs(&list);

        assert!(unknown.is_empty(), "Stylo has no preference {unknown:?}");
        for (name, value) in STYLO_PREFS {
            assert_eq!(servo::prefs::stylo_bool_pref(name), Some(*value), "{name}");
        }
        assert_eq!(
            servo::prefs::stylo_bool_pref("layout.css.nth-child-of.enabled"),
            Some(true)
        );
        // ED5 counters and ED7 text-overflow (lane txt-1) are on, because they work.
        for name in [
            "layout.css.counters.enabled",
            "layout.css.text-overflow.enabled",
        ] {
            assert_eq!(servo::prefs::stylo_bool_pref(name), Some(true), "{name}");
        }
    }

    #[test]
    fn applying_twice_changes_nothing() {
        let once = applied();
        let mut twice = applied();
        apply(&mut twice);
        assert!(twice.diff(&once).is_empty());
    }

    #[test]
    fn the_media_and_network_preferences_of_the_first_two_waves_exist() {
        for name in [
            "dom_media_source_enabled",
            "dom_media_capabilities_enabled",
            "dom_eme_enabled",
            "dom_picture_in_picture_enabled",
            "dom_video_frame_callback_enabled",
            "dom_webcodecs_enabled",
            "dom_display_capture_enabled",
            "dom_audio_output_enabled",
            "dom_audioworklet_enabled",
            "dom_media_recorder_enabled",
            "dom_webrtc_peerconnection_enabled",
            "media_eme_clearkey_enabled",
            "media_autoplay_policy",
            "media_autoplay_allow_origins",
            "media_autoplay_block_origins",
            "media_webrtc_ip_policy",
            "privacy_gpc_enabled",
            "network_cookie_samesite_lax_default",
            "dom_web_locks_enabled",
            "stylo_prefs",
        ] {
            assert!(Preferences::exists(name), "{name}");
        }
    }

    #[test]
    fn the_string_preferences_default_to_servos_behaviour() {
        let defaults = Preferences::default();
        for name in [
            "media_autoplay_policy",
            "media_autoplay_allow_origins",
            "media_autoplay_block_origins",
            "media_webrtc_ip_policy",
            "stylo_prefs",
        ] {
            assert_eq!(
                defaults.get_value(name),
                PrefValue::Str(String::new()),
                "{name}"
            );
        }
    }
}
