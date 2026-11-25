// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! The fragmented MP4 parser on hand-built boxes: the `trun`/`tfhd`/`tfdt` variants the committed
//! fixtures do not use (per-sample and first-sample flags, version 0 and 1 composition offsets, no `tfdt`,
//! several `traf`, every base data offset rule, several runs, boxes to skip, 64-bit and open-ended sizes),
//! the sample entries of the other codecs, and what the parser refuses. Every stream is run under all five
//! chunk plans and must give the same events or the same error.
//! Run with `cargo test -p mercury-mse --test 'parse_*'` (or the filter `parse_`).

mod parse_support;

mod parse_boxes {
    use super::parse_support::*;
    use mercury_mse::api::{ByteStreamParser, CodecConfig, Container, ParseError, TrackKind};
    use mercury_mse::parse::mp4::Mp4Parser;

    const M: Container = Container::Mp4;

    /// Container ticks as nanoseconds, rounded to the nearest, computed here independently.
    fn ns(ticks: i64, timescale: i64) -> i64 {
        let n = i128::from(ticks) * 1_000_000_000;
        let t = i128::from(timescale);
        (if n >= 0 {
            (n + t / 2) / t
        } else {
            -((-n + t / 2) / t)
        }) as i64
    }

    fn video_init() -> Vec<u8> {
        init_of_tracks(&[TrackSpec::video(1, 90_000)])
    }

    fn av_init() -> Vec<u8> {
        init_of_tracks(&[TrackSpec::video(1, 90_000), TrackSpec::audio(2, 48_000)])
    }

    fn base_moof(track_id: u32) -> Tfhd {
        Tfhd {
            track_id,
            default_base_is_moof: true,
            ..Tfhd::default()
        }
    }

    fn sample(duration: u32, size: u32, flags: u32, cto: i32) -> RunSample {
        RunSample {
            duration,
            size,
            flags,
            cto,
        }
    }

    fn full_run(offset: u32, samples: Vec<RunSample>) -> Run {
        Run {
            data_offset: Some(offset as i32),
            has_duration: true,
            has_size: true,
            has_flags: true,
            has_cto: true,
            samples,
            ..Run::default()
        }
    }

    fn frames_for(parts: &[&[u8]]) -> Vec<Fr> {
        frames_of(&same_under_all_plans(M, parts).unwrap())
    }

    const KEY: u32 = 0x0200_0000;
    const NON_KEY: u32 = 0x0101_0000;

    #[test]
    fn per_sample_flags_and_first_sample_flags_decide_the_keyframes() {
        let data = cat(&[
            &payload(1, 10),
            &payload(2, 20),
            &payload(3, 30),
            &payload(4, 5),
        ]);
        // Run 1: per-sample flags; run 2: first_sample_flags plus the tfhd default (non-sync).
        let seg = fragment(
            |off| {
                let r1 = Run {
                    first_sample_flags: None,
                    ..full_run(
                        off,
                        vec![sample(3000, 10, KEY, 0), sample(3000, 20, NON_KEY, 0)],
                    )
                };
                let r2 = Run {
                    data_offset: Some(off as i32 + 30),
                    first_sample_flags: Some(KEY),
                    has_duration: true,
                    has_size: true,
                    samples: vec![sample(3000, 30, 0, 0), sample(3000, 5, 0, 0)],
                    ..Run::default()
                };
                vec![traf(&[
                    tfhd(&Tfhd {
                        default_flags: Some(NON_KEY),
                        ..base_moof(1)
                    }),
                    tfdt(0, 0),
                    trun(&r1),
                    trun(&r2),
                ])]
            },
            &data,
        );
        let frames = frames_for(&[&video_init(), &seg]);
        let keys: Vec<bool> = frames.iter().map(|f| f.keyframe).collect();
        assert_eq!(keys, vec![true, false, true, false]);
        let sizes: Vec<usize> = frames.iter().map(|f| f.data.len()).collect();
        assert_eq!(sizes, vec![10, 20, 30, 5]);
        assert_eq!(frames[2].data, payload(3, 30));
        // Times: four samples of 3000 ticks at 90 kHz, exact.
        for (i, f) in frames.iter().enumerate() {
            let i = i as i64;
            assert_eq!(f.dts, ns(i * 3000, 90_000));
            assert_eq!(
                f.duration,
                ns((i + 1) * 3000, 90_000) - ns(i * 3000, 90_000)
            );
            assert_eq!(f.pts, f.dts);
        }
    }

    #[test]
    fn first_sample_flags_beat_a_per_sample_flag_for_the_first_sample_only() {
        let data = payload(1, 12);
        let seg = fragment(
            |off| {
                let r = Run {
                    first_sample_flags: Some(NON_KEY),
                    ..full_run(
                        off,
                        vec![
                            sample(3000, 4, KEY, 0),
                            sample(3000, 4, KEY, 0),
                            sample(3000, 4, NON_KEY, 0),
                        ],
                    )
                };
                vec![traf(&[tfhd(&base_moof(1)), trun(&r)])]
            },
            &data,
        );
        let keys: Vec<bool> = frames_for(&[&video_init(), &seg])
            .iter()
            .map(|f| f.keyframe)
            .collect();
        assert_eq!(keys, vec![false, true, false]);
    }

    #[test]
    fn composition_offsets_version_0_unsigned_and_version_1_signed() {
        let data = payload(1, 9);
        for (version, cto, expect_pts_ticks) in [
            (0u8, 6000i32, 6000i64),
            (0, i32::MAX, i64::from(i32::MAX)),
            (1, -3000, -3000),
            (1, 6000, 6000),
        ] {
            let seg = fragment(
                |off| {
                    let r = Run {
                        version,
                        ..full_run(off, vec![sample(3000, 9, KEY, cto)])
                    };
                    vec![traf(&[tfhd(&base_moof(1)), tfdt(0, 90_000), trun(&r)])]
                },
                &data,
            );
            let f = &frames_for(&[&video_init(), &seg])[0];
            assert_eq!(f.dts, 1_000_000_000, "v{version} {cto}");
            assert_eq!(
                f.pts,
                ns(90_000 + expect_pts_ticks, 90_000),
                "v{version} cto {cto}"
            );
        }
        // A version 0 offset with the high bit set is a large positive number, not a negative one.
        let seg = fragment(
            |off| {
                let r = Run {
                    version: 0,
                    ..full_run(off, vec![sample(3000, 9, KEY, -2)])
                };
                vec![traf(&[tfhd(&base_moof(1)), trun(&r)])]
            },
            &data,
        );
        let f = &frames_for(&[&video_init(), &seg])[0];
        assert_eq!(f.pts, ns(0xFFFF_FFFE, 90_000));
    }

