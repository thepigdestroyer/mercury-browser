// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! Where everything on the "Switch group" card sits (lane brief §3 and §6 item 3). Plain `f32`
//! logical pixels, no Slint types. All positions are from the card's top-left corner, tiles'
//! inner positions from the tile's top-left corner.

/// The most tiles the card shows: one per letter.
pub const MAX_TILES: usize = 26;

/// How big the tiles are: `Full` is the mockup's (1 to 6 tiles), `Compact` fits 7 to 26.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TileSize {
    /// 108 x 168 tiles, six per row.
    Full,
    /// 72 x 134 tiles, nine per row.
    Compact,
}

/// The inner measures of one tile; they map one to one onto the Slint `KeyMetrics`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TileMetrics {
    /// Tile width.
    pub tile_width: f32,
    /// Tile height.
    pub tile_height: f32,
    /// The side of the ring's square box (ring-local path coordinates start at its top-left).
    pub ring_size: f32,
    /// The ring's centre in the tile.
    pub ring_cx: f32,
    /// The ring's centre in the tile.
    pub ring_cy: f32,
    /// The tab segments' band radius.
    pub ring_radius: f32,
    /// The tab segments' band width.
    pub ring_band: f32,
    /// The halo's outer radius.
    pub halo_outer: f32,
    /// The halo's width (a ring of the group's tint).
    pub halo_width: f32,
    /// The keycap's side.
    pub key_size: f32,
    /// The keycap's top in the tile.
    pub key_y: f32,
    /// The keycap's corner radius.
    pub key_radius: f32,
    /// The keycap letter's font size.
    pub key_font: f32,
    /// The name's top in the tile.
    pub name_y: f32,
    /// The name's font size.
    pub name_font: f32,
    /// The count's top in the tile.
    pub count_y: f32,
    /// The count's font size.
    pub count_font: f32,
}

/// The card's layout for a number of tiles.
#[derive(Clone, Debug, PartialEq)]
pub struct CardLayout {
    /// Full or compact tiles.
    pub size: TileSize,
    /// Tiles per row.
    pub columns: usize,
    /// Rows used (0 for no tile).
    pub rows: usize,
    /// The card's width (760).
    pub card_width: f32,
    /// The card's height: `tiles_bottom + 96`.
    pub card_height: f32,
    /// Where the tiles start, from the card's left (30).
    pub tiles_left: f32,
    /// Where the tiles start, from the card's top (66).
    pub tiles_top: f32,
    /// Where the last row ends, from the card's top.
    pub tiles_bottom: f32,
    /// The inner measures of a tile.
    pub metrics: TileMetrics,
    /// The top-left of each tile, from the card's top-left, in order.
    pub positions: Vec<(f32, f32)>,
}

/// The most tiles that still get the mockup's full size (one row of six).
pub const MAX_FULL_TILES: usize = 6;

/// The card's width, in the mockup.
const CARD_WIDTH: f32 = 760.0;
/// Where the tiles start, from the card's left and top.
const TILES_LEFT: f32 = 30.0;
const TILES_TOP: f32 = 66.0;
/// What the card keeps below the last row of tiles: the two hints (their baselines are 32 and
/// 56 px below the tiles) and the bottom margin.
const BELOW_TILES: f32 = 96.0;
/// With no tile at all, the band where the empty text goes (one or two lines of 13 px text).
const EMPTY_AREA: f32 = 48.0;
/// A line of text has its baseline about this share of the font size below the top of its box
/// (the ascent of the system sans-serif fonts is 0.90 to 0.95 of the size).
const ASCENT: f32 = 0.92;

/// The top of a text box whose baseline is `baseline`, to the nearest half pixel. The mockup's
/// SVG gives baselines; the Slint `Text` is placed by its top.
fn top_from_baseline(baseline: f32, font: f32) -> f32 {
    ((baseline - ASCENT * font) * 2.0).round() / 2.0
}

/// The side of the ring's square box: the tab band's outer edge plus one pixel of room for the
/// anti-aliasing at each side, so no edge is cut. The box is centred on the ring's centre.
fn ring_box(radius: f32, band: f32) -> f32 {
    2.0 * ((radius + band / 2.0).ceil() + 1.0)
}

