// SPDX-License-Identifier: MPL-2.0
slint::include_modules!();

// tabs-plus
mod tab_rows;
pub use tab_rows::{chip_offsets, drop_index, run_starts, update_tab_rows};

// ring-geometry
pub mod ring_geometry;

// ring-bands
pub mod ring_bands;

// corner-dial
pub mod corner_dial;

// group-dock
pub mod group_dock;

// hub-ring
pub mod hub_ring;

// theme-plumbing
mod theme;
pub use theme::{Look, apply_look};

// key-switcher
pub mod key_switcher;

// settings-ui: the Appearance and Tab groups sections of the settings window
pub mod settings_sections;

// layout-switch: the layouts, the plan and the feed of the tab-group overlays
pub mod window_layout;
