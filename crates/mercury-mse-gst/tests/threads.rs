// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! T01 and T02 at GStreamer level (R17 section 11.2): who calls the feed, and that nothing deadlocks when
//! the feed, the sinks and the wake closure all call each other.

mod common;

use std::sync::Arc;
use std::time::Duration;

use common::fakefeed::FakeFeed;
use common::rig::{Options, Rig, Shape, watchdog};
use common::scenario::seek_and_measure;
use mercury_mse::{FrameFeed, TrackKind};

const LIMIT: Duration = Duration::from_secs(30);

/// T01: the feed is called from the element's own thread and from nowhere else: not from the thread that
/// built the pipeline, not from a GStreamer streaming thread, not from a thread of the bus. And there is
/// no GLib main loop in the test at all.
#[test]
fn t01_the_feed_is_only_called_from_the_feeder_thread() {
    require!(
        "avdec_h264",
        "avdec_aac",
        "playbin3",
        "videoconvert",
        "appsrc"
    );
    watchdog("t01", 90, || {
        for shape in [Shape::PlayBin, Shape::Bare] {
            let feed = FakeFeed::from_fixtures(&["h264-aac"]);
            feed.release_all();
            feed.end();
            let rig = Rig::new(
                feed.clone(),
                Options {
                    shape,
                    sync: true,
                    ..Options::default()
                },
            );
            rig.play();
            rig.wait_until(LIMIT, "some frames", |r| {
                r.video_stats().lock().unwrap().count() >= 25
            });
            seek_and_measure(&rig, 5_000_000_000, true, LIMIT);
            rig.wait_eos(LIMIT);
            let callers = feed.callers();
            eprintln!("{shape:?}: the feed was called from {callers:?}");
            assert!(!callers.is_empty());
            for name in &callers {
                assert!(
                    name.starts_with("mse-feed-"),
                    "{shape:?}: the feed was called from {name:?}, only mse-feed-* may"
                );
            }
        }
    });
}

/// T02: a feed that wakes from inside `pull` and `seek` with its lock held, sinks that call the feed from
/// their streaming threads on every buffer, a seek in the middle: it plays and ends.
#[test]
fn t02_reentrant_calls_do_not_deadlock() {
    require!(
        "avdec_h264",
        "avdec_aac",
        "playbin3",
        "videoconvert",
        "appsrc"
    );
    watchdog("t02", 90, || {
        let feed = FakeFeed::from_fixtures(&["h264-aac"]);
        feed.set_wake_in_calls(true);
        feed.release_all();
        feed.end();
        let hostile = {
            let feed = feed.clone();
            Arc::new(move || {
                // Every method that has no effect on the cursor, from a streaming thread.
                let _ = feed.tracks();
                let _ = feed.all_initialized();
                let _ = feed.duration();
                let _ = feed.pull(TrackKind::Text, 1);
            })
        };
        let rig = Rig::with_hook(
            feed.clone(),
            Options {
                sync: true,
                ..Options::default()
            },
            Some(hostile),
        );
        rig.play();
        rig.wait_until(LIMIT, "some frames", |r| {
            r.video_stats().lock().unwrap().count() >= 25
        });
        let outcome = seek_and_measure(&rig, 3_000_000_000, true, LIMIT);
        eprintln!("seek under re-entrant calls: {outcome:?}");
        rig.wait_eos(LIMIT);
    });
}

// ---- T10: four pipelines at once ----

/// Four pipelines at once, each with a source of its own: nothing leaks from one into another, and no
/// element of two pipelines has the same name.
#[test]
fn t10_four_pipelines_at_once_do_not_cross_talk() {
    use common::real::{RealSource, load};
    use mercury_mse_testkit::ChunkPlan;
    require!(
        "avdec_h264",
        "avdec_aac",
        "vp9dec",
        "opusdec",
        "playbin3",
        "videoconvert",
        "appsrc"
    );
    watchdog("t10", 120, || {
        // (video, audio, the video size that must come out)
        let cases: [(&str, &str, (i32, i32)); 4] = [
            ("h264-low", "aac", (320, 180)),
            ("h264-high", "aac", (640, 360)),
            ("vp9-low", "opus", (320, 180)),
            ("vp9-high", "opus", (640, 360)),
        ];
        let mut sources = Vec::new();
        let mut rigs = Vec::new();
        for (video, audio, _) in &cases {
            let source = RealSource::new();
            for name in [video, audio] {
                let stream = load(name);
                let id = source.add(&stream.mime);
                source.append_stream(id, &stream, &ChunkPlan::Whole);
            }
            source.end_of_stream();
            rigs.push(Rig::new(
                source.feed(),
                Options {
                    shape: Shape::PlayBin,
                    ..Options::default()
                },
            ));
            sources.push(source);
        }
        for rig in &rigs {
            rig.play();
        }
        for rig in &rigs {
            rig.wait_eos(LIMIT);
        }
        // No element name appears twice across the four pipelines' appsrcs.
        let mut names: Vec<String> = Vec::new();
        for rig in &rigs {
            for line in rig.dump_tree().lines() {
                if line.contains("[appsrc]") {
                    names.push(line.trim().split(' ').next().unwrap().to_owned());
                }
            }
        }
        let unique: std::collections::BTreeSet<_> = names.iter().collect();
        assert_eq!(names.len(), 8, "{names:?}");
        assert_eq!(unique.len(), names.len(), "appsrc names collide: {names:?}");
        for (rig, (video, audio, size)) in rigs.iter().zip(&cases) {
            let v = rig.video_stats();
            let v = v.lock().unwrap();
            let a = rig.audio_stats();
            let a = a.lock().unwrap();
            eprintln!(
                "T10 {video}+{audio}: {} frames {:?}, {:.3} s audio",
                v.count(),
                v.caps.first().map(|c| c.len()),
                a.total_seconds()
            );
            assert!(
                (198..=202).contains(&v.count()),
                "{video}: {} frames",
                v.count()
            );
            assert!(
                a.total_seconds() > 7.9 && a.total_seconds() < 8.1,
                "{audio}"
            );
            assert!(
                v.caps
                    .iter()
                    .all(|c| c.contains(&format!("width=(int){}", size.0))
                        && c.contains(&format!("height=(int){}", size.1))),
                "{video}: another pipeline's frames arrived: {:?}",
                v.caps
            );
            assert!(
                common::scenario::is_increasing(&v, 0),
                "{video}: timestamps out of order"
            );
        }
    });
}

