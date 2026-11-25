// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! T12 of the GstMSE spike at core level (lane med-1, item 3): 50 seeds of 300 random operations on a
//! real backend (add, append with random chunking, seek, remove, abort, set_duration, end_of_stream,
//! change_type, close, position, the attribute setters, the feed). No panic, no hang over 10 s, every
//! error of a documented kind, and the events obey the rules of decision D9. A failing seed prints its
//! last ten operations; `MSE_T12_SEED=n` replays that seed alone.

mod model_support;

use std::collections::{HashMap, HashSet};
use std::panic::{AssertUnwindSafe, catch_unwind, resume_unwind};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use mercury_mse::api::{
    AppendMode, EosError, Limits, MediaSourceBackend, MseError, MseEvent, SourceBufferId, TrackKind,
};
use mercury_mse_testkit::SegmentedStream;
use model_support::*;

const SEEDS: u64 = 50;
const OPS: usize = 300;

fn catalog() -> &'static Vec<SegmentedStream> {
    static CATALOG: OnceLock<Vec<SegmentedStream>> = OnceLock::new();
    CATALOG.get_or_init(|| {
        [
            "h264-low",
            "aac",
            "vp9-low",
            "opus",
            "h264-aac",
            "h264-bframes",
        ]
        .iter()
        .map(|n| load(n))
        .collect()
    })
}

const BAD_MIMES: [&str; 8] = [
    "",
    "video/foo",
    "video/mp4; codecs=\"hev1.1.6.L93.B0\"",
    "video/mp4; codecs=\"avc1.42C00C, avc1.42C01E\"",
    "audio/mp4; codecs=\"avc1.42C00C\"",
    "video/mp4; codecs=",
    "garbage",
    "video/webm; codecs=\"vorbis\"",
];

type Log = Arc<Mutex<Vec<String>>>;

struct Fuzz {
    rig: Rig,
    rng: Rng,
    log: Log,
    /// Buffers the harness has created and not removed itself (it may hold stale ids after `close`).
    ids: Vec<SourceBufferId>,
    accepted: HashMap<SourceBufferId, usize>,
    disturbed: HashSet<SourceBufferId>,
    /// Which stream each buffer was made for and how far a well-behaved player got into it.
    progress: HashMap<SourceBufferId, (usize, usize)>,
    /// How often the harness called `close()` (it resets the duration to NaN without an event).
    closes: usize,
}

fn kind_ok(op: &str, error: &MseError) -> bool {
    use MseError::*;
    match (op, error) {
        (_, Other(_)) => false,
        ("add_source_buffer", Type(_) | NotSupported(_) | QuotaExceeded(_) | InvalidState(_)) => {
            true
        }
        ("remove_source_buffer", NotFound(_)) => true,
        ("set_duration", Type(_) | InvalidState(_)) => true,
        ("end_of_stream", InvalidState(_)) => true,
        ("live_range", InvalidState(_) | Type(_)) => true,
        ("append", InvalidState(_) | NotFound(_) | QuotaExceeded(_)) => true,
        ("abort", InvalidState(_) | NotFound(_)) => true,
        ("remove", InvalidState(_) | NotFound(_) | Type(_)) => true,
        ("change_type", Type(_) | NotSupported(_) | InvalidState(_) | NotFound(_)) => true,
        ("setter", InvalidState(_) | Type(_) | NotFound(_)) => true,
        ("getter", NotFound(_)) => true,
        _ => false,
    }
}

impl Fuzz {
    fn note(&self, text: String) {
        self.log
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(text);
    }

    /// Records the operation, then checks the error kind if there was one.
    fn check<T: std::fmt::Debug>(&self, op: &str, desc: String, result: &Result<T, MseError>) {
        self.note(format!(
            "{desc} -> {}",
            match result {
                Ok(_) => "ok".to_string(),
                Err(e) => format!("{e}"),
            }
        ));
        if let Err(e) = result {
            assert!(
                kind_ok(op, e),
                "{op}: an undocumented error: {e:?} ({desc})"
            );
        }
    }

    fn pick_id(&mut self) -> SourceBufferId {
        if self.ids.is_empty() || self.rng.chance(5) {
            return self.rng.below(40) as SourceBufferId;
        }
        self.ids[self.rng.below(self.ids.len())]
    }

    fn some_time(&mut self) -> f64 {
        match self.rng.below(8) {
            0 => f64::NAN,
            1 => -1.0,
            2 => f64::INFINITY,
            3 => 0.0,
            4 => 8.0,
            _ => self.rng.below(2000) as f64 / 100.0,
        }
    }

