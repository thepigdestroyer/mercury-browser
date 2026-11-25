// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! The group dock of SPEC `design/ui-final/SPEC.md` §4.4: the pure Rust behind `GroupDock`,
//! `GroupDockBar` and `DockCapsule` (`ui/groups/dock.slint`, `dock-capsule.slint`): the reserved
//! band and the settle rule, the dock data (pill widths, fit, magnified frame, hit test), the
//! capsule's sliver fit and the persistent models.
//!
//! **The public items below are the pinned contract of lane group-dock**
//! (`.superpowers/sdd/lane-group-dock-brief.md` §3.2). Other lanes build on them; only additions are
//! allowed. The scaffold commit has the final signatures with stub bodies (marked `STUB`); the
//! lane's items fill them in.
//!
//! # Using it (the pattern of the window that hosts the dock)
//!
//! 1. Own one [`DockModels`] and set its `ModelRc`s on the dock once; own one [`DockState`].
//! 2. On every change of rows, theme (the colours are in the rows), container width or ⌥ state:
//!    [`DockData::new`], then [`DockData::frame`] with `None` and [`DockModels::update`].
//! 3. On every pointer move over the bar: `frame(Some(window_x))` (one `magnified_layout` call, from the
//!    resting layout) and `update`; set the dock's `dock-x`, `dock-width`, `dock-height`, `new-tab-x`,
//!    `shrink-x` and `hovered-tab` (`frame.hovered_tab`, -1 for none) from the frame, and its
//!    tooltip from [`DockData::tooltip_for`]. Keep the last frame: `on_hit` calls [`DockData::hit`]
//!    with it. The dock is bottom-anchored: give it any height, its `hit` and `pointer-moved` y are
//!    measured in a box `DockGeometry.box-height` (77.5 px) tall with the same bottom edge.
//! 4. On "shrink" or "expand": [`DockState::toggle`], then call [`DockState::tick`] from a
//!    `slint::Timer` after [`SHRINK_MS`]; when it returns `Some(band)` resize the web view, once.
//!    The dock itself follows [`DockState::shrunk`] at once. If nothing calls `tick` the web view
//!    keeps the old band forever.
//! 5. Call `ring_bands::metrics()` once and keep the result for `DockData::new`.

mod band;
mod capsule;
mod data;
mod fit;
mod models;

pub use band::{
    BAND_OPEN, BAND_SHRUNK, BOTTOM_MARGIN, CAPSULE_HEIGHT, CAPSULE_WIDTH, DOCK_HEIGHT, DockState,
    PAD_Y, PILL_HEIGHT, SHRINK_MS, dock_height, initial_shrunk, page_height, reserved_band_height,
};
pub use capsule::{
    CHEVRON_X, COUNT_WIDTH, COUNT_X, CapsuleFrame, MIN_SLIVER_WIDTH, SLIVER_GAP, SLIVER_LEFT,
    SLIVER_RIGHT, SLIVER_WIDTH, capsule_frame, capsule_label,
};
pub use data::{DockData, DockFrame, DockOptions};
pub use models::DockModels;
