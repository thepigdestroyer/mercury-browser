// SPDX-License-Identifier: MPL-2.0
slint::include_modules!();

// tabs-plus
mod tab_rows;
pub use tab_rows::update_tab_rows;

// ring-geometry
pub mod ring_geometry;

// settings-ui: the Appearance and Tab groups sections of the settings window
pub mod settings_sections;
