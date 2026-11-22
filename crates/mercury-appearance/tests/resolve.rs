// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! Resolving the stored appearance settings: the layout, the palettes in use, and the tab-group
//! policy, with a note (and no rewrite) for a palette this version does not have.

use std::time::Duration;

use mercury_appearance::keymap::{LetterKey, parse_sequence};
use mercury_appearance::palette::{Role, Scheme};
use mercury_appearance::palettes::{self, hermes, mercury_b2, signal};
use mercury_appearance::resolve::{AppearanceSettings, Layout, Repair, Resolved, resolve};
use mercury_settings::{
    DEFAULT_KEY_SEQUENCE, DEFAULT_PALETTE, DialCorner, DockStart, KeyCapture, LAYOUT_IDS, Settings,
};

/// Settings with this much changed from the defaults.
fn settings(change: impl FnOnce(&mut AppearanceSettings)) -> AppearanceSettings {
    let mut appearance = AppearanceSettings::default();
    change(&mut appearance);
    appearance
}

/// The `field` of each repair, in order.
fn fields(repairs: &[Repair]) -> Vec<&'static str> {
    repairs.iter().map(|repair| repair.field).collect()
}

/// Resolves and asserts that nothing was reported.
fn clean(appearance: &AppearanceSettings) -> Resolved {
    let (resolved, repairs) = resolve(appearance);
    assert!(repairs.is_empty(), "unexpected notes: {repairs:?}");
    resolved
}

/// Sequence letters as a string of capitals.
fn letters(sequence: &[LetterKey]) -> String {
    sequence.iter().map(|key| key.letter()).collect()
}

// --- Layout ---

#[test]
fn layout_ids_round_trip_and_agree_with_the_stored_list() {
    let ids: Vec<&str> = Layout::ALL.iter().map(|layout| layout.id()).collect();
    assert_eq!(
        ids, LAYOUT_IDS,
        "Layout and LAYOUT_IDS must list the same layouts in order"
    );
    for layout in Layout::ALL {
        assert_eq!(Layout::from_id(layout.id()), Some(layout));
    }
    assert_eq!(Layout::default(), Layout::Mercury);
}

#[test]
fn only_the_exact_ids_are_layouts() {
    for id in [
        "",
        "Mercury",
        "CLASSIC",
        " mercury",
        "mercury ",
        "sidebar",
        "compact",
        "mercury\0",
        "é",
    ] {
        assert_eq!(Layout::from_id(id), None, "{id:?}");
    }
}

// --- The defaults ---

#[test]
fn the_defaults_resolve_to_signal_with_the_dock_and_the_dial() {
    let resolved = clean(&AppearanceSettings::default());
    assert_eq!(resolved.layout, Layout::Mercury);
    assert_eq!(resolved.palette_light.id, signal::ID);
    assert_eq!(resolved.palette_dark.id, signal::ID);
    assert_eq!(resolved.palette_light.id, DEFAULT_PALETTE);
    assert!(resolved.dock_visible());
    let groups = &resolved.tab_groups;
    assert!(groups.dock);
    assert!(!groups.dock_in_classic);
    assert_eq!(groups.dock_starts, DockStart::Open);
    assert!(groups.ring);
    assert!(groups.orbit);
    assert!(groups.orbit_pinch);
    assert!(groups.dial);
    assert_eq!(groups.dial_corner, DialCorner::BottomRight);
    assert_eq!(groups.key_hold, Duration::from_millis(400));
    assert_eq!(groups.snapshot_cap_bytes, 128 * 1024 * 1024);
    let keys = &groups.keys;
    assert!(keys.enabled);
    assert_eq!(keys.capture, KeyCapture::OutsideTextOrHold);
    assert!(keys.show_beside_names);
    assert_eq!(letters(&keys.default_sequence), "ASDFGHJKL");
    assert_eq!(letters(&keys.default_sequence), DEFAULT_KEY_SEQUENCE);
}

