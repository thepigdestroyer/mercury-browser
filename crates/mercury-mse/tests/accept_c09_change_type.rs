// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! C09 of the GstMSE spike at core level (lane med-1, item 3): `changeType` mid-stream, what a player
//! does when it changes codec. One video source buffer is created for one type and switched with
//! `change_type` to another, each switch followed by the new type's initialization segment.

mod model_support;

use mercury_mse::api::{
    AppendErrorKind, CodecId, EosError, MediaSourceBackend, MediaSourceReadyState, MseError,
    MseEvent, TrackKind,
};
use mercury_mse_testkit::SegmentedStream;
use model_support::*;

/// Plays `segments[i]`'s segment `i` through one source buffer, switching type and appending the new
/// init segment whenever the stream changes. Returns the rig and the buffer, with the stream ended.
fn change_run(segments: [&SegmentedStream; 4]) -> (Rig, u32) {
    let rig = Rig::new();
    let first = segments[0];
    let id = rig.add(&first.mime);
    rig.append_ok(id, &first.init.bytes);
    let mut current = first.mime.clone();
    for (i, stream) in segments.iter().enumerate() {
        if stream.mime != current {
            rig.backend
                .change_type(id, &stream.mime)
                .unwrap_or_else(|e| panic!("change_type({:?}): {e}", stream.mime));
            rig.append_ok(id, &stream.init.bytes);
            current = stream.mime.clone();
        }
        rig.append_ok(id, &stream.media[i].bytes);
    }
    (rig, id)
}

fn played(rig: &Rig, id: u32, configs: usize) {
    assert_ranges("buffered", &rig.backend.buffered(id), &[(0.0, 8.0)], 2.0);
    rig.backend.end_of_stream(None).unwrap();
    let feed = rig.backend.frame_feed();
    let d = drain(&*feed, TrackKind::Video);
    assert_eq!(d.frames.len(), 200, "frames served across the switches");
    assert_in_order("video", &d.frames);
    assert_eq!(
        d.configs.len(),
        configs,
        "Config items: {:?}",
        d.configs
            .iter()
            .map(|c| c.codec_string.clone())
            .collect::<Vec<_>>()
    );
    assert_eq!(
        rig.rec.count(|e| matches!(e, MseEvent::AppendError { .. })),
        0
    );
}

#[test]
fn c09_h264_to_vp9_and_back_in_one_source_buffer() {
    let h264 = load("h264-low");
    let vp9 = load("vp9-low");
    let (rig, id) = change_run([&h264, &h264, &vp9, &vp9]);
    played(&rig, id, 2);
    // The feed's configurations are H.264 and then VP9.
    let feed = rig.backend.frame_feed();
    feed.seek(0);
    let d = drain(&*feed, TrackKind::Video);
    assert_eq!(d.configs[0].codec.id(), CodecId::H264);
    assert_eq!(d.configs[1].codec.id(), CodecId::Vp9);

    let (rig, id) = change_run([&vp9, &h264, &vp9, &h264]);
    played(&rig, id, 4);
}

#[test]
fn c09_avc1_42c00c_to_avc1_42c01e_inside_mp4() {
    let low = load("h264-low");
    let high = load("h264-high");
    assert_ne!(low.mime, high.mime);
    let (rig, id) = change_run([&low, &low, &high, &high]);
    played(&rig, id, 2);
}

