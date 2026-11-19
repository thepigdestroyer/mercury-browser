// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! Where a page's text, links and search hits are, in the viewer's page space.
//!
//! PDFium reports characters, links and search rectangles in the page's **user space**: the
//! coordinates the PDF file uses, origin at the bottom-left of the page's coordinate system, y
//! growing upward, and not turned by the page's own `/Rotate`. The viewer works in **page space**
//! (see `mercury_pdf::types`): the page as it is shown, origin at the top-left, y growing downward,
//! `/Rotate` applied, measured from the corner of the visible box (the media box cut down by the
//! crop box, which is what PDFium calls the page's bounding box).
//!
//! [`PageGeometry`] is that conversion, and it is pure arithmetic, so it is tested without
//! PDFium; the tests that do use PDFium then check that what it says lands on the glyphs it
//! rendered.

use mercury_pdf::types::{PageSize, Rect};

/// The visible box of a page in user space and its `/Rotate`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PageGeometry {
    pub left: f32,
    pub bottom: f32,
    pub right: f32,
    pub top: f32,
    /// The page's own rotation: 0, 90, 180 or 270 degrees clockwise.
    pub rotate: u16,
}

impl PageGeometry {
    /// A visible box `[left, bottom, right, top]` (in either order of corners) and a rotation in
    /// degrees (any multiple of 90, negative or above 360 included; anything else counts as 0).
    pub fn new(bounds: [f32; 4], rotate_degrees: i32) -> Self {
        let rotate = match rotate_degrees.rem_euclid(360) {
            90 => 90,
            180 => 180,
            270 => 270,
            _ => 0,
        };
        Self {
            left: bounds[0].min(bounds[2]),
            bottom: bounds[1].min(bounds[3]),
            right: bounds[0].max(bounds[2]),
            top: bounds[1].max(bounds[3]),
            rotate,
        }
    }

    /// The size of the visible box before it is turned.
    fn unturned_size(&self) -> (f32, f32) {
        (self.right - self.left, self.top - self.bottom)
    }

    /// The size of the page as shown: the visible box with `/Rotate` applied.
    pub fn shown_size(&self) -> PageSize {
        let (width, height) = self.unturned_size();
        if self.rotate == 90 || self.rotate == 270 {
            PageSize::new(height, width)
        } else {
            PageSize::new(width, height)
        }
    }

    /// A point in user space, as a point in page space.
    pub fn point(&self, x: f32, y: f32) -> (f32, f32) {
        let (width, height) = self.unturned_size();
        // Relative to the visible box, with y growing downward, before the turn.
        let across = x - self.left;
        let down = height - (y - self.bottom);
        match self.rotate {
            90 => (height - down, across),
            180 => (width - across, height - down),
            270 => (down, width - across),
            _ => (across, down),
        }
    }

    /// A rectangle in user space (`left`, `bottom`, `right`, `top`), as a normalised rectangle in
    /// page space.
    pub fn rect(&self, left: f32, bottom: f32, right: f32, top: f32) -> Rect {
        let (x1, y1) = self.point(left, bottom);
        let (x2, y2) = self.point(right, top);
        Rect::new(x1.min(x2), y1.min(y2), x1.max(x2), y1.max(y2))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mercury_pdf::testing::{PageSpec, rotated_spec};

    #[test]
    fn rotations_are_normalised() {
        for (given, wanted) in [(0, 0), (90, 90), (-90, 270), (450, 90), (180, 180), (45, 0)] {
            assert_eq!(
                PageGeometry::new([0.0, 0.0, 1.0, 1.0], given).rotate,
                wanted
            );
        }
    }

    #[test]
    fn an_upright_page_flips_only_the_y_axis() {
        let geometry = PageGeometry::new([10.0, 20.0, 110.0, 220.0], 0);
        assert_eq!(geometry.shown_size(), PageSize::new(100.0, 200.0));
        assert_eq!(geometry.point(10.0, 220.0), (0.0, 0.0));
        assert_eq!(geometry.point(110.0, 20.0), (100.0, 200.0));
        assert_eq!(
            geometry.rect(30.0, 120.0, 50.0, 140.0),
            Rect::new(20.0, 80.0, 40.0, 100.0)
        );
    }

    #[test]
    fn a_page_turned_a_quarter_swaps_its_axes() {
        let geometry = PageGeometry::new([0.0, 0.0, 100.0, 200.0], 90);
        assert_eq!(geometry.shown_size(), PageSize::new(200.0, 100.0));
        // The top-left of the box goes to the top-right of the shown page.
        assert_eq!(geometry.point(0.0, 200.0), (200.0, 0.0));
        // The bottom-left goes to the top-left.
        assert_eq!(geometry.point(0.0, 0.0), (0.0, 0.0));
    }

    #[test]
    fn the_four_turns_keep_every_corner_inside_the_shown_page() {
        for rotate in [0, 90, 180, 270] {
            let geometry = PageGeometry::new([50.0, 60.0, 450.0, 360.0], rotate);
            let shown = geometry.shown_size();
            for (x, y) in [(50.0, 60.0), (450.0, 60.0), (50.0, 360.0), (450.0, 360.0)] {
                let (shown_x, shown_y) = geometry.point(x, y);
                assert!(
                    (0.0..=shown.width).contains(&shown_x)
                        && (0.0..=shown.height).contains(&shown_y),
                    "rotate {rotate}: ({x}, {y}) went to ({shown_x}, {shown_y}) outside {shown:?}"
                );
            }
        }
    }

    /// The test generator maps shown points to user space; this maps them back. The two were
    /// written separately, so agreement for every rotation and every crop box in the fixtures
    /// is a real check on both.
    #[test]
    fn this_is_the_inverse_of_the_generators_mapping() {
        let mut pages: Vec<PageSpec> = rotated_spec().pages;
        for rotate in [0, 90, 180, 270] {
            let mut page = PageSpec::letter(["x"]);
            page.crop_box = Some([12.0, 34.0, 412.0, 334.0]);
            page.rotate = rotate;
            pages.push(page);
        }
        for page in pages {
            let crop = page.crop_box.unwrap_or([0.0, 0.0, page.width, page.height]);
            let geometry = PageGeometry::new(crop, i32::from(page.rotate));
            let shown = page.shown_size();
            assert_eq!(geometry.shown_size(), PageSize::new(shown.0, shown.1));
            for (shown_x, shown_y) in [(0.0, 0.0), (36.0, 50.0), (shown.0, shown.1), (10.5, 99.25)]
            {
                let (x, y) = page.user_point(shown_x, shown_y);
                let (back_x, back_y) = geometry.point(x, y);
                assert!(
                    (back_x - shown_x).abs() < 1e-3 && (back_y - shown_y).abs() < 1e-3,
                    "rotate {}: ({shown_x}, {shown_y}) -> ({x}, {y}) -> ({back_x}, {back_y})",
                    page.rotate
                );
            }
        }
    }
}
