// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! Sample rate and channel count of an audio track (review finding 8 of lane med-1b round 2): both reach
//! the decoder's caps as numbers, so the parsers refuse a value outside 1..=768,000 Hz and 1..=8 channels
//! at the initialization segment (`Unsupported` when the stream is well formed but out of range,
//! `Malformed` for 0), for every codec and both containers, not later with a GStreamer negotiation error.
//! Run with `cargo test -p mercury-mse --test parse_limits_audio`.

mod parse_support;

use mercury_mse::api::{Container, InitSegment, ParseError, ParseEvent};
use mercury_mse::parse::{MAX_AUDIO_CHANNELS, MAX_AUDIO_SAMPLE_RATE, new_parser};
use parse_support::*;

fn push_all(container: Container, bytes: &[u8]) -> Result<InitSegment, ParseError> {
    new_parser(container).push(bytes).map(|events| {
        events
            .into_iter()
            .find_map(|e| match e {
                ParseEvent::Init(i) => Some(i),
                _ => None,
            })
            .expect("an initialization segment")
    })
}

#[track_caller]
fn assert_unsupported(r: Result<InitSegment, ParseError>, what: &str) {
    match r {
        Err(ParseError::Unsupported(_)) => {}
        other => panic!("{what}: expected Unsupported, got {other:?}"),
    }
}

#[track_caller]
fn assert_audio(r: Result<InitSegment, ParseError>, rate: u32, channels: u32, what: &str) {
    let init = r.unwrap_or_else(|e| panic!("{what}: {e:?}"));
    let t = &init.tracks[0];
    assert_eq!((t.sample_rate, t.channels), (rate, channels), "{what}");
}

// ---- WebM ----

fn lacing(n: usize) -> Vec<u8> {
    let mut v = vec![255; n / 255];
    v.push((n % 255) as u8);
    v
}

/// A Vorbis `CodecPrivate` (three packets in Xiph lacing) with the given identification header values.
fn vorbis_private(channels: u8, rate: u32) -> Vec<u8> {
    let mut ident = vec![1];
    ident.extend_from_slice(b"vorbis");
    ident.extend_from_slice(&0u32.to_le_bytes());
    ident.push(channels);
    ident.extend_from_slice(&rate.to_le_bytes());
    ident.extend_from_slice(&[0; 12]);
    ident.extend_from_slice(&[0xB8, 1]);
    let mut comment = vec![3];
    comment.extend_from_slice(b"vorbis");
    comment.extend_from_slice(&[0; 8]);
    comment.push(1);
    let mut setup = vec![5];
    setup.extend_from_slice(b"vorbis");
    setup.extend_from_slice(&[0x55; 20]);
    let mut v = vec![2];
    v.extend_from_slice(&lacing(ident.len()));
    v.extend_from_slice(&lacing(comment.len()));
    v.extend_from_slice(&ident);
    v.extend_from_slice(&comment);
    v.extend_from_slice(&setup);
    v
}

/// A WebM `TrackEntry` of an audio track; `audio` is the `Audio` element's (rate, channels), if any.
fn webm_audio(codec: &str, private: Option<&[u8]>, audio: Option<(f64, u64)>) -> Vec<u8> {
    let mut p = cat(&[
        &uint_elem(0xD7, 1),
        &uint_elem(0x83, 2),
        &elem(0x86, codec.as_bytes()),
    ]);
    if let Some(private) = private {
        p.extend_from_slice(&elem(0x63A2, private));
    }
    if let Some((rate, channels)) = audio {
        p.extend_from_slice(&elem(
            0xE1,
            &cat(&[&float_elem(0xB5, rate), &uint_elem(0x9F, channels)]),
        ));
    }
    elem(TRACK_ENTRY, &p)
}

fn webm(entry: Vec<u8>) -> Result<InitSegment, ParseError> {
    push_all(Container::WebM, &webm_init(&[entry]))
}

#[test]
fn a_webm_vorbis_rate_above_768_khz_or_with_more_than_8_channels_is_refused() {
    let vorbis = |channels: u8, rate: u32| {
        webm(webm_audio(
            "A_VORBIS",
            Some(&vorbis_private(channels, rate)),
            None,
        ))
    };
    // At the limits it is read.
    assert_audio(
        vorbis(8, MAX_AUDIO_SAMPLE_RATE),
        MAX_AUDIO_SAMPLE_RATE,
        8,
        "at the limits",
    );
    assert_audio(vorbis(1, 1), 1, 1, "the smallest");
    // One past them, or far past (the rate that became `rate=-1` in the caps), it is not.
    for rate in [MAX_AUDIO_SAMPLE_RATE + 1, 1_000_000, u32::MAX] {
        assert_unsupported(vorbis(2, rate), &format!("rate {rate}"));
    }
    for channels in [9u8, 16, 255] {
        assert_unsupported(vorbis(channels, 44_100), &format!("{channels} channels"));
    }
    // 0 stays Malformed.
    for e in [vorbis(0, 44_100), vorbis(2, 0)] {
        assert!(matches!(e, Err(ParseError::Malformed(_))), "{e:?}");
    }
}