#[test]
fn c09_the_next_initialization_segment_is_required_after_change_type() {
    let h264 = load("h264-low");
    let vp9 = load("vp9-low");
    // A media segment right after changeType, with no init segment: a decode error.
    let rig = Rig::new();
    let id = rig.add(&h264.mime);
    rig.append_ok(id, &h264.init.bytes);
    rig.append_ok(id, &h264.media[0].bytes);
    rig.backend.change_type(id, &vp9.mime).unwrap();
    assert_eq!(
        rig.append(id, &vp9.media[1].bytes),
        Outcome::Error(AppendErrorKind::Decode),
        "a media segment before the new init segment"
    );
    assert!(!rig.backend.updating(id));
    assert_eq!(rig.backend.ready_state(), MediaSourceReadyState::Ended);

    // Even for the same container and codec string: the old init segment's configuration is gone.
    let rig = Rig::new();
    let id = rig.add(&h264.mime);
    rig.append_ok(id, &h264.init.bytes);
    rig.append_ok(id, &h264.media[0].bytes);
    rig.backend.change_type(id, &h264.mime).unwrap();
    assert_eq!(
        rig.append(id, &h264.media[1].bytes),
        Outcome::Error(AppendErrorKind::Decode)
    );

    // The old type's init segment after the change is the wrong container.
    let rig = Rig::new();
    let id = rig.add(&h264.mime);
    rig.append_ok(id, &h264.init.bytes);
    rig.backend.change_type(id, &vp9.mime).unwrap();
    assert_eq!(
        rig.append(id, &h264.init.bytes),
        Outcome::Error(AppendErrorKind::Decode)
    );
}

#[test]
fn c09_change_type_checks() {
    let h264 = load("h264-low");
    let vp9 = load("vp9-low");
    let rig = Rig::new();
    let id = rig.add(&h264.mime);
    assert!(matches!(
        rig.backend.change_type(id, ""),
        Err(MseError::Type(_))
    ));
    assert!(matches!(
        rig.backend.change_type(id, "video/foo"),
        Err(MseError::NotSupported(_))
    ));
    assert!(matches!(
        rig.backend
            .change_type(id, "video/mp4; codecs=\"hev1.1.6.L93.B0\""),
        Err(MseError::NotSupported(_))
    ));
    assert!(matches!(
        rig.backend.change_type(99, &vp9.mime),
        Err(MseError::NotFound(_))
    ));
    // Accepted: nothing changed by the refusals, and the buffer still takes its own init segment.
    rig.append_ok(id, &h264.init.bytes);
    rig.backend.change_type(id, &vp9.mime).unwrap();
    rig.append_ok(id, &vp9.init.bytes);
    rig.append_ok(id, &vp9.media[0].bytes);
    assert_ranges("buffered", &rig.backend.buffered(id), &[(0.0, 2.0)], 2.0);

    // Removed buffers refuse.
    rig.backend.remove_source_buffer(id).unwrap();
    assert!(matches!(
        rig.backend.change_type(id, &h264.mime),
        Err(MseError::InvalidState(_))
    ));
}

#[test]
fn c09_change_type_while_updating_is_invalid_state() {
    use std::sync::mpsc;
    use std::sync::{Arc, Mutex};
    let h264 = load("h264-low");
    let rig = Rig::new();
    let id = rig.add(&h264.mime);
    // Park the worker inside the InitSegment closure: the append is then updating, deterministically.
    let (parked_tx, parked_rx) = mpsc::channel();
    let (go_tx, go_rx) = mpsc::channel::<()>();
    let go_rx = Arc::new(Mutex::new(go_rx));
    rig.rec.add_hook(move |e| {
        if matches!(e, MseEvent::InitSegment { .. }) {
            parked_tx.send(()).ok();
            go_rx.lock().unwrap().recv().ok();
        }
    });
    rig.backend.append(id, h264.init.bytes.clone()).unwrap();
    parked_rx
        .recv_timeout(WAIT)
        .expect("the worker reaches the InitSegment closure");
    assert!(rig.backend.updating(id));
    assert!(matches!(
        rig.backend.change_type(id, &h264.mime),
        Err(MseError::InvalidState(_))
    ));
    go_tx.send(()).unwrap();
    assert!(rig.rec.wait_for(WAIT, |log| {
        log.iter()
            .any(|r| matches!(r.event, MseEvent::AppendDone { .. }))
    }));
    rig.wait_idle(&[id]);
    rig.backend.change_type(id, &h264.mime).unwrap();
    let _ = EosError::Network;
}
