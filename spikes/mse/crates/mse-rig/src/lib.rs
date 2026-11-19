// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! The GstMSE test rig, shared by `mse-conformance` and `mse-stress`.
//!
//! OWNER: item 1 of lane sp-mse (see `.superpowers/sdd/lane-sp-mse-brief.md`). The public names and
//! signatures in this crate are the contract the other crates compile against: add what you need,
//! rename or remove nothing, and keep `cargo check` green at every step. Bodies marked `todo!` are
//! filled in by item 1.
//!
//! * [`env`]: a private GStreamer 1.28.9 (the bundled framework), set up the way Mercury sets it up.
//! * [`backend`]: a thin, GStreamer-neutral wrapper over `gstreamer-mse`. This is the sketch of what
//!   MED-1 will put behind servo-media's traits; no `gstreamer_mse` type appears in its public API.
//! * [`events`]: every callback GstMSE and GstPlay make, with the thread it came on and when.
//! * [`frames`]: counting sinks (frames, timestamps, caps changes, colour) for video and audio.
//! * [`rig`]: the two pipeline shapes under test.
//! * [`harness`]: the scenario runner and command line both binaries share (each scenario in a child process).
//! * [`report`]: the JSON the scenario binaries print.

pub mod backend;
pub mod env;
pub mod events;
pub mod frames;
pub mod harness;
pub mod report;
pub mod rig;

pub use gstreamer::ClockTime;

/// The error type of the whole rig.
#[derive(Debug, thiserror::Error)]
pub enum RigError {
    #[error("GStreamer set-up failed: {0}")]
    Setup(String),
    #[error("MSE call refused: {0:?}")]
    Mse(backend::MseError),
    #[error("pipeline: {0}")]
    Pipeline(String),
    #[error("timed out after {0:?} waiting for {1}")]
    Timeout(std::time::Duration, String),
}

impl From<backend::MseError> for RigError {
    fn from(error: backend::MseError) -> Self {
        RigError::Mse(error)
    }
}
