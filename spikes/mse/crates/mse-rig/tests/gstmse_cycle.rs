// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! Integration tests of the rig over the bundled GStreamer 1.28.9: type support, a full append/play
//! cycle in both pipeline shapes, the error mapping and the teardown orders. One `gstreamer::init`
//! for the process (`common::init`), plugins loaded the way Servo loads them (`servo` mode), one rig at
//! a time. Run through `spikes/mse/run.sh test -p mse-rig`.
//!
//! The fixtures are played as ONE source buffer holding audio and video (`h264-aac`): GstMSE 1.28.9
//! deadlocks when audio and video are two source buffers (the README explains); that case is the
//! ignored test at the end.

mod common;

use std::time::Duration;

use gstreamer::ClockTime;
use mse_rig::RigError;
use mse_rig::backend::{EosError, MediaSourceReadyState, MseBackend, MseError};
use mse_rig::env::PluginMode;
use mse_rig::events::{ElementReadyState, MseEvent, PlayEvent};
use mse_rig::rig::{Rig, RigConfig, Shape, TeardownOrder};

use common::{
    ALL_ORDERS, colour_letters, init, one_rig_at_a_time, play_fixture, updates_are_well_ordered,
};

fn gst() {
    let info = init(PluginMode::servo_default());
    assert!(info.msesrc_available, "msesrc is missing: {info:?}");
    assert!(
        info.failed_plugins.is_empty(),
        "plugins that did not load: {:?}",
        info.failed_plugins
    );
    assert_eq!(info.version, "1.28.9");
}

#[test]
fn the_fixture_type_strings_are_supported_and_bogus_ones_are_not() {
    gst();
    for fixture in mse_segments::FIXTURES {
        let derived = mse_segments::load(fixture.name)
            .expect("fixture loads")
            .mime;
        let supported = MseBackend::is_type_supported(&derived);
        if fixture.name.starts_with("av1") {
            // AV1 needs dav1d, which Mercury does not ship yet: only reported.
            println!("{}: {derived} supported = {supported}", fixture.name);
        } else {
            assert!(
                supported,
                "{} says {derived:?}, which GstMSE refuses",
                fixture.name
            );
        }
    }
    assert!(!MseBackend::is_type_supported(""));
    assert!(!MseBackend::is_type_supported(
        "video/mp4; codecs=\"nonsense\""
    ));
}

fn assert_full_cycle(shape: Shape) {
    let _one = one_rig_at_a_time();
    gst();
    let cycle = play_fixture(shape, "h264-aac");
    let rig = &cycle.rig;
    assert!(cycle.eos, "no end of stream in 30 s ({shape:?})");

    let video = rig.video.lock().unwrap().clone();
    let audio = rig.audio.lock().unwrap().clone();
    assert!(
        (198..=202).contains(&video.count),
        "{} video frames",
        video.count
    );
    assert!(
        (370..=380).contains(&audio.count),
        "{} audio buffers",
        audio.count
    );
    assert_eq!(video.out_of_order, 0);
    assert_eq!(video.caps_changes.len(), 1, "{:?}", video.caps_changes);
    assert!(
        video.caps_changes[0].caps.contains("width=(int)320"),
        "{:?}",
        video.caps_changes
    );
    let letters = colour_letters(&video.colour);
    assert!(
        letters.starts_with('R') && letters.ends_with('B'),
        "colour per second: {letters}"
    );
    assert!(!letters.contains('?'), "colour per second: {letters}");
    let first_blue = video
        .colour
        .iter()
        .find(|c| c.dominant() == mse_rig::frames::Dominant::Blue)
        .unwrap();
    assert!(
        (3_000_000_000..=5_000_000_000).contains(&first_blue.pts.nseconds()),
        "the picture turns blue at {first_blue:?}"
    );
    assert!(!video.arrivals.is_empty() && video.arrivals.len() as u64 == video.count);

    let events = rig.log.snapshot();
    // GstMSE 1.28.9 also emits a lone `Update`, `UpdateEnd` pair (no `UpdateStart`) when
    // `end_of_stream` aborts the source buffers: the appends themselves are what is checked here.
    let appends: Vec<_> = events
        .iter()
        .take_while(|e| e.event != MseEvent::SourceEnded)
        .cloned()
        .collect();
    assert!(
        updates_are_well_ordered(&appends, cycle.ids[0]),
        "update events out of order: {:?}",
        appends.iter().map(|e| &e.event).collect::<Vec<_>>()
    );
    let after_eos: Vec<_> = events
        .iter()
        .skip(appends.len())
        .map(|e| &e.event)
        .collect();
    println!("events after SourceEnded ({shape:?}): {after_eos:?}");
    assert_eq!(
        events
            .iter()
            .filter(|e| e.event == MseEvent::SourceOpen)
            .count(),
        1
    );
    assert!(
        events
            .iter()
            .any(|e| e.event == MseEvent::SourceBufferAdded(0))
    );
    assert!(
        events
            .iter()
            .any(|e| e.event == MseEvent::Play(PlayEvent::EndOfStream))
    );
    assert!(
        !events
            .iter()
            .any(|e| matches!(e.event, MseEvent::Error(_) | MseEvent::Abort(_))),
        "an error or abort event"
    );
    assert_eq!(rig.backend.ready_state(), MediaSourceReadyState::Ended);
    assert!(rig.element_ready_state() >= ElementReadyState::HaveMetadata);
    assert!(rig.msesrc().is_some());
    assert_eq!(rig.backend.source_buffer_count(), 1);

    let buffered = rig.backend.buffered(cycle.ids[0]).expect("buffered");
    assert_eq!(buffered.len(), 1, "{buffered:?}");
    assert_eq!(buffered[0].start, ClockTime::ZERO);
    assert!(
        (7_900_000_000..=8_100_000_000).contains(&buffered[0].end.nseconds()),
        "{buffered:?}"
    );

    let decoders = rig.chosen_decoders();
    assert_eq!(
        decoders.len(),
        2,
        "one video and one audio decoder: {decoders:?}"
    );
    assert!(!rig.describe_pipeline().is_empty());

    let info = cycle.rig.shutdown().expect("teardown");
    assert!(
        info.took < Duration::from_secs(10),
        "teardown took {:?}",
        info.took
    );
}