// ---- T12: random operations with a pipeline attached ----

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        // xorshift64*
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
}

/// One seed: 100 operations against a real backend and a running pipeline. Nothing may hang or panic;
/// errors of the backend are allowed (they are the documented kinds), a pipeline error is not a failure
/// either (a refused configuration is an error by design), a deadlock is.
fn fuzz_one(
    seed: u64,
    video: &common::real::Loaded,
    audio: &common::real::Loaded,
    log: &std::sync::Mutex<Vec<String>>,
) {
    use common::real::RealSource;
    use mercury_mse::MediaSourceBackend;
    let mut rng = Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
    let source = RealSource::new();
    let ids = [source.add(&video.mime), source.add(&audio.mime)];
    let streams = [video, audio];
    source.append_init(ids[0], &streams[0].stream);
    source.append_init(ids[1], &streams[1].stream);
    let mut next = [0usize, 0usize];
    let sync = rng.below(2) == 0;
    let rig = Rig::new(
        source.feed(),
        Options {
            shape: Shape::PlayBin,
            sync,
            colour: false,
            ..Options::default()
        },
    );
    rig.play();
    let mut paused = false;
    for step in 0..100 {
        let op = rng.below(10);
        let which = rng.below(2) as usize;
        let id = ids[which];
        let line = match op {
            0..=3 => {
                let stream = &streams[which].stream;
                let index = next[which] % stream.media.len();
                let result = source.backend.append(id, stream.media[index].bytes.clone());
                if result.is_ok() {
                    next[which] += 1;
                }
                format!(
                    "append buffer {which} segment {index}: {:?}",
                    result.map_err(|e| e.dom_exception_name())
                )
            }
            4 => {
                let target = rng.below(9_000) * 1_000_000;
                format!("seek {target} ns: {}", rig.seek(target, rng.below(2) == 0))
            }
            5 => {
                paused = !paused;
                if paused {
                    rig.pause();
                } else {
                    rig.play();
                }
                format!("paused {paused}")
            }
            6 => {
                let start = rng.below(6) as f64;
                let result = source
                    .backend
                    .remove(id, start, start + 1.0 + rng.below(3) as f64);
                format!(
                    "remove buffer {which} from {start}: {:?}",
                    result.map_err(|e| e.dom_exception_name())
                )
            }
            7 => match rng.below(4) {
                0 => format!(
                    "end_of_stream: {:?}",
                    source
                        .backend
                        .end_of_stream(None)
                        .map_err(|e| e.dom_exception_name())
                ),
                1 => format!(
                    "abort {which}: {:?}",
                    source.backend.abort(id).map_err(|e| e.dom_exception_name())
                ),
                2 => format!(
                    "set_duration: {:?}",
                    source
                        .backend
                        .set_duration(Some(8.0 + rng.below(4) as f64))
                        .map_err(|e| e.dom_exception_name())
                ),
                _ => format!("seeking: {}", source.backend.seeking(rng.below(8) as f64)),
            },
            8 => {
                let ms = rng.below(15);
                std::thread::sleep(Duration::from_millis(ms));
                format!("sleep {ms} ms")
            }
            _ => {
                rig.pump_bus();
                "pump bus".to_owned()
            }
        };
        let mut log = log.lock().unwrap();
        log.push(format!("{step:3} {line}"));
        if log.len() > 10 {
            log.remove(0);
        }
    }
    rig.stop();
    drop(rig);
    drop(source);
}

#[test]
fn t12_random_operations_with_a_pipeline() {
    require!(
        "avdec_h264",
        "avdec_aac",
        "playbin3",
        "videoconvert",
        "appsrc"
    );
    let only: Option<u64> = std::env::var("MSE_T12_SEED")
        .ok()
        .and_then(|s| s.parse().ok());
    let seeds: Vec<u64> = match only {
        Some(seed) => vec![seed],
        None => (1..=10).collect(),
    };
    let video = std::sync::Arc::new(common::real::loaded("h264-low"));
    let audio = std::sync::Arc::new(common::real::loaded("aac"));
    for seed in seeds {
        let (v, a) = (video.clone(), audio.clone());
        // 10 s per seed: a hang is the failure; the last operations are in the message.
        let log = std::sync::Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
        let log_out = log.clone();
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            watchdog(&format!("t12-{seed}"), 10, move || {
                fuzz_one(seed, &v, &a, &log_out)
            })
        }));
        if let Err(panic) = outcome {
            panic!(
                "T12 seed {seed} failed (replay: MSE_T12_SEED={seed}); last operations:\n{}\n{panic:?}",
                log.lock().unwrap().join("\n")
            );
        }
        eprintln!("T12 seed {seed}: ok");
    }
}
