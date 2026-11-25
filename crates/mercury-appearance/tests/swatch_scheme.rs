// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! The scheme-aware swatch (lane polish-1): `deep` is the colour to write a group's name in on the
//! chrome surface, in both schemes, and `mark` is the fill white reads on. Run over every
//! selectable palette on its own popup surface, so a palette added later is held to it too.

use mercury_appearance::group_colour::{GroupColour, MIN_TEXT_CONTRAST, PRESETS, Swatch};
use mercury_appearance::palettes::selectable;
use mercury_appearance::{Colour, Role, Scheme};

/// Colours a user might pick, with both ends and the primaries.
const CUSTOMS: [u32; 7] = [
    0x000000, 0xffffff, 0xff0000, 0x00ff00, 0x0000ff, 0xffff00, 0x3a7bd5,
];

/// Every colour a swatch is built for: the 14 presets and the custom ones.
fn bases() -> Vec<Colour> {
    PRESETS
        .iter()
        .map(|preset| preset.base)
        .chain(CUSTOMS.iter().map(|hex| Colour::hex(*hex)))
        .collect()
}

/// The surface and ink of the ring's popover in a palette and scheme.
fn popover(palette: &mercury_appearance::Palette, scheme: Scheme) -> (Colour, Colour) {
    (
        palette.colour(scheme, Role::PopupBackground),
        palette.colour(scheme, Role::Text),
    )
}

/// The first 1/255 step of `base` lightened toward white at which it reads on `surface` (the loop
/// the desktop's ring adapter used before this constructor existed).
fn stop_gap(base: Colour, surface: Colour) -> Colour {
    (0..=255_u32)
        .map(|step| base.lighter(f64::from(step) / 255.0))
        .find(|candidate| candidate.contrast_ratio(surface) >= MIN_TEXT_CONTRAST)
        .unwrap_or(Colour::WHITE)
}

#[test]
fn there_is_at_least_one_selectable_palette_to_hold_to_this() {
    assert!(selectable().count() >= 1);
}

#[test]
fn the_dark_deep_reads_on_the_popover_of_every_selectable_palette() {
    for palette in selectable() {
        let (surface, ink) = popover(palette, Scheme::Dark);
        for base in bases() {
            let swatch = Swatch::for_scheme(base, surface, ink, Scheme::Dark);
            assert!(
                swatch.deep.contrast_ratio(surface) >= MIN_TEXT_CONTRAST,
                "{} dark: deep {:?} of {:?} on {:?} is {:.2}",
                palette.id,
                swatch.deep,
                base,
                surface,
                swatch.deep.contrast_ratio(surface)
            );
        }
    }
}

#[test]
fn the_dark_deep_is_the_lightening_the_desktop_stop_gap_computed() {
    for palette in selectable() {
        let (surface, ink) = popover(palette, Scheme::Dark);
        for base in bases() {
            assert_eq!(
                Swatch::for_scheme(base, surface, ink, Scheme::Dark).deep,
                stop_gap(base, surface),
                "{} {:?}",
                palette.id,
                base
            );
        }
    }
}

#[test]
fn the_light_deep_is_the_old_deep_and_the_mark() {
    for palette in selectable() {
        let (surface, ink) = popover(palette, Scheme::Light);
        for base in bases() {
            let light = Swatch::for_scheme(base, surface, ink, Scheme::Light);
            assert_eq!(light, Swatch::new(base, surface, ink));
            assert_eq!(light.deep, light.mark);
            assert_eq!(light.on_deep, Colour::WHITE);
            assert!(light.deep.contrast_ratio(Colour::WHITE) >= MIN_TEXT_CONTRAST);
        }
    }
}

#[test]
fn white_reads_on_the_mark_and_on_deep_reads_on_deep_in_both_schemes() {
    for palette in selectable() {
        for scheme in Scheme::ALL {
            let (surface, ink) = popover(palette, scheme);
            for base in bases() {
                let swatch = Swatch::for_scheme(base, surface, ink, scheme);
                assert!(
                    Colour::WHITE.contrast_ratio(swatch.mark) >= MIN_TEXT_CONTRAST,
                    "{} {scheme:?} {base:?}: white on mark {:?}",
                    palette.id,
                    swatch.mark
                );
                assert!(
                    swatch.on_deep.contrast_ratio(swatch.deep) >= MIN_TEXT_CONTRAST,
                    "{} {scheme:?} {base:?}: {:?} on deep {:?}",
                    palette.id,
                    swatch.on_deep,
                    swatch.deep
                );
            }
        }
    }
}

#[test]
fn the_scheme_changes_deep_and_on_deep_and_nothing_else() {
    for palette in selectable() {
        let (surface, ink) = popover(palette, Scheme::Dark);
        for base in bases() {
            let light = Swatch::for_scheme(base, surface, ink, Scheme::Light);
            let dark = Swatch::for_scheme(base, surface, ink, Scheme::Dark);
            assert_eq!(light.colour, dark.colour);
            assert_eq!(light.tint, dark.tint);
            assert_eq!(light.mark, dark.mark);
        }
    }
}

#[test]
fn a_colour_that_already_reads_on_a_dark_surface_is_kept() {
    let surface = Colour::hex(0x101418);
    let ink = Colour::hex(0xf2f4f6);
    let pale = Colour::hex(0x9fd3ff);
    assert!(pale.contrast_ratio(surface) >= MIN_TEXT_CONTRAST);
    assert_eq!(
        Swatch::for_scheme(pale, surface, ink, Scheme::Dark).deep,
        pale
    );
}

#[test]
fn a_surface_nothing_reads_on_gives_white_and_does_not_panic() {
    let surface = Colour::WHITE;
    let pale = Colour::hex(0xdde6f2);
    assert!(pale.contrast_ratio(surface) < MIN_TEXT_CONTRAST);
    let swatch = Swatch::for_scheme(pale, surface, Colour::BLACK, Scheme::Dark);
    assert_eq!(swatch.deep, Colour::WHITE);
    assert_eq!(swatch.on_deep, Colour::BLACK);
}

#[test]
fn a_group_colour_builds_the_same_swatch_as_the_constructor() {
    let preset = GroupColour::parse("teal").expect("a preset");
    let surface = Colour::hex(0x151a1f);
    let ink = Colour::hex(0xeef1f4);
    assert_eq!(
        preset.swatch_for_scheme(surface, ink, Scheme::Dark),
        Swatch::for_scheme(preset.base(), surface, ink, Scheme::Dark)
    );
    assert_eq!(
        preset.swatch(surface, ink),
        Swatch::for_scheme(preset.base(), surface, ink, Scheme::Light)
    );
}
