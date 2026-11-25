// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! The colours of the app icon (the Dock tile) for a palette: the Sun in the palette's accent, on
//! a dark plate that belongs to the palette.
//!
//! The icon artwork (`apps/mercury-desktop/resources/macos/AppIcon.svg`) has four sun stops, a
//! halo, a two-stop plate and the ink of the M, all in the brand's orange and violet.
//! [`AppIconColours::new`] derives a replacement for each from a palette and a scheme;
//! `mercury-iconset` writes them into the artwork and the desktop app sets the picture as the
//! Dock icon while it runs. Pure data: no drawing here.
//!
//! **Why a dark plate in every scheme.** The Dock tile is a free-standing picture on the user's
//! wallpaper, not window chrome, so it does not become light with the light scheme: a light
//! plate would lose the plate's edge on a bright wallpaper and put the pale core of the sun on a
//! pale ground. The plate is the palette's own dark frame (`window-background` at the bottom,
//! `toolbar-background` at the top, from its *dark* table), so it still belongs to the theme
//! (Signal's is black, Forest's a forest floor, Abyss's a trench, Glacier's a polar night). What
//! the scheme changes is the sun: the fill accent of the scheme being shown, and when that is too
//! dim on the dark plate (a deep light-scheme accent) the dark scheme's accent instead.

use crate::colour::Colour;
use crate::palette::{Palette, Role, Scheme};

/// The least contrast the sun's middle colour keeps against the plate, as for a graphic.
pub const SUN_ON_PLATE_MINIMUM: f64 = 3.0;

/// The least contrast the M keeps against the inner stops of the sun (where it is drawn).
pub const INK_ON_SUN_MINIMUM: f64 = 3.0;

/// What replaces each colour of the icon artwork.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AppIconColours {
    /// The four stops of the sun, from the pale core to the rim.
    pub sun: [Colour; 4],
    /// The glow around the sun (drawn with the artwork's own opacity).
    pub halo: Colour,
    /// The top of the plate (the lighter end).
    pub plate_top: Colour,
    /// The bottom of the plate (the darker end).
    pub plate_bottom: Colour,
    /// The M drawn across the sun.
    pub ink: Colour,
}

impl AppIconColours {
    /// The icon colours for `palette` while `scheme` is shown.
    pub fn new(palette: &Palette, scheme: Scheme) -> Self {
        let plate_top = palette.colour(Scheme::Dark, Role::ToolbarBackground);
        let plate_bottom = palette.colour(Scheme::Dark, Role::WindowBackground);

        let mut accent = palette.colour(scheme, Role::PrimaryButtonBackground);
        if accent.contrast_ratio(plate_top) < SUN_ON_PLATE_MINIMUM {
            accent = palette.colour(Scheme::Dark, Role::PrimaryButtonBackground);
        }
        // The pale core, a light middle, the accent itself and a deeper rim, as the brand's
        // sun runs from cream to deep orange.
        let sun = [
            accent.lighter(0.75),
            accent.lighter(0.38),
            accent,
            accent.darker(0.12),
        ];
        let halo = accent.lighter(0.2);

        // The M is drawn across the core and the middle of the sun: the darker of a deep
        // shade of the accent and white that reads on the three inner stops.
        let deep = accent.darker(0.88);
        let weakest = |ink: Colour| {
            sun[..3]
                .iter()
                .map(|stop| ink.contrast_ratio(*stop))
                .fold(f64::MAX, f64::min)
        };
        let ink = if weakest(deep) >= weakest(Colour::WHITE) {
            deep
        } else {
            Colour::WHITE
        };

        Self {
            sun,
            halo,
            plate_top,
            plate_bottom,
            ink,
        }
    }

    /// The contrast of the M against the weakest of the three inner stops of the sun.
    pub fn ink_contrast(&self) -> f64 {
        self.sun[..3]
            .iter()
            .map(|stop| self.ink.contrast_ratio(*stop))
            .fold(f64::MAX, f64::min)
    }

    /// The contrast of the sun's middle colour against the lighter end of the plate.
    pub fn sun_on_plate_contrast(&self) -> f64 {
        self.sun[2].contrast_ratio(self.plate_top)
    }
}
