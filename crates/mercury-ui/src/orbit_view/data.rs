// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! From the layout and the rows to what `OrbitView` draws (brief §6 item 2): [`OrbitData`] is plain
//! data, [`OrbitModels`] keeps persistent Slint models and fills the `OrbitPlanet` rows (one
//! `ring_bands::RingBandsModels` per planet), the previews, chips, card and the other singles.
//!
//! STUB in the scaffold commit: empty data and models with the final signatures; item 2 `layout`
//! of lane orbit-view writes the real ones.

use slint::{Image, ModelRc, VecModel};

use super::layout::OrbitLayout;
use super::{OrbitInput, Stage, View};
use crate::{
    OrbitCard, OrbitChip, OrbitDragView, OrbitNeighbour, OrbitNewGroup, OrbitPlanet, OrbitPopover,
    OrbitPreviewCard, OrbitSun,
};

/// What a build depends on besides the rows and the view.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Options {
    /// The colour scheme (the label colours of the rows are already resolved; this picks the
    /// glyph colours through `RingColours`).
    pub dark: bool,
    /// The diameter of a state flag on a planet's tab band (Orbit passes less than the hub's 20).
    pub flag_size: f32,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            dark: false,
            flag_size: 0.0,
        }
    }
}

/// The plain data of one frame.
#[derive(Clone, Debug, PartialEq)]
pub struct OrbitData {
    pub layout: OrbitLayout,
}

impl OrbitData {
    /// Builds the frame for `view`. `snapshots(tab)` gives a tab's picture (the glue caches them);
    /// `None` draws the placeholder.
    pub fn build(
        stage: Stage,
        input: &OrbitInput,
        view: &View,
        _snapshots: &dyn Fn(i32) -> Option<Image>,
        _options: &Options,
    ) -> Self {
        Self {
            layout: OrbitLayout::new(stage, input, view),
        }
    }
}

/// The persistent models of `OrbitView`: `update` keeps every `ModelRc` identity.
#[derive(Default)]
pub struct OrbitModels {
    planets: ModelRc<OrbitPlanet>,
    previews: ModelRc<OrbitPreviewCard>,
    chips: ModelRc<OrbitChip>,
    neighbours: ModelRc<OrbitNeighbour>,
    crumbs: ModelRc<slint::SharedString>,
}

impl OrbitModels {
    /// Empty models.
    pub fn new() -> Self {
        Self {
            planets: ModelRc::new(VecModel::default()),
            previews: ModelRc::new(VecModel::default()),
            chips: ModelRc::new(VecModel::default()),
            neighbours: ModelRc::new(VecModel::default()),
            crumbs: ModelRc::new(VecModel::default()),
        }
    }

    /// Fills the models from a frame.
    pub fn update(&self, _data: &OrbitData) {}

    pub fn planets(&self) -> ModelRc<OrbitPlanet> {
        self.planets.clone()
    }

    pub fn previews(&self) -> ModelRc<OrbitPreviewCard> {
        self.previews.clone()
    }

    pub fn chips(&self) -> ModelRc<OrbitChip> {
        self.chips.clone()
    }

    pub fn neighbours(&self) -> ModelRc<OrbitNeighbour> {
        self.neighbours.clone()
    }

    pub fn crumbs(&self) -> ModelRc<slint::SharedString> {
        self.crumbs.clone()
    }

    /// The single-valued rows of the last `update`.
    pub fn centre(&self) -> OrbitPlanet {
        OrbitPlanet::default()
    }

    pub fn has_centre(&self) -> bool {
        false
    }

    pub fn card(&self) -> OrbitCard {
        OrbitCard::default()
    }

    pub fn sun(&self) -> OrbitSun {
        OrbitSun::default()
    }

    pub fn new_group(&self) -> OrbitNewGroup {
        OrbitNewGroup::default()
    }

    pub fn drag(&self) -> OrbitDragView {
        OrbitDragView::default()
    }

    pub fn popover(&self) -> OrbitPopover {
        OrbitPopover::default()
    }

    pub fn more_previews(&self) -> i32 {
        0
    }
}
