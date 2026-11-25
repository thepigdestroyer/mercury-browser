// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! A coded frame as a `GstBuffer`: the bytes are not copied (the memory wraps the frame's `Arc<[u8]>`),
//! the timestamps and flags are set from the frame.

use std::sync::Arc;

use gstreamer as gst;
use mercury_mse::{CodedFrame, TrackKind};

/// Keeps the frame's bytes alive for as long as GStreamer's memory points at them.
struct FrameBytes(Arc<[u8]>);

impl AsRef<[u8]> for FrameBytes {
    fn as_ref(&self) -> &[u8] {
        &self.0
    }
}

/// Nanoseconds to a `ClockTime`; a negative time (before the start of the presentation timeline) has none.
fn clock_time(ns: i64) -> Option<gst::ClockTime> {
    u64::try_from(ns).ok().map(gst::ClockTime::from_nseconds)
}

/// The buffer for `frame`: `pts`, `dts` and `duration` set, the delta-unit flag on every frame that is
/// not a random access point, and DISCONT when the feed says the frame follows a seek or a gap.
///
/// Video frames carry no `dts`, as they do from `matroskademux`: the decode time of a B-frame stream can be
/// negative on the presentation timeline (the first composition offset puts it before zero), GStreamer's
/// `ClockTime` cannot hold that, and a stream with some decode times and some not confuses the parsers and
/// decoders (each reordered frame came out twice). The decoders reorder by `pts`. Audio frames keep theirs
/// (they equal the `pts`).
/// A negative time is clamped for the `pts` (the half-visible frame at zero) and dropped for the `dts`.
pub fn buffer_for(frame: &CodedFrame, discont: bool) -> gst::Buffer {
    let mut buffer = gst::Buffer::from_slice(FrameBytes(Arc::clone(&frame.data)));
    {
        let buffer = buffer
            .get_mut()
            .expect("a new buffer has one reference and is writable");
        // A frame cannot start before the presentation timeline does: the processor's append window
        // drops those, so the clamp only ever sees the half-visible frame at zero.
        buffer.set_pts(clock_time(frame.pts.max(0)));
        buffer.set_dts(match frame.kind {
            TrackKind::Video => None,
            _ => clock_time(frame.dts),
        });
        buffer.set_duration(clock_time(frame.duration.max(0)));
        let mut flags = gst::BufferFlags::empty();
        if !frame.keyframe {
            flags |= gst::BufferFlags::DELTA_UNIT;
        }
        if discont {
            flags |= gst::BufferFlags::DISCONT;
        }
        buffer.set_flags(flags);
    }
    buffer
}
