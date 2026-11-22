// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! The `pssh` / protected-track hand-off of EA15 (decision D16 of lane med-1b), through a real
//! `MseBackend` (worker thread, parser, processor): an initialization segment with no protection data
//! says `protected: false`; one with `pssh` boxes only plays and says `protected: true` and the boxes
//! come back byte for byte from the parser (T08); one that declares a protected track (`encv`/`enca`,
//! `sinf`, `tenc`) fails the append with `AppendErrorKind::Unsupported` (a clear error, not a hang and
//! not silence) because no decryptor exists before MED-5.
//! Run with `cargo test -p mercury-mse --test parse_protection`.

use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use mercury_mse::MseBackend;
use mercury_mse::api::{
    AllDecoders, AppendErrorKind, EventSink, InitInfo, Limits, MediaSourceBackend, MseEvent, OpId,
};
use mercury_mse_testkit::{load, with_cenc_protection, with_pssh};

const WIDEVINE: [u8; 16] = [
    0xED, 0xEF, 0x8B, 0xA9, 0x79, 0xD6, 0x4A, 0xCE, 0xA3, 0xC8, 0x27, 0xDC, 0xD5, 0x1D, 0x21, 0xED,
];

#[derive(Default)]
struct Log {
    events: Mutex<Vec<MseEvent>>,
    changed: Condvar,
}

impl Log {
    fn sink(self: &Arc<Self>) -> EventSink {
        let log = Arc::clone(self);
        Arc::new(move |event| {
            log.events.lock().unwrap().push(event);
            log.changed.notify_all();
        })
    }

    /// Waits for the completion (`AppendDone` or `AppendError`) of `op` and returns every event seen so
    /// far. A hang is a failure, never a wait.
    fn completion_of(&self, op: OpId) -> Vec<MseEvent> {
        let deadline = Instant::now() + Duration::from_secs(10);
        let mut events = self.events.lock().unwrap();
        loop {
            let done = events.iter().any(|e| {
                matches!(e, MseEvent::AppendDone { op: o, .. } | MseEvent::AppendError { op: o, .. } if *o == op)
            });
            if done {
                return events.clone();
            }
            let left = deadline
                .checked_duration_since(Instant::now())
                .expect("no completion event for the append within 10 s");
            events = self.changed.wait_timeout(events, left).unwrap().0;
        }
    }
}

fn init_of(events: &[MseEvent]) -> Option<&InitInfo> {
    events.iter().find_map(|e| match e {
        MseEvent::InitSegment { init, .. } => Some(init),
        _ => None,
    })
}

fn error_of(events: &[MseEvent]) -> Option<AppendErrorKind> {
    events.iter().find_map(|e| match e {
        MseEvent::AppendError { kind, .. } => Some(*kind),
        _ => None,
    })
}

/// A backend with one source buffer of type `mime`, an init segment appended.
fn append_init(mime: &str, bytes: Vec<u8>) -> Vec<MseEvent> {
    let log = Arc::new(Log::default());
    let backend = MseBackend::new(log.sink(), Arc::new(AllDecoders), Limits::default());
    backend.open();
    let id = backend.add_source_buffer(mime).expect("a source buffer");
    let op = backend.append(id, bytes).expect("an accepted append");
    log.completion_of(op)
}

#[test]
fn a_clear_initialization_segment_is_not_protected() {
    for name in ["h264-low", "aac", "h264-aac", "hevc-low", "eac3"] {
        let stream = load(name).unwrap();
        let events = append_init(&stream.mime, stream.init.bytes.clone());
        assert_eq!(error_of(&events), None, "{name}");
        let init = init_of(&events).expect("an initialization segment");
        assert!(!init.protected, "{name}");
        assert!(init.first, "{name}");
    }
}

#[test]
fn pssh_boxes_alone_play_and_mark_the_segment_protected() {
    for name in ["h264-low", "aac", "hevc-low", "eac3"] {
        let stream = load(name).unwrap();
        let init = with_pssh(&stream.init, WIDEVINE, b"license server data").unwrap();
        let events = append_init(&stream.mime, init.bytes);
        assert_eq!(
            error_of(&events),
            None,
            "{name}: a clear stream with pssh plays"
        );
        let info = init_of(&events).expect("an initialization segment");
        assert!(info.protected, "{name}");
        assert_eq!(info.tracks.len(), 1, "{name}");
    }
}

#[test]
fn a_protected_track_fails_the_append_with_unsupported() {
    for name in ["h264-low", "aac", "h264-aac"] {
        let stream = load(name).unwrap();
        for iv in [8u8, 16] {
            let protected = with_cenc_protection(&stream.init, [9; 16], iv).unwrap();
            let events = append_init(&stream.mime, protected.bytes.clone());
            assert_eq!(
                error_of(&events),
                Some(AppendErrorKind::Unsupported),
                "{name} iv {iv}: {events:?}"
            );
            assert!(
                init_of(&events).is_none(),
                "{name}: no init segment is reported"
            );
            // With pssh boxes next to it (the real shape of an EME stream) it is refused the same way.
            let with_box = with_pssh(&protected, WIDEVINE, b"x").unwrap();
            let events = append_init(&stream.mime, with_box.bytes);
            assert_eq!(
                error_of(&events),
                Some(AppendErrorKind::Unsupported),
                "{name}"
            );
        }
    }
}

#[test]
fn the_refusal_leaves_the_source_buffer_not_updating() {
    // The spec's append error algorithm clears `updating`; the DOM then fires the error events.
    let stream = load("h264-low").unwrap();
    let protected = with_cenc_protection(&stream.init, [9; 16], 8).unwrap();
    let log = Arc::new(Log::default());
    let backend = MseBackend::new(log.sink(), Arc::new(AllDecoders), Limits::default());
    backend.open();
    let id = backend.add_source_buffer(&stream.mime).unwrap();
    let op = backend.append(id, protected.bytes).unwrap();
    let events = log.completion_of(op);
    assert_eq!(error_of(&events), Some(AppendErrorKind::Unsupported));
    assert!(!backend.updating(id), "updating is false after the error");
}

#[test]
fn every_d7_fixture_buffers_its_whole_duration_through_the_backend() {
    // Not only the parser: the processor, the track buffers and the quota take the new codecs too, one
    // append per segment as a page does it (the append is complete when its event arrives).
    for name in [
        "hevc-low",
        "ac3",
        "eac3",
        "flac",
        "mp3",
        "vorbis",
        "flac-webm",
    ] {
        let stream = load(name).unwrap();
        let log = Arc::new(Log::default());
        let backend = MseBackend::new(log.sink(), Arc::new(AllDecoders), Limits::default());
        backend.open();
        let id = backend
            .add_source_buffer(&stream.mime)
            .expect("a source buffer");
        let mut segments = vec![stream.init.bytes.clone()];
        segments.extend(stream.media.iter().map(|m| m.bytes.clone()));
        for (i, bytes) in segments.into_iter().enumerate() {
            let op = backend.append(id, bytes).expect("an accepted append");
            let events = log.completion_of(op);
            assert_eq!(error_of(&events), None, "{name}: segment {i}: {events:?}");
        }
        let ranges = backend.buffered(id);
        assert_eq!(ranges.len(), 1, "{name}: {ranges:?}");
        let (start, end) = (ranges[0].start_secs(), ranges[0].end_secs());
        let want = stream.duration_ms().unwrap() as f64 / 1000.0;
        assert!(start.abs() < 0.1, "{name}: starts at {start}");
        assert!(
            (end - want).abs() < 0.1,
            "{name}: buffered to {end}, the file is {want} s"
        );
    }
}
