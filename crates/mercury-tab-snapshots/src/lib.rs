// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! Small pictures of tabs (snapshots) for the Orbit view, kept in memory only.
//!
//! The pieces, each in its own module so that they can be written and tested apart:
//!
//! - [`frame`]: [`RgbaFrame`], a checked RGBA8 picture, rows from top to bottom;
//! - [`downscale`]: [`downscale()`] shrinks a frame to fit [`Limits`] (480 by 300 by default);
//! - [`store`]: [`SnapshotStore`], a least-recently-used store with a memory cap (128 MB by
//!   default);
//! - [`schedule`]: [`CaptureScheduler`] decides when a picture is taken (when a tab stops being
//!   the active one, every 30 seconds while it is, and on demand) and refuses a capture that could
//!   attach one tab's picture to another;
//! - [`registry`]: [`TabSnapshots`] puts the three together and is what the desktop app holds.
//!
//! Time is always passed in as a [`std::time::Duration`] since an arbitrary start (the app uses
//! the time since it began). Nothing here reads a clock, touches the disk or names a tab type: the
//! key is any `Copy + Eq + Hash` type (the app uses its `TabId`).

#![forbid(unsafe_code)]

pub mod downscale;
pub mod frame;
pub mod registry;
pub mod schedule;
pub mod store;

pub use downscale::{Limits, MAX_HEIGHT, MAX_WIDTH, downscale, fit_within};
pub use frame::{FrameError, MAX_DIMENSION, RgbaFrame};
pub use registry::{FinishOutcome, Stats, TabSnapshots};
pub use schedule::{
    CaptureReason, CaptureRequest, CaptureScheduler, FAILURE_BACKOFF, IN_FLIGHT_TIMEOUT,
    MAX_CONSECUTIVE_FAILURES, MIN_REPEAT_GAP, PERIODIC_INTERVAL, Skip,
};
pub use store::{DEFAULT_CAP_BYTES, ENTRY_OVERHEAD_BYTES, InsertOutcome, Snapshot, SnapshotStore};
