// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

use serde::{Deserialize, Serialize};

use crate::paper::{MarginPreset, Orientation, PaperSize};

/// Device pixels per CSS pixel for the print capture (about 190 dpi on A4).
pub const PRINT_DEVICE_SCALE: f32 = 2.0;
/// The tallest tile the capture asks the engine for, in device pixels.
pub const MAX_TILE_DEVICE_PX: u32 = 8192;
/// Pages in one print job.
pub const MAX_PAGES: usize = 200;
/// The tallest full-page screenshot, in device pixels.
pub const MAX_SCREENSHOT_DEVICE_PX: u32 = 32_768;
/// The largest full-page screenshot, in device pixels (width times height).
pub const MAX_SCREENSHOT_PIXELS: u64 = 400_000_000;

const MIN_SCALE_PERCENT: u16 = 50;
const MAX_SCALE_PERCENT: u16 = 200;
const MIN_LAYOUT_CSS_PX: u32 = 200;

/// Paper, orientation, margins and scale of a print: one model shared by the preview, the
/// capture, the PDF writer and the backend.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct PageSetup {
    pub paper: PaperSize,
    pub orientation: Orientation,
    pub margins: MarginPreset,
    pub scale_percent: u16,
}

impl PageSetup {
    /// The same setup with the scale clamped into 50 to 200 percent.
    pub fn sanitized(self) -> PageSetup {
        PageSetup {
            scale_percent: self.scale_percent.clamp(MIN_SCALE_PERCENT, MAX_SCALE_PERCENT),
            ..self
        }
    }

    /// The page width and height in points, after the orientation.
    pub fn page_points(&self) -> (f32, f32) {
        let (width, height) = self.paper.points();
        match self.orientation {
            Orientation::Portrait => (width, height),
            Orientation::Landscape => (height, width),
        }
    }

    pub fn margin_points(&self) -> f32 {
        self.margins.points()
    }

    /// The page minus its margins, in points.
    pub fn content_points(&self) -> (f32, f32) {
        let (width, height) = self.page_points();
        let margin = self.margin_points();
        ((width - 2.0 * margin).max(0.0), (height - 2.0 * margin).max(0.0))
    }

    /// The layout width in CSS pixels, so that the printed content is `content_points` wide at the
    /// chosen scale.
    pub fn content_width_css_px(&self) -> u32 {
        css_px(self.content_points().0, self.scale())
    }

    pub fn content_height_css_px(&self) -> u32 {
        css_px(self.content_points().1, self.scale())
    }

    /// Whether a change from `other` to this setup changes the width the page lays out at, so the
    /// capture must run again.
    pub fn needs_recapture(&self, other: &PageSetup) -> bool {
        self != other
    }

    fn scale(&self) -> f32 {
        f32::from(self.sanitized().scale_percent) / 100.0
    }
}

/// Points to CSS pixels (96 per inch, 72 points per inch), divided by the print scale.
fn css_px(points: f32, scale: f32) -> u32 {
    let px = points / 72.0 * 96.0 / scale;
    (px.round() as u32).max(MIN_LAYOUT_CSS_PX)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::paper::{MarginPreset, Orientation, PaperSize};

    fn setup(paper: PaperSize, orientation: Orientation, margins: MarginPreset, scale: u16) -> PageSetup {
        PageSetup { paper, orientation, margins, scale_percent: scale }
    }

    fn a4_default(scale: u16) -> PageSetup {
        setup(PaperSize::A4, Orientation::Portrait, MarginPreset::Default, scale)
    }

    #[test]
    fn content_width_css_px_for_a4_default_is_718() {
        assert_eq!(a4_default(100).content_width_css_px(), 718);
    }

    #[test]
    fn content_height_css_px_a4() {
        assert_eq!(a4_default(100).content_height_css_px(), 1047);
    }

    #[test]
    fn scale_200_halves_the_css_width() {
        assert_eq!(a4_default(200).content_width_css_px(), 359);
    }

    #[test]
    fn scale_50_doubles_the_css_width() {
        assert_eq!(a4_default(50).content_width_css_px(), 1436);
    }

    #[test]
    fn scale_is_clamped() {
        assert_eq!(a4_default(10).sanitized().scale_percent, 50);
        assert_eq!(a4_default(500).sanitized().scale_percent, 200);
        assert_eq!(a4_default(125).sanitized().scale_percent, 125);
    }

    #[test]
    fn tiny_paper_with_huge_margin_is_at_least_200_css_px() {
        let page = setup(PaperSize::A5, Orientation::Landscape, MarginPreset::Default, 200);
        assert!(page.content_width_css_px() >= 200, "{}", page.content_width_css_px());
        assert!(page.content_height_css_px() >= 200, "{}", page.content_height_css_px());
    }

    #[test]
    fn needs_recapture_true_when_paper_orientation_margins_or_scale_differ() {
        let base = a4_default(100);
        assert!(base.needs_recapture(&setup(PaperSize::Letter, Orientation::Portrait, MarginPreset::Default, 100)));
        assert!(base.needs_recapture(&setup(PaperSize::A4, Orientation::Landscape, MarginPreset::Default, 100)));
        assert!(base.needs_recapture(&setup(PaperSize::A4, Orientation::Portrait, MarginPreset::None, 100)));
        assert!(base.needs_recapture(&a4_default(90)));
    }

    #[test]
    fn needs_recapture_false_when_equal() {
        let base = a4_default(100);
        assert!(!base.needs_recapture(&a4_default(100)));
    }
}