#[test]
fn a_webm_ac3_or_eac3_track_with_more_than_8_channels_is_refused() {
    for codec in ["A_AC3", "A_EAC3"] {
        let track =
            |rate: f64, channels: u64| webm(webm_audio(codec, None, Some((rate, channels))));
        assert_audio(track(48_000.0, 6), 48_000, 6, codec);
        assert_audio(track(48_000.0, 8), 48_000, 8, codec);
        for channels in [9u64, 255, 65_536] {
            assert_unsupported(
                track(48_000.0, channels),
                &format!("{codec} with {channels} channels"),
            );
        }
        // Too large for 32 bits, so never a number at all: refused as well (it counts as no channels).
        for channels in [u64::from(u32::MAX) + 1, u64::MAX] {
            let r = track(48_000.0, channels);
            assert!(
                matches!(
                    r,
                    Err(ParseError::Malformed(_) | ParseError::Unsupported(_))
                ),
                "{codec} with {channels} channels: {r:?}"
            );
        }
    }
}

#[test]
fn a_webm_opus_track_with_more_than_8_channels_or_an_absurd_rate_is_refused() {
    let head_with = |channels: u8, rate: u32| {
        let mut h = opus_head(channels);
        h[12..16].copy_from_slice(&rate.to_le_bytes());
        h
    };
    let opus =
        |head: &[u8], audio: Option<(f64, u64)>| webm(webm_audio("A_OPUS", Some(head), audio));
    assert_audio(opus(&head_with(8, 48_000), None), 48_000, 8, "8 channels");
    assert_unsupported(opus(&head_with(9, 48_000), None), "9 channels");
    assert_unsupported(opus(&head_with(255, 48_000), None), "255 channels");
    // The rate of the Audio element, and the original rate of the OpusHead, are bounded the same way.
    assert_unsupported(
        opus(&head_with(2, 48_000), Some((1.0e9, 2))),
        "an Audio rate of 1 GHz",
    );
    assert_unsupported(
        opus(&head_with(2, u32::MAX), None),
        "an OpusHead rate of 4 GHz",
    );
    assert_audio(
        opus(&head_with(2, MAX_AUDIO_SAMPLE_RATE), None),
        MAX_AUDIO_SAMPLE_RATE,
        2,
        "at the limit",
    );
}

#[test]
fn a_webm_flac_streaminfo_with_a_rate_above_768_khz_is_refused() {
    // STREAMINFO (34 bytes) with a 20-bit rate (up to 1,048,575 Hz), 3 bits of channels - 1.
    let flac = |rate: u32, channels: u8| {
        let mut v = vec![0x80, 0, 0, 34, 0x10, 0, 0x10, 0, 0, 0, 0, 0, 0, 0];
        let packed = (u64::from(rate) << 44) | (u64::from(channels - 1) << 41) | (15u64 << 36);
        v.extend_from_slice(&packed.to_be_bytes());
        v.extend_from_slice(&[0; 16]);
        webm(webm_audio("A_FLAC", Some(&v), None))
    };
    assert_audio(
        flac(MAX_AUDIO_SAMPLE_RATE, 8),
        MAX_AUDIO_SAMPLE_RATE,
        8,
        "limits",
    );
    assert_unsupported(flac(MAX_AUDIO_SAMPLE_RATE + 1, 2), "768,001 Hz");
    assert_unsupported(flac(1_048_575, 2), "the largest 20-bit rate");
}

// ---- MP4 ----

fn mp4(spec: TrackSpec) -> Result<InitSegment, ParseError> {
    push_all(Container::Mp4, &init_of_tracks(&[spec]))
}

fn audio_spec(typ: &str, channels: u16, rate: u32, children: &[u8]) -> TrackSpec {
    TrackSpec {
        id: 1,
        handler: "soun",
        timescale: 48_000,
        entry: audio_entry(typ, channels, rate, children),
    }
}

