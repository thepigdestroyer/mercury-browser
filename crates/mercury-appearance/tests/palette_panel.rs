// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! The palette picker's pure half (lane polish-1): one card per selectable palette in the data's
//! order, which card is chosen for what is stored, and what choosing a card writes. Nothing here
//! names a palette except the ones the stored values are about, so a palette added later is
//! covered by the loops over `selectable()`.

use mercury_appearance::palettes::{self, selectable};
use mercury_appearance::panels::layout_panel::{Choice, set_choice};
use mercury_appearance::panels::palette_panel::{PaletteSwatch, palette_panel, set_palette};
use mercury_appearance::{Role, Scheme};
use mercury_settings::{AppearanceSettings, DEFAULT_PALETTE, Settings};

fn stored(light: &str, dark: &str) -> AppearanceSettings {
    AppearanceSettings {
        palette_light: light.to_owned(),
        palette_dark: dark.to_owned(),
        ..AppearanceSettings::default()
    }
}

fn assert_survives_validation(appearance: &AppearanceSettings, what: &str) {
    let settings = Settings {
        appearance: appearance.clone(),
        ..Settings::default()
    };
    let (validated, repairs) = settings.validated();
    assert!(repairs.is_empty(), "{what}: repaired {repairs:?}");
    assert_eq!(&validated.appearance, appearance, "{what}");
}

fn default_index() -> usize {
    selectable()
        .position(|palette| palette.id == DEFAULT_PALETTE)
        .expect("the default palette is selectable")
}

#[test]
fn the_cards_are_the_selectable_palettes_in_order() {
    let panel = palette_panel(&AppearanceSettings::default());
    let expected: Vec<(&str, &str)> = selectable()
        .map(|palette| (palette.id, palette.name))
        .collect();
    let shown: Vec<(&str, &str)> = panel.cards.iter().map(|card| (card.id, card.name)).collect();
    assert_eq!(shown, expected);
    assert!(!panel.cards.is_empty());
}

#[test]
fn exactly_the_default_palette_says_default() {
    let panel = palette_panel(&AppearanceSettings::default());
    let flagged: Vec<&str> = panel
        .cards
        .iter()
        .filter(|card| card.is_default)
        .map(|card| card.id)
        .collect();
    assert_eq!(flagged, [palettes::default_palette().id]);
    assert_eq!(flagged, [DEFAULT_PALETTE]);
}

#[test]
fn each_drawing_has_the_colours_of_its_palette_in_its_scheme() {
    let panel = palette_panel(&AppearanceSettings::default());
    for (card, palette) in panel.cards.iter().zip(selectable()) {
        for (swatch, scheme) in [(card.light, Scheme::Light), (card.dark, Scheme::Dark)] {
            let colour = |role| palette.colour(scheme, role);
            assert_eq!(swatch.window, colour(Role::WindowBackground), "{}", card.id);
            assert_eq!(swatch.toolbar, colour(Role::ToolbarBackground));
            assert_eq!(swatch.field, colour(Role::FieldBackground));
            assert_eq!(swatch.content, colour(Role::ContentBackground));
            assert_eq!(swatch.text, colour(Role::Text));
            assert_eq!(swatch.accent, colour(Role::PrimaryButtonBackground));
            assert_eq!(swatch.on_accent, colour(Role::PrimaryButtonText));
            assert_eq!(swatch.tab, colour(Role::TabActive));
            assert_eq!(swatch.border, colour(Role::FieldBorder));
            assert_eq!(swatch, PaletteSwatch::of(palette, scheme));
        }
        assert_ne!(card.light, card.dark, "{}: one drawing per scheme", card.id);
    }
}

#[test]
fn the_defaults_select_the_default_palette_with_no_note() {
    let panel = palette_panel(&AppearanceSettings::default());
    assert_eq!(panel.selected, Some(default_index()));
    assert_eq!(panel.note, "");
}

#[test]
fn every_stored_selectable_id_selects_its_card() {
    for (index, palette) in selectable().enumerate() {
        let panel = palette_panel(&stored(palette.id, palette.id));
        assert_eq!(panel.selected, Some(index), "{}", palette.id);
        assert_eq!(panel.note, "");
    }
}

#[test]
fn an_id_this_version_cannot_show_counts_as_the_default_as_the_chrome_does() {
    for id in [
        "mercury-b2",
        "hermes",
        "graphite",
        "midnight",
        "a-later-palette",
    ] {
        let panel = palette_panel(&stored(id, id));
        assert_eq!(panel.selected, Some(default_index()), "{id}");
        assert_eq!(panel.note, "", "{id}");
    }
}

#[test]
fn a_hand_edited_pair_selects_nothing_and_the_note_names_both() {
    let names: Vec<&str> = selectable().map(|palette| palette.name).collect();
    if names.len() < 2 {
        return;
    }
    let ids: Vec<&str> = selectable().map(|palette| palette.id).collect();
    let panel = palette_panel(&stored(ids[0], ids[1]));
    assert_eq!(panel.selected, None);
    assert_eq!(
        panel.note,
        format!(
            "Light uses {} and dark uses {}. Choose a palette to use it in both.",
            names[0], names[1]
        )
    );
    // One field unknown: it shows as the default, so the pair is mixed unless that is the other.
    let mixed = palette_panel(&stored(ids[1], "a-later-palette"));
    if ids[1] == DEFAULT_PALETTE {
        assert_eq!(mixed.selected, Some(default_index()));
    } else {
        assert_eq!(mixed.selected, None);
        assert!(mixed.note.contains(names[1]));
    }
}

#[test]
fn choosing_a_card_writes_both_ids_and_the_panel_then_selects_it() {
    for (index, palette) in selectable().enumerate() {
        let mut settings = stored("a-later-palette", "hermes");
        assert!(set_palette(&mut settings, index));
        assert_eq!(settings.palette_light, palette.id);
        assert_eq!(settings.palette_dark, palette.id);
        assert_survives_validation(&settings, palette.id);
        assert_eq!(palette_panel(&settings).selected, Some(index));
    }
}

#[test]
fn the_choice_is_named_palette_and_goes_through_set_choice() {
    assert_eq!(Choice::Palette.name(), "palette");
    assert_eq!(Choice::from_name("palette"), Some(Choice::Palette));
    for (index, palette) in selectable().enumerate() {
        let mut settings = AppearanceSettings::default();
        assert!(set_choice(&mut settings, Choice::Palette, index));
        assert_eq!(settings.palette_light, palette.id);
        assert_eq!(settings.palette_dark, palette.id);
    }
}

#[test]
fn an_index_outside_the_list_changes_nothing() {
    let start = stored("a-later-palette", "hermes");
    for index in [selectable().count(), selectable().count() + 1, usize::MAX] {
        let mut settings = start.clone();
        assert!(!set_palette(&mut settings, index), "{index}");
        assert_eq!(settings, start, "{index}");
    }
}

#[test]
fn choosing_leaves_every_other_appearance_setting_alone() {
    let mut start = AppearanceSettings::default();
    start.tab_groups.dial = false;
    start.show_home_button = !start.show_home_button;
    for index in 0..selectable().count() {
        let mut settings = start.clone();
        assert!(set_palette(&mut settings, index));
        settings.palette_light.clone_from(&start.palette_light);
        settings.palette_dark.clone_from(&start.palette_dark);
        assert_eq!(settings, start);
    }
}