    #[test]
    fn without_a_tfdt_a_fragment_continues_from_the_end_of_the_previous_one() {
        let data = payload(1, 9);
        let with_time = |t: Option<u64>| {
            fragment(
                |off| {
                    let mut parts = vec![tfhd(&base_moof(1))];
                    if let Some(t) = t {
                        parts.push(tfdt(0, t));
                    }
                    parts.push(trun(&full_run(
                        off,
                        vec![
                            sample(3000, 3, KEY, 0),
                            sample(3001, 3, NON_KEY, 0),
                            sample(2999, 3, NON_KEY, 0),
                        ],
                    )));
                    vec![traf(&parts)]
                },
                &data,
            )
        };
        let init = video_init();
        // The very first fragment without a tfdt starts at 0; the next two follow; a tfdt jumps (a gap).
        let (a, b, c, d) = (
            with_time(None),
            with_time(None),
            with_time(None),
            with_time(Some(900_000)),
        );
        let frames = frames_for(&[&init, &a, &b, &c, &d]);
        assert_eq!(frames.len(), 12);
        let ticks = [
            0i64, 3000, 6001, 9000, 12000, 15001, 18000, 21000, 24001, 900_000, 903_000, 906_001,
        ];
        for (f, t) in frames.iter().zip(ticks) {
            assert_eq!(f.dts, ns(t, 90_000), "tick {t}");
        }
        // Unequal durations are carried exactly: 3001 ticks.
        assert_eq!(frames[1].duration, ns(6001, 90_000) - ns(3000, 90_000));
        // Each track continues on its own: audio first at 48 kHz, video later without a tfdt.
        let audio = fragment(
            |off| {
                vec![traf(&[
                    tfhd(&base_moof(2)),
                    tfdt(0, 48_000),
                    trun(&full_run(off, vec![sample(1024, 3, KEY, 0)])),
                ])]
            },
            &data[..3],
        );
        let video_after = with_time(None);
        let frames = frames_for(&[&av_init(), &a, &audio, &video_after]);
        let v: Vec<i64> = frames
            .iter()
            .filter(|f| f.kind == TrackKind::Video)
            .map(|f| f.dts)
            .collect();
        assert_eq!(
            v[3],
            ns(9000, 90_000),
            "video continues at 9000 whatever audio did"
        );
        let au: Vec<&Fr> = frames
            .iter()
            .filter(|f| f.kind == TrackKind::Audio)
            .collect();
        assert_eq!(au[0].dts, 1_000_000_000);
    }

    #[test]
    fn tfdt_version_1_carries_64_bit_times() {
        let data = payload(1, 3);
        let big: u64 = 1 << 40;
        let seg = fragment(
            |off| {
                vec![traf(&[
                    tfhd(&base_moof(1)),
                    tfdt(1, big),
                    trun(&full_run(off, vec![sample(3000, 3, KEY, 0)])),
                ])]
            },
            &data,
        );
        let f = &frames_for(&[&video_init(), &seg])[0];
        assert_eq!(f.dts, ns(big as i64, 90_000));
        assert_eq!(
            f.duration,
            ns(big as i64 + 3000, 90_000) - ns(big as i64, 90_000)
        );
    }

    #[test]
    fn defaults_come_from_the_trun_then_the_tfhd_then_the_trex() {
        // trex: duration 3600, size 7, flags non-sync. tfhd: size 5, flags key. trun: nothing but offsets.
        let moov = moov_of(
            &[TrackSpec::video(1, 90_000)],
            &[(1, 3600, 7, NON_KEY)],
            &[],
        );
        let init = cat(&[&ftyp(), &moov]);
        let data = payload(1, 14);
        // 1. Only the trex: two samples of 7 bytes, 3600 ticks, non-sync.
        let seg = fragment(
            |off| {
                let r = Run {
                    data_offset: Some(off as i32),
                    samples: vec![RunSample::default(); 2],
                    ..Run::default()
                };
                vec![traf(&[tfhd(&base_moof(1)), trun(&r)])]
            },
            &data,
        );
        let f = frames_for(&[&init, &seg]);
        assert_eq!(
            f.iter().map(|f| f.data.len()).collect::<Vec<_>>(),
            vec![7, 7]
        );
        assert_eq!(f[1].dts, ns(3600, 90_000));
        assert!(f.iter().all(|f| !f.keyframe));
        // 2. The tfhd overrides the trex: size 5 and key, duration 1000.
        let seg = fragment(
            |off| {
                let r = Run {
                    data_offset: Some(off as i32),
                    samples: vec![RunSample::default(); 2],
                    ..Run::default()
                };
                let h = Tfhd {
                    default_duration: Some(1000),
                    default_size: Some(5),
                    default_flags: Some(KEY),
                    ..base_moof(1)
                };
                vec![traf(&[tfhd(&h), trun(&r)])]
            },
            &data[..10],
        );
        let f = frames_for(&[&init, &seg]);
        assert_eq!(
            f.iter().map(|f| f.data.len()).collect::<Vec<_>>(),
            vec![5, 5]
        );
        assert_eq!(f[1].dts, ns(1000, 90_000));
        assert!(f.iter().all(|f| f.keyframe));
        // 3. The trun overrides both: one sample with its own duration and size and flags.
        let seg = fragment(
            |off| {
                let r = Run {
                    data_offset: Some(off as i32),
                    has_size: true,
                    has_duration: true,
                    has_flags: true,
                    samples: vec![sample(1800, 14, NON_KEY, 0)],
                    ..Run::default()
                };
                let h = Tfhd {
                    default_duration: Some(1000),
                    default_size: Some(5),
                    default_flags: Some(KEY),
                    ..base_moof(1)
                };
                vec![traf(&[tfhd(&h), trun(&r)])]
            },
            &data,
        );
        let f = frames_for(&[&init, &seg]);
        assert_eq!(
            (f[0].data.len(), f[0].duration, f[0].keyframe),
            (14, ns(1800, 90_000), false)
        );
    }