#[test]
fn a_full_append_play_cycle_in_the_playbin_shape() {
    assert_full_cycle(Shape::PlayBin);
}

#[test]
fn a_full_append_play_cycle_in_the_bare_shape() {
    assert_full_cycle(Shape::Bare);
}

#[test]
fn the_error_classes_come_through_the_backend() {
    let _one = one_rig_at_a_time();
    gst();
    let rig = Rig::new(RigConfig {
        sync: false,
        ..RigConfig::default()
    })
    .expect("a rig");
    let b = &rig.backend;
    assert_eq!(b.ready_state(), MediaSourceReadyState::Open);
    assert!(matches!(b.append(99, b"x"), Err(MseError::NotFound(_))));
    assert!(matches!(b.add_source_buffer(""), Err(MseError::Type(_))));
    assert!(!b.updating(99));
    assert!(matches!(b.buffered(99), Err(MseError::NotFound(_))));
    assert!(matches!(
        b.set_timestamp_offset(99, 1),
        Err(MseError::NotFound(_))
    ));
    let id = b
        .add_source_buffer("video/mp4; codecs=\"avc1.42C00C\"")
        .expect("a source buffer");
    assert!(matches!(
        b.set_timestamp_offset(id, -1),
        Err(MseError::NotSupported(_))
    ));
    assert_eq!(b.timestamp_offset(id).unwrap(), Some(ClockTime::ZERO));
    b.end_of_stream(EosError::None)
        .expect("the first end of stream");
    assert!(matches!(
        b.end_of_stream(EosError::None),
        Err(MseError::InvalidState(_))
    ));
    assert_eq!(b.ready_state(), MediaSourceReadyState::Ended);
    let _ = rig.shutdown_with_timeout(
        TeardownOrder::PipelineNullThenBackend,
        Duration::from_secs(20),
    );
}

#[test]
fn every_teardown_order_returns_without_panicking() {
    let _one = one_rig_at_a_time();
    gst();
    for order in ALL_ORDERS {
        let cycle = play_fixture(Shape::PlayBin, "h264-aac");
        let result = cycle
            .rig
            .shutdown_with_timeout(order, Duration::from_secs(20));
        println!("{order:?}: {result:?}");
        match (order, result) {
            (TeardownOrder::PipelineNullThenBackend, result) => {
                result.expect("the default order tears down cleanly");
            }
            (_, Ok(_) | Err(RigError::Timeout(..) | RigError::Pipeline(_))) => {}
            (order, Err(other)) => panic!("{order:?} gave {other:?}"),
        }
    }
}

/// GstMSE 1.28.9 deadlocks with audio and video in two source buffers (the second source buffer's
/// init segment waits for a msesrc lock that the first track's pad task keeps). The test documents it:
/// it passes when playback stalls cleanly (the rig times out and tears down) or when GstMSE has been
/// fixed and all frames arrive; it fails on a crash or a hang the rig's watchdogs do not catch.
#[test]
#[ignore = "takes about 30 s: documents the GstMSE split A/V stall"]
fn split_audio_and_video_either_plays_or_stalls_cleanly() {
    let _one = one_rig_at_a_time();
    gst();
    let video = mse_segments::load("h264-low").unwrap();
    let audio = mse_segments::load("aac").unwrap();
    let rig = Rig::new(RigConfig {
        sync: false,
        ..RigConfig::default()
    })
    .expect("a rig");
    let ids = [
        rig.backend.add_source_buffer(&video.mime).unwrap(),
        rig.backend.add_source_buffer(&audio.mime).unwrap(),
    ];
    let step = Duration::from_secs(10);
    rig.append_and_wait(ids[0], &video.init.bytes, step)
        .unwrap();
    rig.append_and_wait(ids[1], &audio.init.bytes, step)
        .unwrap();
    for index in 0..4 {
        rig.append_and_wait(ids[0], &video.media[index].bytes, step)
            .unwrap();
        rig.append_and_wait(ids[1], &audio.media[index].bytes, step)
            .unwrap();
    }
    rig.play().unwrap();
    rig.backend.end_of_stream(EosError::None).unwrap();
    let played = rig.wait_for_eos(Duration::from_secs(8));
    let frames = rig.video.lock().unwrap().count;
    println!("split A/V: end of stream {played}, {frames} video frames");
    assert!(
        played == (frames == 200),
        "end of stream and the frame count disagree"
    );
    let _ = rig.shutdown_with_timeout(
        TeardownOrder::PipelineNullThenBackend,
        Duration::from_secs(20),
    );
}
