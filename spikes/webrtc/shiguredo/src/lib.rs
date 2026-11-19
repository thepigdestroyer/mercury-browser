// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! Route S of the libwebrtc spike: what Mercury would write on top of `shiguredo_webrtc` (a thin C
//! wrapper over libwebrtc's classes, 1:1 Rust types) to make calls with it.
//!
//! The crate gives callbacks and raw handles; everything below turns them into something a test
//! (and, later, `mercury-webrtc`) can use. It is the line count that route S would add to MED-2:
//!
//! * [`engine`]: threads, the audio device, codec factories, the peer connection factory.
//! * [`peer`]: observers to shared state, offer/answer and ICE through channels, candidate relay,
//!   stats as JSON.
//! * [`channel`]: data channels with an inbox.
//! * [`audio`]: the virtual audio device (the only way this crate can push or pull audio) and a
//!   meter for remote audio.
//! * [`video`]: the pushed video source and a meter for remote video.
//! * [`sinks`]: `MediaFrameSink` adapters, the contract with the capture streams of the
//!   media-capture lane (`mercury-frame-sink`).
//!
//! No part of this crate opens a camera, a microphone or a speaker: the audio device module is
//! either libwebrtc's `Dummy` layer or the virtual device of [`audio`], and nothing else is
//! created. See `engine::Engine::new`.

#![deny(unsafe_code)]

pub mod audio;
pub mod channel;
pub mod engine;
pub mod peer;
pub mod sinks;
pub mod video;

pub use shiguredo_webrtc as rtc;

/// Every fallible call of the crate returns this (the binding crate's own error type).
pub type Result<T> = shiguredo_webrtc::Result<T>;

/// An error with a message, for failures that are ours and not libwebrtc's.
pub fn error(message: impl Into<String>) -> shiguredo_webrtc::Error {
    shiguredo_webrtc::Error::Message(message.into())
}