    /// One video traf (two samples) and one audio traf (three), data in that order. `audio_offset(off)` is
    /// the audio run's data offset (`off` is where the video data starts).
    fn two_tracks(
        video_tfhd: Tfhd,
        audio_tfhd: Tfhd,
        audio_offset: impl Fn(u32) -> Option<i32>,
    ) -> Vec<u8> {
        let v = cat(&[&payload(10, 8), &payload(11, 8)]);
        let a = cat(&[&payload(20, 5), &payload(21, 5), &payload(22, 5)]);
        fragment(
            |off| {
                let vr = full_run(
                    off,
                    vec![sample(3000, 8, KEY, 0), sample(3000, 8, NON_KEY, 0)],
                );
                let ar = Run {
                    data_offset: audio_offset(off),
                    ..full_run(0, vec![sample(1024, 5, KEY, 0); 3])
                };
                vec![
                    traf(&[tfhd(&video_tfhd), tfdt(0, 90_000), trun(&vr)]),
                    traf(&[tfhd(&audio_tfhd), tfdt(0, 96_000), trun(&ar)]),
                ]
            },
            &cat(&[&v, &a]),
        )
    }

    fn check_two_tracks(frames: &[Fr]) {
        assert_eq!(frames.len(), 5);
        assert_eq!(
            frames.iter().map(|f| f.kind).collect::<Vec<_>>(),
            vec![
                TrackKind::Video,
                TrackKind::Video,
                TrackKind::Audio,
                TrackKind::Audio,
                TrackKind::Audio
            ]
        );
        assert_eq!(frames[0].data, payload(10, 8));
        assert_eq!(frames[1].data, payload(11, 8));
        assert_eq!(frames[2].data, payload(20, 5));
        assert_eq!(frames[4].data, payload(22, 5));
        assert_eq!(frames[0].dts, 1_000_000_000);
        assert_eq!(frames[2].dts, 2_000_000_000);
        assert_eq!(frames[3].dts, ns(96_000 + 1024, 48_000));
        assert_eq!((frames[0].track_id, frames[2].track_id), (1, 2));
    }

    #[test]
    fn several_trafs_with_default_base_is_moof() {
        let seg = two_tracks(base_moof(1), base_moof(2), |off| Some(off as i32 + 16));
        check_two_tracks(&frames_for(&[&av_init(), &seg]));
    }

    #[test]
    fn the_legacy_base_is_the_moof_for_the_first_traf_and_the_end_of_the_previous_data_after_it() {
        let plain = |id| Tfhd {
            track_id: id,
            ..Tfhd::default()
        };
        // The audio run has no data offset: it starts where the video data ended.
        let seg = two_tracks(plain(1), plain(2), |_| None);
        check_two_tracks(&frames_for(&[&av_init(), &seg]));
    }

    #[test]
    fn an_explicit_base_data_offset_stands_for_the_moof_and_later_ones_are_relative_to_it() {
        // Absolute offsets of a file that is not here: the first one is the moof (ffmpeg without
        // default_base_moof writes the moof's own file position), the second points 16 bytes past the
        // start of the sample data.
        let v = cat(&[&payload(10, 8), &payload(11, 8)]);
        let a = cat(&[&payload(20, 5), &payload(21, 5), &payload(22, 5)]);
        let seg = fragment(
            |off| {
                let h1 = Tfhd {
                    track_id: 1,
                    base_data_offset: Some(123_456),
                    ..Tfhd::default()
                };
                let h2 = Tfhd {
                    track_id: 2,
                    base_data_offset: Some(123_456 + u64::from(off) + 16),
                    ..Tfhd::default()
                };
                let vr = full_run(
                    off,
                    vec![sample(3000, 8, KEY, 0), sample(3000, 8, NON_KEY, 0)],
                );
                let ar = full_run(0, vec![sample(1024, 5, KEY, 0); 3]);
                vec![
                    traf(&[tfhd(&h1), tfdt(0, 90_000), trun(&vr)]),
                    traf(&[tfhd(&h2), tfdt(0, 96_000), trun(&ar)]),
                ]
            },
            &cat(&[&v, &a]),
        );
        check_two_tracks(&frames_for(&[&av_init(), &seg]));
    }

    #[test]
    fn several_runs_continue_and_may_point_backwards() {
        let data = cat(&[&payload(1, 10), &payload(2, 10), &payload(3, 10)]);
        // Run A has no data offset: it continues at the base (the moof for default-base-is-moof it must
        // say where; here B comes first in the list but holds the later bytes).
        let seg = fragment(
            |off| {
                let r_b = full_run(off + 10, vec![sample(3000, 10, KEY, 0)]); // bytes 10..20, listed first
                let r_a = full_run(off, vec![sample(3000, 10, NON_KEY, 0)]); // bytes 0..10, listed second
                let r_c = Run {
                    data_offset: None,
                    ..full_run(0, vec![sample(3000, 10, NON_KEY, 0)])
                }; // continues after A: 10..20
                vec![traf(&[
                    tfhd(&base_moof(1)),
                    tfdt(0, 0),
                    trun(&r_b),
                    trun(&r_a),
                    trun(&r_c),
                ])]
            },
            &data,
        );
        let f = frames_for(&[&video_init(), &seg]);
        assert_eq!(f.len(), 3);
        assert_eq!(f[0].data, payload(2, 10), "listed first, stored second");
        assert_eq!(f[1].data, payload(1, 10));
        assert_eq!(
            f[2].data,
            payload(2, 10),
            "a run without an offset continues after the previous run"
        );
        // The timeline follows the list, not the byte order.
        assert_eq!(
            (f[0].dts, f[1].dts, f[2].dts),
            (0, ns(3000, 90_000), ns(6000, 90_000))
        );
    }