/// The inner measures of one tile of `size` (the mockup's SVG, `keys-overlay-light.svg`, for
/// `Full`; about 70 % of it for `Compact`).
fn tile_metrics(size: TileSize) -> TileMetrics {
    match size {
        TileSize::Full => TileMetrics {
            tile_width: 108.0,
            tile_height: 168.0,
            ring_size: ring_box(20.0, 7.0),
            ring_cx: 54.0,
            ring_cy: 46.0,
            ring_radius: 20.0,
            ring_band: 7.0,
            halo_outer: 47.5,
            halo_width: 22.0,
            key_size: 40.0,
            key_y: 88.0,
            key_radius: 9.0,
            key_font: 20.0,
            // Baselines 148 and 162 in the mockup.
            name_y: top_from_baseline(148.0, 12.5),
            name_font: 12.5,
            count_y: top_from_baseline(162.0, 10.5),
            count_font: 10.5,
        },
        TileSize::Compact => TileMetrics {
            tile_width: 72.0,
            tile_height: 134.0,
            ring_size: ring_box(14.0, 5.0),
            ring_cx: 36.0,
            ring_cy: 33.0,
            ring_radius: 14.0,
            ring_band: 5.0,
            halo_outer: 33.0,
            halo_width: 15.0,
            key_size: 28.0,
            key_y: 70.0,
            key_radius: 7.0,
            key_font: 14.0,
            name_y: top_from_baseline(114.0, 11.0),
            name_font: 11.0,
            count_y: top_from_baseline(126.0, 9.5),
            count_font: 9.5,
        },
    }
}

