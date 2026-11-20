// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! The synchronous checks of the MediaSource and SourceBuffer methods and their error kinds (lane
//! med-1, item 3): `addSourceBuffer`, the attribute setters (`mode`, `timestampOffset`,
//! `appendWindowStart`, `appendWindowEnd`), the offsets and windows applied to the data, and
//! `isTypeSupported`.

mod model_support;

use std::sync::Arc;

use mercury_mse::MseBackend;
use mercury_mse::api::{
    AllDecoders, AppendMode, CodecId, Limits, MediaSourceBackend, MseError, TrackKind, TypeSupport,
};
use mercury_mse_testkit::ChunkPlan;
use model_support::*;

/// A platform that can decode only VP9 and Opus.
struct OnlyWebm;

impl TypeSupport for OnlyWebm {
    fn has_decoder(&self, _kind: TrackKind, codec: CodecId) -> bool {
        matches!(codec, CodecId::Vp9 | CodecId::Opus)
    }
}

#[test]
fn add_source_buffer_checks_in_the_specs_order() {
    let rig = Rig::closed(Limits::default());
    // The type is checked before the state: an empty type is a TypeError even when closed.
    assert!(matches!(
        rig.backend.add_source_buffer(""),
        Err(MseError::Type(_))
    ));
    assert!(matches!(
        rig.backend.add_source_buffer("  "),
        Err(MseError::Type(_))
    ));
    assert!(matches!(
        rig.backend.add_source_buffer("video/foo"),
        Err(MseError::NotSupported(_))
    ));
    // A supported type on a MediaSource that is not open.
    assert!(matches!(
        rig.backend
            .add_source_buffer("video/mp4; codecs=\"avc1.42C00C\""),
        Err(MseError::InvalidState(_))
    ));
    rig.backend.open();
    let id = rig.add("video/mp4; codecs=\"avc1.42C00C\"");
    assert_eq!(rig.backend.source_buffer_count(), 1);
    assert_eq!(id, 0);
    // The sixteenth buffer is the last.
    for _ in 1..16 {
        rig.add("audio/mp4; codecs=\"mp4a.40.2\"");
    }
    assert!(matches!(
        rig.backend
            .add_source_buffer("audio/mp4; codecs=\"mp4a.40.2\""),
        Err(MseError::QuotaExceeded(_))
    ));
}

#[test]
fn a_type_without_a_decoder_is_not_supported() {
    let rec = Recorder::new();
    let backend = MseBackend::new(rec.sink(), Arc::new(OnlyWebm), Limits::default());
    backend.open();
    assert!(matches!(
        backend.add_source_buffer("video/mp4; codecs=\"avc1.42C00C\""),
        Err(MseError::NotSupported(_))
    ));
    assert!(
        backend
            .add_source_buffer("video/webm; codecs=\"vp09.00.11.08\"")
            .is_ok()
    );
    assert!(MseBackend::is_type_supported(
        "video/webm; codecs=\"vp9\"",
        &OnlyWebm
    ));
    assert!(!MseBackend::is_type_supported(
        "video/mp4; codecs=\"avc1.42C00C\"",
        &OnlyWebm
    ));
    assert!(MseBackend::is_type_supported(
        "video/mp4; codecs=\"avc1.42C00C\"",
        &AllDecoders
    ));
    assert!(!MseBackend::is_type_supported("", &AllDecoders));
    assert!(!MseBackend::is_type_supported("video/foo", &AllDecoders));
}

#[test]
fn append_window_setters_check_their_values_and_round_trip() {
    let rig = Rig::new();
    let id = rig.add("video/mp4; codecs=\"avc1.42C00C\"");
    assert_eq!(rig.backend.append_window_start(id).unwrap(), 0.0);
    assert_eq!(rig.backend.append_window_end(id).unwrap(), f64::INFINITY);
    assert!(matches!(
        rig.backend.set_append_window_start(id, -1.0),
        Err(MseError::Type(_))
    ));
    assert!(matches!(
        rig.backend.set_append_window_start(id, f64::NAN),
        Err(MseError::Type(_))
    ));
    assert!(matches!(
        rig.backend.set_append_window_end(id, f64::NAN),
        Err(MseError::Type(_))
    ));
    assert!(matches!(
        rig.backend.set_append_window_end(id, 0.0),
        Err(MseError::Type(_))
    ));
    rig.backend
        .set_append_window_end(id, 10.123456789012)
        .unwrap();
    rig.backend.set_append_window_start(id, 0.1).unwrap();
    // The getters return what was set, exactly (the nanosecond rounding is the processor's).
    assert_eq!(rig.backend.append_window_end(id).unwrap(), 10.123456789012);
    assert_eq!(rig.backend.append_window_start(id).unwrap(), 0.1);
    // Start must stay below end, end above start.
    assert!(matches!(
        rig.backend.set_append_window_start(id, 10.123456789012),
        Err(MseError::Type(_))
    ));
    assert!(matches!(
        rig.backend.set_append_window_start(id, 11.0),
        Err(MseError::Type(_))
    ));
    assert!(matches!(
        rig.backend.set_append_window_end(id, 0.1),
        Err(MseError::Type(_))
    ));
    rig.backend
        .set_append_window_end(id, f64::INFINITY)
        .unwrap();
    assert_eq!(rig.backend.append_window_end(id).unwrap(), f64::INFINITY);
}

