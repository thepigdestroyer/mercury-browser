// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! The card's models for Slint: the one file of the card's Rust side that names generated Slint
//! types (lane brief §6 item 3). The rings come from [`crate::ring_geometry`]
//! (`RingSpec::planet`, `RingModel`); the layout and words from the sibling modules.
//!
//! GROUNDWORK STUB. The types and signatures below are the frozen contract; item 3 `card-model`
//! writes the body. Remove this paragraph when you do.

use crate::key_switcher::layout::CardLayout;
use crate::key_switcher::text::HintMode;
use crate::{GroupRow, KeyChip, KeyMetrics, KeyTile, TabRow};

/// The most segments one tile's ring draws. A group with more tabs shows its first 39 and the
/// active tab as the 40th when it lies beyond.
pub const MAX_RING_TABS: usize = 40;

/// What the card is built from.
#[derive(Clone, Copy, Debug)]
pub struct CardInput<'a> {
    /// The window's groups in ring order; a group whose `key-label` is empty is not a tile.
    pub groups: &'a [GroupRow],
    /// Every tab of those groups (`TabRow.group` is a `GroupRow.id`).
    pub tabs: &'a [TabRow],
    /// The hold setting in milliseconds, for the second hint.
    pub hold_ms: u16,
    /// The capture mode, for the second hint.
    pub mode: HintMode,
    /// The chosen group's `GroupRow.id`, if any.
    pub selected_group: Option<i32>,
    /// The chosen tab as a digit `1..=9`, if any.
    pub selected_tab: Option<u8>,
}

/// Everything the `KeySwitcher` component shows, ready to set.
#[derive(Clone, Debug)]
pub struct CardModel {
    /// One tile per keyed group, in order, at most 26.
    pub tiles: Vec<KeyTile>,
    /// The numbered tab chips of the chosen group (at most 9), empty without a chosen group.
    pub chips: Vec<KeyChip>,
    /// The tile measures for the component.
    pub metrics: KeyMetrics,
    /// The layout the model was built with.
    pub layout: CardLayout,
    /// The first hint.
    pub hint_primary: String,
    /// The second hint.
    pub hint_secondary: String,
    /// The text for an empty tile area.
    pub empty_text: String,
    /// The chosen group's `id`, or -1.
    pub selected_group: i32,
    /// The chosen digit, or -1.
    pub selected_tab: i32,
    /// The chosen group's name, or empty.
    pub chosen_name: String,
}

/// Builds the card's models from the rows (brief §6 item 3 has the rules).
pub fn build_card(_input: &CardInput) -> CardModel {
    todo!("key-switcher item 3: build_card")
}
