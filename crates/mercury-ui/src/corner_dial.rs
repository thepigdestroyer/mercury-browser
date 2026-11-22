// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! The optional corner dial of the tab-group interface (SPEC `design/ui-final/SPEC.md` §4.5): the
//! round button parked in a corner of the page area that fans out into a quarter ring of tab
//! segments. `CornerDial` (`ui/groups/corner-dial.slint`) draws it; this module is everything the
//! drawing must not decide: where the dial sits, which corner a release snaps to, the fan's
//! `RingSpec` and data, the caption, the accessible label and the keyboard stepping. Every function
//! is pure and tested without a window.
//!
//! **The public items below are the pinned contract of lane corner-dial**
//! (`.superpowers/sdd/lane-corner-dial-brief.md` §6). Other lanes build on them; only additions are
//! allowed.
//!
//! # Using it (the pattern of the host)
//!
//! 1. Own one [`crate::ring_bands::RingBandsModels`] and set its `ModelRc`s on the dial once.
//! 2. Keep a [`DialGeometry`] of the overlay area (the page area; update it on resize).
//! 3. On every change of rows, theme, corner or area: [`DialFan::new`] and
//!    `RingBandsModels::update(&fan.data)`.
//! 4. Set `on_place`, `on_clamp_drag`, `on_snap_corner` and `on_step_focus` once (they call
//!    [`DialGeometry::centre`], [`DialGeometry::clamp_drag`], [`DialGeometry::snap`] and
//!    [`step_focus`]); `on_hit_test` and `on_drop_target` call `fan.data.hit` and `.drop_target`.
//! 5. On `corner-changed` save [`corner_name`] as the window's `dial_corner`; on `tab-hovered` **and
//!    on `focus-moved`** (the arrow keys) set the caption from [`DialFan::caption`] for that tab id (the
//!    component shows it while a tab is hovered or has the keyboard focus). The corner at start is
//!    [`effective_corner`].
//!
//! # Who owns `expanded`
//!
//! `CornerDial.expanded` is an in-out property that the component also changes itself (a click,
//! Space, Esc, a drag, `suppressed`), always followed by `toggled(bool)` (or `close-requested()` for
//! Esc and a press outside). A one-way binding (`expanded: state.open`) is therefore replaced by the
//! first of those. Either bind it two ways (`expanded <=> state.open`) or keep the host's state in
//! step through `toggled` and `close-requested`, and set `expanded` from outside (⌘⇧E) as a plain
//! assignment: the fan then takes the keyboard focus, so Esc and the arrows work at once.

mod fan;
mod geometry;
mod state;

pub use fan::DialFan;
pub use geometry::{DialGeometry, fan_spec, fan_start_deg};
pub use state::{
    corner_from_name, corner_from_slint, corner_name, corner_to_slint, dial_label,
    effective_corner, step_focus,
};

/// Diameter of the dial at rest (SPEC §4.5).
pub const DIAL_DIAMETER: f32 = 72.0;
/// The gap between the dial's edge and the edge of the overlay area (the mockup: 38 px, so the
/// centre is 74 px from each edge).
pub const DIAL_MARGIN: f32 = 38.0;
/// Pointer movement with the primary button down before a press on the dial becomes a drag.
pub const DRAG_THRESHOLD: f32 = 6.0;
/// The outer edge of the fan's label band from the dial's centre (`RingSpec::dial_fan`: radius 340,
/// band 28, label gap 2, label band 24).
pub const FAN_REACH: f32 = 380.0;
/// The edge of the square `RingBands` box that holds the fan, centred on the dial.
pub const FAN_BOX: f32 = 2.0 * (FAN_REACH + 4.0);
/// The widest the hover caption gets: its text is elided beyond this. [`DialFan::caption`] places the
/// box with this width and hands it to the component in `DialCaption.max-width`, so Rust and Slint
/// have one number. (The box is as narrow as its text, up to this; 260 keeps the longest box clear of
/// the dial for the first tab of any fan.)
pub const CAPTION_MAX_WIDTH: f32 = 260.0;
/// The height of the hover caption (`DialMetrics.caption-height` in `corner-dial.slint`).
pub const CAPTION_HEIGHT: f32 = 30.0;