    #[test]
    fn every_audio_frame_is_a_keyframe_whatever_its_flags_say() {
        let data = cat(&[&payload(1, 4), &payload(2, 4)]);
        let seg = fragment(
            |off| {
                let v = full_run(off, vec![sample(3000, 4, NON_KEY, 0)]);
                let a = full_run(off + 4, vec![sample(1024, 4, NON_KEY, 0)]);
                vec![
                    traf(&[tfhd(&base_moof(1)), tfdt(0, 0), trun(&v)]),
                    traf(&[tfhd(&base_moof(2)), tfdt(0, 0), trun(&a)]),
                ]
            },
            &data,
        );
        let f = frames_for(&[&av_init(), &seg]);
        assert_eq!(f.len(), 2);
        assert_eq!((f[0].kind, f[0].keyframe), (TrackKind::Video, false));
        assert_eq!((f[1].kind, f[1].keyframe), (TrackKind::Audio, true));
    }

    #[test]
    fn boxes_to_skip_are_skipped_without_being_read() {
        let data = payload(1, 9);
        let frag = fragment(
            |off| {
                vec![traf(&[
                    tfhd(&base_moof(1)),
                    tfdt(0, 0),
                    trun(&full_run(off, vec![sample(3000, 9, KEY, 0)])),
                ])]
            },
            &data,
        );
        let sidx = full(
            "sidx",
            0,
            0,
            &cat(&[
                &be32(1),
                &be32(90_000),
                &be32(0),
                &be32(0),
                &[0, 0],
                &[0, 1],
                &be32(9),
                &be32(3000),
                &be32(0x9000_0000),
            ]),
        );
        let junk = |n| payload(0xEE, n);
        let uuid = bx("uuid", &cat(&[&[7; 16], &junk(30)]));
        let skipped = cat(&[
            &bx("styp", b"msdh\0\0\0\0msdhmsix"),
            &sidx,
            &full(
                "emsg",
                0,
                0,
                &cat(&[
                    b"urn:x\0",
                    b"v\0",
                    &be32(1),
                    &be32(0),
                    &be32(0),
                    &be32(9),
                    b"data",
                ]),
            ),
            &bx("free", &junk(11)),
            &bx64("skip", &junk(5)),
            &full("prft", 0, 0, &junk(20)),
            &uuid,
            &bx("zzzz", &[]),
        ]);
        let stream = cat(&[
            &video_init(),
            &skipped,
            &frag,
            &bx("free", &junk(40)),
            &bx("mfra", &junk(12)),
        ]);
        let events = same_under_all_plans(M, &[&stream]).unwrap();
        let shape: String = events
            .iter()
            .map(|e| match e {
                Ev::Init(_) => 'I',
                Ev::Frame(_) => 'F',
                Ev::End => 'E',
            })
            .collect();
        assert_eq!(shape, "IFE");
        assert_eq!(frames_of(&events)[0].data, data);
        // The same boxes before the init segment's moov (a free box and a styp) are fine too.
        let stream = cat(&[&bx("free", &junk(3)), &video_init(), &frag]);
        assert_eq!(
            frames_of(&same_under_all_plans(M, &[&stream]).unwrap()).len(),
            1
        );
    }

    #[test]
    fn mdat_with_a_64_bit_size_and_with_size_zero() {
        let data = payload(1, 9);
        let make = |mdat: fn(&str, &[u8]) -> Vec<u8>| {
            fragment_with(
                |off| {
                    vec![traf(&[
                        tfhd(&base_moof(1)),
                        tfdt(0, 0),
                        trun(&full_run(
                            off,
                            vec![sample(3000, 4, KEY, 0), sample(3000, 5, NON_KEY, 0)],
                        )),
                    ])]
                },
                mdat,
                &data,
            )
        };
        for (what, mdat) in [
            ("plain", bx as fn(&str, &[u8]) -> Vec<u8>),
            ("64-bit", bx64),
            ("to the end", bx_to_end),
        ] {
            let events = same_under_all_plans(M, &[&video_init(), &make(mdat)]).unwrap();
            let frames = frames_of(&events);
            assert_eq!(
                frames.iter().map(|f| f.data.len()).collect::<Vec<_>>(),
                vec![4, 5],
                "{what}"
            );
            // The segment ends with the mdat, or with the last sample when the mdat has no end.
            assert_eq!(ends_of(&events), 1, "{what}");
        }
        // A 64-bit `free` and a box of size 0 inside the skipped part are fine; size 0 is only for the last box.
        let stream = cat(&[&video_init(), &bx64("free", &[1, 2, 3]), &make(bx)]);
        assert_eq!(
            frames_of(&same_under_all_plans(M, &[&stream]).unwrap()).len(),
            2
        );
    }

    #[test]
    fn data_may_follow_the_moof_after_other_boxes() {
        let data = payload(1, 9);
        let free = bx("free", &payload(0, 100));
        let probe = moof(&[traf(&[
            tfhd(&base_moof(1)),
            trun(&full_run(0, vec![sample(3000, 9, KEY, 0)])),
        ])]);
        let off = probe.len() as u32 + free.len() as u32 + 8;
        let moof_bytes = moof(&[traf(&[
            tfhd(&base_moof(1)),
            trun(&full_run(off, vec![sample(3000, 9, KEY, 0)])),
        ])]);
        let stream = cat(&[&video_init(), &moof_bytes, &free, &bx("mdat", &data)]);
        let events = same_under_all_plans(M, &[&stream]).unwrap();
        assert_eq!(frames_of(&events)[0].data, data);
        assert_eq!(ends_of(&events), 1);
    }

