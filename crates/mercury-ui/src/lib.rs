// SPDX-License-Identifier: MPL-2.0
slint::include_modules!();

// tabs-plus
mod tab_rows;
pub use tab_rows::update_tab_rows;

// ring-geometry
pub mod ring_geometry;

// ring-bands
pub mod ring_bands;

// corner-dial
pub mod corner_dial;

// theme-plumbing
mod theme;
pub use theme::{Look, apply_look};
