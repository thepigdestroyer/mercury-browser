// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! The letter on a pinned tab's bead (lane polish-1): a role of its own, white in every palette,
//! that reads on the white-safe `Swatch::mark` of every group colour in both schemes. The mark
//! is a derived colour, not a role, so the contrast rules of `contrast::RULES` cannot hold the
//! pair; this file does, over every selectable palette.

use mercury_appearance::group_colour::{GroupColour, MIN_TEXT_CONTRAST, PRESETS, Swatch};
use mercury_appearance::palettes::{all, selectable};
use mercury_appearance::{Colour, Role, Scheme};

#[test]
fn the_role_is_named_for_the_slint_property() {
    assert_eq!(Role::PinnedBeadLetter.slint_name(), "pinned-bead-letter");
    assert_eq!(
        Role::from_slint_name("pinned-bead-letter"),
        Some(Role::PinnedBeadLetter)
    );
}

#[test]
fn every_palette_gives_the_letter_white_in_both_schemes() {
    for palette in all() {
        for scheme in Scheme::ALL {
            assert_eq!(
                palette.colour(scheme, Role::PinnedBeadLetter),
                Colour::WHITE,
                "{} {scheme:?}",
                palette.id
            );
        }
    }
}

#[test]
fn the_letter_reads_on_every_group_mark_in_every_selectable_palette_and_scheme() {
    let customs = [
        0x000000_u32,
        0xffffff,
        0xff0000,
        0x00ff00,
        0x0000ff,
        0xffff00,
    ];
    for palette in selectable() {
        for scheme in Scheme::ALL {
            let letter = palette.colour(scheme, Role::PinnedBeadLetter);
            let surface = palette.colour(scheme, Role::PopupBackground);
            let ink = palette.colour(scheme, Role::Text);
            let colours = PRESETS.iter().map(GroupColour::Preset).chain(
                customs
                    .iter()
                    .map(|hex| GroupColour::custom(Colour::hex(*hex))),
            );
            for colour in colours {
                let mark = Swatch::for_scheme(colour.base(), surface, ink, scheme).mark;
                let ratio = letter.contrast_ratio(mark);
                assert!(
                    ratio >= MIN_TEXT_CONTRAST,
                    "{} {scheme:?}: the letter {letter:?} on the {} mark {mark:?} is {ratio:.2}",
                    palette.id,
                    colour.id()
                );
            }
        }
    }
}
