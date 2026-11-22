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
//! * HEVC (lane med-1b, D7): `video/x-h265, stream-format=hvc1|hev1, alignment=au, codec_data=<hvcC>`; the
//!   stream format follows the codec string (`hev1.` or `hvc1.`)
//! * AC-3: `audio/x-ac3, framed=true`; E-AC-3: `audio/x-eac3, framed=true`
//! * MP3: `audio/mpeg, mpegversion=1, layer=3, parsed=true`
//! * FLAC: `audio/x-flac, framed=true, streamheader=[fLaC, STREAMINFO, ...]` from the metadata blocks
//! * Vorbis: `audio/x-vorbis, streamheader=[identification, comment, setup]` from the Xiph-laced
//!   `CodecPrivate`
//!
//! Width, height, rate and channel count are added when the container gave them (non-zero). Nothing here
//! needs a running pipeline, only an initialised GStreamer.

use gstreamer as gst;
use mercury_mse::{CodecConfig, CodecId, TrackConfig, TrackKind};

/// The smallest `avcC` there is: version, three profile bytes, `lengthSizeMinusOne`, an SPS count, a PPS
/// count.
const AVCC_MIN: usize = 7;

/// The fixed part of an `hvcC` (22 bytes) and the array count that follows it.
const HVCC_MIN: usize = 23;

/// A FLAC STREAMINFO block is 34 bytes; each metadata block has a 4-byte header.
const FLAC_STREAMINFO_LEN: usize = 34;
const FLAC_BLOCK_HEADER: usize = 4;

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
        CodecConfig::Hevc { hvcc } => {
            if hvcc.len() < HVCC_MIN || hvcc[0] != 1 {
                return Err(format!(
                    "the hvcC of the HEVC track is not usable ({} bytes)",
                    hvcc.len()
                ));
            }
            // `hev1` keeps the parameter sets in the stream as well; anything else is `hvc1`.
            let format = if config.codec_string.starts_with("hev1") {
                "hev1"
            } else {
                "hvc1"
            };
            gst::Caps::builder("video/x-h265")
                .field("stream-format", format)
                .field("alignment", "au")
                .field("codec_data", gst::Buffer::from_slice(hvcc.clone()))
        }
        CodecConfig::Other {
            codec,
            config: bytes,
        } => match codec {
            CodecId::Ac3 => gst::Caps::builder("audio/x-ac3").field("framed", true),
            CodecId::Eac3 => gst::Caps::builder("audio/x-eac3").field("framed", true),
            CodecId::Mp3 => gst::Caps::builder("audio/mpeg")
                .field("mpegversion", 1i32)
                .field("layer", 3i32)
                .field("parsed", true),
            CodecId::Flac => gst::Caps::builder("audio/x-flac")
                .field("framed", true)
                .field("streamheader", flac_stream_headers(bytes)?),
            CodecId::Vorbis => gst::Caps::builder("audio/x-vorbis")
                .field("streamheader", vorbis_stream_headers(bytes)?),
            other => {
                return Err(format!("{other:?} has no caps in an `Other` configuration"));
            }
        },
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

/// A header buffer for a `streamheader` field: the bytes with the `HEADER` flag.
fn header_buffer(bytes: &[u8]) -> gst::Buffer {
    let mut buffer = gst::Buffer::from_slice(bytes.to_vec());
    buffer
        .get_mut()
        .expect("a new buffer is writable")
        .set_flags(gst::BufferFlags::HEADER);
    buffer
}

/// The FLAC `streamheader`: the `fLaC` marker, then each metadata block with its 4-byte header, the way
/// GStreamer's own demuxers build it. `blocks` is what the parsers hand over: the metadata blocks with
/// STREAMINFO first and the marker removed. A truncated block, a first block that is not a 34-byte
/// STREAMINFO or nothing after the last block's declared end is refused.
fn flac_stream_headers(blocks: &[u8]) -> Result<gst::Array, String> {
    let mut headers = vec![header_buffer(b"fLaC")];
    let mut rest = blocks;
    let mut first = true;
    loop {
        if rest.len() < FLAC_BLOCK_HEADER {
            return Err("the FLAC metadata blocks are truncated".to_owned());
        }
        let last = rest[0] & 0x80 != 0;
        let kind = rest[0] & 0x7F;
        let length = usize::from(rest[1]) << 16 | usize::from(rest[2]) << 8 | usize::from(rest[3]);
        let total = FLAC_BLOCK_HEADER + length;
        if rest.len() < total {
            return Err("a FLAC metadata block is truncated".to_owned());
        }
        if first && (kind != 0 || length != FLAC_STREAMINFO_LEN) {
            return Err("the first FLAC metadata block is not a STREAMINFO".to_owned());
        }
        first = false;
        headers.push(header_buffer(&rest[..total]));
        rest = &rest[total..];
        // The last-block flag ends the list; a configuration that stops without it is still taken
        // whole (an MP4 `dfLa` always sets it, a hand-built one may not).
        if last || rest.is_empty() {
            return Ok(gst::Array::new(headers));
        }
    }
}

/// The three Vorbis header packets of a Matroska `CodecPrivate` (Xiph lacing: a count byte that is 2, the
/// sizes of the first two packets in 255-runs, then the packets), as a `streamheader`.
fn vorbis_stream_headers(private: &[u8]) -> Result<gst::Array, String> {
    let bad = || "the Vorbis CodecPrivate is not three Xiph-laced header packets".to_owned();
    if private.first() != Some(&2) {
        return Err(bad());
    }
    let mut at = 1;
    let mut sizes = [0usize; 2];
    for size in &mut sizes {
        loop {
            let byte = *private.get(at).ok_or_else(bad)?;
            at += 1;
            *size = size.checked_add(usize::from(byte)).ok_or_else(bad)?;
            if byte != 255 {
                break;
            }
        }
    }
    let first_end = at.checked_add(sizes[0]).ok_or_else(bad)?;
    let second_end = first_end.checked_add(sizes[1]).ok_or_else(bad)?;
    if sizes[0] == 0 || sizes[1] == 0 || second_end >= private.len() {
        return Err(bad());
    }
    Ok(gst::Array::new([
        header_buffer(&private[at..first_end]),
        header_buffer(&private[first_end..second_end]),
        header_buffer(&private[second_end..]),
    ]))
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

/// The pre-skip of an `OpusHead`, in samples at 48 kHz: how many samples the decoder produces before
/// the audio starts (`None` for a head that is not usable). `opusdec` does not take it from the caps: it
/// drops them when the first buffer carries a `GstAudioClippingMeta`, which the feeder adds.
pub fn opus_pre_skip(head: &[u8]) -> Option<u32> {
    if head.len() < OPUS_HEAD_MIN || &head[..8] != b"OpusHead" {
        return None;
    }
    Some(u32::from(u16::from_le_bytes([head[10], head[11]])))
}

/// The `gst::StreamType` of a kind of track.
pub fn stream_type(kind: TrackKind) -> gst::StreamType {
    match kind {
        TrackKind::Video => gst::StreamType::VIDEO,
        TrackKind::Audio => gst::StreamType::AUDIO,
        TrackKind::Text => gst::StreamType::TEXT,
    }
}
