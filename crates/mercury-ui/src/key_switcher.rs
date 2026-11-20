// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! The "Switch group" card's Rust side (design/ui-final/SPEC.md §4.7; lane brief
//! `lane-key-switcher-brief.md` §6 item 3). Slint only draws what it is given, so everything the
//! card shows is worked out here:
//!
//! | Module | Content |
//! | --- | --- |
//! | [`layout`] | tile sizes, rows and columns, positions and the card's height (no Slint types) |
//! | [`text`] | the card's words: hold seconds, counts, hints, accessible labels (no Slint types) |
//! | [`tiles`] | [`build_card`]: group and tab rows to the `KeyTile` and `KeyChip` models, with the ring shapes from [`crate::ring_geometry`] |
//!
//! The state machine that decides when the card shows and what is chosen is the crate
//! `mercury-option-keys`; this crate knows nothing of it.

pub mod layout;
pub mod text;
pub mod tiles;

pub use layout::{CardLayout, MAX_TILES, TileMetrics, TileSize, card_layout};
pub use text::{
    HintMode, chip_label, count_text, empty_text, hint_primary, hint_secondary, hold_text,
    tile_label,
};
pub use tiles::{CardInput, CardModel, MAX_RING_TABS, build_card};
