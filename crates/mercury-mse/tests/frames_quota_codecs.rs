// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! The quota (review finding S1: every frame counts its bytes plus a fixed overhead) for the codecs of
//! lane med-1b's EA2 round: HEVC frames count against the video quota, AC-3, E-AC-3, MP3, FLAC and
//! Vorbis frames against the audio quota. A page that appends a stream of one-byte frames is stopped by
//! the quota, whatever the codec says (item 1 `core`; the parsers and fixtures are item 3's).

mod frames_support;

use frames_support::*;
use mercury_mse::{
    CodecConfig, CodecId, CodedFrame, Limits, SourceBufferProcessor, TrackConfig, TrackKind,
};

const QUOTA: usize = 128 * 1024;
const FRAMES: usize = 500_000;

fn limits() -> Limits {
    Limits {
        video_bytes: QUOTA,
        audio_bytes: QUOTA,
        text_bytes: 1_000,
        keep_behind: 5 * SEC,
        keep_ahead: 30 * SEC,
    }
}

/// `config(kind, 0)` with another codec.
fn with_codec(kind: TrackKind, codec: CodecConfig, codec_string: &str) -> TrackConfig {
    TrackConfig {
        codec,
        codec_string: codec_string.to_string(),
        ..config(kind, 0)
    }
}

fn hevc() -> TrackConfig {
    with_codec(
        TrackKind::Video,
        CodecConfig::Hevc {
            hvcc: vec![
                1, 1, 0x60, 0, 0, 0, 0x90, 0, 0, 0, 0, 0, 93, 0xF0, 0, 0xFC, 0xFD, 0xF8, 0xF8, 0,
                0, 3, 0,
            ],
        },
        "hvc1.1.6.L93.B0",
    )
}

fn audio_codec(codec: CodecId) -> TrackConfig {
    let name = match codec {
        CodecId::Ac3 => "ac-3",
        CodecId::Eac3 => "ec-3",
        CodecId::Mp3 => "mp4a.6B",
        CodecId::Flac => "flac",
        CodecId::Vorbis => "vorbis",
        other => panic!("not a new audio codec: {other:?}"),
    };
    with_codec(
        TrackKind::Audio,
        CodecConfig::Other {
            codec,
            config: vec![1, 2, 3, 4],
        },
        name,
    )
}

fn processor_for(configs: Vec<TrackConfig>) -> SourceBufferProcessor {
    let mut p = SourceBufferProcessor::new(limits());
    p.init_segment(&init(configs)).expect("init segment");
    p
}

/// `count` one-byte frames of `kind` from frame number `first`. Video has a keyframe every 50 frames,
/// audio is all keyframes.
fn one_byte_frames(kind: TrackKind, first: usize, count: usize) -> Vec<CodedFrame> {
    let (track_id, dur, gop) = match kind {
        TrackKind::Video => (1, 40 * MS, 50),
        _ => (2, 10 * MS, 1),
    };
    let data = payload(1);
    (first..first + count)
        .map(|i| {
            let t = i as i64 * dur;
            CodedFrame {
                kind,
                track_id,
                pts: t,
                dts: t,
                duration: dur,
                keyframe: i % gop == 0,
                data: data.clone(),
            }
        })
        .collect()
}

fn new_codecs() -> Vec<(&'static str, TrackConfig)> {
    vec![
        ("hevc", hevc()),
        ("ac3", audio_codec(CodecId::Ac3)),
        ("eac3", audio_codec(CodecId::Eac3)),
        ("mp3", audio_codec(CodecId::Mp3)),
        ("flac", audio_codec(CodecId::Flac)),
        ("vorbis", audio_codec(CodecId::Vorbis)),
    ]
}

#[test]
fn quota_500000_one_byte_frames_are_refused_at_once_for_every_new_codec() {
    for (name, cfg) in new_codecs() {
        let kind = cfg.kind;
        let mut p = processor_for(vec![cfg]);
        let frames = one_byte_frames(kind, 0, FRAMES);
        let report = p.evict_report_for_frames(&frames, 0);
        assert!(
            !report.ok,
            "{name}: 500,000 one-byte frames (about {} MB of structures) fit a {QUOTA}-byte quota",
            FRAMES * (1 + OV) / 1_000_000
        );
        assert_eq!(p.bytes(), 0, "{name}: the refusal stored nothing");
    }
}

#[test]
fn quota_one_byte_frames_appended_in_batches_stop_early_for_every_new_codec() {
    for (name, cfg) in new_codecs() {
        let kind = cfg.kind;
        let mut p = processor_for(vec![cfg]);
        let mut stored = 0usize;
        let mut refused_at = None;
        for batch in 0..(FRAMES / 200) {
            let frames = one_byte_frames(kind, batch * 200, 200);
            if !p.evict_report_for_frames(&frames, 0).ok {
                refused_at = Some(stored);
                break;
            }
            p.process_frames(frames).unwrap();
            stored += 200;
            assert!(
                p.bytes() <= QUOTA,
                "{name}: {} bytes over the quota",
                p.bytes()
            );
        }
        let refused_at = refused_at.unwrap_or_else(|| panic!("{name}: {stored} frames accepted"));
        assert!(
            refused_at <= QUOTA / (1 + OV),
            "{name}: {refused_at} frames stored against a quota that holds {}",
            QUOTA / (1 + OV)
        );
        assert!(refused_at > 0, "{name}: nothing at all fits");
        p.check_invariants().unwrap();
    }
}

#[test]
fn quota_hevc_frames_use_the_video_quota_and_not_the_audio_quota() {
    let mut p = processor_for(vec![hevc(), audio_codec(CodecId::Ac3)]);
    let mut stored = 0usize;
    loop {
        let frames = one_byte_frames(TrackKind::Video, stored, 200);
        if !p.evict_report_for_frames(&frames, 0).ok {
            break;
        }
        p.process_frames(frames).unwrap();
        stored += 200;
        assert!(stored < FRAMES, "the video quota never filled");
    }
    // The video quota is full; the audio quota (a different one) is untouched.
    let audio = one_byte_frames(TrackKind::Audio, 0, 200);
    assert!(
        p.evict_report_for_frames(&audio, 0).ok,
        "a full video quota must not refuse AC-3 frames"
    );
    p.process_frames(audio).unwrap();
}

#[test]
fn quota_every_new_audio_codec_uses_the_audio_quota_and_not_the_video_quota() {
    for (name, audio) in new_codecs()
        .into_iter()
        .filter(|(_, c)| c.kind == TrackKind::Audio)
    {
        let mut p = processor_for(vec![hevc(), audio]);
        let mut stored = 0usize;
        loop {
            let frames = one_byte_frames(TrackKind::Audio, stored, 200);
            if !p.evict_report_for_frames(&frames, 0).ok {
                break;
            }
            p.process_frames(frames).unwrap();
            stored += 200;
            assert!(stored < FRAMES, "{name}: the audio quota never filled");
        }
        let video = one_byte_frames(TrackKind::Video, 0, 50);
        assert!(
            p.evict_report_for_frames(&video, 0).ok,
            "{name}: a full audio quota must not refuse HEVC frames"
        );
        p.process_frames(video).unwrap();
    }
}
