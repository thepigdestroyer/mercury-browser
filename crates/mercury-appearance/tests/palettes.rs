// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! The role registry, the palette registry and the shape of the tables: every palette has every
//! role in both schemes, ids are unique and well formed, the default (`signal`) is selectable, only
//! Signal, Forest, Abyss and Glacier can be chosen, and `hermes`, `mercury-b2`, `graphite` and
//! `midnight` can be found but not chosen. Also the values the owner gave for `mercury-b2`, so
//! that nobody changes them by accident.

use std::collections::HashSet;

use mercury_appearance::Colour;
use mercury_appearance::palette::{Role, SUN_INK, SUN_STOPS, Scheme};
use mercury_appearance::palettes::{self, hermes, mercury_b2, signal};
use mercury_settings::{DEFAULT_PALETTE, MAX_PALETTE_ID_BYTES};

fn hex(rgb: u32) -> Colour {
    Colour::hex(rgb)
}

#[test]
fn there_is_a_role_for_every_property_of_the_chrome_palette() {
    // 55 colour properties in chrome-palette.slint; hermes_matches_chrome_palette.rs checks the
    // names against the file itself.
    assert_eq!(Role::COUNT, 55);
    assert_eq!(Role::ALL.len(), 55);
    for (index, role) in Role::ALL.into_iter().enumerate() {
        assert_eq!(
            role as usize, index,
            "{role:?} is out of order in Role::ALL"
        );
    }
}

#[test]
fn role_names_round_trip() {
    let mut seen = HashSet::new();
    for role in Role::ALL {
        let name = role.slint_name();
        assert!(seen.insert(name), "two roles are called {name:?}");
        assert_eq!(Role::from_slint_name(name), Some(role), "{name}");
        assert!(
            !name.is_empty()
                && name.split('-').all(|word| {
                    !word.is_empty() && word.bytes().all(|byte| byte.is_ascii_lowercase())
                }),
            "{name:?} is not a kebab-case property name"
        );
    }
    assert_eq!(Role::WindowBackground.slint_name(), "window-background");
    assert_eq!(
        Role::CaptureIndicatorGlyph.slint_name(),
        "capture-indicator-glyph"
    );
    assert_eq!(Role::PinnedBeadLetter.slint_name(), "pinned-bead-letter");
    for name in [
        "",
        "Window-Background",
        "window_background",
        " window-background",
        "window-background ",
        "dark",
        "theme",
        "nope",
    ] {
        assert_eq!(Role::from_slint_name(name), None, "{name:?}");
    }
}

#[test]
fn schemes_know_which_is_dark() {
    assert_eq!(Scheme::ALL, [Scheme::Light, Scheme::Dark]);
    assert!(!Scheme::Light.is_dark());
    assert!(Scheme::Dark.is_dark());
    assert_eq!(Scheme::Light.name(), "light");
    assert_eq!(Scheme::Dark.name(), "dark");
}

#[test]
fn every_palette_has_every_role_in_both_schemes() {
    assert!(!palettes::all().is_empty());
    for palette in palettes::all() {
        for scheme in Scheme::ALL {
            let roles: Vec<Role> = palette.colours(scheme).map(|(role, _)| role).collect();
            assert_eq!(
                roles,
                Role::ALL.to_vec(),
                "{} {}",
                palette.id,
                scheme.name()
            );
            for (role, colour) in palette.colours(scheme) {
                assert_eq!(palette.colour(scheme, role), colour);
            }
        }
    }
}

#[test]
fn only_the_scrim_and_the_shadow_are_translucent() {
    // Contrast is measured for opaque colours; the two roles that are veils are exempt from it
    // (contrast_rules.rs), and nothing else may quietly become translucent.
    for palette in palettes::all() {
        for scheme in Scheme::ALL {
            for (role, colour) in palette.colours(scheme) {
                let veil = matches!(role, Role::Scrim | Role::Shadow);
                assert_eq!(
                    colour.a != 255,
                    veil,
                    "{} {} {}: {}",
                    palette.id,
                    scheme.name(),
                    role.slint_name(),
                    colour.to_hex()
                );
            }
        }
    }
}

