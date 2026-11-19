// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! Route L of the WebRTC spike: libwebrtc through LiveKit's Rust bindings (`libwebrtc` 0.4.1 over
//! `webrtc-sys` 0.3.54) on the pinned Shiguredo prebuilt.
//!
//! - [`sinks`]: [`LiveKitVideoSink`] and [`LiveKitAudioSink`], `MediaFrameSink` implementations
//!   that push the frames of a capture stream into a `NativeVideoSource` / `NativeAudioSource`.
//! - [`loopback`]: two peer connections in one process, with trickled ICE forwarded between them
//!   and no ICE server, so every test of `tests/calls.rs` can connect without a network service.
//!
//! Nothing here opens a device. The factory's audio device module is the synthetic one (see
//! `R18-parts/3-livekit.md`, E5), and the tests never call the platform audio functions.

pub mod loopback;
pub mod sinks;

pub use libwebrtc;
pub use sinks::{LiveKitAudioSink, LiveKitVideoSink, SinkCounters, consumer_now};
