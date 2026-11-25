// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! The processor's part of "initialization segment received", `changeType` bookkeeping, and "reset parser
//! state".

mod frames_support;

mod frames_init {
    use super::frames_support::*;
    use mercury_mse::{
        AppendMode, CodecConfig, Container, InitSegment, ProcessError, SourceBufferProcessor,
        TrackKind,
    };

    const V: TrackKind = TrackKind::Video;
    const A: TrackKind = TrackKind::Audio;

    #[test]
    fn the_first_init_segment_creates_the_track_buffers() {
        let mut p = SourceBufferProcessor::new(Default::default());
        assert!(!p.first_init_received());
        let out = p.init_segment(&init_of(&[V, A])).unwrap();
        assert!(out.first && !out.config_changed);
        assert!(p.first_init_received());
        assert_eq!(p.track_kinds(), [V, A]);
        assert_eq!(p.tracks().len(), 2);
        assert_eq!(p.tracks()[0].kind, V);
        assert_eq!(p.tracks()[1].kind, A);
        assert!(p.has_track(V) && p.has_track(A) && !p.has_track(TrackKind::Text));
    }

    #[test]
    fn a_segment_without_tracks_is_a_decode_error() {
        let mut p = SourceBufferProcessor::new(Default::default());
        let err = p.init_segment(&init(vec![])).unwrap_err();
        assert!(matches!(err, ProcessError::Decode(_)), "{err:?}");
        assert!(!p.first_init_received());
    }

    #[test]
    fn two_tracks_of_one_kind_are_unsupported() {
        let mut p = SourceBufferProcessor::new(Default::default());
        let err = p
            .init_segment(&init(vec![config(V, 0), config(V, 1)]))
            .unwrap_err();
        assert!(matches!(err, ProcessError::Unsupported(_)), "{err:?}");
    }

    #[test]
    fn a_later_init_segment_must_have_the_same_kinds() {
        let mut p = SourceBufferProcessor::new(Default::default());
        p.init_segment(&init_of(&[V, A])).unwrap();
        for kinds in [
            &[V][..],
            &[A][..],
            &[V, TrackKind::Text][..],
            &[V, A, TrackKind::Text][..],
        ] {
            let err = p.init_segment(&init_of(kinds)).unwrap_err();
            assert!(matches!(err, ProcessError::Decode(_)), "{kinds:?}: {err:?}");
        }
        // the same kinds again are fine, in any order
        let out = p
            .init_segment(&init(vec![config(A, 0), config(V, 0)]))
            .unwrap();
        assert!(!out.first && !out.config_changed);
    }

    #[test]
    fn a_changed_configuration_is_recorded_and_waits_for_a_keyframe() {
        let mut p = processor(&[V]);
        p.process_frames(video_25fps(0, 2)).unwrap();
        let before = p.config_id(V).unwrap();
        let out = p.init_segment(&init(vec![config(V, 1)])).unwrap();
        assert!(!out.first && out.config_changed);
        assert_ne!(p.config_id(V).unwrap(), before);
        assert_eq!(p.tracks()[0].width, 640);
        // need random access point is set again: a delta frame is dropped, the keyframe taken
        let mut delta = video(2 * SEC, 2, 40 * MS, 50, 8000, 1000);
        delta[0].keyframe = false;
        let r = p.process_frames(delta).unwrap();
        assert_eq!(r.accepted, 0);
        let r = p.process_frames(video_25fps(2, 2)).unwrap();
        assert_eq!(r.accepted, 50);
        // the old configuration again reuses its id
        p.init_segment(&init(vec![config(V, 0)])).unwrap();
        assert_eq!(p.config_id(V).unwrap(), before);
    }

    #[test]
    fn a_different_codec_with_the_same_size_is_a_change() {
        let mut p = processor(&[V]);
        let mut vp9 = config(V, 0);
        vp9.codec = CodecConfig::Vp9 { vpcc: None };
        vp9.codec_string = "vp09.00.10.08".into();
        let out = p.init_segment(&init(vec![vp9])).unwrap();
        assert!(out.config_changed);
    }

    #[test]
    fn vp9_renditions_that_differ_only_in_size_are_a_change() {
        // WebM VP9 has no configuration record (`vpcc: None` at every resolution): the size decides
        let mut low = config(V, 0);
        low.codec = CodecConfig::Vp9 { vpcc: None };
        low.codec_string = "vp09.00.10.08".into();
        let mut high = low.clone();
        high.width = 1280;
        high.height = 720;
        let mut p = SourceBufferProcessor::new(Default::default());
        p.init_segment(&init(vec![low.clone()])).unwrap();
        p.process_frames(video_25fps(0, 2)).unwrap();
        let out = p.init_segment(&init(vec![high])).unwrap();
        assert!(
            out.config_changed,
            "a size change alone is a configuration change"
        );
        p.process_frames(video_25fps(2, 2)).unwrap();
        assert_eq!(config_count(&read_all(&p, V, 0)), 2);
        // and the codec string alone is one too (a profile or level change with the same record)
        let mut other = low.clone();
        other.codec_string = "vp09.02.10.10".into();
        assert!(p.init_segment(&init(vec![other])).unwrap().config_changed);
    }