    /// What to append to `id`: mostly the next piece of its own stream, as a player does, sometimes
    /// something else (another stream's piece, half a segment, noise, nothing).
    fn bytes_for_append(&mut self, id: SourceBufferId) -> Vec<u8> {
        let cat = catalog();
        if self.rng.chance(75)
            && let Some((index, next)) = self.progress.get_mut(&id)
        {
            let s = &cat[*index];
            let piece = if *next == 0 {
                s.init.bytes.clone()
            } else {
                s.media[(*next - 1) % s.media.len()].bytes.clone()
            };
            *next += 1;
            return piece;
        }
        let s = &cat[self.rng.below(cat.len())];
        match self.rng.below(12) {
            0 => Vec::new(),
            1 => {
                let n = self.rng.below(3000) + 1;
                self.rng.bytes(n)
            }
            2 | 3 => s.init.bytes.clone(),
            4 => {
                let m = &s.media[self.rng.below(s.media.len())].bytes;
                let cut = self.rng.below(m.len());
                m[cut..].to_vec()
            }
            _ => s.media[self.rng.below(s.media.len())].bytes.clone(),
        }
    }

    fn step(&mut self, n: usize) {
        let b = Arc::clone(&self.rig.backend);
        // A player whose stream failed starts over (a new attach) more often than not.
        if b.ready_state() == mercury_mse::api::MediaSourceReadyState::Ended && self.rng.chance(25)
        {
            self.note(format!("#{n} recover: close() and open()"));
            b.close();
            b.open();
            self.closes += 1;
            self.disturbed.extend(self.ids.iter().copied());
            self.ids.clear();
            return;
        }
        match self.rng.below(40) {
            0..=2 => {
                let cat = catalog();
                let index = self.rng.below(cat.len());
                let mime = if self.rng.chance(10) {
                    BAD_MIMES[self.rng.below(BAD_MIMES.len())].to_string()
                } else {
                    cat[index].mime.clone()
                };
                let r = b.add_source_buffer(&mime);
                self.check(
                    "add_source_buffer",
                    format!("#{n} add_source_buffer({mime:?})"),
                    &r,
                );
                if let Ok(id) = r {
                    self.ids.push(id);
                    self.progress.insert(id, (index, 0));
                }
            }
            3..=14 => {
                let id = self.pick_id();
                let data = self.bytes_for_append(id);
                let len = data.len();
                let r = b.append(id, data);
                self.check("append", format!("#{n} append({id}, {len} bytes)"), &r);
                if r.is_ok() {
                    *self.accepted.entry(id).or_default() += 1;
                }
            }
            15..=17 => {
                // wait for the buffers to be idle (a hang here is a failure)
                let ids = self.ids.clone();
                self.note(format!("#{n} wait_idle"));
                self.rig.wait_idle(&ids);
            }
            18 => {
                let id = self.pick_id();
                let r = b.abort(id);
                self.check("abort", format!("#{n} abort({id})"), &r);
                self.disturbed.insert(id);
            }
            19 | 20 => {
                let id = self.pick_id();
                let (s, e) = if self.rng.chance(60) {
                    let s = self.rng.below(7) as f64;
                    let e = if self.rng.chance(30) {
                        f64::INFINITY
                    } else {
                        s + 1.0 + self.rng.below(3) as f64
                    };
                    (s, e)
                } else {
                    (self.some_time(), self.some_time())
                };
                let r = b.remove(id, s, e);
                self.check("remove", format!("#{n} remove({id}, {s}, {e})"), &r);
                if r.is_ok() {
                    *self.accepted.entry(id).or_default() += 1;
                }
            }
            21 => {
                let d = if self.rng.chance(10) {
                    None
                } else {
                    Some(self.some_time())
                };
                let r = b.set_duration(d);
                self.check("set_duration", format!("#{n} set_duration({d:?})"), &r);
            }
            22 => {
                let e = [None, Some(EosError::Network), Some(EosError::Decode)][self.rng.below(3)];
                let r = b.end_of_stream(e);
                self.check("end_of_stream", format!("#{n} end_of_stream({e:?})"), &r);
            }
            23 => {
                let id = self.pick_id();
                let mime = if self.rng.chance(20) {
                    BAD_MIMES[self.rng.below(BAD_MIMES.len())].to_string()
                } else {
                    let cat = catalog();
                    cat[self.rng.below(cat.len())].mime.clone()
                };
                let r = b.change_type(id, &mime);
                self.check(
                    "change_type",
                    format!("#{n} change_type({id}, {mime:?})"),
                    &r,
                );
            }
            24 => {
                let id = self.pick_id();
                let r = b.remove_source_buffer(id);
                self.check(
                    "remove_source_buffer",
                    format!("#{n} remove_source_buffer({id})"),
                    &r,
                );
                self.disturbed.insert(id);
            }
            25 => {
                if self.rng.chance(30) {
                    self.note(format!("#{n} close()"));
                    b.close();
                    self.closes += 1;
                    self.disturbed.extend(self.ids.iter().copied());
                } else {
                    self.note(format!("#{n} open()"));
                    b.open();
                }
            }
            26 | 27 => {
                let t = self.some_time();
                if self.rng.chance(50) {
                    self.note(format!("#{n} set_position({t})"));
                    b.set_position(t);
                } else {
                    self.note(format!("#{n} seeking({t})"));
                    let _ = b.seeking(t);
                }
            }
            28 | 29 => {
                let feed = b.frame_feed();
                let t = self.some_time();
                if self.rng.chance(40) {
                    let ns = if t.is_finite() { (t * 1e9) as i64 } else { 0 };
                    self.note(format!("#{n} feed.seek({ns})"));
                    feed.seek(ns);
                } else {
                    let kind = TrackKind::ALL[self.rng.below(3)];
                    let max = self.rng.below(40);
                    self.note(format!("#{n} feed.pull({kind:?}, {max})"));
                    let batch = feed.pull(kind, max);
                    assert!(
                        batch.items.len() <= max.max(1) * 2 + 2,
                        "pull returned {} items for max {max}",
                        batch.items.len()
                    );
                }
            }
            30 => {
                let id = self.pick_id();
                let mode = [AppendMode::Segments, AppendMode::Sequence][self.rng.below(2)];
                let r = b.set_mode(id, mode);
                self.check("setter", format!("#{n} set_mode({id}, {mode:?})"), &r);
            }
            31 | 32 => {
                let id = self.pick_id();
                let v = self.some_time();
                let which = self.rng.below(3);
                let r = match which {
                    0 => b.set_timestamp_offset(id, v),
                    1 => b.set_append_window_start(id, v),
                    _ => b.set_append_window_end(id, v),
                };
                self.check("setter", format!("#{n} setter{which}({id}, {v})"), &r);
            }
            33 => {
                let (s, e) = (self.some_time(), self.some_time());
                let r = if self.rng.chance(70) {
                    b.set_live_seekable_range(s, e)
                } else {
                    b.clear_live_seekable_range()
                };
                self.check("live_range", format!("#{n} live range ({s}, {e})"), &r);
            }
            34 => {
                let id = self.pick_id();
                self.note(format!("#{n} getters({id})"));
                let _ = b.buffered(id);
                let _ = b.updating(id);
                for r in [
                    b.mode(id).map(|_| ()),
                    b.timestamp_offset(id).map(|_| ()),
                    b.append_window_start(id).map(|_| ()),
                    b.append_window_end(id).map(|_| ()),
                ] {
                    self.check("getter", format!("#{n} getter({id})"), &r);
                }
            }
            35 => {
                let t = self.some_time();
                self.note(format!("#{n} element queries at {t}"));
                let _ = b.element_buffered();
                let _ = b.seekable();
                let _ = b.element_ready_state(t);
                let _ = b.ready_state();
                let _ = b.duration();
                let _ = b.source_buffer_count();
            }
            _ => {
                // a short pause lets the worker overlap with the next operations
                self.note(format!("#{n} yield"));
                std::thread::yield_now();
            }
        }
    }
}

