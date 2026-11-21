// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! The stage layout of the three Orbit levels (SPEC §4.6, brief §4 and D12): where the sun, the
//! planets, the "New group" ring, the preview cards, the chips and the level-3 card sit for a window
//! size, and which of them a point is over. Pure functions of the rows, the machine's [`View`] and the
//! stage; positions come from `ring_geometry::orbit` (`planet_layout`, `preview_layout`).
//!
//! STUB in the scaffold commit: `new` keeps the stage and `hit` answers `Backdrop`; item 2 `layout`
//! of lane orbit-view writes the real ones and adds the cells (planets, previews, chips, card).

use super::{OrbitInput, Stage, Target, View};

/// Everything positioned for one frame.
#[derive(Clone, Debug, PartialEq)]
pub struct OrbitLayout {
    pub stage: Stage,
}

impl OrbitLayout {
    /// Lays out the level and focus in `view` for these rows on `stage`.
    pub fn new(stage: Stage, _input: &OrbitInput, _view: &View) -> Self {
        Self { stage }
    }

    /// What the stage point (`x`, `y`) is over, by Orbit's own geometry (halo circle, card rectangle,
    /// chip rectangle, sun, "New group" ring). A planet's segments are `ring_bands`' to resolve.
    pub fn hit(&self, _x: f32, _y: f32) -> Target {
        Target::Backdrop
    }
}
