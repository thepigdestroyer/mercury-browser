// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! Measures each row of a captured page so the paginator can prefer quiet rows (gaps between
//! paragraphs, margins) as page breaks.

use image::RgbaImage;

use crate::breaks::{BreakCandidate, BreakCost};

/// One candidate per row of `rows`, at the absolute row index `start_row + y`. The cost is the
/// mean absolute difference between neighbouring pixels (RGB), so a uniform row, whatever its
/// colour, costs 0 and a row of text costs more.
pub fn row_candidates(rows: &RgbaImage, start_row: u32) -> Vec<BreakCandidate> {
    let stride = rows.width() as usize * 4;
    let raw = rows.as_raw();
    (0..rows.height())
        .map(|y| {
            let cost = if stride == 0 {
                0
            } else {
                let from = y as usize * stride;
                row_cost(&raw[from..from + stride])
            };
            BreakCandidate {
                at: start_row.saturating_add(y),
                cost: BreakCost::Cheap(cost),
            }
        })
        .collect()
}

fn row_cost(row: &[u8]) -> u16 {
    let (pixels, _) = row.as_chunks::<4>();
    if pixels.len() < 2 {
        return 0;
    }
    let total: u64 = pixels
        .windows(2)
        .map(|pair| {
            (0..3)
                .map(|c| u64::from(pair[0][c].abs_diff(pair[1][c])))
                .sum::<u64>()
        })
        .sum();
    u16::try_from(total / pixels.len() as u64).unwrap_or(u16::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::Rgba;

    fn solid(width: u32, height: u32, colour: [u8; 4]) -> RgbaImage {
        RgbaImage::from_pixel(width, height, Rgba(colour))
    }

    fn cost_of(candidate: &BreakCandidate) -> u16 {
        match candidate.cost {
            BreakCost::Cheap(cost) => cost,
            other => panic!("expected Cheap, got {other:?}"),
        }
    }

    #[test]
    fn blank_rows_are_cheap_zero() {
        let rows = solid(64, 3, [255, 255, 255, 255]);
        let candidates = row_candidates(&rows, 0);
        assert!(candidates.iter().all(|c| cost_of(c) == 0));
        let dark = solid(64, 2, [20, 20, 20, 255]);
        assert!(row_candidates(&dark, 0).iter().all(|c| cost_of(c) == 0));
    }

    #[test]
    fn text_rows_cost_more() {
        let mut rows = RgbaImage::new(10, 1);
        for x in 0..10 {
            let v = if x % 2 == 0 { 0 } else { 255 };
            rows.put_pixel(x, 0, Rgba([v, v, v, 255]));
        }
        assert!(cost_of(&row_candidates(&rows, 0)[0]) > 0);
    }

    #[test]
    fn a_gradient_row_is_not_blank() {
        let mut rows = RgbaImage::new(16, 1);
        for x in 0..16 {
            rows.put_pixel(x, 0, Rgba([(x * 10) as u8, 0, 0, 255]));
        }
        assert!(cost_of(&row_candidates(&rows, 0)[0]) > 0);
    }

    #[test]
    fn candidate_positions_are_absolute() {
        let rows = solid(8, 3, [255, 255, 255, 255]);
        let at: Vec<u32> = row_candidates(&rows, 500).iter().map(|c| c.at).collect();
        assert_eq!(at, vec![500, 501, 502]);
    }

    #[test]
    fn zero_width_rows_are_cheap() {
        let rows = RgbaImage::new(0, 4);
        let candidates = row_candidates(&rows, 7);
        assert_eq!(candidates.len(), 4);
        assert!(candidates.iter().all(|c| cost_of(c) == 0));
    }
}