#[test]
fn light_and_dark_are_different_halves() {
    for palette in palettes::all() {
        for role in [
            Role::WindowBackground,
            Role::ToolbarBackground,
            Role::Text,
            Role::PopupBackground,
        ] {
            assert_ne!(
                palette.colour(Scheme::Light, role),
                palette.colour(Scheme::Dark, role),
                "{} {}",
                palette.id,
                role.slint_name()
            );
        }
        let window_light = palette.colour(Scheme::Light, Role::WindowBackground);
        let window_dark = palette.colour(Scheme::Dark, Role::WindowBackground);
        assert!(
            window_light.luminance() > window_dark.luminance(),
            "{}",
            palette.id
        );
    }
}

fn well_formed(id: &str) -> bool {
    let mut bytes = id.bytes();
    let first_ok = bytes
        .next()
        .is_some_and(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit());
    first_ok
        && id.len() <= MAX_PALETTE_ID_BYTES
        && bytes.all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
}

#[test]
fn ids_are_unique_and_well_formed() {
    let mut ids = HashSet::new();
    let mut names = HashSet::new();
    for palette in palettes::all() {
        assert!(
            well_formed(palette.id),
            "{:?} is not a well formed palette id (settings would repair it)",
            palette.id
        );
        assert!(
            ids.insert(palette.id),
            "two palettes have the id {:?}",
            palette.id
        );
        assert!(
            !palette.name.trim().is_empty() && names.insert(palette.name),
            "{:?} needs a distinct, non-empty name",
            palette.id
        );
    }
    // The checker itself.
    for good in ["mercury-b2", "a", "0", "a-b-c", "hermes"] {
        assert!(well_formed(good), "{good}");
    }
    for bad in [
        "",
        "-a",
        "A",
        "mercury_b2",
        "mercury b2",
        "é",
        &"a".repeat(MAX_PALETTE_ID_BYTES + 1),
    ] {
        assert!(!well_formed(bad), "{bad:?}");
    }
}

#[test]
fn the_default_palette_is_signal_and_can_be_chosen() {
    let default = palettes::default_palette();
    assert_eq!(default.id, DEFAULT_PALETTE);
    assert_eq!(default.id, "signal");
    assert_eq!(default.id, signal::ID);
    assert!(default.selectable);
    assert_eq!(palettes::all()[0].id, default.id, "the default comes first");
    assert_eq!(
        palettes::find(DEFAULT_PALETTE).map(|palette| palette.id),
        Some(default.id)
    );
    assert_eq!(mercury_b2::PALETTE.id, mercury_b2::ID);
}

#[test]
fn four_palettes_can_be_chosen_and_the_other_four_are_data() {
    // The owner's choice of 22 Nov 2026, in the order a picker lists them.
    let chosen: Vec<&str> = palettes::selectable().map(|palette| palette.id).collect();
    assert_eq!(chosen, ["signal", "forest", "abyss", "glacier"]);
    let names: Vec<&str> = palettes::selectable().map(|palette| palette.name).collect();
    assert_eq!(names, ["Signal", "Forest", "Abyss", "Glacier"]);
    for id in ["graphite", "midnight", "mercury-b2", "hermes"] {
        let palette = palettes::find(id).unwrap_or_else(|| panic!("{id} is still in the code"));
        assert!(!palette.selectable, "{id}");
        assert!(palettes::find_selectable(id).is_none(), "{id}");
    }
    assert_eq!(palettes::all().len(), 8);
}

#[test]
fn hermes_is_a_baseline_and_not_a_choice() {
    let legacy = palettes::find("hermes").expect("the pixel gate asks for hermes by id");
    assert_eq!(legacy.id, hermes::ID);
    assert!(!legacy.selectable);
    assert!(palettes::find_selectable("hermes").is_none());
    assert_ne!(palettes::default_palette().id, "hermes");
}

