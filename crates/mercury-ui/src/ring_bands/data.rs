// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! `RingBandsData`: everything one `RingBands` needs, as plain data (no Slint models, so it is
//! testable without a window). STUB in the scaffold commit: `new` returns an empty ring, `hit` and
//! `hit_polar` say `outside`; item 1 `data` of lane ring-bands writes the real bodies. The field list
//! and the signatures are the pinned contract.

use slint::SharedString;

use crate::ring_geometry::slint_models::RingModel;
use crate::ring_geometry::{GlyphMetrics, RingSpec};
use crate::{ArcGlyph, ArcSeg, FlagPin, GroupRow, RingDropTarget, RingHit, RingHitKind, TabRow};

use super::RingOptions;

/// The arrays one `RingBands` takes, built from the browser's rows (see the module documentation of
/// `ring_bands` for the rules).
#[derive(Clone, Debug, PartialEq)]
pub struct RingBandsData {
    /// The layout and the id of every position.
    pub model: RingModel,
    /// The groups that have a shown tab, in ring order (`label_bands[g]` belongs to `groups[g]`).
    pub groups: Vec<GroupRow>,
    /// The shown tabs, group by group, in `index-in-group` order (`segments[i]` belongs to `tabs[i]`).
    pub tabs: Vec<TabRow>,
    /// One shape per tab; `ArcSeg.tab` is the `TabRow.id`.
    pub segments: Vec<ArcSeg>,
    /// The hovered shape of each segment (same length and order as `segments`).
    pub lifted_segments: Vec<ArcSeg>,
    /// The accessible label of each segment (same length and order as `segments`).
    pub segment_labels: Vec<SharedString>,
    /// One band per group in `groups`; `ArcSeg.tab` is the `GroupRow.id`.
    pub label_bands: Vec<ArcSeg>,
    /// The curved labels of every group, colours resolved.
    pub label_glyphs: Vec<ArcGlyph>,
    /// The markers (spinner, speaker, dots, digits).
    pub flags: Vec<FlagPin>,
}

impl RingBandsData {
    /// Lays the ring out for `groups` and `tabs` (any order, any tabs: pinned tabs and tabs of
    /// unknown groups are dropped) and builds every array. `metrics` measures the label glyphs.
    pub fn new(
        spec: RingSpec,
        _groups: &[GroupRow],
        _tabs: &[TabRow],
        opts: &RingOptions,
        _metrics: &dyn GlyphMetrics,
    ) -> Self {
        let model = RingModel::new(spec, &[], &[]);
        let flags = super::flags::flag_pins(&model, &[], opts);
        Self {
            model,
            groups: Vec::new(),
            tabs: Vec::new(),
            segments: Vec::new(),
            lifted_segments: Vec::new(),
            segment_labels: Vec::new(),
            label_bands: Vec::new(),
            label_glyphs: Vec::new(),
            flags,
        }
    }

    /// What the point (`x`, `y`) (ring-local) is over; `hovered_tab` is the `TabRow.id` of the
    /// tab that is lifted now (-1 for none), so that its raised zone does not flicker.
    pub fn hit(&self, _x: f32, _y: f32, _hovered_tab: i32) -> RingHit {
        RingHit {
            kind: RingHitKind::Outside,
            id: -1,
        }
    }

    /// The same for a radius and an angle (degrees clockwise from 12 o'clock), for keyboard
    /// callers and tests.
    pub fn hit_polar(&self, _radius: f32, _theta_deg: f32) -> RingHit {
        RingHit {
            kind: RingHitKind::Outside,
            id: -1,
        }
    }

    /// Where `dragging_tab` (a `TabRow.id`, -1 for none) would land if released at (`x`, `y`).
    pub fn drop_target(&self, x: f32, y: f32, dragging_tab: i32) -> RingDropTarget {
        super::drag::drop_target_for(&self.model, x, y, dragging_tab)
    }
}