/// The layout for `tile_count` tiles (counts above [`MAX_TILES`] count as [`MAX_TILES`]).
///
/// - 1 to 6 tiles are `Full` (108 x 168, 10 px apart, one row of up to six); 7 to 26 are `Compact`
///   (72 x 134, 6 px apart and 8 px between rows, nine per row). No tile is `Full` with no rows.
/// - Tiles are left-aligned at x 30 and start at y 66, row by row.
/// - `tiles_bottom` is where the last row ends (`tiles_top + rows * tile_height + (rows - 1) *
///   row gap`), and the card ends 96 px below it: 330 for six full tiles, 296, 438 and 580 for
///   one, two and three compact rows. With no tile there are no rows and the empty text gets a
///   48 px band, so the card is 210 tall.
/// - The ring's box is centred on the ring's centre (`ring_cx`, `ring_cy`): its top-left in the
///   tile is `(ring_cx - ring_size / 2, ring_cy - ring_size / 2)`.
pub fn card_layout(tile_count: usize) -> CardLayout {
    let count = tile_count.min(MAX_TILES);
    let size = if count > MAX_FULL_TILES {
        TileSize::Compact
    } else {
        TileSize::Full
    };
    let (columns, gap_x, gap_y) = match size {
        TileSize::Full => (6, 10.0, 10.0),
        TileSize::Compact => (9, 6.0, 8.0),
    };
    let metrics = tile_metrics(size);
    let rows = count.div_ceil(columns);
    let tiles_bottom = if rows == 0 {
        TILES_TOP + EMPTY_AREA
    } else {
        let rows = rows as f32;
        TILES_TOP + rows * metrics.tile_height + (rows - 1.0) * gap_y
    };
    let positions = (0..count)
        .map(|index| {
            let (column, row) = ((index % columns) as f32, (index / columns) as f32);
            (
                TILES_LEFT + column * (metrics.tile_width + gap_x),
                TILES_TOP + row * (metrics.tile_height + gap_y),
            )
        })
        .collect();
    CardLayout {
        size,
        columns,
        rows,
        card_width: CARD_WIDTH,
        card_height: tiles_bottom + BELOW_TILES,
        tiles_left: TILES_LEFT,
        tiles_top: TILES_TOP,
        tiles_bottom,
        metrics,
        positions,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(layout: &CardLayout, index: usize) -> (f32, f32, f32, f32) {
        let (x, y) = layout.positions[index];
        (
            x,
            y,
            x + layout.metrics.tile_width,
            y + layout.metrics.tile_height,
        )
    }

    fn overlaps(a: (f32, f32, f32, f32), b: (f32, f32, f32, f32)) -> bool {
        a.0 < b.2 && b.0 < a.2 && a.1 < b.3 && b.1 < a.3
    }

    #[test]
    fn six_full_tiles_are_the_mockup() {
        let layout = card_layout(6);
        assert_eq!(layout.size, TileSize::Full);
        assert_eq!((layout.columns, layout.rows), (6, 1));
        assert_eq!((layout.card_width, layout.card_height), (760.0, 330.0));
        assert_eq!((layout.tiles_left, layout.tiles_top), (30.0, 66.0));
        assert_eq!(layout.tiles_bottom, 234.0);
        assert_eq!(layout.metrics.tile_width, 108.0);
        assert_eq!(layout.metrics.tile_height, 168.0);
        let xs: Vec<f32> = layout.positions.iter().map(|p| p.0).collect();
        assert_eq!(xs, vec![30.0, 148.0, 266.0, 384.0, 502.0, 620.0]);
        assert!(layout.positions.iter().all(|p| p.1 == 66.0));
        // The sixth tile ends at x 728, as in the mockup.
        assert_eq!(rect(&layout, 5).2, 728.0);
    }

    #[test]
    fn full_tile_metrics_are_the_mockups_numbers() {
        let m = card_layout(3).metrics;
        assert_eq!((m.ring_cx, m.ring_cy), (54.0, 46.0));
        assert_eq!((m.ring_radius, m.ring_band), (20.0, 7.0));
        assert_eq!((m.halo_outer, m.halo_width), (47.5, 22.0));
        assert_eq!(
            (m.key_size, m.key_y, m.key_radius, m.key_font),
            (40.0, 88.0, 9.0, 20.0)
        );
        assert_eq!((m.name_font, m.count_font), (12.5, 10.5));
        // The keycap is centred in the tile (34 + 40 + 34 = 108).
        assert_eq!((108.0 - m.key_size) / 2.0, 34.0);
    }

    #[test]
    fn compact_tile_metrics_are_the_scaled_down_numbers() {
        let layout = card_layout(7);
        let m = layout.metrics;
        assert_eq!(layout.size, TileSize::Compact);
        assert_eq!((m.tile_width, m.tile_height), (72.0, 134.0));
        assert_eq!((m.ring_cx, m.ring_cy), (36.0, 33.0));
        assert_eq!((m.ring_radius, m.ring_band), (14.0, 5.0));
        assert_eq!((m.halo_outer, m.halo_width), (33.0, 15.0));
        assert_eq!(
            (m.key_size, m.key_y, m.key_radius, m.key_font),
            (28.0, 70.0, 7.0, 14.0)
        );
        assert_eq!((m.name_font, m.count_font), (11.0, 9.5));
        assert_eq!((72.0 - m.key_size) / 2.0, 22.0);
    }

    #[test]
    fn name_and_count_are_top_based_and_follow_the_baselines_of_the_mockup() {
        // Baselines 148 and 162 in the mockup (name 12.5 px, count 10.5 px). The component
        // needs the tops of the text boxes: the baseline sits about 0.92 of the font size below
        // the top (rounded to half a pixel).
        let full = card_layout(1).metrics;
        assert_eq!((full.name_y, full.count_y), (136.5, 152.5));
        for (top, font, baseline) in [
            (full.name_y, full.name_font, 148.0),
            (full.count_y, full.count_font, 162.0),
        ] {
            assert!(
                (top + 0.92 * font - baseline).abs() <= 0.5,
                "{top} {font} {baseline}"
            );
        }
        let compact = card_layout(9).metrics;
        assert_eq!((compact.name_y, compact.count_y), (104.0, 117.5));
        for (top, font, baseline) in [
            (compact.name_y, compact.name_font, 114.0),
            (compact.count_y, compact.count_font, 126.0),
        ] {
            assert!(
                (top + 0.92 * font - baseline).abs() <= 0.5,
                "{top} {font} {baseline}"
            );
        }
    }

    #[test]
    fn inside_a_tile_the_pieces_are_in_order_and_fit() {
        for count in [1, 7] {
            let m = card_layout(count).metrics;
            // Keycap below the ring band, name below the keycap, count below the name.
            assert!(
                m.key_y > m.ring_cy + m.ring_radius + m.ring_band / 2.0,
                "keycap under the ring"
            );
            assert!(m.name_y >= m.key_y + m.key_size, "name under the keycap");
            assert!(m.count_y > m.name_y + m.name_font, "count under the name");
            // A text line is about 1.2 of its font size tall; the last one still fits the tile.
            assert!(
                m.count_y + 1.2 * m.count_font <= m.tile_height,
                "count fits the tile"
            );
            // The keycap is centred and inside the tile.
            assert!(m.key_size < m.tile_width && m.key_y + m.key_size < m.tile_height);
            // The halo fits the tile's width, and pokes out of the top by at most 2 px.
            assert!(m.ring_cx - m.halo_outer >= 0.0 && m.ring_cx + m.halo_outer <= m.tile_width);
            assert!(m.ring_cy - m.halo_outer >= -2.0);
            assert!(m.halo_width > 0.0 && m.halo_width < m.halo_outer);
            // The tab ring sits inside the halo's hole.
            let ring_outer = m.ring_radius + m.ring_band / 2.0;
            assert!(ring_outer < m.halo_outer - m.halo_width);
        }
    }

    #[test]
    fn the_ring_box_holds_the_ring_and_is_square_around_its_centre() {
        for count in [1, 7] {
            let m = card_layout(count).metrics;
            let outer = m.ring_radius + m.ring_band / 2.0;
            assert!(
                m.ring_size >= 2.0 * outer,
                "box {} holds the ring {}",
                m.ring_size,
                outer
            );
            // Not wasteful: at most four pixels of slack around the band.
            assert!(m.ring_size <= 2.0 * outer + 4.0, "box {}", m.ring_size);
            assert!(m.ring_size.is_finite() && m.ring_size > 0.0);
        }
    }

    #[test]
    fn layout_numbers_for_the_counts_of_the_brief() {
        // count, size, columns, rows, card height
        let cases = [
            (1, TileSize::Full, 6, 1, 330.0),
            (6, TileSize::Full, 6, 1, 330.0),
            (7, TileSize::Compact, 9, 1, 296.0),
            (9, TileSize::Compact, 9, 1, 296.0),
            (10, TileSize::Compact, 9, 2, 438.0),
            (18, TileSize::Compact, 9, 2, 438.0),
            (26, TileSize::Compact, 9, 3, 580.0),
        ];
        for (count, size, columns, rows, height) in cases {
            let layout = card_layout(count);
            assert_eq!(layout.size, size, "{count} tiles");
            assert_eq!(layout.columns, columns, "{count} tiles");
            assert_eq!(layout.rows, rows, "{count} tiles");
            assert_eq!(layout.card_width, 760.0, "{count} tiles");
            assert_eq!(layout.card_height, height, "{count} tiles");
            assert_eq!(layout.positions.len(), count, "{count} tiles");
        }
    }

    #[test]
    fn the_card_ends_96_px_below_the_tiles() {
        for count in 0..=MAX_TILES + 3 {
            let layout = card_layout(count);
            assert_eq!(
                layout.card_height,
                layout.tiles_bottom + 96.0,
                "{count} tiles"
            );
            if layout.rows > 0 {
                let m = layout.metrics;
                let gap = if layout.size == TileSize::Full {
                    10.0
                } else {
                    8.0
                };
                let rows = layout.rows as f32;
                assert_eq!(
                    layout.tiles_bottom,
                    layout.tiles_top + rows * m.tile_height + (rows - 1.0) * gap,
                    "{count} tiles"
                );
            }
        }
    }

    #[test]
    fn tiles_are_left_aligned_row_major_with_the_gaps_of_their_size() {
        let full = card_layout(4);
        assert_eq!(full.positions[1].0 - full.positions[0].0, 108.0 + 10.0);
        let compact = card_layout(26);
        // Column pitch 72 + 6, row pitch 134 + 8, nine columns.
        assert_eq!(compact.positions[1].0 - compact.positions[0].0, 78.0);
        assert_eq!(compact.positions[9].1 - compact.positions[0].1, 142.0);
        assert_eq!(
            compact.positions[9].0, compact.positions[0].0,
            "row two starts at the left"
        );
        assert_eq!(compact.positions[0], (30.0, 66.0));
        assert_eq!(compact.positions[8].0, 30.0 + 8.0 * 78.0);
        assert_eq!(compact.positions[17].1, 66.0 + 142.0);
        assert_eq!(
            compact.positions[25],
            (30.0 + 7.0 * 78.0, 66.0 + 2.0 * 142.0)
        );
        // Row-major: x never decreases inside a row and y never decreases overall.
        for pair in compact.positions.windows(2) {
            assert!(pair[1].1 >= pair[0].1);
            if pair[1].1 == pair[0].1 {
                assert!(pair[1].0 > pair[0].0);
            }
        }
    }

    #[test]
    fn every_tile_is_inside_the_card_and_none_overlap() {
        for count in 0..=MAX_TILES + 3 {
            let layout = card_layout(count);
            let shown = count.min(MAX_TILES);
            assert_eq!(layout.positions.len(), shown, "{count} tiles");
            for index in 0..shown {
                let (left, top, right, bottom) = rect(&layout, index);
                assert!(left >= layout.tiles_left, "{count}/{index}");
                assert!(
                    right <= layout.card_width - 30.0 + 0.001,
                    "{count}/{index} right {right}"
                );
                assert!(top >= layout.tiles_top, "{count}/{index}");
                assert!(bottom <= layout.tiles_bottom, "{count}/{index}");
                assert!(
                    bottom + 96.0 <= layout.card_height + 0.001,
                    "{count}/{index}"
                );
                for other in 0..index {
                    assert!(
                        !overlaps(rect(&layout, index), rect(&layout, other)),
                        "{count} tiles: {index} overlaps {other}"
                    );
                }
            }
        }
    }

    #[test]
    fn the_last_row_of_26_tiles_ends_at_tiles_bottom() {
        let layout = card_layout(26);
        assert_eq!(rect(&layout, 25).3, layout.tiles_bottom);
        assert_eq!(layout.tiles_bottom, 484.0);
    }

    #[test]
    fn no_tiles_means_no_rows_and_a_card_that_still_has_room_for_the_empty_text() {
        let layout = card_layout(0);
        assert_eq!(layout.rows, 0);
        assert!(layout.positions.is_empty());
        assert_eq!(layout.size, TileSize::Full);
        assert_eq!(layout.card_width, 760.0);
        assert_eq!((layout.tiles_left, layout.tiles_top), (30.0, 66.0));
        // The empty text takes a 48 px band where the tiles would start.
        assert_eq!(layout.tiles_bottom, 66.0 + 48.0);
        assert_eq!(layout.card_height, 66.0 + 48.0 + 96.0);
    }

    #[test]
    fn more_than_26_tiles_count_as_26() {
        let most = card_layout(MAX_TILES);
        for count in [27, 100, usize::MAX] {
            assert_eq!(card_layout(count), most, "{count} tiles");
        }
    }

    #[test]
    fn every_number_is_finite_and_positive() {
        for count in 0..=MAX_TILES {
            let layout = card_layout(count);
            let m = layout.metrics;
            for value in [
                m.tile_width,
                m.tile_height,
                m.ring_size,
                m.ring_cx,
                m.ring_cy,
                m.ring_radius,
                m.ring_band,
                m.halo_outer,
                m.halo_width,
                m.key_size,
                m.key_y,
                m.key_radius,
                m.key_font,
                m.name_y,
                m.name_font,
                m.count_y,
                m.count_font,
                layout.card_width,
                layout.card_height,
                layout.tiles_left,
                layout.tiles_top,
                layout.tiles_bottom,
            ] {
                assert!(value.is_finite() && value > 0.0, "{count} tiles: {value}");
            }
        }
    }
}
