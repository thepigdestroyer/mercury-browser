// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! The palette picker of the Appearance section, as plain data (lane polish-1).
//!
//! One card per [`palettes::selectable`] palette, in that order, with the colours of a small
//! drawing of the browser for the light and for the dark scheme. The list is read from the
//! palette data every time, so a palette added later appears with no change here. Choosing a card
//! is [`Choice::Palette`](super::layout_panel::Choice::Palette): it writes the id to both
//! `appearance.palette_light` and `appearance.palette_dark`.
//!
//! No Slint and no I/O.

use mercury_settings::AppearanceSettings;

use crate::colour::Colour;
use crate::palette::{Palette, Role, Scheme};
use crate::palettes;

/// The colours of a small drawing of the browser in one scheme of one palette: the frame, the
/// toolbar with its address field, the page with a line of text and a button, and a tab.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PaletteSwatch {
    /// `window-background`: the frame and the tab strip.
    pub window: Colour,
    /// `toolbar-background`.
    pub toolbar: Colour,
    /// `field-background`: the address field.
    pub field: Colour,
    /// `content-background`: the page.
    pub content: Colour,
    /// `text`: the lines of text.
    pub text: Colour,
    /// `primary-button-background`: the fill accent, which is also the Mercury mark.
    pub accent: Colour,
    /// `primary-button-text`: what is drawn on the accent.
    pub on_accent: Colour,
    /// `tab-active`: the selected tab.
    pub tab: Colour,
    /// `field-border`: the outlines.
    pub border: Colour,
}

impl PaletteSwatch {
    /// The drawing's colours for `scheme` of `palette`.
    pub fn of(palette: &Palette, scheme: Scheme) -> Self {
        let colour = |role| palette.colour(scheme, role);
        Self {
            window: colour(Role::WindowBackground),
            toolbar: colour(Role::ToolbarBackground),
            field: colour(Role::FieldBackground),
            content: colour(Role::ContentBackground),
            text: colour(Role::Text),
            accent: colour(Role::PrimaryButtonBackground),
            on_accent: colour(Role::PrimaryButtonText),
            tab: colour(Role::TabActive),
            border: colour(Role::FieldBorder),
        }
    }
}

/// One palette the user can choose.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PaletteCard {
    /// The id stored in `appearance.palette_light` and `palette_dark`.
    pub id: &'static str,
    /// The name shown on the card.
    pub name: &'static str,
    /// Whether this is the palette new profiles get (it says so on its card).
    pub is_default: bool,
    /// The drawing in the light scheme.
    pub light: PaletteSwatch,
    /// The drawing in the dark scheme.
    pub dark: PaletteSwatch,
}

/// What the picker shows. Plain fields, so the Slint side only copies them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PalettePanel {
    /// The cards, one per selectable palette, in picker order.
    pub cards: Vec<PaletteCard>,
    /// The chosen card: the one both schemes use. `None` when the two stored ids name different
    /// palettes (a hand-edited file).
    pub selected: Option<usize>,
    /// A line under the title: empty unless `selected` is `None`, when it names both palettes.
    pub note: String,
}

/// The card position of the palette the chrome shows for a stored `id`: the selectable palette
/// of that id, or the default one for any other id (an unknown palette of a later version, one
/// that cannot be chosen, the retired default), as the chrome does.
fn shown_index(id: &str) -> usize {
    let shown = palettes::find_selectable(id).unwrap_or_else(palettes::default_palette);
    palettes::selectable()
        .position(|palette| palette.id == shown.id)
        .unwrap_or_default()
}

/// What the picker shows for `settings`.
pub fn palette_panel(settings: &AppearanceSettings) -> PalettePanel {
    let default_id = palettes::default_palette().id;
    let cards: Vec<PaletteCard> = palettes::selectable()
        .map(|palette| PaletteCard {
            id: palette.id,
            name: palette.name,
            is_default: palette.id == default_id,
            light: PaletteSwatch::of(palette, Scheme::Light),
            dark: PaletteSwatch::of(palette, Scheme::Dark),
        })
        .collect();
    let light = shown_index(&settings.palette_light);
    let dark = shown_index(&settings.palette_dark);
    let (selected, note) = if light == dark {
        (Some(light), String::new())
    } else {
        let name = |index: usize| cards.get(index).map_or("", |card| card.name);
        (
            None,
            format!(
                "Light uses {} and dark uses {}. Choose a palette to use it in both.",
                name(light),
                name(dark)
            ),
        )
    };
    PalettePanel {
        cards,
        selected,
        note,
    }
}

/// Chooses the palette of card `index` for both schemes. `false`, and nothing changes, for an
/// index outside the list.
pub fn set_palette(settings: &mut AppearanceSettings, index: usize) -> bool {
    let Some(palette) = palettes::selectable().nth(index) else {
        return false;
    };
    palette.id.clone_into(&mut settings.palette_light);
    palette.id.clone_into(&mut settings.palette_dark);
    true
}
