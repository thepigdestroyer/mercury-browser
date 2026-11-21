// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! The GStreamer half of Mercury's MSE backend (lane med-1, item 4; brief section 6).
//!
//! `MseSrc` is a `GstBin` registered as the URI handler of [`MSE_URI_SCHEME`], so that `playbin3`
//! creates it from `mercury-mse://<n>` and the `source-setup` signal hands it to [`attach`]. It holds one
//! `appsrc` per track and fills them from a [`FrameFeed`]: a feeder thread pulls when an appsrc says
//! `need-data` and sleeps when the feed says `WaitingForData` until the feed's wake closure fires. Seeks
//! re-feed from the random access point; a codec change changes the caps in front of the next buffer; the
//! end of the stream is an EOS after the last frame.
//!
//! The rules (R17 section 11.2, brief risk 1): no GStreamer callback and no closure the backend calls ever
//! calls into the feed except on the feeder thread, the feeder thread never sleeps on a timer, teardown
//! joins it without waiting, and no GLib main loop is needed anywhere.
//!
//! OWNER: item 4. **The four items below are the contract with the glue (item 5)**: keep the signatures.
//!
//! `cfg` attributes in this crate: none. The code uses GStreamer 1.18 API only (`features = ["v1_18"]`),
//! so it compiles with the 1.20 of Linux CI (not compiled here).

use std::sync::Arc;
use std::sync::Mutex;

use gstreamer as gst;
use gstreamer::prelude::*;
use mercury_mse::FrameFeed;

mod buffers;
pub mod caps;
mod element;
mod feeder;
mod shared;
mod typesupport;

pub use buffers::buffer_for;
pub use element::MseSrc;
pub use typesupport::{GstTypeSupport, decoder_caps};

/// The URI scheme `playbin3` is given for an MSE source: `mercury-mse://<n>`.
pub const MSE_URI_SCHEME: &str = "mercury-mse";

/// The name the element is registered under.
pub const ELEMENT_NAME: &str = "mercurymsesrc";

/// Registers the `MseSrc` element and its URI handler with GStreamer. Call once after `gst::init`
/// (idempotent).
pub fn register() -> Result<(), glib::BoolError> {
    // Two threads may register at once (two players starting): the lock makes the check and the
    // registration one step.
    static LOCK: Mutex<()> = Mutex::new(());
    let _guard = LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if gst::ElementFactory::find(ELEMENT_NAME).is_some() {
        return Ok(());
    }
    gst::Element::register(
        None,
        ELEMENT_NAME,
        gst::Rank::NONE,
        <element::MseSrc as glib::types::StaticType>::static_type(),
    )
}

/// Connects the element `playbin3` made for a `mercury-mse://` URI (the `source-setup` argument) to
/// `feed`. Does not block and does not call into the player: it starts the element's feeder thread and
/// hands the feed a wake closure.
pub fn attach(source: &gst::Element, feed: Arc<dyn FrameFeed>) -> Result<(), String> {
    let source = source
        .downcast_ref::<MseSrc>()
        .ok_or_else(|| format!("{} is not a {ELEMENT_NAME} element", source.name()))?;
    source.attach(feed)
}
