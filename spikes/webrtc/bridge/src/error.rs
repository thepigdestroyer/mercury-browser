// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! The one error type of the bridge.

use mercury_frame_sink::FrameError;

/// Why a bridge call failed.
#[derive(Debug, thiserror::Error)]
pub enum BridgeError {
    /// GStreamer could not be initialised (no installation, or the plugin scanner failed).
    #[error("GStreamer could not be initialised: {0}")]
    Init(String),
    /// A GStreamer element the pipeline needs is not installed.
    #[error("the GStreamer element `{0}` is not installed")]
    MissingElement(&'static str),
    /// Building, adding or linking elements failed.
    #[error("the pipeline could not be built: {0}")]
    Pipeline(String),
    /// A state change was refused by the pipeline.
    #[error("the pipeline refused a state change: {0}")]
    StateChange(String),
    /// A configuration value is outside what the bridge supports.
    #[error("invalid configuration: {0}")]
    Config(String),
    /// The frame handed to a `RemoteSource` does not describe valid data.
    #[error("the frame is not valid: {0}")]
    Frame(#[from] FrameError),
    /// A `RemoteSource` frame has a different kind or size than the source was built for.
    #[error("the frame does not fit the source: {0}")]
    Mismatch(String),
    /// The appsrc refused the buffer (flushing, EOS, or not started).
    #[error("the appsrc refused the buffer: {0}")]
    Push(String),
}

impl From<gstreamer::glib::BoolError> for BridgeError {
    fn from(error: gstreamer::glib::BoolError) -> Self {
        BridgeError::Pipeline(error.to_string())
    }
}

impl From<gstreamer::StateChangeError> for BridgeError {
    fn from(error: gstreamer::StateChangeError) -> Self {
        BridgeError::StateChange(error.to_string())
    }
}
