// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! The caps the source element gives each codec (brief section 5), checked without a pipeline.

mod common;

use gstreamer as gst;
use mercury_mse::{CodecConfig, TrackConfig, TrackKind};
use mercury_mse_gst::caps::{caps_for, stream_type};

fn init() {
    common::env::init();
}

fn track(kind: TrackKind, codec: CodecConfig) -> TrackConfig {
    TrackConfig {
        kind,
        track_id: 1,
        codec,
        codec_string: String::new(),
        width: 0,
        height: 0,
        sample_rate: 0,
        channels: 0,
        timescale: 0,
        language: String::new(),
        default: true,
    }
}

fn avcc() -> Vec<u8> {
    // version 1, profile 66, compat 0xC0, level 12, 4-byte lengths, one SPS, one PPS (contents unused here).
    vec![
        1, 66, 0xC0, 12, 0xFF, 0xE1, 0, 2, 0x67, 0x42, 1, 0, 2, 0x68, 0xCE,
    ]
}

#[test]
fn h264_caps_carry_avcc_and_the_size() {
    init();
    let mut config = track(TrackKind::Video, CodecConfig::H264 { avcc: avcc() });
    config.width = 320;
    config.height = 180;
    let caps = caps_for(&config).unwrap();
    let s = caps.structure(0).unwrap();
    assert_eq!(s.name(), "video/x-h264");
    assert_eq!(s.get::<&str>("stream-format").unwrap(), "avc");
    assert_eq!(s.get::<&str>("alignment").unwrap(), "au");
    assert_eq!(s.get::<i32>("width").unwrap(), 320);
    let data = s.get::<gst::Buffer>("codec_data").unwrap();
    assert_eq!(data.map_readable().unwrap().as_slice(), avcc().as_slice());
}

#[test]
fn an_unusable_avcc_is_refused() {
    init();
    let config = track(TrackKind::Video, CodecConfig::H264 { avcc: vec![1, 2] });
    assert!(caps_for(&config).unwrap_err().contains("avcC"));
    let config = track(
        TrackKind::Video,
        CodecConfig::H264 {
            avcc: vec![9, 66, 0, 12, 0xFF, 0xE1, 0],
        },
    );
    assert!(caps_for(&config).is_err());
}

#[test]
fn vp8_and_vp9_are_plain_video_caps() {
    init();
    let mut config = track(TrackKind::Video, CodecConfig::Vp8);
    config.width = 640;
    config.height = 360;
    let caps = caps_for(&config).unwrap();
    assert_eq!(caps.structure(0).unwrap().name(), "video/x-vp8");
    let vp9 = track(TrackKind::Video, CodecConfig::Vp9 { vpcc: None });
    let caps = caps_for(&vp9).unwrap();
    assert_eq!(caps.structure(0).unwrap().name(), "video/x-vp9");
    assert!(caps.structure(0).unwrap().get::<i32>("width").is_err());
}

#[test]
fn av1_caps_are_obu_stream_temporal_units() {
    init();
    let config = track(
        TrackKind::Video,
        CodecConfig::Av1 {
            av1c: vec![0x81, 0, 0x0C, 0, 0x0A, 0x0B],
        },
    );
    let caps = caps_for(&config).unwrap();
    let s = caps.structure(0).unwrap();
    assert_eq!(s.name(), "video/x-av1");
    assert_eq!(s.get::<&str>("stream-format").unwrap(), "obu-stream");
    assert_eq!(s.get::<&str>("alignment").unwrap(), "tu");
    assert!(s.get::<gst::Buffer>("codec_data").is_ok());
    // A four-byte-short record leaves the field out rather than lie.
    let config = track(
        TrackKind::Video,
        CodecConfig::Av1 {
            av1c: vec![0x81, 0],
        },
    );
    let caps = caps_for(&config).unwrap();
    assert!(
        caps.structure(0)
            .unwrap()
            .get::<gst::Buffer>("codec_data")
            .is_err()
    );
}