#[test]
fn the_default_settings_file_resolves_without_notes_and_its_colours_are_the_signal_ones() {
    let (settings, repairs) = Settings::default().validated();
    assert!(repairs.is_empty());
    let resolved = clean(&settings.appearance);
    for scheme in Scheme::ALL {
        for role in Role::ALL {
            assert_eq!(
                resolved.colour(scheme, role),
                palettes::default_palette().colour(scheme, role),
                "{scheme:?} {role:?}"
            );
        }
    }
}

// --- The dock ---

#[test]
fn the_mercury_layout_follows_dock_and_the_classic_layout_follows_dock_in_classic() {
    for layout in Layout::ALL {
        for dock in [false, true] {
            for dock_in_classic in [false, true] {
                let appearance = settings(|a| {
                    a.layout = layout.id().to_owned();
                    a.tab_groups.dock = dock;
                    a.tab_groups.dock_in_classic = dock_in_classic;
                });
                let resolved = clean(&appearance);
                assert_eq!(resolved.layout, layout);
                let expected = match layout {
                    Layout::Mercury => dock,
                    Layout::Classic => dock_in_classic,
                };
                assert_eq!(
                    resolved.dock_visible(),
                    expected,
                    "{layout:?} dock {dock} dock_in_classic {dock_in_classic}"
                );
                assert_eq!(resolved.tab_groups.dock_visible(layout), expected);
                // The other layout's answer is independent of this layout's choice.
                let other = if layout == Layout::Mercury {
                    Layout::Classic
                } else {
                    Layout::Mercury
                };
                let other_expected = match other {
                    Layout::Mercury => dock,
                    Layout::Classic => dock_in_classic,
                };
                assert_eq!(resolved.tab_groups.dock_visible(other), other_expected);
            }
        }
    }
}

#[test]
fn classic_hides_the_dock_by_default_and_shows_it_when_asked() {
    let classic = settings(|a| a.layout = "classic".to_owned());
    assert!(!clean(&classic).dock_visible());
    let with_dock = settings(|a| {
        a.layout = "classic".to_owned();
        a.tab_groups.dock_in_classic = true;
    });
    assert!(clean(&with_dock).dock_visible());
}

#[test]
fn turning_the_mercury_dock_off_does_not_touch_classic() {
    let appearance = settings(|a| {
        a.tab_groups.dock = false;
        a.tab_groups.dock_in_classic = true;
    });
    let resolved = clean(&appearance);
    assert!(!resolved.dock_visible());
    assert!(resolved.tab_groups.dock_visible(Layout::Classic));
}

// --- Every non-default value arrives ---

#[test]
fn every_tab_group_setting_arrives_in_the_policy() {
    let appearance = settings(|a| {
        let groups = &mut a.tab_groups;
        groups.dock = false;
        groups.dock_in_classic = true;
        groups.dock_starts = DockStart::Shrunk;
        groups.ring = false;
        groups.orbit = false;
        groups.orbit_pinch = false;
        groups.dial = false;
        groups.dial_corner = DialCorner::TopLeft;
        groups.snapshot_cap_mb = 512;
        groups.keys.enabled = false;
        groups.keys.capture = KeyCapture::Always;
        groups.keys.hold_ms = 750;
        groups.keys.show_beside_names = false;
        groups.keys.default_sequence = "QWERTY".to_owned();
    });
    let policy = clean(&appearance).tab_groups;
    assert!(!policy.dock);
    assert!(policy.dock_in_classic);
    assert_eq!(policy.dock_starts, DockStart::Shrunk);
    assert!(!policy.ring);
    assert!(!policy.orbit);
    assert!(!policy.orbit_pinch);
    assert!(!policy.dial);
    assert_eq!(policy.dial_corner, DialCorner::TopLeft);
    assert_eq!(policy.snapshot_cap_bytes, 512 * 1024 * 1024);
    assert_eq!(policy.key_hold, Duration::from_millis(750));
    assert!(!policy.keys.enabled);
    assert_eq!(policy.keys.capture, KeyCapture::Always);
    assert!(!policy.keys.show_beside_names);
    assert_eq!(letters(&policy.keys.default_sequence), "QWERTY");
}

