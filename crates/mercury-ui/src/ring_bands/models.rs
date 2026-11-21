// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! The persistent Slint models of one ring: created once, set on the host component once and
//! refilled in place by [`RingBandsModels::update`], so the `ModelRc`s never change identity.

use std::rc::Rc;

use slint::{ModelRc, SharedString, VecModel};

use crate::{ArcGlyph, ArcSeg, FlagPin, GroupRow, TabRow};

use super::RingBandsData;

/// One `VecModel` per array of `RingBands`.
#[derive(Default)]
pub struct RingBandsModels {
    groups: Rc<VecModel<GroupRow>>,
    tabs: Rc<VecModel<TabRow>>,
    segments: Rc<VecModel<ArcSeg>>,
    lifted_segments: Rc<VecModel<ArcSeg>>,
    segment_labels: Rc<VecModel<SharedString>>,
    label_bands: Rc<VecModel<ArcSeg>>,
    label_glyphs: Rc<VecModel<ArcGlyph>>,
    flags: Rc<VecModel<FlagPin>>,
}

impl RingBandsModels {
    /// Eight empty models.
    pub fn new() -> Self {
        Self::default()
    }

    /// Replaces the content of every model with `data`'s arrays (`set_vec`; the models stay).
    pub fn update(&self, data: &RingBandsData) {
        self.groups.set_vec(data.groups.clone());
        self.tabs.set_vec(data.tabs.clone());
        self.segments.set_vec(data.segments.clone());
        self.lifted_segments.set_vec(data.lifted_segments.clone());
        self.segment_labels.set_vec(data.segment_labels.clone());
        self.label_bands.set_vec(data.label_bands.clone());
        self.label_glyphs.set_vec(data.label_glyphs.clone());
        self.flags.set_vec(data.flags.clone());
    }

    /// The model for `RingBands.groups` (the same `ModelRc` identity for the life of `self`).
    pub fn groups(&self) -> ModelRc<GroupRow> {
        ModelRc::from(Rc::clone(&self.groups))
    }

    /// The model for `RingBands.tabs`.
    pub fn tabs(&self) -> ModelRc<TabRow> {
        ModelRc::from(Rc::clone(&self.tabs))
    }

    /// The model for `RingBands.segments`.
    pub fn segments(&self) -> ModelRc<ArcSeg> {
        ModelRc::from(Rc::clone(&self.segments))
    }

    /// The model for `RingBands.lifted-segments`.
    pub fn lifted_segments(&self) -> ModelRc<ArcSeg> {
        ModelRc::from(Rc::clone(&self.lifted_segments))
    }

    /// The model for `RingBands.segment-labels`.
    pub fn segment_labels(&self) -> ModelRc<SharedString> {
        ModelRc::from(Rc::clone(&self.segment_labels))
    }

    /// The model for `RingBands.label-bands`.
    pub fn label_bands(&self) -> ModelRc<ArcSeg> {
        ModelRc::from(Rc::clone(&self.label_bands))
    }

    /// The model for `RingBands.label-glyphs`.
    pub fn label_glyphs(&self) -> ModelRc<ArcGlyph> {
        ModelRc::from(Rc::clone(&self.label_glyphs))
    }

    /// The model for `RingBands.flags`.
    pub fn flags(&self) -> ModelRc<FlagPin> {
        ModelRc::from(Rc::clone(&self.flags))
    }
}