/// The rules of decision D9 over everything that was recorded.
fn check_events(f: &Fuzz) {
    let log = f.rig.rec.snapshot();
    let mut last_duration: Option<Option<f64>> = None;
    let mut append_errors = 0;
    let mut ended_decode = 0;
    for r in &log {
        assert!(
            r.thread_name.starts_with("mse-"),
            "an event on {:?}: {:?}",
            r.thread_name,
            r.event
        );
        match &r.event {
            MseEvent::DurationChanged(d) => {
                if let Some(prev) = last_duration.filter(|_| f.closes == 0) {
                    let same = match (prev, d) {
                        (Some(a), Some(b)) => a == *b,
                        (None, None) => true,
                        _ => false,
                    };
                    assert!(
                        !same,
                        "two identical DurationChanged in a row: {prev:?} then {d:?}"
                    );
                }
                last_duration = Some(*d);
            }
            MseEvent::AppendError { .. } => append_errors += 1,
            MseEvent::SourceEnded {
                error: Some(EosError::Decode),
            } => ended_decode += 1,
            _ => {}
        }
    }
    // The append error algorithm ends the stream with the decode error once per error.
    assert!(
        ended_decode >= append_errors,
        "{append_errors} append errors, {ended_decode} decode endings"
    );
    // One completion per accepted operation for the buffers nothing cancelled.
    for (id, accepted) in &f.accepted {
        if f.disturbed.contains(id) {
            assert!(
                f.rig.rec.completions(*id) <= *accepted,
                "buffer {id}: {} completions for {accepted} accepted operations",
                f.rig.rec.completions(*id)
            );
        } else {
            assert_eq!(
                f.rig.rec.completions(*id),
                *accepted,
                "buffer {id}: completions and accepted operations differ"
            );
        }
    }
}

