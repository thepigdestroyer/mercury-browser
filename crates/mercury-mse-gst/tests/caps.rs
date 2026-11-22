// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! The caps the source element gives each codec (brief section 5), checked without a pipeline.

mod common;

use gstreamer as gst;
use mercury_mse::{CodecConfig, CodecId, TrackConfig, TrackKind};
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

fn other(kind: TrackKind, codec: CodecId, config: Vec<u8>) -> TrackConfig {
    track(kind, CodecConfig::Other { codec, config })
}

/// The smallest usable `hvcC`: the 22 fixed bytes and an array count of zero.
fn hvcc() -> Vec<u8> {
    let mut record = vec![1, 1, 0x60, 0, 0, 0, 0x90, 0, 0, 0, 0, 0, 93];
    record.extend_from_slice(&[0xF0, 0, 0xFC, 0xFD, 0xF8, 0xF8, 0, 0, 0x0F, 0]);
    assert_eq!(record.len(), 23);
    record
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// The table of D7: the configuration the parsers hand over (section 6 of the med-1b brief) against the
/// caps string `MseSrc` gives GStreamer.
#[test]
fn caps_table_of_the_new_codecs() {
    init();
    let mut hevc = track(TrackKind::Video, CodecConfig::Hevc { hvcc: hvcc() });
    hevc.codec_string = "hvc1.1.6.L93.B0".to_owned();
    hevc.width = 1280;
    hevc.height = 720;
    let mut hev1 = hevc.clone();
    hev1.codec_string = "hev1.1.6.L93.B0".to_owned();
    let mut ac3 = other(TrackKind::Audio, CodecId::Ac3, vec![0x10, 0x3D, 0x40]);
    ac3.sample_rate = 48_000;
    ac3.channels = 6;
    let mut eac3 = other(
        TrackKind::Audio,
        CodecId::Eac3,
        vec![0x06, 0x00, 0x20, 0x00],
    );
    eac3.sample_rate = 48_000;
    eac3.channels = 2;
    let mut mp3 = other(TrackKind::Audio, CodecId::Mp3, vec![]);
    mp3.sample_rate = 44_100;
    mp3.channels = 2;
    // A WebM AC-3 track has no configuration, no rate and no channel count: the caps say less.
    let bare_ac3 = other(TrackKind::Audio, CodecId::Ac3, vec![]);

    let hvcc_hex = hex(&hvcc());
    let table: Vec<(&str, TrackConfig, String)> = vec![
        (
            "HEVC hvc1",
            hevc,
            format!(
                "video/x-h265, stream-format=(string)hvc1, alignment=(string)au, codec_data=(buffer){hvcc_hex}, width=(int)1280, height=(int)720"
            ),
        ),
        (
            "HEVC hev1",
            hev1,
            format!(
                "video/x-h265, stream-format=(string)hev1, alignment=(string)au, codec_data=(buffer){hvcc_hex}, width=(int)1280, height=(int)720"
            ),
        ),
        (
            "AC-3",
            ac3,
            "audio/x-ac3, framed=(boolean)true, rate=(int)48000, channels=(int)6".to_owned(),
        ),
        (
            "E-AC-3",
            eac3,
            "audio/x-eac3, framed=(boolean)true, rate=(int)48000, channels=(int)2".to_owned(),
        ),
        (
            "MP3",
            mp3,
            "audio/mpeg, mpegversion=(int)1, layer=(int)3, parsed=(boolean)true, rate=(int)44100, channels=(int)2"
                .to_owned(),
        ),
        (
            "AC-3 without configuration",
            bare_ac3,
            "audio/x-ac3, framed=(boolean)true".to_owned(),
        ),
    ];
    for (name, config, expected) in table {
        let caps = caps_for(&config).unwrap_or_else(|e| panic!("{name}: {e}"));
        assert_eq!(caps.to_string(), expected, "{name}");
    }
}

#[test]
fn an_unusable_hvcc_is_refused() {
    init();
    let mut short = hvcc();
    short.truncate(22);
    for record in [vec![], short, {
        let mut wrong_version = hvcc();
        wrong_version[0] = 2;
        wrong_version
    }] {
        let config = track(TrackKind::Video, CodecConfig::Hevc { hvcc: record });
        assert!(caps_for(&config).unwrap_err().contains("hvcC"));
    }
}

/// A metadata block: the header byte (last flag and type), a 24-bit length, the payload.
fn flac_block(last: bool, kind: u8, payload: &[u8]) -> Vec<u8> {
    let mut block = vec![(u8::from(last) << 7) | kind];
    block.extend_from_slice(&(payload.len() as u32).to_be_bytes()[1..]);
    block.extend_from_slice(payload);
    block
}

fn header_bytes(array: &gst::Array) -> Vec<Vec<u8>> {
    array
        .iter()
        .map(|value| {
            let buffer = value.get::<gst::Buffer>().expect("a buffer");
            assert!(
                buffer.flags().contains(gst::BufferFlags::HEADER),
                "header buffers carry the HEADER flag"
            );
            buffer.map_readable().unwrap().as_slice().to_vec()
        })
        .collect()
}

#[test]
fn flac_caps_carry_the_marker_and_every_metadata_block() {
    init();
    let streaminfo = flac_block(false, 0, &[7u8; 34]);
    let comment = flac_block(true, 4, b"\x00\x00\x00\x00\x00\x00\x00\x00");
    let mut blocks = streaminfo.clone();
    blocks.extend_from_slice(&comment);
    let mut config = other(TrackKind::Audio, CodecId::Flac, blocks);
    config.sample_rate = 44_100;
    config.channels = 2;
    let caps = caps_for(&config).unwrap();
    let s = caps.structure(0).unwrap();
    assert_eq!(s.name(), "audio/x-flac");
    assert!(s.get::<bool>("framed").unwrap());
    assert_eq!(s.get::<i32>("rate").unwrap(), 44_100);
    assert_eq!(s.get::<i32>("channels").unwrap(), 2);
    let headers = header_bytes(&s.get::<gst::Array>("streamheader").unwrap());
    assert_eq!(headers, vec![b"fLaC".to_vec(), streaminfo, comment]);

    // A single STREAMINFO is enough; the last-block flag may be missing in a hand-built configuration.
    let only = flac_block(false, 0, &[1u8; 34]);
    let config = other(TrackKind::Audio, CodecId::Flac, only.clone());
    let caps = caps_for(&config).unwrap();
    let headers = header_bytes(
        &caps
            .structure(0)
            .unwrap()
            .get::<gst::Array>("streamheader")
            .unwrap(),
    );
    assert_eq!(headers, vec![b"fLaC".to_vec(), only]);
}

#[test]
fn bad_flac_configurations_are_refused() {
    init();
    let streaminfo = flac_block(true, 0, &[7u8; 34]);
    let mut truncated = streaminfo.clone();
    truncated.truncate(20);
    let wrong_first = flac_block(true, 4, &[0u8; 34]);
    let short_streaminfo = flac_block(true, 0, &[0u8; 33]);
    let mut second_cut = flac_block(false, 0, &[7u8; 34]);
    second_cut.extend_from_slice(&[0x84, 0, 0, 200, 1, 2]);
    for blocks in [
        vec![],
        vec![0x80, 0],
        truncated,
        wrong_first,
        short_streaminfo,
        second_cut,
    ] {
        let config = other(TrackKind::Audio, CodecId::Flac, blocks.clone());
        assert!(caps_for(&config).is_err(), "{blocks:?}");
    }
}

/// `CodecPrivate` of a Vorbis track: Xiph lacing of the three header packets.
fn xiph(packets: [&[u8]; 3]) -> Vec<u8> {
    let mut out = vec![2];
    for packet in &packets[..2] {
        let mut left = packet.len();
        while left >= 255 {
            out.push(255);
            left -= 255;
        }
        out.push(left as u8);
    }
    for packet in packets {
        out.extend_from_slice(packet);
    }
    out
}

#[test]
fn vorbis_caps_carry_the_three_headers() {
    init();
    let (id, comment, setup) = (vec![1u8; 30], vec![3u8; 300], vec![5u8; 1000]);
    let mut config = other(
        TrackKind::Audio,
        CodecId::Vorbis,
        xiph([&id, &comment, &setup]),
    );
    config.sample_rate = 44_100;
    config.channels = 2;
    let caps = caps_for(&config).unwrap();
    let s = caps.structure(0).unwrap();
    assert_eq!(s.name(), "audio/x-vorbis");
    assert_eq!(s.get::<i32>("rate").unwrap(), 44_100);
    let headers = header_bytes(&s.get::<gst::Array>("streamheader").unwrap());
    // The 300-byte comment packet is laced as 255 + 45.
    assert_eq!(headers, vec![id, comment, setup]);
}

#[test]
fn bad_vorbis_configurations_are_refused() {
    init();
    let good = xiph([&[1u8; 30], &[3u8; 20], &[5u8; 100]]);
    let mut wrong_count = good.clone();
    wrong_count[0] = 1;
    let mut truncated = good.clone();
    truncated.truncate(good.len() - 101);
    let mut zero_size = good.clone();
    zero_size[1] = 0;
    for private in [
        vec![],
        vec![2],
        vec![2, 255],
        wrong_count,
        truncated,
        zero_size,
        // Sizes that overflow what is there.
        vec![2, 255, 255, 255, 1, 2, 3],
    ] {
        let config = other(TrackKind::Audio, CodecId::Vorbis, private.clone());
        assert!(caps_for(&config).is_err(), "{private:?}");
    }
}

#[test]
fn an_other_configuration_of_a_codec_with_its_own_variant_is_refused() {
    init();
    let config = other(TrackKind::Video, CodecId::H264, vec![1, 2, 3]);
    assert!(caps_for(&config).is_err());
    let config = other(TrackKind::Audio, CodecId::Aac, vec![0x11, 0x88]);
    assert!(caps_for(&config).is_err());
}

/// Review finding 8: a rate, a channel count or a size that the stream declares as any `u32` is refused
/// by name, not turned into `rate=-1` by `as i32` and left for GStreamer's negotiation to trip over.
#[test]
fn numbers_that_do_not_fit_a_caps_integer_are_refused() {
    init();
    let vorbis = || {
        other(
            TrackKind::Audio,
            CodecId::Vorbis,
            xiph([&[1u8; 30], &[3u8; 20], &[5u8; 100]]),
        )
    };
    let mut config = vorbis();
    config.sample_rate = u32::MAX;
    config.channels = 2;
    let error = caps_for(&config).unwrap_err();
    assert!(error.contains("sample rate"), "{error}");
    let mut config = vorbis();
    config.sample_rate = 44_100;
    config.channels = u32::MAX;
    let error = caps_for(&config).unwrap_err();
    assert!(error.contains("channel count"), "{error}");
    // The edges that play are kept, and no value ever becomes negative.
    let mut config = vorbis();
    config.sample_rate = 768_000;
    config.channels = 8;
    let caps = caps_for(&config).unwrap();
    let s = caps.structure(0).unwrap();
    assert_eq!(s.get::<i32>("rate").unwrap(), 768_000);
    assert_eq!(s.get::<i32>("channels").unwrap(), 8);
    let mut config = vorbis();
    config.sample_rate = 768_001;
    assert!(caps_for(&config).is_err());

    let mut video = track(TrackKind::Video, CodecConfig::Vp8);
    video.width = u32::MAX;
    video.height = 180;
    assert!(caps_for(&video).unwrap_err().contains("width"));
    video.width = 320;
    video.height = 0x8000_0000;
    assert!(caps_for(&video).unwrap_err().contains("height"));

    // The Opus rate comes from the head, as a u32 as well.
    let mut head = opus_head(2, 0, &[]);
    head[12..16].copy_from_slice(&u32::MAX.to_le_bytes());
    let config = track(TrackKind::Audio, CodecConfig::Opus { head });
    assert!(caps_for(&config).unwrap_err().contains("sample rate"));
}

/// The packet length the feeder takes off the pre-skip comes from the TOC byte (RFC 6716 section 3.1).
#[test]
fn an_opus_packet_says_how_many_samples_it_decodes_to() {
    use mercury_mse_gst::caps::opus_packet_samples as samples;
    // CELT fullband 20 ms (config 31), one frame; two frames of equal and of different size; code 3 with
    // a frame count byte (here 5 frames of 2.5 ms: CELT NB config 16).
    assert_eq!(samples(&[31 << 3, 0xAA]), Some(960));
    assert_eq!(samples(&[31 << 3 | 1, 0xAA]), Some(1_920));
    assert_eq!(samples(&[31 << 3 | 2, 0xAA]), Some(1_920));
    assert_eq!(samples(&[16 << 3 | 3, 5]), Some(600));
    // SILK narrowband 60 ms, hybrid SWB 10 ms.
    assert_eq!(samples(&[3 << 3]), Some(2_880));
    assert_eq!(samples(&[12 << 3]), Some(480));
    // Empty, a code 3 without its count byte, a zero count, and more than the 120 ms one packet may hold.
    assert_eq!(samples(&[]), None);
    assert_eq!(samples(&[31 << 3 | 3]), None);
    assert_eq!(samples(&[31 << 3 | 3, 0]), None);
    assert_eq!(samples(&[31 << 3 | 3, 7]), None);
    assert_eq!(samples(&[31 << 3 | 3, 6]), Some(5_760));
}

#[test]
fn stream_types_follow_the_kind() {
    assert_eq!(stream_type(TrackKind::Video), gst::StreamType::VIDEO);
    assert_eq!(stream_type(TrackKind::Audio), gst::StreamType::AUDIO);
    assert_eq!(stream_type(TrackKind::Text), gst::StreamType::TEXT);
}
