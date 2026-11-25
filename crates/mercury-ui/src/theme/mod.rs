// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! The look of the browser's own chrome, pushed into Slint.
//!
//! Slint holds the chrome's colours in `ChromePalette` and its sizes in `ChromeMetrics`; both have
//! defaults that are the chrome as it always looked (the Hermes colours, the Roomy sizes), so a
//! window nobody pushes to is unchanged. [`apply_look`] sets them from a [`Look`]: two palettes (the
//! light table of one, the dark table of the other: `appearance.palette_light` and
//! `palette_dark`) and a sizes preset. The scheme itself (`ChromePalette.dark`) stays in Slint,
//! so a change of the system's colour scheme needs no push.
//!
//! The desktop app calls this once per window and on every settings change
//! (`settings::apply_chrome_theme`); repeating it changes nothing that did not change.

mod colours;
mod metrics;

use mercury_appearance::resolve::{AppearanceSettings, Repair};
use mercury_appearance::{Metrics, Palette, palettes, resolve};
use slint::{ComponentHandle, Global};

use crate::{ChromeMetrics, ChromePalette};

/// What the chrome looks like: the palette for each scheme and the sizes.
///
/// Build one with [`Look::from_appearance`] (the real look) or [`Look::new`]; the struct is
/// `non_exhaustive` so that a field added later (the density preset) breaks no other crate.
#[derive(Clone, Copy, Debug)]
#[non_exhaustive]
pub struct Look {
    /// The palette whose light table is used in the light scheme.
    pub palette_light: &'static Palette,
    /// The palette whose dark table is used in the dark scheme.
    pub palette_dark: &'static Palette,
    /// The sizes preset.
    pub metrics: Metrics,
}

impl Look {
    /// A look from its three parts.
    pub const fn new(
        palette_light: &'static Palette,
        palette_dark: &'static Palette,
        metrics: Metrics,
    ) -> Self {
        Self {
            palette_light,
            palette_dark,
            metrics,
        }
    }

    /// The look the stored appearance settings ask for, and the notes about anything that had to
    /// fall back (a palette id this version does not have). Nothing is written back.
    pub fn from_appearance(appearance: &AppearanceSettings) -> (Self, Vec<Repair>) {
        let (resolved, notes) = resolve(appearance);
        let look = Self {
            palette_light: resolved.palette_light,
            palette_dark: resolved.palette_dark,
            metrics: Metrics::ROOMY,
        };
        (look, notes)
    }

    /// Today's colours and sizes, the chrome as it looked before the look was data. It is the
    /// baseline of the theme-plumbing pixel gate and no user can choose it: use it in tests only, and
    /// note that a window nobody pushes to already shows these colours. **Delete it with the
    /// `hermes` palette** in the layout-switch lane, once that gate has passed.
    pub fn hermes_roomy() -> Self {
        let hermes = palettes::find("hermes").unwrap_or_else(palettes::default_palette);
        Self {
            palette_light: hermes,
            palette_dark: hermes,
            metrics: Metrics::ROOMY,
        }
    }
}

/// Gives `component`'s own copies of `ChromePalette` and `ChromeMetrics` the values of `look`.
pub fn apply_look<C>(component: &C, look: &Look)
where
    C: ComponentHandle,
    for<'a> ChromePalette<'a>: Global<'a, C>,
    for<'a> ChromeMetrics<'a>: Global<'a, C>,
{
    colours::push(component, look.palette_light, look.palette_dark);
    metrics::push(component, &look.metrics);
}
