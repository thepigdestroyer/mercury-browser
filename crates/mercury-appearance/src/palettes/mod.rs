// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! The palettes Mercury ships, as data.
//!
//! - [`mercury_b2`]: `mercury-b2`, the default look, selectable.
//! - [`hermes`]: `hermes`, the chrome's colours from before B2, **not selectable**: a regression
//!   baseline for the theme-plumbing pixel-diff gate, to be deleted afterwards.
//!
//! Only these two exist. The other built-ins R12 sketched (Graphite, Paper, Ink, follow the macOS
//! accent) are not built: the owner chose B2, and every extra palette is unreviewed design. Adding
//! one is a new module with two exhaustive tables, an entry in `ALL` below, and a run of
//! `cargo test -p mercury-appearance`, whose contrast rules every selectable palette must pass.

pub mod candidates;
pub mod hermes;
pub mod mercury_b2;

use crate::palette::Palette;

/// Every palette, the default first.
static ALL: [Palette; 2] = [mercury_b2::PALETTE, hermes::PALETTE];

/// Every palette, including the ones a user cannot choose. The default is first.
pub fn all() -> &'static [Palette] {
    &ALL
}

/// The palettes a user may choose, in [`all`] order.
pub fn selectable() -> impl Iterator<Item = &'static Palette> {
    ALL.iter().filter(|palette| palette.selectable)
}

/// The palette with this id, selectable or not, or `None`.
///
/// This is for code that needs a palette by name, such as the pixel gate asking for `hermes`.
/// Code that resolves an id a user stored should use [`find_selectable`], so that a
/// hand-edited `settings.json` cannot switch the legacy palette on.
pub fn find(id: &str) -> Option<&'static Palette> {
    ALL.iter().find(|palette| palette.id == id)
}

/// The selectable palette with this id, or `None` when there is none or it cannot be chosen.
pub fn find_selectable(id: &str) -> Option<&'static Palette> {
    find(id).filter(|palette| palette.selectable)
}

/// The palette of a new or upgraded profile: [`mercury_b2`]. Its id is
/// [`mercury_settings::DEFAULT_PALETTE`], which `tests/palettes.rs` checks.
pub fn default_palette() -> &'static Palette {
    &ALL[0]
}
