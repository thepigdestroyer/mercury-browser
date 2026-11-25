// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! The words on the "Switch group" card (lane brief §3 and §6 item 3), and the accessible labels
//! that say what a tile or a chip does. No Slint types.
//!
//! Every function is total. Names and titles pass through whole: the card never cuts a string
//! (Slint elides the text that does not fit, and shapes CJK, emoji and right-to-left scripts).

/// The shortest hold the setting allows, in milliseconds (SPEC §3.4).
const MIN_HOLD_MS: u16 = 250;
/// The longest hold the setting allows, in milliseconds.
const MAX_HOLD_MS: u16 = 1000;

/// Which capture mode the second hint explains (the caller maps `mercury_settings::KeyCapture`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HintMode {
    /// Outside text fields at once, inside them only after the hold.
    OutsideTextOrHold,
    /// Outside text fields only.
    OutsideText,
    /// Everywhere.
    Always,
}

/// The hold as the card says it: 400 gives "0.4 s", 250 "0.25 s", 1000 "1 s", 650 "0.65 s". The
/// value is clamped to 250..=1000 first; no trailing zero or dot.
pub fn hold_text(hold_ms: u16) -> String {
    let ms = u32::from(hold_ms.clamp(MIN_HOLD_MS, MAX_HOLD_MS));
    let (seconds, millis) = (ms / 1000, ms % 1000);
    if millis == 0 {
        return format!("{seconds} s");
    }
    let fraction = format!("{millis:03}");
    format!("{seconds}.{} s", fraction.trim_end_matches('0'))
}

/// "1 tab", "5 tabs", "0 tabs".
pub fn count_text(tabs: usize) -> String {
    if tabs == 1 {
        "1 tab".to_owned()
    } else {
        format!("{tabs} tabs")
    }
}

/// A tile's accessible label: "Option A, Research, 5 tabs". "Option" is the word VoiceOver users
/// know for the key. A missing key label or name leaves its part out.
pub fn tile_label(key_label: &str, name: &str, tabs: usize) -> String {
    let mut parts = Vec::with_capacity(3);
    if !key_label.is_empty() {
        parts.push(format!("Option {key_label}"));
    }
    if !name.is_empty() {
        parts.push(name.to_owned());
    }
    parts.push(count_text(tabs));
    parts.join(", ")
}

/// A tab chip's accessible label: "Tab 2, WebRender notes" (`digit` is 1-based). A tab without a
/// title is just "Tab 2".
pub fn chip_label(digit: usize, title: &str) -> String {
    if title.is_empty() {
        format!("Tab {digit}")
    } else {
        format!("Tab {digit}, {title}")
    }
}

/// The first hint, verbatim from the mockup: "Tap a letter for a group, then 1–9 for one of its
/// tabs. Release ⌥ to go there."
pub fn hint_primary() -> &'static str {
    "Tap a letter for a group, then 1–9 for one of its tabs. Release ⌥ to go there."
}

/// The second hint for a mode: the mockup's text with `hold_text` for `OutsideTextOrHold`, and
/// one sentence each for the other two (the hold plays no part in them).
pub fn hint_secondary(mode: HintMode, hold_ms: u16) -> String {
    match mode {
        HintMode::OutsideTextOrHold => format!(
            "Opens after ⌥ is held for {} on its own, so quick ⌥ + letter still types å, ß, ∂ in text fields.",
            hold_text(hold_ms)
        ),
        HintMode::Always => "⌥ + a group letter always switches groups in Mercury, so å, ß and ∂ cannot be typed here. Change this in Settings.".to_owned(),
        HintMode::OutsideText => "⌥ + a group letter switches groups outside text fields; in text fields ⌥ always types.".to_owned(),
    }
}

