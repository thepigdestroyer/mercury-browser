// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! The model behind Mercury's protections (R13 W1-3: MH12, MH13, MH14, MH15, MH31, MH32, MH49, MH56).
//!
//! Nothing here touches the engine or a window. The crate decides; lane W2-2 (protections UI and wiring) asks:
//!
//! * [`ProtectionSettings`] and [`ExceptionList`] resolve to an [`Effective`] table for a top-level site
//!   ([`ProtectionSettings::resolve`]); the content blocker, the net policy and the shield read it.
//! * [`TabProtection`] counts what was blocked per tab; [`WeeklyTally`] counts it per day; [`panel_view`] shapes
//!   both for the shield panel.
//! * [`lna`] classifies target addresses for local-network-access protection ([`lna_decision`]).
//! * [`BounceTracker`] is the bounce-tracking state machine; it never reads the clock.
//! * [`round_geometry`] rounds the screen geometry Mercury reports to pages.
//! * [`CompatList`] is the web-compatibility exception list.
//! * [`persist`] stores exceptions, the tally and bounce records through `mercury-store`.
//!
//! Every time-dependent method takes `now_ms`, milliseconds since the Unix epoch, as `mercury-store` does.

#![warn(missing_docs)]
#![cfg_attr(
    not(test),
    deny(clippy::unwrap_used, clippy::expect_used, clippy::panic)
)]

pub mod bounce;
pub mod compat;
pub mod counters;
pub mod effective;
pub mod exceptions;
pub mod geometry;
pub mod level;
pub mod lna;
pub mod panel;
pub mod persist;
pub mod settings;
pub mod site;
pub mod tally;

pub use bounce::*;
pub use compat::*;
pub use counters::*;
pub use effective::*;
pub use exceptions::*;
pub use geometry::*;
pub use level::*;
pub use lna::*;
pub use panel::*;
pub use settings::*;
pub use site::*;
pub use tally::*;
