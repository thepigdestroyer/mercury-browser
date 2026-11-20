// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! The pure geometry behind the tab-group interface (SPEC `design/ui-final/SPEC.md` §4.9): ring
//! angles, arc path strings, hit-testing, curved labels, dock magnification and the Orbit layouts.
//! Servo-free and Slint-free in its logic, so every function is unit-tested without a window.
//!
//! # Conventions (frozen; every submodule follows them)
//!
//! - Lengths are logical pixels (`f32`). Positions are **ring-local**: the origin is the top-left
//!   corner of the component's square box and y grows downwards. `RingSpec::centre` is where the
//!   ring's centre sits in that box.
//! - Angles are degrees **clockwise from 12 o'clock** (0 = top, 90 = 3 o'clock, 180 = bottom).
//!   Extents (`TabArc`, `GroupArc`) are unwrapped: `start < end`, `end - start <= 360`, and they
//!   may exceed 360 when a ring starts late. Pointer angles are normalised to `[0, 360)` and
//!   compared modulo 360.
//! - Everything is deterministic and total: no panics, no NaN, no negative widths for any input
//!   (zero groups, zero tabs, 64 groups, 500 tabs).
//!
//! # Who owns what
//!
//! | File | Content |
//! | --- | --- |
//! | `ring_geometry/types.rs` | shared plain types; **frozen**, change only through the integrator |
//! | `ring_geometry/layout.rs` | `RingLayout::new`: angles of every tab and group; `RingSpec` presets |
//! | `ring_geometry/paths.rs` | SVG path strings for segments and label bands |
//! | `ring_geometry/hit.rs` | `hit_test` |
//! | `ring_geometry/labels.rs` | curved label glyphs: measuring, shortening, flipping |
//! | `ring_geometry/dock.rs` | `dock_scales_over`, `dock_scales`, `dock_layout`, `magnified_layout` |
//! | `ring_geometry/orbit.rs` | `planet_layout`, `preview_layout` |
//! | `ring_geometry/slint_models.rs` | conversions to the Slint structs of `ui/groups/models.slint` |
//!
//! # Entry points for the components
//!
//! One frame of a ring, in the order the components use it:
//!
//! 1. [`slint_models::RingModel::new`] orders the `GroupRow`s and `TabRow`s into a [`RingLayout`]
//!    (or call [`RingLayout::new`] directly with the tab count of every group). Presets:
//!    [`RingSpec::hub`], [`RingSpec::dial_fan`], [`RingSpec::planet`].
//! 2. [`slint_models::RingModel::segments_for_slint`] and
//!    [`slint_models::RingModel::label_bands_for_slint`] give the filled shapes
//!    ([`paths::segments`], [`paths::label_bands`], [`paths::lifted_segment`] for the hovered tab).
//!    The path commands are ring-local: the Slint `Path` must set `viewbox-width` and
//!    `viewbox-height` to the size of the ring's box so that the commands are not rescaled.
//! 3. [`label_glyphs`] places one group's curved label (`ApproxMetrics` is an estimate;
//!    implement [`GlyphMetrics`] with measured advances to replace it) and
//!    [`slint_models::glyph_to_slint`] turns each glyph into an `ArcGlyph` with the palette colours.
//! 4. [`hit_test`] answers the pointer: pass the hovered tab as `lifted` so that its raised zone
//!    does not flicker. [`hit::hit_test_polar`] takes a radius and angle for keyboard callers.
//!
//! The group dock uses [`magnified_layout`] with the pointer x measured from the resting layout's
//! left edge ([`resting_left_edge`] of the container width, never the width of the dock as
//! drawn). It magnifies the pill whose resting extent contains the pointer, so a wide pill keeps
//! its whole width. Without it: [`dock_scales_over`] on the `xs` and `widths` of
//! `dock_layout(widths, &[], gap)`, then [`dock_layout`]. [`dock_scales`] on [`unscaled_centres`]
//! picks the nearest centre, which is the pill under the pointer only for equal widths. Orbit
//! uses [`planet_layout`] and [`preview_layout`].

pub mod dock;
pub mod hit;
pub mod labels;
pub mod layout;
pub mod orbit;
pub mod paths;
pub mod slint_models;
mod types;

pub use dock::{
    DockLayout, dock_layout, dock_scales, dock_scales_over, magnified_layout, resting_left_edge,
    unscaled_centres,
};
pub use hit::hit_test;
pub use labels::{
    ApproxMetrics, GlyphMetrics, LabelFit, LabelLayout, LabelRequest, LabelStyle, is_flipped,
    label_glyphs,
};
pub use orbit::{PlanetLayout, planet_layout, preview_layout};
pub use types::{
    ArcSegment, Ellipse, GlyphPlacement, GlyphRole, GroupArc, Hit, Point, RingLayout, RingSpec,
    TabArc, normalise_deg, point_on_circle,
};
