// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! The GStreamer half of Mercury's MSE backend (lane med-1, item 4; brief section 6).
//!
//! `MseSrc` is a `GstBin` registered as the URI handler of [`MSE_URI_SCHEME`], so that `playbin3`
//! creates it from `mercury-mse://<n>` and the `source-setup` signal hands it to [`attach`]. It holds one
//! `appsrc` per track and fills them from a [`FrameFeed`] (`need-data` pulls, the feed's wake closure
//! resumes). Seeks re-feed from the random access point; a codec change changes the caps in front of the
//! next buffer.
//!
//! OWNER: item 4. **The four items below are the contract with the glue (item 5)**: committed by the lead
//! as `todo!()`, keep the signatures, add freely.

use std::sync::Arc;

use mercury_mse::{CodecId, FrameFeed, TrackKind, TypeSupport};

/// The URI scheme `playbin3` is given for an MSE source: `mercury-mse://<n>`.
pub const MSE_URI_SCHEME: &str = "mercury-mse";

/// Registers the `MseSrc` element and its URI handler with GStreamer. Call once after `gst::init`
/// (idempotent).
pub fn register() -> Result<(), glib::BoolError> {
    todo!("item 4: register MseSrc")
}

/// Connects the element `playbin3` made for a `mercury-mse://` URI (the `source-setup` argument) to
/// `feed`.
pub fn attach(source: &gstreamer::Element, feed: Arc<dyn FrameFeed>) -> Result<(), String> {
    let _ = (source, feed);
    todo!("item 4: attach the feed")
}

/// Answers `TypeSupport` from the GStreamer registry: is there a decoder for the codec (H.264, VP8, VP9,
/// AV1, AAC, Opus).
#[derive(Clone, Copy, Debug, Default)]
pub struct GstTypeSupport;

impl TypeSupport for GstTypeSupport {
    fn has_decoder(&self, kind: TrackKind, codec: CodecId) -> bool {
        let _ = (kind, codec);
        todo!("item 4: registry lookup")
    }
}
