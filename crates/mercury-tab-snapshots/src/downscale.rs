// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! Shrinking a frame to a snapshot.
//!
//! The shrink is a box filter written here, in integers. Each new pixel is the average of the
//! block of source pixels that falls into it; the blocks tile the source exactly (every source
//! pixel is in one block, none in two), so a one-pixel line fades but never disappears.
//!
//! Why not `image::imageops::thumbnail`, the first choice: it computes the block edges in `f32`
//! and rounds them up, and where an edge lands on a whole number the two neighbouring blocks can
//! disagree by one source pixel, so a source column or row is dropped. Measured with a one-pixel
//! black line on white: the line vanished at 46 of the 2,520 widths from 481 to 3000, for example
//! at width 1024, column 96. A surface 1024 pixels wide is an ordinary window, so the first choice
//! was wrong on a common case. `tests/downscale.rs` now checks that every column and row of a list
//! of sizes lands in exactly one new pixel. Speed was not the reason: for a 2880 by 1800 frame in
//! release on the Intel development Mac the two take about the same (about 10 ms here, 13 ms for
//! `thumbnail`), well inside the 40 ms that is fine on a worker thread.
//!
//! The average is taken over the stored (sRGB-encoded) values, as every thumbnailer does, not over
//! linear light: black next to white gives mid grey 127 or 128, not the lighter 188 that linear
//! averaging would give. Averaging in linear light costs a lookup table and a second pass, for a
//! preview picture that nobody compares with the page pixel by pixel.

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
///
/// The side that meets its limit first gets exactly that limit; the other side is the proportional
/// size rounded to the nearest pixel (a tie rounds up).
pub fn fit_within(width: u32, height: u32, limits: Limits) -> (u32, u32) {
    if width == 0 || height == 0 {
        return (1, 1);
    }
    if width <= limits.max_width() && height <= limits.max_height() {
        return (width, height);
    }
    // Integer arithmetic, so a size that is exactly in proportion lands exactly on the limit.
    // `u128` holds any product of three `u32` values, so no size or limit can overflow.
    let (width, height) = (u128::from(width), u128::from(height));
    let (max_width, max_height) = (
        u128::from(limits.max_width()),
        u128::from(limits.max_height()),
    );
    // The width reaches its limit first when width / max_width >= height / max_height.
    let (new_width, new_height) = if max_width * height <= max_height * width {
        // new_height = height * max_width / width, rounded to the nearest.
        (max_width, (height * max_width * 2 + width) / (2 * width))
    } else {
        ((width * max_height * 2 + height) / (2 * height), max_height)
    };
    (to_u32(new_width).max(1), to_u32(new_height).max(1))
}

/// A value that is at most `u32::MAX` (each new side is at most its limit); the fallback only
/// keeps the function total.
fn to_u32(value: u128) -> u32 {
    u32::try_from(value).unwrap_or(u32::MAX)
}

/// Shrinks `frame` to [`fit_within`] its size, averaging the source pixels that fall into each
/// new pixel (a box filter, so thin text and lines fade instead of flickering the way
/// nearest-neighbour sampling would make them). A frame that already fits keeps its size and
/// pixels. In every case the result is opaque: each pixel's alpha is 255, because a page picture
/// is shown over nothing and a stray transparent pixel would show the overlay's colour through.
/// The page's own colours are not changed.
///
/// New pixel `i` along one axis averages the source pixels from `i * source / new` up to
/// `(i + 1) * source / new` (integer division, so the blocks tile the source), and the average is
/// rounded to the nearest value. Red, green and blue are averaged as they are stored, whatever the
/// alpha of the source pixel was.
pub fn downscale(frame: RgbaFrame, limits: Limits) -> RgbaFrame {
    let (width, height) = (frame.width(), frame.height());
    let (new_width, new_height) = fit_within(width, height, limits);
    let pixels = if (new_width, new_height) == (width, height) {
        let mut pixels = frame.into_pixels();
        make_opaque(&mut pixels);
        pixels
    } else {
        box_filter(
            frame.pixels(),
            (width as usize, height as usize),
            (new_width as usize, new_height as usize),
        )
    };
    // `fit_within` never makes a side larger or zero, and the buffer above is exactly
    // `new_width * new_height * 4` bytes long, so `RgbaFrame::new` cannot refuse it.
    RgbaFrame::new(new_width, new_height, pixels)
        .expect("a shrunk frame has the size and the buffer length that fit_within gave")
}

/// Sets every alpha byte (the fourth of each pixel) to 255.
fn make_opaque(pixels: &mut [u8]) {
    for alpha in pixels.iter_mut().skip(3).step_by(4) {
        *alpha = 255;
    }
}

/// Where block `index` starts when `source` pixels are cut into `count` blocks. Every side is at
/// most `MAX_DIMENSION` (16,384), so the product is at most 2^28 and fits a `usize` everywhere.
fn block_start(index: usize, source: usize, count: usize) -> usize {
    index * source / count
}

/// The box filter. `source` is `source_size.0 * source_size.1` RGBA pixels, top row first; the
/// result is `new_size.0 * new_size.1` opaque RGBA pixels. Neither new side is larger than its
/// source side, so every block holds at least one pixel.
///
/// Two passes per new row: the source rows of the block are added up column by column (`u32`
/// sums: at most 16,384 rows of 255), then the columns of each block are added up (`u64`, as a
/// block can hold 2^28 pixels) and divided. The source is read once, in order.
fn box_filter(source: &[u8], source_size: (usize, usize), new_size: (usize, usize)) -> Vec<u8> {
    let (source_width, source_height) = source_size;
    let (new_width, new_height) = new_size;
    let mut shrunk = Vec::with_capacity(new_width * new_height * 4);
    // Four channels per column so that the adds are in lanes; the alpha sum is never read.
    let mut column_sums = vec![[0u32; 4]; source_width];

    for new_y in 0..new_height {
        let first_row = block_start(new_y, source_height, new_height);
        let end_row = block_start(new_y + 1, source_height, new_height);

        column_sums.fill([0; 4]);
        for row in source
            .chunks_exact(source_width * 4)
            .skip(first_row)
            .take(end_row - first_row)
        {
            let (pixels, _) = row.as_chunks::<4>();
            for (sums, pixel) in column_sums.iter_mut().zip(pixels) {
                sums[0] += u32::from(pixel[0]);
                sums[1] += u32::from(pixel[1]);
                sums[2] += u32::from(pixel[2]);
                sums[3] += u32::from(pixel[3]);
            }
        }

        let rows_in_block = (end_row - first_row) as u64;
        for new_x in 0..new_width {
            let first_column = block_start(new_x, source_width, new_width);
            let end_column = block_start(new_x + 1, source_width, new_width);

            let mut totals = [0u64; 3];
            for sums in &column_sums[first_column..end_column] {
                totals[0] += u64::from(sums[0]);
                totals[1] += u64::from(sums[1]);
                totals[2] += u64::from(sums[2]);
            }

            // At least one pixel in a block; `max` only keeps the division total.
            let pixels_in_block = ((end_column - first_column) as u64 * rows_in_block).max(1);
            let half = pixels_in_block / 2;
            for total in totals {
                // The average of values that are at most 255 is at most 255.
                shrunk.push(u8::try_from((total + half) / pixels_in_block).unwrap_or(u8::MAX));
            }
            shrunk.push(255);
        }
    }
    shrunk
}