#[test]
fn aac_caps_are_raw_with_the_audio_specific_config() {
    init();
    let mut config = track(
        TrackKind::Audio,
        CodecConfig::Aac {
            asc: vec![0x11, 0x88],
        },
    );
    config.sample_rate = 48_000;
    config.channels = 1;
    let caps = caps_for(&config).unwrap();
    let s = caps.structure(0).unwrap();
    assert_eq!(s.name(), "audio/mpeg");
    assert_eq!(s.get::<i32>("mpegversion").unwrap(), 4);
    assert_eq!(s.get::<&str>("stream-format").unwrap(), "raw");
    assert_eq!(s.get::<i32>("rate").unwrap(), 48_000);
    assert_eq!(s.get::<i32>("channels").unwrap(), 1);
    assert_eq!(
        s.get::<gst::Buffer>("codec_data")
            .unwrap()
            .map_readable()
            .unwrap()
            .as_slice(),
        &[0x11, 0x88]
    );
    let config = track(TrackKind::Audio, CodecConfig::Aac { asc: vec![0x11] });
    assert!(caps_for(&config).is_err());
}

fn opus_head(channels: u8, family: u8, mapping: &[u8]) -> Vec<u8> {
    let mut head = b"OpusHead".to_vec();
    head.push(1);
    head.push(channels);
    head.extend_from_slice(&312u16.to_le_bytes());
    head.extend_from_slice(&48_000u32.to_le_bytes());
    head.extend_from_slice(&0u16.to_le_bytes());
    head.push(family);
    head.extend_from_slice(mapping);
    head
}

#[test]
fn opus_family_zero_caps() {
    init();
    let config = track(
        TrackKind::Audio,
        CodecConfig::Opus {
            head: opus_head(2, 0, &[]),
        },
    );
    let caps = caps_for(&config).unwrap();
    let s = caps.structure(0).unwrap();
    assert_eq!(s.name(), "audio/x-opus");
    assert_eq!(s.get::<i32>("rate").unwrap(), 48_000);
    assert_eq!(s.get::<i32>("channels").unwrap(), 2);
    assert_eq!(s.get::<i32>("channel-mapping-family").unwrap(), 0);
    assert!(s.get::<i32>("stream-count").is_err());
}

#[test]
fn opus_family_one_carries_the_mapping() {
    init();
    // Six channels (5.1): four streams, two of them coupled, one mapping byte per channel.
    let config = track(
        TrackKind::Audio,
        CodecConfig::Opus {
            head: opus_head(6, 1, &[4, 2, 0, 4, 1, 2, 3, 5]),
        },
    );
    // The table needs 21 + 6 = 27 bytes: this head has 19 + 8 = 27.
    let caps = caps_for(&config).unwrap();
    let s = caps.structure(0).unwrap();
    assert_eq!(s.get::<i32>("stream-count").unwrap(), 4);
    assert_eq!(s.get::<i32>("coupled-count").unwrap(), 2);
    let mapping = s.get::<gst::Array>("channel-mapping").unwrap();
    assert_eq!(mapping.len(), 6);
    let short = track(
        TrackKind::Audio,
        CodecConfig::Opus {
            head: opus_head(6, 1, &[4, 2]),
        },
    );
    assert!(caps_for(&short).unwrap_err().contains("truncated"));
}

#[test]
fn bad_opus_heads_are_refused() {
    init();
    for head in [
        vec![],
        b"OpusHeaX\x01\x02\x38\x01\x80\xbb\x00\x00\x00\x00\x00".to_vec(),
        opus_head(0, 0, &[]),
    ] {
        let config = track(TrackKind::Audio, CodecConfig::Opus { head });
        assert!(caps_for(&config).is_err());
    }
}

#[test]
fn codecs_left_for_later_have_no_caps() {
    init();
    let config = track(TrackKind::Video, CodecConfig::Hevc { hvcc: vec![1; 30] });
    assert!(caps_for(&config).unwrap_err().contains("Hevc"));
    let config = track(
        TrackKind::Audio,
        CodecConfig::Other {
            codec: mercury_mse::CodecId::Flac,
            config: vec![],
        },
    );
    assert!(caps_for(&config).is_err());
}

#[test]
fn stream_types_follow_the_kind() {
    assert_eq!(stream_type(TrackKind::Video), gst::StreamType::VIDEO);
    assert_eq!(stream_type(TrackKind::Audio), gst::StreamType::AUDIO);
    assert_eq!(stream_type(TrackKind::Text), gst::StreamType::TEXT);
}