#[test]
fn the_edges_of_the_ranges_convert_exactly() {
    for (mb, bytes) in [(32_u16, 33_554_432_u64), (1024, 1_073_741_824)] {
        let appearance = settings(|a| a.tab_groups.snapshot_cap_mb = mb);
        assert_eq!(clean(&appearance).tab_groups.snapshot_cap_bytes, bytes);
    }
    for ms in [250_u16, 1000] {
        let appearance = settings(|a| a.tab_groups.keys.hold_ms = ms);
        assert_eq!(
            clean(&appearance).tab_groups.key_hold,
            Duration::from_millis(u64::from(ms))
        );
    }
}

#[test]
fn an_empty_sequence_gives_new_groups_no_key_and_is_not_a_note() {
    let appearance = settings(|a| a.tab_groups.keys.default_sequence.clear());
    assert!(
        clean(&appearance)
            .tab_groups
            .keys
            .default_sequence
            .is_empty()
    );
}

#[test]
fn the_sequence_is_parsed_the_same_way_as_the_key_map_parses_it() {
    let appearance = settings(|a| a.tab_groups.keys.default_sequence = "ZXCVBNM".to_owned());
    let resolved = clean(&appearance);
    assert_eq!(
        resolved.tab_groups.keys.default_sequence,
        parse_sequence("ZXCVBNM")
    );
}

// --- Palettes (D8, D9, D2) ---

#[test]
fn an_unknown_palette_falls_back_with_a_note_and_the_stored_value_is_kept() {
    let appearance = settings(|a| a.palette_light = "tangerine".to_owned());
    let before = appearance.clone();
    let (resolved, repairs) = resolve(&appearance);
    assert_eq!(appearance, before, "resolve must not change what is stored");
    assert_eq!(appearance.palette_light, "tangerine");
    assert_eq!(resolved.palette_light.id, DEFAULT_PALETTE);
    assert_eq!(resolved.palette_dark.id, DEFAULT_PALETTE);
    assert_eq!(fields(&repairs), ["appearance.palette_light"]);
    let reason = &repairs[0].reason;
    assert!(reason.contains("\"tangerine\""), "{reason}");
    assert!(
        reason.contains("not a palette this version has"),
        "{reason}"
    );
    assert!(reason.contains("\"signal\""), "{reason}");
    assert!(reason.contains("kept"), "{reason}");
}

#[test]
fn light_and_dark_are_resolved_independently_and_in_that_order() {
    let only_dark = settings(|a| a.palette_dark = "paper".to_owned());
    let (_, repairs) = resolve(&only_dark);
    assert_eq!(fields(&repairs), ["appearance.palette_dark"]);

    let both = settings(|a| {
        a.palette_light = "ink".to_owned();
        a.palette_dark = "paper".to_owned();
    });
    let (resolved, repairs) = resolve(&both);
    assert_eq!(
        fields(&repairs),
        ["appearance.palette_light", "appearance.palette_dark"]
    );
    assert_eq!(resolved.palette_light.id, DEFAULT_PALETTE);
    assert_eq!(resolved.palette_dark.id, DEFAULT_PALETTE);
}

#[test]
fn a_palette_that_cannot_be_chosen_is_not_used_even_when_the_file_names_it() {
    let appearance = settings(|a| {
        a.palette_light = hermes::ID.to_owned();
        a.palette_dark = hermes::ID.to_owned();
    });
    let (resolved, repairs) = resolve(&appearance);
    assert_eq!(resolved.palette_light.id, DEFAULT_PALETTE);
    assert_eq!(resolved.palette_dark.id, DEFAULT_PALETTE);
    assert_eq!(
        fields(&repairs),
        ["appearance.palette_light", "appearance.palette_dark"]
    );
    assert!(
        repairs[0].reason.contains("cannot be chosen"),
        "{}",
        repairs[0].reason
    );
    assert_eq!(
        appearance.palette_light, "hermes",
        "the stored value is kept"
    );
}

