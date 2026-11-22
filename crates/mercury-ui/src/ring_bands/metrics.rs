// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! Measured glyph advances for the curved ring labels: the width the renderer's real font gives a
//! character, read through a never-shown `RingTextMeasure` component and cached.
//!
//! Slint 1.17 has no font-metrics API for Rust, so the advance of `ch` is
//! `width("x{ch}x") - width("xx")` of a `Text` inside a hidden window. The surrounding "x" keep a
//! lone or trailing space from being trimmed by the layout. Kerning with the "x" is included, which
//! is a small error that the curved labels (one `Text` per glyph, no kerning) share.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;

use slint::{PlatformError, SharedString};

use crate::RingTextMeasure;
use crate::ring_geometry::{ApproxMetrics, GlyphMetrics};

/// Probe string and size used by [`MeasuredMetrics::new`] to check that the component measures.
const PROBE: &str = "W";
const PROBE_SIZE: f32 = 12.0;
const PROBE_WEIGHT: u16 = 400;

/// The size every advance is measured at, then scaled to the asked size (see `measure_advance`).
const REFERENCE_SIZE: f32 = 100.0;

type CacheKey = (char, u32, u16);

/// `GlyphMetrics` that measures with the real font (cached per character, size and weight).
/// Not `Send`: it owns a Slint component and belongs to the UI thread that made it.
pub struct MeasuredMetrics {
    measure: RingTextMeasure,
    cache: RefCell<HashMap<CacheKey, f32>>,
    reads: Cell<usize>,
}

impl MeasuredMetrics {
    /// Creates the measuring component. Needs a Slint platform (a window system or the headless
    /// test platform); use [`metrics`] to fall back to `ApproxMetrics` when it cannot be made.
    /// Fails when the component cannot be created or does not give a positive width for "W".
    pub fn new() -> Result<Self, PlatformError> {
        let measured = Self {
            measure: RingTextMeasure::new()?,
            cache: RefCell::new(HashMap::new()),
            reads: Cell::new(0),
        };
        let probe = measured.string_width(PROBE, PROBE_SIZE, PROBE_WEIGHT);
        if probe.is_finite() && probe > 0.0 {
            Ok(measured)
        } else {
            Err(PlatformError::Other(format!(
                "the text measuring component gave {probe} for {PROBE:?}"
            )))
        }
    }

    /// How many times the component was asked for a width (the probe in `new` counts). A test hook
    /// to prove that the cache answers repeated questions.
    #[cfg(any(test, feature = "test-support"))]
    pub fn component_reads(&self) -> usize {
        self.reads.get()
    }

    /// The width of `text` read from the component, in logical px.
    fn string_width(&self, text: &str, size: f32, weight: u16) -> f32 {
        self.reads.set(self.reads.get() + 1);
        self.measure.set_sample(SharedString::from(text));
        self.measure.set_font_size(size);
        self.measure.set_font_weight(i32::from(weight));
        self.measure.get_text_width()
    }

    /// Measured at `REFERENCE_SIZE` and scaled: the software renderer snaps glyph advances to whole
    /// pixels, which at 12 px is a 5 to 10 percent error per glyph (and can make a heavier weight
    /// read narrower).
    fn measure_advance(&self, ch: char, size: f32, weight: u16) -> f32 {
        let with = self.string_width(&format!("x{ch}x"), REFERENCE_SIZE, weight);
        let without = self.string_width("xx", REFERENCE_SIZE, weight);
        let advance = (with - without) * size / REFERENCE_SIZE;
        if advance.is_finite() && advance > 0.0 {
            advance
        } else {
            0.0
        }
    }
}

impl std::fmt::Debug for MeasuredMetrics {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MeasuredMetrics")
            .field("cached", &self.cache.borrow().len())
            .field("reads", &self.reads.get())
            .finish()
    }
}

/// Control characters and the joiners and variation selectors have no advance of their own.
fn has_no_advance(ch: char) -> bool {
    ch.is_control() || matches!(ch as u32, 0x200C..=0x200D | 0xFE00..=0xFE0F)
}

impl GlyphMetrics for MeasuredMetrics {
    fn advance(&self, ch: char, size: f32, weight: u16) -> f32 {
        if !size.is_finite() || size <= 0.0 || has_no_advance(ch) {
            return 0.0;
        }
        let key = (ch, size.to_bits(), weight);
        if let Some(advance) = self.cache.borrow().get(&key) {
            return *advance;
        }
        let advance = self.measure_advance(ch, size, weight);
        self.cache.borrow_mut().insert(key, advance);
        advance
    }
}

/// `MeasuredMetrics`, or `ApproxMetrics` when the measuring component cannot be made. Call it once
/// per UI thread and keep the result.
pub fn metrics() -> Box<dyn GlyphMetrics> {
    choose(MeasuredMetrics::new())
}

fn choose(result: Result<MeasuredMetrics, PlatformError>) -> Box<dyn GlyphMetrics> {
    match result {
        Ok(measured) => Box::new(measured),
        Err(_) => Box::new(ApproxMetrics),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_failing_constructor_gives_approx_metrics() {
        let chosen = choose(Err(PlatformError::Other("no platform".into())));
        for ch in ['W', 'i', 'm', ' '] {
            assert_eq!(
                chosen.advance(ch, 12.0, 600),
                ApproxMetrics.advance(ch, 12.0, 600),
                "{ch:?}"
            );
        }
    }

    #[test]
    fn characters_without_an_advance_are_zero_without_a_component() {
        for ch in ['\u{200D}', '\u{200C}', '\u{FE0F}', '\n', '\u{7}'] {
            assert_eq!(ApproxMetrics.advance(ch, 12.0, 400), 0.0, "{ch:?}");
            assert!(has_no_advance(ch), "{ch:?}");
        }
        assert!(!has_no_advance('x'));
        assert!(!has_no_advance(' '));
        assert!(!has_no_advance('\u{0301}'));
    }
}
