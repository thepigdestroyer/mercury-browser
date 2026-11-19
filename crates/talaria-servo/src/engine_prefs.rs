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
    "dom_serviceworker_enabled",
    // Servo 0.8's Web Animations is incomplete.
    "dom_web_animations_enabled",
    // The accessibility tree is built for the active tab only, by its own lane.
    "accessibility_enabled",
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
