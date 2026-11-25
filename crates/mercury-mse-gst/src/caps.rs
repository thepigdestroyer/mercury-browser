// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! GStreamer caps from a [`TrackConfig`] (brief section 5, "Caps from `CodecConfig`").
//!
//! * H.264: `video/x-h264, stream-format=avc, alignment=au, codec_data=<avcC>`
//! * VP8: `video/x-vp8`; VP9: `video/x-vp9`
//! * AV1: `video/x-av1, stream-format=obu-stream, alignment=tu, codec_data=<av1C>`
//! * AAC: `audio/mpeg, mpegversion=4, stream-format=raw, codec_data=<AudioSpecificConfig>`
//! * Opus: `audio/x-opus, rate, channels, channel-mapping-family` (and, for families above 0, the stream
//!   and channel mapping) read from `OpusHead`
//!
//! Width, height, rate and channel count are added when the container gave them (non-zero). Nothing here
//! needs a running pipeline, only an initialised GStreamer.

use gstreamer as gst;
use mercury_mse::{CodecConfig, TrackConfig, TrackKind};

/// The smallest `avcC` there is: version, three profile bytes, `lengthSizeMinusOne`, an SPS count, a PPS
/// count.
const AVCC_MIN: usize = 7;

/// The smallest `av1C`: marker/version, profile/level, flags, delay.
const AV1C_MIN: usize = 4;

/// `OpusHead` with channel-mapping family 0: magic, version, channels, pre-skip, rate, gain, family.
const OPUS_HEAD_MIN: usize = 19;

/// The fixed caps of a track's configuration, or why there are none.
pub fn caps_for(config: &TrackConfig) -> Result<gst::Caps, String> {
    let mut builder = match &config.codec {
        CodecConfig::H264 { avcc } => {
            if avcc.len() < AVCC_MIN || avcc[0] != 1 {
                return Err(format!(
                    "the avcC of the H.264 track is not usable ({} bytes)",
                    avcc.len()
                ));
            }
            gst::Caps::builder("video/x-h264")
                .field("stream-format", "avc")
                .field("alignment", "au")
                .field("codec_data", gst::Buffer::from_slice(avcc.clone()))
        }
        CodecConfig::Vp8 => gst::Caps::builder("video/x-vp8"),
        CodecConfig::Vp9 { .. } => gst::Caps::builder("video/x-vp9"),
        CodecConfig::Av1 { av1c } => {
            let mut builder = gst::Caps::builder("video/x-av1")
                .field("stream-format", "obu-stream")
                .field("alignment", "tu");
            if av1c.len() >= AV1C_MIN {
                builder = builder.field("codec_data", gst::Buffer::from_slice(av1c.clone()));
            }
            builder
        }
        CodecConfig::Aac { asc } => {
            if asc.len() < 2 {
                return Err(format!(
                    "the AudioSpecificConfig of the AAC track is not usable ({} bytes)",
                    asc.len()
                ));
            }
            gst::Caps::builder("audio/mpeg")
                .field("mpegversion", 4i32)
                .field("stream-format", "raw")
                .field("codec_data", gst::Buffer::from_slice(asc.clone()))
        }
        CodecConfig::Opus { head } => opus_caps(head, config)?,
        CodecConfig::Hevc { .. } | CodecConfig::Other { .. } => {
            return Err(format!(
                "{:?} is not fed to GStreamer in this round",
                config.codec.id()
            ));
        }
    };

    match config.kind {
        TrackKind::Video => {
            if config.width > 0 && config.height > 0 {
                builder = builder
                    .field("width", config.width as i32)
                    .field("height", config.height as i32);
            }
        }
        TrackKind::Audio => {
            // The Opus branch already set the rate and channels from OpusHead.
            if !matches!(config.codec, CodecConfig::Opus { .. }) {
                if config.sample_rate > 0 {
                    builder = builder.field("rate", config.sample_rate as i32);
                }
                if config.channels > 0 {
                    builder = builder.field("channels", config.channels as i32);
                }
            }
        }
        TrackKind::Text => {}
    }
    Ok(builder.build())
}

/// `audio/x-opus` caps from an `OpusHead`, as GStreamer's `gst_codec_utils_opus_create_caps` builds them
/// (the helper is in gst-plugins-base's pbutils; kept out of the dependencies, the fields are four).
fn opus_caps(
    head: &[u8],
    config: &TrackConfig,
) -> Result<gst::caps::Builder<gst::caps::NoFeature>, String> {
    if head.len() < OPUS_HEAD_MIN || &head[..8] != b"OpusHead" {
        return Err(format!(
            "the OpusHead of the Opus track is not usable ({} bytes)",
            head.len()
        ));
    }
    let channels = i32::from(head[9]);
    let input_rate = u32::from_le_bytes([head[12], head[13], head[14], head[15]]);
    let family = i32::from(head[18]);
    if channels == 0 {
        return Err("the OpusHead declares no channel".to_owned());
    }
    // GStreamer's decoder works at 48 kHz whatever the original rate; the caps say the original.
    let rate = match (input_rate, config.sample_rate) {
        (0, 0) => 48_000,
        (0, rate) => rate,
        (rate, _) => rate,
    } as i32;
    let mut builder = gst::Caps::builder("audio/x-opus")
        .field("rate", rate)
        .field("channels", channels)
        .field("channel-mapping-family", family);
    if family > 0 {
        // Family 1 and 255 carry the stream count, the coupled stream count and one mapping byte per channel.
        let table = 21 + channels as usize;
        if head.len() < table {
            return Err("the OpusHead channel mapping table is truncated".to_owned());
        }
        builder = builder
            .field("stream-count", i32::from(head[19]))
            .field("coupled-count", i32::from(head[20]))
            .field(
                "channel-mapping",
                gst::Array::new(head[21..table].iter().map(|&byte| i32::from(byte))),
            );
    }
    Ok(builder)
}

/// The `gst::StreamType` of a kind of track.
pub fn stream_type(kind: TrackKind) -> gst::StreamType {
    match kind {
        TrackKind::Video => gst::StreamType::VIDEO,
        TrackKind::Audio => gst::StreamType::AUDIO,
        TrackKind::Text => gst::StreamType::TEXT,
    }
}