/// An `esds` box (whole) for the given object type and AudioSpecificConfig.
fn esds_of(object_type: u8, asc: &[u8]) -> Vec<u8> {
    let mut dcd = vec![object_type, 0x15, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
    dcd.extend_from_slice(&[0x05, asc.len() as u8]);
    dcd.extend_from_slice(asc);
    let mut es = vec![0, 1, 0, 0x04, dcd.len() as u8];
    es.extend_from_slice(&dcd);
    let mut p = vec![0x03, es.len() as u8];
    p.extend_from_slice(&es);
    full("esds", 0, 0, &p)
}

/// An AudioSpecificConfig of AAC-LC with an explicit 24-bit sampling frequency and `config` channels.
fn asc_with_rate(rate: u32, config: u8) -> Vec<u8> {
    // 5 bits object type (2), 4 bits index (15), 24 bits rate, 4 bits channel configuration: 37 bits.
    let bits = (2u64 << 32) | (15u64 << 28) | (u64::from(rate) << 4) | u64::from(config);
    (bits << 3).to_be_bytes()[3..].to_vec()
}

const DEC3: [u8; 5] = [0x01, 0x80, 0x00, 0x00, 0x00]; // 48 kHz, acmod 0 + 1 substream
const DOPS_STEREO: [u8; 11] = [0, 2, 0x01, 0x38, 0, 0, 0xBB, 0x80, 0, 0, 0]; // v0, 2 ch, 312, 48000, gain 0, family 0

#[test]
fn an_mp4_eac3_entry_with_more_than_8_channels_is_refused() {
    // The `ec-3` entry's channel count wins over the dec3 box's, and it is a 16-bit field.
    let dec3 = bx("dec3", &DEC3);
    assert_audio(
        mp4(audio_spec("ec-3", 6, 48_000, &dec3)),
        48_000,
        6,
        "6 channels",
    );
    assert_audio(
        mp4(audio_spec("ec-3", 8, 48_000, &dec3)),
        48_000,
        8,
        "8 channels",
    );
    for channels in [9u16, 255, 0xFFFF] {
        assert_unsupported(
            mp4(audio_spec("ec-3", channels, 48_000, &dec3)),
            &format!("{channels} channels"),
        );
    }
}

#[test]
fn an_mp4_aac_or_mp3_entry_with_more_than_8_channels_is_refused() {
    // The entry's channel count (a 16-bit field) is the one the caps use for both.
    for (what, object_type) in [("AAC", 0x40u8), ("MP3", 0x6B)] {
        let children = esds_of(object_type, &[0x11, 0x88]);
        assert_audio(
            mp4(audio_spec("mp4a", 8, 48_000, &children)),
            48_000,
            8,
            what,
        );
        for channels in [9u16, 0xFFFF] {
            assert_unsupported(
                mp4(audio_spec("mp4a", channels, 48_000, &children)),
                &format!("{what} with {channels} channels"),
            );
        }
    }
}

#[test]
fn an_mp4_opus_dops_with_more_than_8_channels_is_refused() {
    let dops = |channels: u8| {
        let mut d = DOPS_STEREO;
        d[1] = channels;
        bx("dOps", &d)
    };
    assert_audio(
        mp4(audio_spec("Opus", 2, 48_000, &dops(8))),
        48_000,
        8,
        "8 channels",
    );
    for channels in [9u8, 255] {
        assert_unsupported(
            mp4(audio_spec("Opus", 2, 48_000, &dops(channels))),
            &format!("{channels} channels"),
        );
    }
}

#[test]
fn an_aac_audio_specific_config_with_an_explicit_rate_above_768_khz_is_refused() {
    let aac = |rate: u32| {
        mp4(audio_spec(
            "mp4a",
            2,
            0, // the entry gives no rate, so the AudioSpecificConfig's is the one that counts
            &esds_of(0x40, &asc_with_rate(rate, 2)),
        ))
    };
    assert_audio(
        aac(MAX_AUDIO_SAMPLE_RATE),
        MAX_AUDIO_SAMPLE_RATE,
        2,
        "at the limit",
    );
    assert_unsupported(aac(MAX_AUDIO_SAMPLE_RATE + 1), "768,001 Hz");
    assert_unsupported(aac(0xFF_FFFF), "the largest 24-bit rate");
    // An explicit rate of 0 names nothing.
    assert!(matches!(aac(0), Err(ParseError::Malformed(_))), "rate 0");
}

#[test]
fn an_mp4_flac_entry_with_a_streaminfo_rate_above_768_khz_is_refused() {
    let dfla = |rate: u32| {
        let mut v = vec![0x80, 0, 0, 34, 0x10, 0, 0x10, 0, 0, 0, 0, 0, 0, 0];
        let packed = (u64::from(rate) << 44) | (1u64 << 41) | (15u64 << 36);
        v.extend_from_slice(&packed.to_be_bytes());
        v.extend_from_slice(&[0; 16]);
        let mut d = vec![0, 0, 0, 0];
        d.extend_from_slice(&v);
        bx("dfLa", &d)
    };
    assert_audio(
        mp4(audio_spec("fLaC", 2, 48_000, &dfla(MAX_AUDIO_SAMPLE_RATE))),
        MAX_AUDIO_SAMPLE_RATE,
        2,
        "at the limit",
    );
    assert_unsupported(
        mp4(audio_spec(
            "fLaC",
            2,
            48_000,
            &dfla(MAX_AUDIO_SAMPLE_RATE + 1),
        )),
        "768,001 Hz",
    );
}

#[test]
fn the_limits_are_the_documented_ones() {
    assert_eq!(MAX_AUDIO_SAMPLE_RATE, 768_000);
    assert_eq!(MAX_AUDIO_CHANNELS, 8);
}
