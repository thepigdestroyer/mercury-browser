// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! Places viewport tiles by absolute row. Tiles overlap by a few rows when the last viewport is
//! clamped to the bottom of the page, so rows already delivered are dropped.

use image::RgbaImage;
use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum AssemblerError {
    #[error("tile starts at row {start}, but row {expected} is needed next")]
    Gap { start: u32, expected: u32 },
    #[error("tile is {found} px wide, the page is {expected} px wide")]
    WidthChanged { expected: u32, found: u32 },
}

/// Delivers each row of the capture once, in order.
#[derive(Debug, Default)]
pub struct RowAssembler {
    width: Option<u32>,
    next_row: u32,
}

impl RowAssembler {
    pub fn new() -> Self {
        Self::default()
    }

    /// Accepts `tile` placed at absolute row `start_row`. Returns the rows that were not delivered
    /// before (`None` when the whole tile was already seen).
    pub fn push_tile(
        &mut self,
        start_row: u32,
        tile: &RgbaImage,
    ) -> Result<Option<RgbaImage>, AssemblerError> {
        let width = *self.width.get_or_insert(tile.width());
        if tile.width() != width {
            return Err(AssemblerError::WidthChanged {
                expected: width,
                found: tile.width(),
            });
        }
        if start_row > self.next_row {
            return Err(AssemblerError::Gap {
                start: start_row,
                expected: self.next_row,
            });
        }
        let end = start_row.saturating_add(tile.height());
        if end <= self.next_row {
            return Ok(None);
        }
        let skip = self.next_row - start_row;
        let fresh = tile.height() - skip;
        self.next_row = end;
        if skip == 0 {
            return Ok(Some(tile.clone()));
        }
        Ok(Some(
            image::imageops::crop_imm(tile, 0, skip, tile.width(), fresh).to_image(),
        ))
    }

    /// Number of rows delivered so far; the next new row has this index.
    pub fn rows_delivered(&self) -> u32 {
        self.next_row
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::Rgba;

    /// A tile whose every pixel encodes its absolute row, so overlaps can be checked by colour.
    fn tile(start: u32, height: u32, width: u32) -> RgbaImage {
        let mut image = RgbaImage::new(width, height);
        for y in 0..height {
            let row = start + y;
            let colour = Rgba([(row & 0xff) as u8, (row >> 8) as u8, 0, 255]);
            for x in 0..width {
                image.put_pixel(x, y, colour);
            }
        }
        image
    }

    #[test]
    fn first_tile_passes_whole() {
        let mut assembler = RowAssembler::new();
        let out = assembler.push_tile(0, &tile(0, 10, 4)).ok().flatten();
        assert_eq!(out.map(|i| i.dimensions()), Some((4, 10)));
        assert_eq!(assembler.rows_delivered(), 10);
    }

    #[test]
    fn overlap_rows_are_dropped() {
        let mut assembler = RowAssembler::new();
        assembler.push_tile(0, &tile(0, 300, 4)).ok();
        let fresh = assembler.push_tile(200, &tile(200, 300, 4)).ok().flatten();
        let fresh = fresh.map(|i| (i.dimensions(), i.get_pixel(0, 0).0));
        assert_eq!(fresh, Some(((4, 200), [(300 & 0xff) as u8, 1, 0, 255])));
        assert_eq!(assembler.rows_delivered(), 500);
    }

    #[test]
    fn fully_seen_tile_gives_nothing() {
        let mut assembler = RowAssembler::new();
        assembler.push_tile(0, &tile(0, 100, 4)).ok();
        assert!(matches!(
            assembler.push_tile(10, &tile(10, 50, 4)),
            Ok(None)
        ));
        assert_eq!(assembler.rows_delivered(), 100);
    }

    #[test]
    fn gap_is_an_error() {
        let mut assembler = RowAssembler::new();
        assembler.push_tile(0, &tile(0, 100, 4)).ok();
        assert_eq!(
            assembler.push_tile(150, &tile(150, 10, 4)).err(),
            Some(AssemblerError::Gap {
                start: 150,
                expected: 100
            })
        );
    }

    #[test]
    fn width_change_is_an_error() {
        let mut assembler = RowAssembler::new();
        assembler.push_tile(0, &tile(0, 10, 4)).ok();
        assert_eq!(
            assembler.push_tile(10, &tile(10, 10, 5)).err(),
            Some(AssemblerError::WidthChanged {
                expected: 4,
                found: 5
            })
        );
    }
}