#[test]
fn the_palettes_that_were_not_chosen_fall_back_to_signal_with_one_note_each() {
    for id in ["mercury-b2", "graphite", "midnight", "hermes"] {
        let appearance = settings(|a| a.palette_dark = id.to_owned());
        let (resolved, repairs) = resolve(&appearance);
        assert_eq!(resolved.palette_light.id, "signal", "{id}");
        assert_eq!(resolved.palette_dark.id, "signal", "{id}");
        assert_eq!(fields(&repairs), ["appearance.palette_dark"], "{id}");
        assert!(
            repairs[0].reason.contains("cannot be chosen"),
            "{id}: {}",
            repairs[0].reason
        );
    }
}

#[test]
fn two_chosen_palettes_are_used_one_per_scheme() {
    let appearance = settings(|a| {
        a.palette_light = "forest".to_owned();
        a.palette_dark = "glacier".to_owned();
    });
    let resolved = clean(&appearance);
    assert_eq!(resolved.palette(Scheme::Light).id, "forest");
    assert_eq!(resolved.palette(Scheme::Dark).id, "glacier");
}

#[test]
fn every_selectable_palette_resolves_to_itself() {
    for palette in palettes::selectable() {
        let appearance = settings(|a| {
            a.palette_light = palette.id.to_owned();
            a.palette_dark = palette.id.to_owned();
        });
        let resolved = clean(&appearance);
        assert_eq!(resolved.palette_light.id, palette.id);
        assert_eq!(resolved.palette_dark.id, palette.id);
    }
}

#[test]
fn the_light_scheme_draws_the_light_palette_and_the_dark_scheme_the_dark_one() {
    // Only one palette can be chosen today, so the two are built by hand: the light slot holds
    // the legacy palette and the dark slot B2, and each scheme must read its own slot's table.
    let resolved = Resolved {
        palette_light: &hermes::PALETTE,
        palette_dark: &mercury_b2::PALETTE,
        ..clean(&AppearanceSettings::default())
    };
    assert_eq!(resolved.palette(Scheme::Light).id, hermes::ID);
    assert_eq!(resolved.palette(Scheme::Dark).id, mercury_b2::ID);
    let mut differing = 0;
    for role in Role::ALL {
        assert_eq!(
            resolved.colour(Scheme::Light, role),
            hermes::PALETTE.colour(Scheme::Light, role)
        );
        assert_eq!(
            resolved.colour(Scheme::Dark, role),
            mercury_b2::PALETTE.colour(Scheme::Dark, role)
        );
        if hermes::PALETTE.colour(Scheme::Light, role)
            != mercury_b2::PALETTE.colour(Scheme::Light, role)
        {
            differing += 1;
        }
    }
    assert!(
        differing > 10,
        "the test needs palettes that differ ({differing} roles do)"
    );
}

// --- Out-of-range and hostile values (resolve is total) ---

#[test]
fn a_repaired_layout_is_already_clean() {
    // Loading repairs an unknown layout to "mercury"; resolving what a load returns is silent.
    let mut file = Settings::default();
    file.appearance.layout = "sidebar".to_owned();
    let (loaded, repairs) = file.validated();
    assert_eq!(fields(&repairs), ["appearance.layout"]);
    assert_eq!(loaded.appearance.layout, "mercury");
    let resolved = clean(&loaded.appearance);
    assert_eq!(resolved.layout, Layout::Mercury);
}

#[test]
fn an_unvalidated_layout_resolves_to_mercury_with_one_repair() {
    for bad in ["sidebar", "Mercury", "", " classic"] {
        let appearance = settings(|a| a.layout = bad.to_owned());
        let (resolved, repairs) = resolve(&appearance);
        assert_eq!(resolved.layout, Layout::Mercury, "{bad:?}");
        assert_eq!(fields(&repairs), ["appearance.layout"], "{bad:?}");
    }
}

