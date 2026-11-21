// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! The drop target of a segment drag: which group and index a released tab lands at and the accent
//! bar that marks the insertion point. STUB in the scaffold commit (always invalid); item 2
//! `overlays` of lane ring-bands writes the real body. The signature is internal.

use crate::ring_geometry::slint_models::RingModel;
use crate::{ArcSeg, RingDropTarget};

/// The slot under (`x`, `y`) for the tab `dragging_tab` (a `TabRow.id`), or an invalid target.
pub(super) fn drop_target_for(
    _model: &RingModel,
    _x: f32,
    _y: f32,
    _dragging_tab: i32,
) -> RingDropTarget {
    RingDropTarget {
        valid: false,
        group: -1,
        index: -1,
        indicator: ArcSeg::default(),
    }
}
