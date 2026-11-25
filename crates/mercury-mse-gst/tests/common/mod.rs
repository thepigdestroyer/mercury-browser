// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! The shared rig of the playback tests: a private GStreamer, fixture frames, a strict fake feed,
//! counting sinks and the two pipeline shapes. See `README` at the top of each file.

#![allow(dead_code)]

pub mod env;
pub mod fakefeed;
pub mod frames;
pub mod real;
pub mod rig;
pub mod scenario;
pub mod sinks;

/// Skips the test (returns) when an element is missing from the registry: `SKIP: <element> missing`.
#[macro_export]
macro_rules! require {
    ($($element:expr),+ $(,)?) => {
        if !$crate::common::env::have(&[$($element),+]) {
            return;
        }
    };
}