/// What the tile area says when no group has a key.
pub fn empty_text() -> &'static str {
    "No group has a key yet. Choose Change keys… to give each group a letter."
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hold_text_says_the_four_examples_of_the_brief() {
        assert_eq!(hold_text(400), "0.4 s");
        assert_eq!(hold_text(250), "0.25 s");
        assert_eq!(hold_text(1000), "1 s");
        assert_eq!(hold_text(650), "0.65 s");
    }

    #[test]
    fn hold_text_for_every_50_ms_step_never_ends_in_a_zero_or_a_dot() {
        for ms in (250..=1000).step_by(50) {
            let text = hold_text(ms);
            let number = text
                .strip_suffix(" s")
                .unwrap_or_else(|| panic!("{ms}: {text:?}"));
            assert!(
                !number.ends_with('0') || number == "0",
                "{ms}: trailing zero in {text:?}"
            );
            assert!(!number.ends_with('.'), "{ms}: trailing dot in {text:?}");
            assert_eq!(text, format!("{} s", f64::from(ms) / 1000.0), "{ms} ms");
        }
        let all: Vec<String> = (250..=1000).step_by(50).map(hold_text).collect();
        assert_eq!(
            all,
            [
                "0.25 s", "0.3 s", "0.35 s", "0.4 s", "0.45 s", "0.5 s", "0.55 s", "0.6 s",
                "0.65 s", "0.7 s", "0.75 s", "0.8 s", "0.85 s", "0.9 s", "0.95 s", "1 s",
            ]
        );
    }

    #[test]
    fn hold_text_is_right_for_every_millisecond() {
        for ms in 250..=1000u16 {
            let text = hold_text(ms);
            assert_eq!(text, format!("{} s", f64::from(ms) / 1000.0), "{ms} ms");
            assert!(!text.contains(".0 "), "{ms}: {text:?}");
        }
        assert_eq!(hold_text(255), "0.255 s");
        assert_eq!(hold_text(999), "0.999 s");
    }

    #[test]
    fn hold_text_clamps_to_the_range_of_the_setting() {
        for ms in [0, 1, 100, 249] {
            assert_eq!(hold_text(ms), "0.25 s", "{ms} ms");
        }
        for ms in [1001, 2000, 5000, u16::MAX] {
            assert_eq!(hold_text(ms), "1 s", "{ms} ms");
        }
    }

    #[test]
    fn counts_are_singular_for_one_and_plural_otherwise() {
        assert_eq!(count_text(0), "0 tabs");
        assert_eq!(count_text(1), "1 tab");
        assert_eq!(count_text(2), "2 tabs");
        assert_eq!(count_text(5), "5 tabs");
        assert_eq!(count_text(500), "500 tabs");
        assert_eq!(count_text(usize::MAX), format!("{} tabs", usize::MAX));
    }

    #[test]
    fn a_tile_label_says_option_key_name_and_count() {
        assert_eq!(tile_label("A", "Research", 5), "Option A, Research, 5 tabs");
        assert_eq!(
            tile_label("S", "Lisbon trip", 1),
            "Option S, Lisbon trip, 1 tab"
        );
        assert_eq!(tile_label("D", "Reading", 0), "Option D, Reading, 0 tabs");
    }

    #[test]
    fn a_tile_label_keeps_any_name_whole() {
        // Never cut by bytes: CJK, emoji and right-to-left names come through unchanged.
        for name in [
            "研究ノート",
            "Trip ✈️🌍",
            "מחקר",
            "بحث",
            &"x".repeat(64),
            &"é".repeat(64),
        ] {
            let label = tile_label("Q", name, 3);
            assert_eq!(label, format!("Option Q, {name}, 3 tabs"));
        }
    }

    #[test]
    fn a_tile_label_without_a_name_or_a_key_leaves_the_part_out() {
        assert_eq!(tile_label("A", "", 2), "Option A, 2 tabs");
        assert_eq!(tile_label("", "Research", 2), "Research, 2 tabs");
        assert_eq!(tile_label("", "", 1), "1 tab");
    }

    #[test]
    fn a_chip_label_says_the_digit_and_the_title() {
        assert_eq!(chip_label(2, "WebRender notes"), "Tab 2, WebRender notes");
        assert_eq!(chip_label(1, "Servo book"), "Tab 1, Servo book");
        assert_eq!(chip_label(9, "研究ノート"), "Tab 9, 研究ノート");
        assert_eq!(chip_label(3, ""), "Tab 3");
    }

    #[test]
    fn the_first_hint_is_the_mockups_text_verbatim() {
        assert_eq!(
            hint_primary(),
            "Tap a letter for a group, then 1\u{2013}9 for one of its tabs. Release \u{2325} to go there."
        );
        assert_eq!(
            hint_primary(),
            "Tap a letter for a group, then 1–9 for one of its tabs. Release ⌥ to go there."
        );
    }

    #[test]
    fn the_second_hint_has_one_text_per_mode() {
        assert_eq!(
            hint_secondary(HintMode::OutsideTextOrHold, 400),
            "Opens after ⌥ is held for 0.4 s on its own, so quick ⌥ + letter still types å, ß, ∂ in text fields."
        );
        assert_eq!(
            hint_secondary(HintMode::Always, 400),
            "⌥ + a group letter always switches groups in Mercury, so å, ß and ∂ cannot be typed here. Change this in Settings."
        );
        assert_eq!(
            hint_secondary(HintMode::OutsideText, 400),
            "⌥ + a group letter switches groups outside text fields; in text fields ⌥ always types."
        );
    }

    #[test]
    fn only_the_first_mode_mentions_the_hold_and_it_follows_the_setting() {
        assert!(hint_secondary(HintMode::OutsideTextOrHold, 250).contains("held for 0.25 s on"));
        assert!(hint_secondary(HintMode::OutsideTextOrHold, 1000).contains("held for 1 s on"));
        assert!(hint_secondary(HintMode::OutsideTextOrHold, 650).contains("held for 0.65 s on"));
        for ms in [250, 400, 1000] {
            assert_eq!(
                hint_secondary(HintMode::Always, ms),
                hint_secondary(HintMode::Always, 400)
            );
            assert_eq!(
                hint_secondary(HintMode::OutsideText, ms),
                hint_secondary(HintMode::OutsideText, 400)
            );
        }
    }

    #[test]
    fn the_empty_text_points_at_change_keys() {
        assert_eq!(
            empty_text(),
            "No group has a key yet. Choose Change keys… to give each group a letter."
        );
    }
}