    #[test]
    fn unknown_tracks_empty_fragments_and_gaps_are_handled() {
        // Track 9 is not in the init segment: its samples are skipped and do not disturb the next traf.
        let v = payload(10, 8);
        let x = payload(99, 6);
        let a = payload(20, 5);
        let seg = fragment(
            |off| {
                vec![
                    traf(&[
                        tfhd(&base_moof(1)),
                        tfdt(0, 0),
                        trun(&full_run(off, vec![sample(3000, 8, KEY, 0)])),
                    ]),
                    traf(&[
                        tfhd(&base_moof(9)),
                        tfdt(0, 5),
                        trun(&full_run(
                            off + 8,
                            vec![sample(1, 3, KEY, 0), sample(1, 3, KEY, 0)],
                        )),
                    ]),
                    traf(&[
                        tfhd(&base_moof(2)),
                        tfdt(0, 0),
                        trun(&full_run(off + 14, vec![sample(1024, 5, KEY, 0)])),
                    ]),
                ]
            },
            &cat(&[&v, &x, &a]),
        );
        let f = frames_for(&[&av_init(), &seg]);
        assert_eq!(f.len(), 2);
        assert_eq!((f[0].data.clone(), f[1].data.clone()), (v, a));
        // duration-is-empty: no samples, but its tfdt moves the track's timeline.
        let empty = fragment(
            |_| {
                vec![traf(&[
                    tfhd(&Tfhd {
                        duration_is_empty: true,
                        default_duration: Some(90_000),
                        ..base_moof(1)
                    }),
                    tfdt(0, 900_000),
                ])]
            },
            &[],
        );
        let after = fragment(
            |off| {
                vec![traf(&[
                    tfhd(&base_moof(1)),
                    trun(&full_run(off, vec![sample(3000, 3, KEY, 0)])),
                ])]
            },
            &payload(1, 3),
        );
        let events = same_under_all_plans(M, &[&video_init(), &empty, &after]).unwrap();
        let frames = frames_of(&events);
        assert_eq!(frames.len(), 1);
        assert_eq!(
            frames[0].dts,
            ns(900_000, 90_000),
            "continues from the empty fragment's tfdt"
        );
        assert_eq!(ends_of(&events), 2);
    }

    #[test]
    fn three_thousand_samples_in_one_run_are_exact() {
        // 3,000 samples in one trun: far more than the fixtures and still exact.
        let n = 3000usize;
        let data: Vec<u8> = (0..n).flat_map(|i| [i as u8, (i >> 8) as u8]).collect();
        let seg = fragment(
            |off| {
                let samples = (0..n)
                    .map(|i| sample(1001, 2, if i % 30 == 0 { KEY } else { NON_KEY }, 0))
                    .collect();
                vec![traf(&[
                    tfhd(&base_moof(1)),
                    tfdt(1, 5_000_000_000),
                    trun(&full_run(off, samples)),
                ])]
            },
            &data,
        );
        let f = frames_for(&[&video_init(), &seg]);
        assert_eq!(f.len(), n);
        for (i, fr) in f.iter().enumerate() {
            assert_eq!(fr.dts, ns(5_000_000_000 + 1001 * i as i64, 90_000));
            assert_eq!(fr.keyframe, i % 30 == 0);
            assert_eq!(fr.data, vec![i as u8, (i >> 8) as u8]);
        }
        // No drift: the last frame ends exactly where the sum of the tick durations says.
        let last = f.last().unwrap();
        assert_eq!(
            last.dts + last.duration,
            ns(5_000_000_000 + 1001 * n as i64, 90_000)
        );
    }

    #[test]
    fn samples_with_no_data_are_dropped_but_their_time_passes() {
        let data = payload(1, 8);
        let seg = fragment(
            |off| {
                let r = full_run(
                    off,
                    vec![
                        sample(3000, 4, KEY, 0),
                        sample(3000, 0, NON_KEY, 0),
                        sample(3000, 4, NON_KEY, 0),
                    ],
                );
                vec![traf(&[tfhd(&base_moof(1)), tfdt(0, 0), trun(&r)])]
            },
            &data,
        );
        let f = frames_for(&[&video_init(), &seg]);
        assert_eq!(f.len(), 2);
        assert_eq!((f[0].dts, f[1].dts), (0, ns(6000, 90_000)));
        assert_eq!(f[1].data, data[4..8]);
        // A fragment of nothing but empty samples is a segment with no frames.
        let empty = fragment(
            |off| {
                let r = full_run(off, vec![sample(3000, 0, KEY, 0)]);
                vec![traf(&[tfhd(&base_moof(1)), trun(&r)])]
            },
            &[],
        );
        let events = same_under_all_plans(M, &[&video_init(), &empty]).unwrap();
        assert!(frames_of(&events).is_empty());
        assert_eq!(ends_of(&events), 1);
    }

    // ---- the initialization segment ----

    fn entry_track(handler: &'static str, timescale: u32, entry: Vec<u8>) -> TrackSpec {
        TrackSpec {
            id: 1,
            handler,
            timescale,
            entry,
        }
    }

    fn only_init(stream: &[u8]) -> mercury_mse::api::InitSegment {
        let events = same_under_all_plans(M, &[stream]).unwrap();
        let inits = init_of(&events);
        assert_eq!(inits.len(), 1);
        inits[0].clone()
    }

