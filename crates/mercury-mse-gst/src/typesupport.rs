// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! `TypeSupport` from the GStreamer registry: is there a decoder for the codec.

use gstreamer as gst;
use mercury_mse::{CodecId, TrackKind, TypeSupport};

/// Answers [`TypeSupport`] from the registry: a decoder element (rank marginal or better, so that
/// `decodebin3` would pick it) that can take the codec's elementary stream. H.264, VP8, VP9, AV1, AAC and
/// Opus are the codecs of this round; the others are refused (decision D4), not looked up.
#[derive(Clone, Copy, Debug, Default)]
pub struct GstTypeSupport;

/// The caps a decoder (or its parser in front) must take for each codec, as the feed hands them over.
pub fn decoder_caps(codec: CodecId) -> Option<gst::Caps> {
    let description = match codec {
        CodecId::H264 => "video/x-h264, stream-format=(string)avc, alignment=(string)au",
        CodecId::Vp8 => "video/x-vp8",
        CodecId::Vp9 => "video/x-vp9",
        CodecId::Av1 => "video/x-av1, stream-format=(string)obu-stream, alignment=(string)tu",
        CodecId::Aac => "audio/mpeg, mpegversion=(int)4, stream-format=(string)raw",
        CodecId::Opus => "audio/x-opus",
        // Decision D4: not fed this round.
        CodecId::Hevc
        | CodecId::Mp3
        | CodecId::Flac
        | CodecId::Vorbis
        | CodecId::Ac3
        | CodecId::Eac3 => return None,
    };
    description.parse().ok()
}

impl TypeSupport for GstTypeSupport {
    fn has_decoder(&self, kind: TrackKind, codec: CodecId) -> bool {
        // GStreamer must be initialised (the registry exists); before that nothing can be decoded.
        // SAFETY: gst_is_initialized only reads a flag.
        if unsafe { gst::ffi::gst_is_initialized() } == glib::ffi::GFALSE {
            return false;
        }
        if codec.kind() != kind {
            return false;
        }
        let Some(caps) = decoder_caps(codec) else {
            return false;
        };
        gst::ElementFactory::factories_with_type(
            gst::ElementFactoryType::DECODER,
            gst::Rank::MARGINAL,
        )
        .iter()
        .any(|factory| factory.can_sink_any_caps(&caps))
    }
}
