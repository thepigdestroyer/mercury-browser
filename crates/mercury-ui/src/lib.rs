// SPDX-License-Identifier: MPL-2.0
slint::include_modules!();

// tabs-plus
mod tab_rows;
pub use tab_rows::update_tab_rows;

// ring-geometry
pub mod ring_geometry;

// key-switcher
pub mod key_switcher;

// ring-bands
pub mod ring_bands;

// orbit-view
pub mod orbit_view;

// theme-plumbing
mod theme;
pub use theme::{Look, apply_look};