    #[test]
    fn sample_entries_of_every_codec_this_round_feeds() {
        // Opus in MP4: `dOps` big-endian becomes an OpusHead.
        let mut dops = vec![0, 2];
        dops.extend_from_slice(&312u16.to_be_bytes());
        dops.extend_from_slice(&44_100u32.to_be_bytes());
        dops.extend_from_slice(&0u16.to_be_bytes());
        dops.push(0);
        let init = only_init(&init_of_tracks(&[entry_track(
            "soun",
            48_000,
            audio_entry("Opus", 2, 48_000, &bx("dOps", &dops)),
        )]));
        let t = &init.tracks[0];
        assert_eq!(
            (t.codec_string.as_str(), t.channels, t.sample_rate, t.kind),
            ("opus", 2, 48_000, TrackKind::Audio)
        );
        let CodecConfig::Opus { head } = &t.codec else {
            panic!()
        };
        assert_eq!(&head[..10], b"OpusHead\x01\x02");
        assert_eq!(u16::from_le_bytes([head[10], head[11]]), 312);
        assert_eq!(
            u32::from_le_bytes([head[12], head[13], head[14], head[15]]),
            44_100
        );

        // VP9 with vpcC (a full box): profile 2, level 51, bit depth 10.
        let vpcc = [2, 51, 0xA0, 1, 1, 1, 0, 0];
        let init = only_init(&init_of_tracks(&[entry_track(
            "vide",
            90_000,
            visual_entry("vp09", 640, 360, &full("vpcC", 1, 0, &vpcc)),
        )]));
        let t = &init.tracks[0];
        assert_eq!(
            (t.codec_string.as_str(), t.width, t.height),
            ("vp09.02.51.10", 640, 360)
        );
        assert!(matches!(&t.codec, CodecConfig::Vp9 { vpcc: Some(r) } if r == &vpcc));

        // AV1 with av1C including a sequence header OBU.
        let av1c = [0x81, 0x00, 0x0c, 0x00, 0x0a, 0x03, 1, 2, 3];
        let init = only_init(&init_of_tracks(&[entry_track(
            "vide",
            90_000,
            visual_entry("av01", 320, 180, &bx("av1C", &av1c)),
        )]));
        assert_eq!(init.tracks[0].codec_string, "av01.0.00M.08");
        assert!(matches!(&init.tracks[0].codec, CodecConfig::Av1 { av1c: r } if r == &av1c));

        // A protected entry is read through `sinf/frma` as the codec it wraps.
        let sinf = bx("sinf", &cat(&[&bx("frma", b"avc1"), &bx("schi", &[0; 8])]));
        let encv = visual_entry("encv", 320, 180, &cat(&[&bx("avcC", &avcc()), &sinf]));
        let init = only_init(&init_of_tracks(&[entry_track("vide", 90_000, encv)]));
        assert_eq!(init.tracks[0].codec_string, "avc1.42C01E");

        // Text and other handlers are ignored next to a video track.
        let text = TrackSpec {
            id: 5,
            handler: "text",
            timescale: 1000,
            entry: bx("tx3g", &[0; 8]),
        };
        let init = only_init(&init_of_tracks(&[TrackSpec::video(1, 90_000), text]));
        assert_eq!(init.tracks.len(), 1);
    }

    #[test]
    fn declared_durations_pssh_boxes_and_the_other_fields_of_the_moov() {
        let pssh1 = full("pssh", 0, 0, &cat(&[&[0xAA; 16], &be32(4), b"data"]));
        let pssh2 = full(
            "pssh",
            1,
            0,
            &cat(&[&[0xBB; 16], &be32(1), &[0xCC; 16], &be32(0)]),
        );
        // mvhd duration 8000 at timescale 1000: 8 s; with a mehd fragment duration of 4000, that wins.
        let mut moov = moov_of(
            &[TrackSpec::video(1, 90_000)],
            &[(1, 3000, 0, KEY)],
            &[pssh1.clone(), pssh2.clone()],
        );
        let pos = moov.windows(4).position(|w| w == b"mvhd").unwrap();
        // mvhd payload: version/flags(4) ctime(4) mtime(4) timescale(4) duration(4)
        moov[pos + 4 + 16..pos + 4 + 20].copy_from_slice(&8000u32.to_be_bytes());
        let init = only_init(&cat(&[&ftyp(), &moov]));
        assert_eq!(init.duration, Some(8_000_000_000));
        assert_eq!(
            init.pssh,
            vec![pssh1.clone(), pssh2.clone()],
            "raw, in order, byte for byte"
        );
        // mehd inside mvex wins over mvhd: build a moov whose mvex has one.
        let mehd = full("mehd", 1, 0, &4000u64.to_be_bytes());
        let mvex_at = moov.windows(4).position(|w| w == b"mvex").unwrap() - 4;
        let old_size = u32::from_be_bytes(moov[mvex_at..mvex_at + 4].try_into().unwrap()) as usize;
        let mut mvex_new = moov[mvex_at..mvex_at + old_size].to_vec();
        mvex_new.splice(8..8, mehd.iter().copied());
        let new_size = mvex_new.len() as u32;
        mvex_new[..4].copy_from_slice(&new_size.to_be_bytes());
        let mut moov2 = moov[..mvex_at].to_vec();
        moov2.extend_from_slice(&mvex_new);
        moov2.extend_from_slice(&moov[mvex_at + old_size..]);
        let total = moov2.len() as u32;
        moov2[..4].copy_from_slice(&total.to_be_bytes());
        let init = only_init(&cat(&[&ftyp(), &moov2]));
        assert_eq!(init.duration, Some(4_000_000_000));
        // Zero and the all-ones value mean unknown.
        let mut moov3 = moov.clone();
        moov3[pos + 4 + 16..pos + 4 + 20].copy_from_slice(&u32::MAX.to_be_bytes());
        assert_eq!(only_init(&cat(&[&ftyp(), &moov3])).duration, None);
    }

    #[test]
    fn a_moov_that_is_not_fragmented_or_already_holds_samples_is_refused() {
        let moov = moov_of(&[TrackSpec::video(1, 90_000)], &[], &[]);
        let mut no_mvex = moov.clone();
        let at = no_mvex.windows(4).position(|w| w == b"mvex").unwrap();
        no_mvex[at..at + 4].copy_from_slice(b"mfrz");
        expect_malformed("no mvex", &cat(&[&ftyp(), &no_mvex]));
        for table in [b"stts", b"stsc", b"stco"] {
            let mut with_samples = moov.clone();
            let at = with_samples.windows(4).position(|w| w == table).unwrap();
            with_samples[at + 8..at + 12].copy_from_slice(&1u32.to_be_bytes());
            expect_malformed(
                &format!("samples in {}", String::from_utf8_lossy(table)),
                &cat(&[&ftyp(), &with_samples]),
            );
        }
        // The same moov, untouched, is an initialization segment.
        assert_eq!(only_init(&cat(&[&ftyp(), &moov])).tracks.len(), 1);
    }

    fn unsupported_entry(handler: &'static str, ts: u32, entry: Vec<u8>) -> ParseError {
        error_of(same_under_all_plans(
            M,
            &[&init_of_tracks(&[entry_track(handler, ts, entry)])],
        ))
    }

