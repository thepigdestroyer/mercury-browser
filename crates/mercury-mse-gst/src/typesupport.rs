// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! `TypeSupport` from the GStreamer registry: is there a decoder for the codec.

use std::sync::OnceLock;

use gstreamer as gst;
use mercury_mse::{CodecId, TrackKind, TypeSupport};

/// The environment variable that turns AV1 back on where [`av1_policy`] hides it (decision D8).
pub const AV1_OVERRIDE_VARIABLE: &str = "MERCURY_MSE_AV1";

/// Answers [`TypeSupport`] from the registry: a decoder element (rank marginal or better, so that
/// `decodebin3` would pick it) that can take the codec's elementary stream. H.264, HEVC, VP8, VP9, AV1,
/// AAC, Opus, MP3, FLAC, Vorbis, AC-3 and E-AC-3 are the codecs of round 2 (decisions D4 and D7); MPEG-TS
/// and raw `audio/mpeg`/`audio/aac` byte streams are not codecs of this layer.
///
/// AV1 is also subject to [`av1_policy`]: on Apple Silicon it is hidden unless `MERCURY_MSE_AV1=1`.
#[derive(Clone, Copy, Debug, Default)]
pub struct GstTypeSupport;

/// The caps a decoder (or its parser in front) must take for each codec, as the feed hands them over.
pub fn decoder_caps(codec: CodecId) -> Option<gst::Caps> {
    let description = match codec {
        CodecId::H264 => "video/x-h264, stream-format=(string)avc, alignment=(string)au",
        CodecId::Hevc => "video/x-h265, stream-format=(string)hvc1, alignment=(string)au",
        CodecId::Vp8 => "video/x-vp8",
        CodecId::Vp9 => "video/x-vp9",
        CodecId::Av1 => "video/x-av1, stream-format=(string)obu-stream, alignment=(string)tu",
        CodecId::Aac => "audio/mpeg, mpegversion=(int)4, stream-format=(string)raw",
        CodecId::Opus => "audio/x-opus",
        CodecId::Mp3 => "audio/mpeg, mpegversion=(int)1, layer=(int)3, parsed=(boolean)true",
        CodecId::Flac => "audio/x-flac, framed=(boolean)true",
        CodecId::Vorbis => "audio/x-vorbis",
        CodecId::Ac3 => "audio/x-ac3, framed=(boolean)true",
        CodecId::Eac3 => "audio/x-eac3, framed=(boolean)true",
    };
    description.parse().ok()
}

/// Decision D8: whether AV1 may be offered. `apple_silicon` is "this build runs on aarch64 macOS";
/// `override_value` is the value of `MERCURY_MSE_AV1`, if set. On Intel and everywhere else the registry
/// decides (dav1d plays, measured); on Apple Silicon the one measurement (a hosted arm64 VM) shows
/// `vtdec_hw` taking AV1, failing to create a session and `decodebin3` not falling back, so AV1 is hidden
/// until real hardware says otherwise, and `MERCURY_MSE_AV1=1` is the way to try it.
pub fn av1_policy(apple_silicon: bool, override_value: Option<&str>) -> bool {
    !apple_silicon || override_value == Some("1")
}

/// [`av1_policy`] for this build and this process: the architecture is compile-time and the variable is
/// read once.
fn av1_allowed() -> bool {
    static ALLOWED: OnceLock<bool> = OnceLock::new();
    *ALLOWED.get_or_init(|| {
        let apple_silicon = cfg!(all(target_arch = "aarch64", target_os = "macos"));
        let value = std::env::var(AV1_OVERRIDE_VARIABLE).ok();
        av1_policy(apple_silicon, value.as_deref())
    })
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
        if codec == CodecId::Av1 && !av1_allowed() {
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

#[cfg(test)]
mod tests {
    use super::av1_policy;

    #[test]
    fn av1_is_offered_everywhere_but_apple_silicon() {
        assert!(av1_policy(false, None));
        assert!(av1_policy(false, Some("0")));
        assert!(av1_policy(false, Some("1")));
    }

    #[test]
    fn av1_is_hidden_on_apple_silicon_unless_the_variable_is_one() {
        assert!(!av1_policy(true, None));
        assert!(!av1_policy(true, Some("")));
        assert!(!av1_policy(true, Some("0")));
        assert!(!av1_policy(true, Some("true")));
        assert!(av1_policy(true, Some("1")));
    }
}