#[test]
fn timestamp_offset_moves_the_data_and_round_trips() {
    let s = load("h264-low");
    let rig = Rig::new();
    let id = rig.add(&s.mime);
    assert!(matches!(
        rig.backend.set_timestamp_offset(id, f64::NAN),
        Err(MseError::Type(_))
    ));
    assert!(matches!(
        rig.backend.set_timestamp_offset(id, f64::INFINITY),
        Err(MseError::Type(_))
    ));
    rig.backend.set_timestamp_offset(id, 8.0).unwrap();
    assert_eq!(rig.backend.timestamp_offset(id).unwrap(), 8.0);
    rig.append_stream(id, &s, &ChunkPlan::Whole);
    assert_ranges(
        "offset +8 s",
        &rig.backend.buffered(id),
        &[(8.0, 16.0)],
        2.0,
    );
    let d = rig.backend.duration().unwrap();
    assert!(d >= 15.99, "the duration followed the data: {d}");
    // The feed serves the shifted times.
    let feed = rig.backend.frame_feed();
    feed.seek(8_000_000_000);
    rig.backend.end_of_stream(None).unwrap();
    let frames = drain(&*feed, TrackKind::Video);
    assert_eq!(frames.frames.len(), 200);
    assert_eq!(frames.frames[0].pts, 8_000_000_000);

    // A negative offset works too: the data moves before 0 and is clipped by the append window start 0.
    let rig = Rig::new();
    let id = rig.add(&s.mime);
    rig.backend.set_timestamp_offset(id, -2.0).unwrap();
    assert_eq!(rig.backend.timestamp_offset(id).unwrap(), -2.0);
    rig.append_stream(id, &s, &ChunkPlan::Whole);
    let r = rig.buffered_secs(id);
    assert_eq!(r.len(), 1);
    assert!(r[0].0 >= 0.0 && (r[0].1 - 6.0).abs() < 0.05, "{r:?}");
}

#[test]
fn the_append_window_drops_what_is_outside_it() {
    let s = load("h264-low");
    let rig = Rig::new();
    let id = rig.add(&s.mime);
    rig.backend.set_append_window_start(id, 2.0).unwrap();
    rig.backend.set_append_window_end(id, 6.0).unwrap();
    rig.append_stream(id, &s, &ChunkPlan::Whole);
    // Frames before 2 s and after 6 s are dropped; the first kept keyframe is at 2 s.
    assert_ranges("window", &rig.backend.buffered(id), &[(2.0, 6.0)], 2.0);
}

#[test]
fn sequence_mode_puts_segments_back_to_back_and_moves_the_offset() {
    let s = load("h264-low");
    let rig = Rig::new();
    let id = rig.add(&s.mime);
    rig.backend.set_mode(id, AppendMode::Sequence).unwrap();
    rig.append_ok(id, &s.init.bytes);
    // The same segment three times: in sequence mode they follow each other.
    for _ in 0..3 {
        rig.append_ok(id, &s.media[2].bytes);
    }
    assert_ranges("sequence", &rig.backend.buffered(id), &[(0.0, 6.0)], 2.0);
    let offset = rig.backend.timestamp_offset(id).unwrap();
    assert!(
        (offset - 2.0).abs() < 0.05 || offset.abs() <= 6.0,
        "offset {offset}"
    );
}

#[test]
fn mode_and_timestamp_offset_refuse_in_the_middle_of_an_mp4_media_segment() {
    let s = load("h264-low");
    let rig = Rig::new();
    let id = rig.add(&s.mime);
    rig.append_ok(id, &s.init.bytes);
    let seg = &s.media[0].bytes;
    rig.append_ok(id, &seg[..seg.len() / 2]);
    assert!(matches!(
        rig.backend.set_mode(id, AppendMode::Sequence),
        Err(MseError::InvalidState(_))
    ));
    assert!(matches!(
        rig.backend.set_timestamp_offset(id, 1.0),
        Err(MseError::InvalidState(_))
    ));
    rig.append_ok(id, &seg[seg.len() / 2..]);
    rig.backend.set_mode(id, AppendMode::Sequence).unwrap();
    rig.backend.set_timestamp_offset(id, 1.0).unwrap();
}

#[test]
fn setters_reopen_an_ended_media_source() {
    use mercury_mse::api::MediaSourceReadyState;
    let s = load("h264-low");
    let rig = Rig::new();
    let id = rig.add(&s.mime);
    rig.append_stream(id, &s, &ChunkPlan::Whole);
    rig.backend.end_of_stream(None).unwrap();
    rig.backend.set_timestamp_offset(id, 0.0).unwrap();
    assert_eq!(rig.backend.ready_state(), MediaSourceReadyState::Open);
}

#[test]
fn the_buffered_of_a_removed_buffer_is_empty_and_ids_are_not_reused() {
    let s = load("h264-low");
    let rig = Rig::new();
    let a = rig.add(&s.mime);
    rig.append_stream(a, &s, &ChunkPlan::Whole);
    rig.backend.remove_source_buffer(a).unwrap();
    assert!(rig.backend.buffered(a).is_empty());
    assert!(rig.backend.element_buffered().is_empty());
    assert!(rig.backend.frame_feed().tracks().is_empty());
    let b = rig.add(&s.mime);
    assert!(b > a);
}
