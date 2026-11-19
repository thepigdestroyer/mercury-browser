// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! Shrinking a frame to a snapshot.
//!
//! GROUNDWORK STUB (lane tab-snapshots, item 1 owns this file): [`Limits`] is final; the two
//! functions are to be written to the contracts in their comments.

use crate::frame::RgbaFrame;

/// The widest snapshot, in pixels (design/ui-final/SPEC.md section 6.1).
pub const MAX_WIDTH: u32 = 480;
/// The tallest snapshot, in pixels.
pub const MAX_HEIGHT: u32 = 300;

/// The box a snapshot must fit in. Both sides are at least 1.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    max_width: u32,
    max_height: u32,
}

impl Limits {
    /// 480 by 300.
    pub const DEFAULT: Self = Self {
        max_width: MAX_WIDTH,
        max_height: MAX_HEIGHT,
    };

    /// `None` if a side is zero.
    pub const fn new(max_width: u32, max_height: u32) -> Option<Self> {
        if max_width == 0 || max_height == 0 {
            None
        } else {
            Some(Self {
                max_width,
                max_height,
            })
        }
    }

    pub const fn max_width(self) -> u32 {
        self.max_width
    }

    pub const fn max_height(self) -> u32 {
        self.max_height
    }
}

impl Default for Limits {
    fn default() -> Self {
        Self::DEFAULT
    }
}

/// The size a `width` by `height` picture shrinks to: it keeps its proportions, fits inside
/// `limits`, is never made larger than it is, and has at least one pixel on each side (a 1 by 4000
/// strip becomes 1 by `max_height`, not 0 by something). A zero `width` or `height` returns
/// `(1, 1)` (the caller never has one: [`RgbaFrame`] refuses them).
pub fn fit_within(_width: u32, _height: u32, _limits: Limits) -> (u32, u32) {
    todo!("tab-snapshots item 1: fit_within")
}

/// Shrinks `frame` to [`fit_within`] its size, averaging the source pixels that fall into each
/// new pixel (a box filter, so thin text and lines fade instead of flickering the way
/// nearest-neighbour sampling would make them). A frame that already fits keeps its size and
/// pixels. In every case the result is opaque: each pixel's alpha is 255, because a page picture
/// is shown over nothing and a stray transparent pixel would show the overlay's colour through.
/// The page's own colours are not changed.
pub fn downscale(_frame: RgbaFrame, _limits: Limits) -> RgbaFrame {
    todo!("tab-snapshots item 1: downscale")
}