    #[test]
    fn only_descriptive_fields_changing_is_not_a_change() {
        let mut p = processor(&[V]);
        let mut again = config(V, 0);
        again.language = "fr".into();
        again.default = false;
        again.timescale = 1000;
        let out = p.init_segment(&init(vec![again])).unwrap();
        assert!(!out.config_changed);
        assert_eq!(
            p.tracks()[0].language,
            "fr",
            "the descriptive fields are refreshed"
        );
    }

    #[test]
    fn change_type_makes_the_next_init_segment_a_first_one() {
        let mut p = processor(&[V]);
        p.process_frames(video_25fps(0, 2)).unwrap();
        assert!(!p.pending_init_segment());
        p.change_type();
        assert!(p.pending_init_segment());
        // media before the init segment is refused until it arrives
        let err = p.process_frames(video_25fps(2, 2)).unwrap_err();
        assert!(matches!(err, ProcessError::Decode(_)));
        // the data already buffered stays
        assert_eq!(ms_ranges(&p.buffered()), [(0, 2000)]);
        let out = p.init_segment(&init(vec![config(V, 1)])).unwrap();
        assert!(out.first && out.config_changed);
        assert!(!p.pending_init_segment());
        let r = p.process_frames(video_25fps(2, 2)).unwrap();
        assert_eq!(r.accepted, 50);
        assert_eq!(ms_ranges(&p.buffered()), [(0, 4000)]);
        // a changeType to the same configuration: first, no change
        p.change_type();
        let out = p.init_segment(&init(vec![config(V, 1)])).unwrap();
        assert!(out.first && !out.config_changed);
    }

    #[test]
    fn change_type_to_another_container_keeps_the_kinds() {
        let mut p = processor(&[V, A]);
        p.change_type();
        let webm = InitSegment {
            container: Container::WebM,
            tracks: vec![config(V, 0), config(A, 0)],
            duration: None,
            pssh: vec![],
        };
        let out = p.init_segment(&webm).unwrap();
        assert!(out.first);
        // a different set of kinds after changeType is refused
        p.change_type();
        let err = p.init_segment(&init_of(&[V])).unwrap_err();
        assert!(matches!(err, ProcessError::Decode(_)));
    }

    #[test]
    fn change_type_resets_the_parser_state() {
        let mut p = processor(&[V]);
        p.process_frames(video_25fps(0, 2)).unwrap();
        p.change_type();
        p.init_segment(&init_of(&[V])).unwrap();
        // the group is over: a stray delta frame is dropped
        let mut delta = video(2 * SEC, 2, 40 * MS, 50, 8000, 1000);
        delta[0].keyframe = false;
        assert_eq!(p.process_frames(delta).unwrap().accepted, 0);
    }

    #[test]
    fn reset_parser_state_waits_for_a_keyframe_on_every_track() {
        let mut p = processor(&[V, A]);
        p.process_frames(video_25fps(0, 2)).unwrap();
        p.process_frames(audio(0, 100, 20 * MS, 10)).unwrap();
        p.reset_parser_state();
        let mut delta = video(2 * SEC, 2, 40 * MS, 50, 8000, 1000);
        delta[0].keyframe = false;
        assert_eq!(p.process_frames(delta).unwrap().accepted, 0);
        // audio frames are all random access points, so they continue
        assert_eq!(
            p.process_frames(audio(2 * SEC, 10, 20 * MS, 10))
                .unwrap()
                .accepted,
            10
        );
        // and the buffered data is untouched
        assert_eq!(ms_ranges(&p.track_buffered(V)), [(0, 2000)]);
    }

    #[test]
    fn reset_parser_state_in_sequence_mode_moves_the_group_start_to_the_group_end() {
        let mut p = processor(&[V]);
        p.set_mode(AppendMode::Sequence);
        p.process_frames(video_25fps(0, 2)).unwrap();
        assert_eq!(p.group_start_timestamp(), None, "unset once used");
        p.reset_parser_state();
        assert_eq!(p.group_start_timestamp(), Some(2 * SEC));
        // in segments mode it does not
        let mut q = processor(&[V]);
        q.process_frames(video_25fps(0, 2)).unwrap();
        q.reset_parser_state();
        assert_eq!(q.group_start_timestamp(), None);
    }

    #[test]
    fn a_dropped_buffer_state_is_independent_per_processor() {
        // two processors do not share anything
        let mut a = processor(&[V]);
        let mut b = processor(&[V]);
        a.process_frames(video_25fps(0, 2)).unwrap();
        b.process_frames(video_25fps(4, 2)).unwrap();
        assert_eq!(ms_ranges(&a.buffered()), [(0, 2000)]);
        assert_eq!(ms_ranges(&b.buffered()), [(4000, 6000)]);
    }

    #[test]
    fn the_processor_can_move_between_threads() {
        fn assert_send<T: Send>() {}
        assert_send::<SourceBufferProcessor>();
        let mut p = processor(&[V]);
        p.process_frames(video_25fps(0, 2)).unwrap();
        let handle = std::thread::spawn(move || {
            p.process_frames(video_25fps(2, 2)).unwrap();
            p.buffered()
        });
        assert_eq!(ms_ranges(&handle.join().unwrap()), [(0, 4000)]);
    }
}