    #[test]
    fn codecs_decision_d4_leaves_out_are_unsupported() {
        let kind = |e: ParseError| matches!(e, ParseError::Unsupported(_));
        assert!(kind(unsupported_entry(
            "vide",
            90_000,
            visual_entry("hvc1", 640, 360, &bx("hvcC", &[1; 23]))
        )));
        assert!(kind(unsupported_entry(
            "vide",
            90_000,
            visual_entry("hev1", 640, 360, &bx("hvcC", &[1; 23]))
        )));
        assert!(kind(unsupported_entry(
            "soun",
            48_000,
            audio_entry("ac-3", 2, 48_000, &bx("dac3", &[0; 3]))
        )));
        assert!(kind(unsupported_entry(
            "soun",
            48_000,
            audio_entry("ec-3", 2, 48_000, &bx("dec3", &[0; 5]))
        )));
        assert!(kind(unsupported_entry(
            "soun",
            48_000,
            audio_entry("fLaC", 2, 48_000, &bx("dfLa", &[0; 5]))
        )));
        assert!(kind(unsupported_entry(
            "soun",
            48_000,
            audio_entry("zzzz", 2, 48_000, &[])
        )));
        // mp4a with the MPEG-1 audio object type (MP3 in MP4).
        let mut esds = esds_aac();
        let at = esds.iter().position(|&b| b == 0x40).unwrap();
        esds[at] = 0x6B;
        assert!(kind(unsupported_entry(
            "soun",
            48_000,
            audio_entry("mp4a", 2, 44_100, &esds)
        )));
        // A sample entry that lacks its configuration box, a video entry in a sound track.
        let e = unsupported_entry("vide", 90_000, visual_entry("avc1", 640, 360, &[]));
        assert!(matches!(e, ParseError::Malformed(_)), "{e:?}");
        let e = unsupported_entry("soun", 90_000, avc1_entry());
        assert!(matches!(e, ParseError::Malformed(_)), "{e:?}");
    }

    #[test]
    fn tracks_one_per_kind() {
        let v = |id| TrackSpec::video(id, 90_000);
        let a = |id| TrackSpec::audio(id, 48_000);
        for tracks in [
            vec![v(1), v(2)],
            vec![a(1), a(2)],
            vec![v(1), a(2), v(3)],
            vec![v(1), v(2), a(3)],
        ] {
            let e = error_of(same_under_all_plans(M, &[&init_of_tracks(&tracks)]));
            assert_eq!(e, ParseError::TooManyTracks, "{} tracks", tracks.len());
        }
        let init = only_init(&init_of_tracks(&[v(1), a(2)]));
        assert_eq!(init.tracks.len(), 2);
        // Same id twice, id 0, nothing usable, no trak.
        let e = error_of(same_under_all_plans(M, &[&init_of_tracks(&[v(1), a(1)])]));
        assert!(matches!(e, ParseError::Malformed(_)));
        let e = error_of(same_under_all_plans(M, &[&init_of_tracks(&[v(0)])]));
        assert!(matches!(e, ParseError::Malformed(_)));
        let text = TrackSpec {
            id: 5,
            handler: "text",
            timescale: 1000,
            entry: bx("tx3g", &[0; 8]),
        };
        let e = error_of(same_under_all_plans(M, &[&init_of_tracks(&[text])]));
        assert!(matches!(e, ParseError::Unsupported(_)));
        let e = error_of(same_under_all_plans(
            M,
            &[&cat(&[&ftyp(), &bx("moov", &full("mvhd", 0, 0, &[0; 96]))])],
        ));
        assert!(matches!(e, ParseError::Malformed(_)));
    }

    // ---- refusals ----

    fn expect_malformed(what: &str, stream: &[u8]) {
        match error_of(same_under_all_plans(M, &[stream])) {
            ParseError::Malformed(_) => {}
            other => panic!("{what}: {other:?}"),
        }
    }

    #[test]
    fn broken_fragments_are_malformed() {
        let init = video_init();
        let good = |size: u32| {
            fragment(
                |off| {
                    vec![traf(&[
                        tfhd(&base_moof(1)),
                        trun(&full_run(off, vec![sample(3000, size, KEY, 0)])),
                    ])]
                },
                &payload(1, 8),
            )
        };
        // Sample data inside the moof itself (data offset 0 from the moof).
        let inside = fragment(
            |_| {
                vec![traf(&[
                    tfhd(&base_moof(1)),
                    trun(&full_run(0, vec![sample(3000, 8, KEY, 0)])),
                ])]
            },
            &payload(1, 8),
        );
        expect_malformed("data inside the moof", &cat(&[&init, &inside]));
        // Data before the moof (a negative offset past the start of the stream).
        let before = fragment(
            |_| {
                vec![traf(&[
                    tfhd(&base_moof(1)),
                    trun(&Run {
                        data_offset: Some(-100),
                        ..full_run(0, vec![sample(3000, 8, KEY, 0)])
                    }),
                ])]
            },
            &payload(1, 8),
        );
        expect_malformed("negative offset", &cat(&[&init, &before]));
        // A sample of 4 GiB (over the 64 MiB limit).
        expect_malformed("sample size", &cat(&[&init, &good(u32::MAX)]));
        expect_malformed("sample size 65 MiB", &cat(&[&init, &good(65 << 20)]));
        // A mdat with no moof.
        expect_malformed("mdat first", &cat(&[&init, &bx("mdat", &payload(1, 8))]));
        // A trun that claims more samples than its box holds.
        let mut r = trun(&full_run(1000, vec![sample(3000, 8, KEY, 0)]));
        r[12..16].copy_from_slice(&1000u32.to_be_bytes()); // sample_count
        let seg = cat(&[
            &moof(&[traf(&[tfhd(&base_moof(1)), r])]),
            &bx("mdat", &payload(1, 8)),
        ]);
        expect_malformed("trun count", &cat(&[&init, &seg]));
        // A traf with no tfhd; a tfhd that is cut short; a duplicate tfhd.
        let seg = cat(&[&moof(&[traf(&[tfdt(0, 0)])]), &bx("mdat", &[])]);
        expect_malformed("no tfhd", &cat(&[&init, &seg]));
        let seg = cat(&[
            &moof(&[traf(&[full("tfhd", 0, 0, &[0, 0])])]),
            &bx("mdat", &[]),
        ]);
        expect_malformed("short tfhd", &cat(&[&init, &seg]));
        let seg = cat(&[
            &moof(&[traf(&[tfhd(&base_moof(1)), tfhd(&base_moof(1))])]),
            &bx("mdat", &[]),
        ]);
        expect_malformed("two tfhd", &cat(&[&init, &seg]));
        // A time that does not fit.
        let seg = fragment(
            |off| {
                vec![traf(&[
                    tfhd(&base_moof(1)),
                    tfdt(1, u64::MAX),
                    trun(&full_run(off, vec![sample(3000, 8, KEY, 0)])),
                ])]
            },
            &payload(1, 8),
        );
        expect_malformed("tfdt out of range", &cat(&[&init, &seg]));
        // A box whose size is smaller than its header, and one of size 1 with a 64-bit size too small.
        expect_malformed(
            "size 4",
            &cat(&[&init, &[0, 0, 0, 4, b'f', b'r', b'e', b'e']]),
        );
        expect_malformed(
            "64-bit size 3",
            &cat(&[
                &init,
                &[0, 0, 0, 1, b'f', b'r', b'e', b'e', 0, 0, 0, 0, 0, 0, 0, 3],
            ]),
        );
        // A moof that extends to the end of the stream (only mdat may).
        expect_malformed(
            "open-ended moof",
            &cat(&[&init, &bx_to_end("moof", &[0; 8])]),
        );
        // A fragment that starts while the previous one's data is still to come (its only sample is far
        // beyond the mdat): the next moof or moov ends it.
        let far = fragment(
            |off| {
                vec![traf(&[
                    tfhd(&base_moof(1)),
                    trun(&full_run(off + 1000, vec![sample(3000, 8, KEY, 0)])),
                ])]
            },
            &payload(1, 8),
        );
        expect_malformed("moof while data is pending", &cat(&[&init, &far, &good(8)]));
        expect_malformed("moov while data is pending", &cat(&[&init, &far, &init]));
        // A sample that is longer than the mdat it starts in: the next box's bytes are not its data.
        let short = fragment(
            |off| {
                vec![traf(&[
                    tfhd(&base_moof(1)),
                    trun(&full_run(off, vec![sample(3000, 8, KEY, 0)])),
                ])]
            },
            &payload(1, 5),
        );
        expect_malformed(
            "sample longer than its mdat",
            &cat(&[&init, &short, &good(8)]),
        );
    }