#[test]
fn out_of_range_numbers_resolve_to_the_defaults_with_one_repair_each_in_field_order() {
    let appearance = settings(|a| {
        a.tab_groups.snapshot_cap_mb = 31;
        a.tab_groups.keys.hold_ms = 5000;
    });
    let (resolved, repairs) = resolve(&appearance);
    assert_eq!(resolved.tab_groups.snapshot_cap_bytes, 128 * 1024 * 1024);
    assert_eq!(resolved.tab_groups.key_hold, Duration::from_millis(400));
    assert_eq!(
        fields(&repairs),
        [
            "appearance.tab_groups.snapshot_cap_mb",
            "appearance.tab_groups.keys.hold_ms"
        ]
    );
}

#[test]
fn a_malformed_palette_id_is_repaired_once_and_is_not_also_a_note() {
    let appearance = settings(|a| a.palette_light = "Not Valid".to_owned());
    let (resolved, repairs) = resolve(&appearance);
    assert_eq!(resolved.palette_light.id, DEFAULT_PALETTE);
    assert_eq!(fields(&repairs), ["appearance.palette_light"]);
    assert!(
        repairs[0].reason.contains("well formed"),
        "{}",
        repairs[0].reason
    );
}

#[test]
fn a_sequence_with_junk_resolves_to_its_letters() {
    let appearance = settings(|a| a.tab_groups.keys.default_sequence = "a1s ås".to_owned());
    let (resolved, repairs) = resolve(&appearance);
    assert_eq!(letters(&resolved.tab_groups.keys.default_sequence), "AS");
    assert_eq!(
        fields(&repairs),
        ["appearance.tab_groups.keys.default_sequence"]
    );
}

#[test]
fn hostile_values_give_short_notes_and_a_usable_result() {
    let huge = "x".repeat(100_000);
    let appearance = settings(|a| {
        a.layout = huge.clone();
        a.palette_light = huge.clone();
        a.palette_dark = format!("{huge}-");
        a.tab_groups.snapshot_cap_mb = u16::MAX;
        a.tab_groups.keys.hold_ms = u16::MAX;
        a.tab_groups.keys.default_sequence = huge.clone();
    });
    let (resolved, repairs) = resolve(&appearance);
    assert_eq!(resolved.layout, Layout::Mercury);
    assert_eq!(resolved.palette_light.id, DEFAULT_PALETTE);
    assert_eq!(resolved.palette_dark.id, DEFAULT_PALETTE);
    assert_eq!(repairs.len(), 6);
    for repair in &repairs {
        assert!(
            repair.reason.len() < 400,
            "{}: {} bytes",
            repair.field,
            repair.reason.len()
        );
    }
    assert_eq!(appearance.layout.len(), 100_000, "the input is not touched");
}

#[test]
fn a_well_formed_unknown_id_at_the_length_limit_is_a_short_note() {
    let id = format!("a{}", "-".repeat(63));
    assert_eq!(id.len(), 64);
    let appearance = settings(|a| a.palette_dark = id.clone());
    let (resolved, repairs) = resolve(&appearance);
    assert_eq!(resolved.palette_dark.id, DEFAULT_PALETTE);
    assert_eq!(fields(&repairs), ["appearance.palette_dark"]);
    assert!(repairs[0].reason.contains(&id));
    assert_eq!(appearance.palette_dark, id);
}

#[test]
fn resolving_is_repeatable_and_resolving_the_repaired_settings_adds_nothing() {
    let appearance = settings(|a| {
        a.layout = "x".to_owned();
        a.palette_light = "future".to_owned();
        a.tab_groups.keys.hold_ms = 1;
    });
    let (first, first_notes) = resolve(&appearance);
    let (second, second_notes) = resolve(&appearance);
    assert_eq!(first.layout, second.layout);
    assert_eq!(first.tab_groups, second.tab_groups);
    assert_eq!(first_notes, second_notes);

    // What a load would store: the stored-value repairs applied, the unknown palette kept.
    let file = Settings {
        appearance,
        ..Settings::default()
    };
    let (stored, _) = file.validated();
    assert_eq!(stored.appearance.palette_light, "future");
    let (_, notes) = resolve(&stored.appearance);
    assert_eq!(
        fields(&notes),
        ["appearance.palette_light"],
        "only the palette note remains"
    );
}
