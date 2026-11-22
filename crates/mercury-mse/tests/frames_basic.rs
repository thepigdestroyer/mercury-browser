// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! M1 of item 2 (`frames`): the processor accepts synthetic frames and answers `buffered` and `remove`.
//! The tests sit in a module named `frames_basic` so that `cargo test frames_` finds them.

mod frames_support;

mod frames_basic {
    use super::frames_support::*;
    use mercury_mse::{AppendMode, ProcessError, SourceBufferProcessor, TrackKind};

    #[test]
    fn m1_accepts_synthetic_frames_and_answers_buffered_and_remove() {
        let mut p = processor(&[TrackKind::Video]);
        let r = p.process_frames(video_25fps(0, 8)).unwrap();
        assert_eq!((r.accepted, r.dropped), (200, 0));
        assert_eq!(p.frame_count(TrackKind::Video), 200);
        assert_eq!(ms_ranges(&p.buffered()), [(0, 8000)]);
        assert_eq!(p.highest_end(), Some(8 * SEC));
        // 4 keyframes of 8000 bytes and 196 deltas of 1000, and the overhead of 200 frames
        assert_eq!(p.bytes(), 4 * 8000 + 196 * 1000 + 200 * OV);

        // remove(0, 4 s) keeps [4, 8]; remove(6 s, +Infinity) leaves [4, 6]
        let removed = p.remove(0, Some(4 * SEC));
        assert_eq!(removed.frames, 100);
        assert_eq!(ms_ranges(&p.buffered()), [(4000, 8000)]);
        let removed = p.remove(6 * SEC, None);
        assert_eq!(removed.frames, 50);
        assert_eq!(ms_ranges(&p.buffered()), [(4000, 6000)]);
        // [4, 6) is one group of 50 frames: a keyframe and 49 deltas
        assert_eq!(p.bytes(), 8000 + 49 * 1000 + 50 * OV);
        p.check_invariants().unwrap();
    }

    #[test]
    fn m1_frames_before_the_init_segment_are_refused() {
        let mut p = SourceBufferProcessor::new(Default::default());
        let err = p.process_frames(video_25fps(0, 1)).unwrap_err();
        assert!(matches!(err, ProcessError::Decode(_)), "{err:?}");
    }

    #[test]
    fn m1_defaults() {
        let p = processor(&[TrackKind::Video, TrackKind::Audio]);
        assert_eq!(p.mode(), AppendMode::Segments);
        assert_eq!(p.timestamp_offset(), 0);
        assert_eq!(p.append_window(), (0, None));
        assert!(p.buffered().is_empty());
        assert_eq!(p.highest_end(), None);
        assert_eq!(p.bytes(), 0);
        assert_eq!(p.tracks().len(), 2);
    }
}
