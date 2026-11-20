// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! The words on the "Switch group" card (lane brief §3 and §6 item 3), and the accessible labels
//! that say what a tile or a chip does. No Slint types.
//!
//! GROUNDWORK STUB. The signatures below are the frozen contract; item 3 `card-model` writes the
//! bodies. Remove this paragraph when you do.

/// Which capture mode the second hint explains (the caller maps `mercury_settings::KeyCapture`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HintMode {
    /// Outside text fields at once, inside them only after the hold.
    OutsideTextOrHold,
    /// Outside text fields only.
    OutsideText,
    /// Everywhere.
    Always,
}

/// The hold as the card says it: 400 gives "0.4 s", 250 "0.25 s", 1000 "1 s", 650 "0.65 s". The
/// value is clamped to 250..=1000 first; no trailing zero or dot.
pub fn hold_text(_hold_ms: u16) -> String {
    todo!("key-switcher item 3: hold_text")
}

/// "1 tab", "5 tabs", "0 tabs".
pub fn count_text(_tabs: usize) -> String {
    todo!("key-switcher item 3: count_text")
}

/// A tile's accessible label: "Option A, Research, 5 tabs".
pub fn tile_label(_key_label: &str, _name: &str, _tabs: usize) -> String {
    todo!("key-switcher item 3: tile_label")
}

/// A tab chip's accessible label: "Tab 2, WebRender notes" (`digit` is 1-based).
pub fn chip_label(_digit: usize, _title: &str) -> String {
    todo!("key-switcher item 3: chip_label")
}

/// The first hint, verbatim from the mockup: "Tap a letter for a group, then 1–9 for one of its
/// tabs. Release ⌥ to go there."
pub fn hint_primary() -> &'static str {
    todo!("key-switcher item 3: hint_primary")
}

/// The second hint for a mode (brief §6 item 3 has the three texts; the first is the mockup's
/// with `hold_text`).
pub fn hint_secondary(_mode: HintMode, _hold_ms: u16) -> String {
    todo!("key-switcher item 3: hint_secondary")
}

/// What the tile area says when no group has a key.
pub fn empty_text() -> &'static str {
    todo!("key-switcher item 3: empty_text")
}