fn fuzz(seed: u64, log: Log) {
    let limits = if seed.is_multiple_of(3) {
        Limits {
            video_bytes: 60_000,
            audio_bytes: 20_000,
            text_bytes: 1_000,
            ..Limits::default()
        }
    } else {
        Limits::default()
    };
    let rig = Rig::closed(limits);
    if !seed.is_multiple_of(5) {
        rig.backend.open();
    }
    let mut f = Fuzz {
        rig,
        rng: Rng::new(seed),
        log,
        ids: Vec::new(),
        accepted: HashMap::new(),
        disturbed: HashSet::new(),
        progress: HashMap::new(),
        closes: 0,
    };
    for n in 0..OPS {
        f.step(n);
    }
    // Everything settles, and the invariants hold.
    f.note("final wait_idle".into());
    let mut all: Vec<SourceBufferId> = f.ids.clone();
    all.extend(f.accepted.keys().copied());
    f.rig.wait_idle(&all);
    check_events(&f);
    let counts = |pred: fn(&MseEvent) -> bool| f.rig.rec.count(pred);
    let stats = Stats {
        accepted: f.accepted.values().sum(),
        done: counts(|e| matches!(e, MseEvent::AppendDone { .. })),
        errors: counts(|e| matches!(e, MseEvent::AppendError { .. })),
        removed: counts(|e| matches!(e, MseEvent::RemoveDone { .. })),
        inits: counts(|e| matches!(e, MseEvent::InitSegment { .. })),
        ended: counts(|e| matches!(e, MseEvent::SourceEnded { .. })),
    };
    TOTALS.lock().unwrap().add(&stats);
}

/// What the random operations really did, summed over the seeds: a fuzz run that never got an append
/// through would pass for nothing.
#[derive(Default, Debug)]
struct Stats {
    accepted: usize,
    done: usize,
    errors: usize,
    removed: usize,
    inits: usize,
    ended: usize,
}

impl Stats {
    fn add(&mut self, other: &Stats) {
        self.accepted += other.accepted;
        self.done += other.done;
        self.errors += other.errors;
        self.removed += other.removed;
        self.inits += other.inits;
        self.ended += other.ended;
    }
}

static TOTALS: Mutex<Stats> = Mutex::new(Stats {
    accepted: 0,
    done: 0,
    errors: 0,
    removed: 0,
    inits: 0,
    ended: 0,
});

fn run_seed(seed: u64) {
    let log: Log = Arc::new(Mutex::new(Vec::new()));
    let for_run = Arc::clone(&log);
    let result = catch_unwind(AssertUnwindSafe(|| {
        watchdog(
            &format!("t12 seed {seed}"),
            Duration::from_secs(10),
            move || fuzz(seed, for_run),
        )
    }));
    if let Err(payload) = result {
        let log = log.lock().unwrap_or_else(|e| e.into_inner());
        let from = log.len().saturating_sub(10);
        eprintln!("t12: seed {seed} failed; the last ten operations:");
        for line in &log[from..] {
            eprintln!("    {line}");
        }
        eprintln!("replay with MSE_T12_SEED={seed}");
        resume_unwind(payload);
    }
}

#[test]
fn t12_fifty_seeds_of_three_hundred_random_operations() {
    if let Ok(only) = std::env::var("MSE_T12_SEED") {
        let seed: u64 = only.parse().expect("MSE_T12_SEED is a number");
        run_seed(seed);
        return;
    }
    for seed in 1..=SEEDS {
        run_seed(seed);
    }
    let totals = TOTALS.lock().unwrap();
    eprintln!("t12: {SEEDS} seeds of {OPS} operations: {totals:?}");
    assert!(
        totals.done > 100,
        "the fuzz got {} appends through",
        totals.done
    );
    assert!(
        totals.errors > 5,
        "the fuzz made {} appends fail",
        totals.errors
    );
    assert!(
        totals.removed > 5,
        "the fuzz made {} removes complete",
        totals.removed
    );
    assert!(totals.inits > 50, "{} init segments", totals.inits);
}
