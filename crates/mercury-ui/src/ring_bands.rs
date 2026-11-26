// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! What the hub ring, the corner dial and the Orbit planets need to draw a ring of tab segments with
//! `RingBands` (`ui/groups/ring-bands.slint`, SPEC `design/ui-final/SPEC.md` §4.2): the data
//! builder, the persistent models, the flag placement, the drop target and the measured glyph
//! metrics. The geometry itself is `ring_geometry`; this module turns the browser's `GroupRow` and
//! `TabRow` lists into the parallel arrays the component takes.
//!
//! **The public items below are the pinned contract of lane ring-bands**
//! (`.superpowers/sdd/lane-ring-bands-brief.md` §3.2). Other lanes build on them; only additions are
//! allowed. The scaffold commit has the final signatures with stub bodies (marked `STUB`); the lane's
//! items fill them in.
//!
//! # Using it (the pattern of every host)
//!
//! 1. Own one [`RingBandsModels`] and set its `ModelRc`s on the host component once.
//! 2. On every change of rows, theme or size: [`RingBandsData::new`] and [`RingBandsModels::update`].
//!    The colours inside the labels are resolved when the data is built ([`RingColours`]).
//! 3. Set `on_hit_test` and `on_drop_target` on the host to call [`RingBandsData::hit`] and
//!    [`RingBandsData::drop_target`]; the host forwards the pure callbacks to its `RingBands`.
//! 4. Call [`metrics`] once and keep the result.
//!
//! # Rules the data follows
//!
//! - Pinned tabs get no segment (they are beads, drawn by the hub); groups left with no shown tab
//!   get no label band.
//! - `groups`, `tabs`, `segments`, `lifted_segments` and `segment_labels` are parallel, in ring order.

use slint::Color;

mod data;
mod drag;
#[cfg(any(test, feature = "test-support"))]
pub mod fixtures;
mod flags;
mod metrics;
mod models;

pub use data::RingBandsData;
pub use metrics::{MeasuredMetrics, metrics};
pub use models::RingBandsModels;

/// Colours that Rust resolves when it builds the labels and flags (the `ArcGlyph.colour` field is a
/// plain colour). The name colour of a label is the group's own `deep`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RingColours {
    /// The "⌥" and the key letter of a label at rest (the palette's `accent`).
    pub key: Color,
    /// The tab count of a label (the palette's `text-muted`).
    pub count: Color,
    /// Every glyph of a label while ⌥ is held (white on the solid band).
    pub held_text: Color,
    /// Alpha of the labels of the unselected groups while held (0.45: the same dimming as the bands).
    pub dimmed_alpha: f32,
}

impl Default for RingColours {
    /// The light B2 values (accent `#C2491A`, muted ink `#8D8499`), for tests and tools.
    fn default() -> Self {
        Self {
            key: Color::from_rgb_u8(0xC2, 0x49, 0x1A),
            count: Color::from_rgb_u8(0x8D, 0x84, 0x99),
            held_text: Color::from_rgb_u8(0xFF, 0xFF, 0xFF),
            dimmed_alpha: 0.45,
        }
    }
}

impl RingColours {
    /// Reads `accent` and `text-muted` from a window's live `ChromePalette` (so the current scheme
    /// and theme are used). `held_text` is white and `dimmed_alpha` 0.45.
    pub fn from_palette(palette: &crate::ChromePalette<'_>) -> Self {
        Self {
            key: palette.get_accent(),
            count: palette.get_text_muted(),
            ..Self::default()
        }
    }
}

/// What a ring is built for besides its rows and its `RingSpec`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RingOptions {
    /// ⌥ is held: key letters are large (`LabelStyle::Held`), digits mark the tabs of
    /// `selected_group`, state flags are hidden.
    pub held: bool,
    /// A `GroupRow.id`; with `held` its tabs get the digits 1 to 9.
    pub selected_group: Option<i32>,
    /// Diameter of a state flag in px (hub 20; Orbit passes less).
    pub flag_size: f32,
    pub colours: RingColours,
}

impl Default for RingOptions {
    fn default() -> Self {
        Self {
            held: false,
            selected_group: None,
            flag_size: 20.0,
            colours: RingColours::default(),
        }
    }
}