#[test]
fn find_matches_an_id_exactly() {
    assert!(palettes::find("mercury-b2").is_some());
    assert!(palettes::find_selectable("mercury-b2").is_none());
    for id in ["signal", "forest", "abyss", "glacier"] {
        assert!(palettes::find_selectable(id).is_some(), "{id}");
    }
    for id in [
        "",
        "Mercury-B2",
        "mercury-b2 ",
        " mercury-b2",
        "mercury",
        "Signal",
        "signal ",
        "paper",
        "hermes2",
    ] {
        assert!(palettes::find(id).is_none(), "{id:?}");
        assert!(palettes::find_selectable(id).is_none(), "{id:?}");
    }
}

#[test]
fn mercury_b2_keeps_the_values_the_owner_gave() {
    let b2 = &mercury_b2::PALETTE;
    // SPEC §1: light chrome, ink and fill accent; dark chrome, text and accent.
    let given = [
        (Scheme::Light, Role::ToolbarBackground, 0xf8f4ee),
        (Scheme::Light, Role::WindowBackground, 0xefe8df),
        (Scheme::Light, Role::Text, 0x1a1230),
        (Scheme::Light, Role::PrimaryButtonBackground, 0xc2491a),
        (Scheme::Light, Role::ItemSelected, 0xc2491a),
        (Scheme::Dark, Role::ToolbarBackground, 0x1c1534),
        (Scheme::Dark, Role::WindowBackground, 0x140f27),
        (Scheme::Dark, Role::Text, 0xf3eef8),
        (Scheme::Dark, Role::Accent, 0xffb547),
        (Scheme::Dark, Role::PrimaryButtonBackground, 0xffb547),
        // Read from the mockups in design/ui-final.
        (Scheme::Light, Role::TextSecondary, 0x5b5168),
        (Scheme::Light, Role::ToolbarDivider, 0xddd3c6),
        (Scheme::Dark, Role::TextSecondary, 0xb4aac6),
        (Scheme::Dark, Role::ToolbarDivider, 0x382e58),
        (Scheme::Dark, Role::ContentBackground, 0x110c21),
    ];
    for (scheme, role, rgb) in given {
        assert_eq!(
            b2.colour(scheme, role),
            hex(rgb),
            "{} {}",
            scheme.name(),
            role.slint_name()
        );
    }
    assert_eq!(b2.name, "Mercury");
}

#[test]
fn the_text_roles_step_down_in_strength() {
    // text > text-secondary > text-muted on the toolbar, and a placeholder is no stronger than
    // muted text on its field: a hierarchy the eye can read, not three names for one grey.
    let b2 = &mercury_b2::PALETTE;
    for scheme in Scheme::ALL {
        let on = |role: Role, surface: Role| {
            b2.colour(scheme, role)
                .contrast_ratio(b2.colour(scheme, surface))
        };
        let toolbar = Role::ToolbarBackground;
        assert!(on(Role::Text, toolbar) > on(Role::TextSecondary, toolbar) + 1.0);
        assert!(on(Role::TextSecondary, toolbar) > on(Role::TextMuted, toolbar));
        assert!(
            on(Role::TextMuted, Role::FieldBackground)
                >= on(Role::Placeholder, Role::FieldBackground)
        );
    }
}

#[test]
fn the_sun_is_the_brand_gradient_with_its_ink() {
    let stops: Vec<String> = SUN_STOPS.iter().map(|stop| stop.to_hex()).collect();
    assert_eq!(stops, ["#fff3c4", "#ffc857", "#ff8a2f", "#ec5b27"]);
    assert_eq!(SUN_INK.to_hex(), "#1e1233");
    // Pale core to deep rim: each stop is darker than the one before it.
    for pair in SUN_STOPS.windows(2) {
        assert!(pair[0].luminance() > pair[1].luminance(), "{pair:?}");
    }
    // The ink reads on every stop it can be drawn on.
    for stop in SUN_STOPS {
        let ratio = SUN_INK.contrast_ratio(stop);
        println!("sun ink on {}: {ratio:.2}", stop.to_hex());
        assert!(ratio >= 4.5, "{} {ratio}", stop.to_hex());
    }
}
