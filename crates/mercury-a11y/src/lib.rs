// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! Mercury's page accessibility logic, without Servo.
//!
//! Everything here is a function over a DOM-shaped trait ([`a11y_dom::Document`] and
//! [`a11y_dom::Rendering`], plus [`HostState`] for what only the engine knows), so it builds and
//! tests in minutes and never needs a browser engine. The layout crate (`servo-layout`) implements
//! the traits for its own DOM and copies the result of [`compute`] onto its accessibility nodes.
//!
//! Module owners in the a11y-1 lane (`.superpowers/sdd/lane-a11y-1-brief.md`): `roles`, `names` and
//! `testing` item 2; `states`, `relations`, `actions`, `host`, `snapshot`, `golden` item 3;
//! `bridge` and `graft` item 1; `semantics` (the orchestration) is shared and only grows through the pinned
//! entry points.

pub mod actions;
pub mod bridge;
pub mod golden;
pub mod graft;
pub mod host;
pub mod names;
pub mod relations;
pub mod roles;
pub mod semantics;
pub mod snapshot;
pub mod states;
pub mod testing;

pub use host::{HostState, NativeFlags, RangeValue};
pub use semantics::{NodeSemantics, Relations, compute};
