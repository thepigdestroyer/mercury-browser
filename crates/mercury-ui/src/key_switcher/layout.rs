// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! Where everything on the "Switch group" card sits (lane brief §3 and §6 item 3). Plain `f32`
//! logical pixels, no Slint types. All positions are from the card's top-left corner, tiles'
//! inner positions from the tile's top-left corner.
//!
//! GROUNDWORK STUB. The types and signatures below are the frozen contract; item 3 `card-model`
//! writes the body. Remove this paragraph when you do.

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

/// The layout for `tile_count` tiles (counts above [`MAX_TILES`] count as [`MAX_TILES`]).
pub fn card_layout(_tile_count: usize) -> CardLayout {
    todo!("key-switcher item 3: card_layout")
}