    #[test]
    fn too_many_samples_is_unsupported() {
        let init = video_init();
        // sample_count 4 billion with no per-sample fields: nothing to read, so only the count limit stops it.
        let r = trun(&Run {
            data_offset: Some(100),
            samples: vec![RunSample::default(); 1],
            ..Run::default()
        });
        let mut r = r;
        r[12..16].copy_from_slice(&u32::MAX.to_be_bytes());
        let h = Tfhd {
            default_size: Some(1),
            default_duration: Some(1),
            ..base_moof(1)
        };
        let seg = cat(&[&moof(&[traf(&[tfhd(&h), r])]), &bx("mdat", &[0; 4])]);
        let e = error_of(same_under_all_plans(M, &[&cat(&[&init, &seg])]));
        assert!(matches!(e, ParseError::Unsupported(_)), "{e:?}");
    }

    #[test]
    fn media_before_the_initialization_segment() {
        let seg = fragment(
            |off| {
                vec![traf(&[
                    tfhd(&base_moof(1)),
                    trun(&full_run(off, vec![sample(3000, 8, KEY, 0)])),
                ])]
            },
            &payload(1, 8),
        );
        for (what, stream) in [
            ("moof", seg.clone()),
            (
                "styp, sidx, moof",
                cat(&[&bx("styp", b"msdh"), &bx("sidx", &[0; 12]), &seg]),
            ),
            ("mdat alone", bx("mdat", &[0; 20])),
            ("ftyp then moof", cat(&[&ftyp(), &seg])),
        ] {
            let e = error_of(same_under_all_plans(M, &[&stream]));
            assert_eq!(e, ParseError::MediaBeforeInit, "{what}");
        }
        // After a reset the init segment is still known: no error.
        let mut p = mercury_mse::parse::new_parser(M);
        push_plan(p.as_mut(), &video_init(), &plans()[0]).unwrap();
        p.reset();
        assert_eq!(
            frames_of(&push_plan(p.as_mut(), &seg, &plans()[0]).unwrap()).len(),
            1
        );
    }

    #[test]
    fn a_declared_four_gigabyte_box_is_refused_before_anything_is_kept() {
        for (what, needs_init, header) in [
            ("moov", false, cat(&[&be32(0xFFFF_FFF0), b"moov"])),
            (
                "64-bit moov",
                false,
                cat(&[&be32(1), b"moov", &(1u64 << 32).to_be_bytes()]),
            ),
            ("moof", true, cat(&[&be32(0xF000_0000), b"moof"])),
            ("free", true, cat(&[&be32(0x1000_0000), b"free"])),
            (
                "64-bit moof one byte over the limit",
                true,
                cat(&[&be32(1), b"moof", &((64u64 << 20) + 1).to_be_bytes()]),
            ),
        ] {
            let mut p = Mp4Parser::new();
            let mut bytes = if needs_init { video_init() } else { Vec::new() };
            bytes.extend_from_slice(&header);
            let mut seen = None;
            for piece in bytes.chunks(7) {
                if let Err(e) = p.push(piece) {
                    seen = Some(e);
                    break;
                }
            }
            assert!(
                matches!(seen, Some(ParseError::Malformed(_))),
                "{what}: {seen:?}"
            );
            assert!(
                p.retained_bytes() < 64,
                "{what}: {} bytes held",
                p.retained_bytes()
            );
        }
        // An mdat of any size is consumed incrementally, never held: 8 MiB through a 4 GiB mdat whose
        // only sample is a gigabyte away.
        let mut p = Mp4Parser::new();
        p.push(&video_init()).unwrap();
        let moof_bytes = moof(&[traf(&[
            tfhd(&base_moof(1)),
            trun(&full_run(1_000_000_000, vec![sample(3000, 8, KEY, 0)])),
        ])]);
        p.push(&moof_bytes).unwrap();
        p.push(&cat(&[&be32(0xFFFF_FFF0), b"mdat"])).unwrap();
        for _ in 0..8 {
            p.push(&vec![0u8; 1 << 20]).unwrap();
            assert!(
                p.retained_bytes() < 1 << 10,
                "{} bytes held",
                p.retained_bytes()
            );
        }
    }
}
