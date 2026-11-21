// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! Measured glyph advances for the curved ring labels: the width the renderer's real font gives a
//! character, read through a never-shown `RingTextMeasure` component and cached.
//! STUB in the scaffold commit (delegates to `ApproxMetrics`); item 3 `metrics` of lane ring-bands
//! writes the real measurement. The public signatures are the pinned contract.

use crate::ring_geometry::{ApproxMetrics, GlyphMetrics};

/// `GlyphMetrics` that measures with the real font (cached per character, size and weight).
#[derive(Debug, Default)]
pub struct MeasuredMetrics {
    fallback: ApproxMetrics,
}

impl MeasuredMetrics {
    /// Creates the measuring component. Needs a Slint platform (a window system or the headless
    /// test platform); use [`metrics`] to fall back to `ApproxMetrics` when it cannot be made.
    pub fn new() -> Result<Self, slint::PlatformError> {
        Ok(Self::default())
    }
}

impl GlyphMetrics for MeasuredMetrics {
    fn advance(&self, ch: char, size: f32, weight: u16) -> f32 {
        self.fallback.advance(ch, size, weight)
    }
}

/// `MeasuredMetrics`, or `ApproxMetrics` when the measuring component cannot be made. Call it once
/// per UI thread and keep the result.
pub fn metrics() -> Box<dyn GlyphMetrics> {
    match MeasuredMetrics::new() {
        Ok(measured) => Box::new(measured),
        Err(_) => Box::new(ApproxMetrics),
    }
}
