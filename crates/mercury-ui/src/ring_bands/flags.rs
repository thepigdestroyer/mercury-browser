// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! Which marker a tab gets and where it sits (SPEC §3.2; ring-bands brief §3.2 "Flag rules").
//! STUB in the scaffold commit (no pins); item 2 `overlays` of lane ring-bands writes the real
//! body. The signature is internal.

use crate::ring_geometry::slint_models::RingModel;
use crate::{FlagPin, TabRow};

use super::RingOptions;

/// The pins of the shown `tabs` (ring order, parallel to `model`'s flat tab positions).
pub(super) fn flag_pins(_model: &RingModel, _tabs: &[TabRow], _opts: &RingOptions) -> Vec<FlagPin> {
    Vec::new()
}
