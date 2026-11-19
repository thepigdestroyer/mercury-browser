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
//! | `ring_geometry/dock.rs` | `dock_scales`, `dock_layout` |
//! | `ring_geometry/orbit.rs` | `planet_layout`, `preview_layout` |
//! | `ring_geometry/slint_models.rs` | conversions to the Slint structs of `ui/groups/models.slint` |

pub mod dock;
pub mod hit;
pub mod labels;
pub mod layout;
pub mod orbit;
pub mod paths;
pub mod slint_models;
mod types;

pub use types::{
    ArcSegment, Ellipse, GlyphPlacement, GlyphRole, GroupArc, Hit, Point, RingLayout, RingSpec,
    TabArc, normalise_deg, point_on_circle,
};
