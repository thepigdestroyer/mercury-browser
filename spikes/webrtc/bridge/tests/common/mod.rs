// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! Helpers shared by the bridge tests: a recording sink and a wait loop.
#![allow(dead_code)]

pub mod proc;

use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use mercury_frame_sink::{MediaFrameSink, Micros, RawAudioFrame, RawVideoFrame};

/// One delivered frame: its producer timestamp, and the wall time at which the sink was called.
#[derive(Debug, Clone, Copy)]
pub struct Seen {
    pub timestamp: Micros,
    pub at: Instant,
}

/// A sink that checks and records everything it is given.
#[derive(Default)]
pub struct Recorder {
    /// Time to sleep in every call (a slow sink).
    pub delay: Duration,
    pub video_ok: AtomicU64,
    pub audio_ok: AtomicU64,
    pub invalid: AtomicU64,
    pub ended: AtomicU64,
    /// Frames that arrived after `ended()`.
    pub after_end: AtomicU64,
    pub video: Mutex<Vec<Seen>>,
    pub audio: Mutex<Vec<Seen>>,
    pub sizes: Mutex<Vec<(u32, u32)>>,
    pub first_error: Mutex<Option<String>>,
    pub peak_sample: Mutex<i32>,
}

impl Recorder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn slow(delay: Duration) -> Self {
        Self {
            delay,
            ..Self::default()
        }
    }

    pub fn video_count(&self) -> u64 {
        self.video_ok.load(Ordering::SeqCst)
    }

    pub fn audio_count(&self) -> u64 {
        self.audio_ok.load(Ordering::SeqCst)
    }

    pub fn ended_count(&self) -> u64 {
        self.ended.load(Ordering::SeqCst)
    }

    pub fn invalid_count(&self) -> u64 {
        self.invalid.load(Ordering::SeqCst)
    }

    pub fn video_times(&self) -> Vec<Seen> {
        self.video.lock().unwrap().clone()
    }

    pub fn audio_times(&self) -> Vec<Seen> {
        self.audio.lock().unwrap().clone()
    }

    fn note_error(&self, text: String) {
        self.invalid.fetch_add(1, Ordering::SeqCst);
        let mut first = self.first_error.lock().unwrap();
        if first.is_none() {
            *first = Some(text);
        }
    }

    fn guard_end(&self) {
        if self.ended_count() > 0 {
            self.after_end.fetch_add(1, Ordering::SeqCst);
        }
    }
}

impl MediaFrameSink for Recorder {
    fn video_frame(&self, frame: RawVideoFrame<'_>) {
        self.guard_end();
        if let Err(error) = frame.validate() {
            self.note_error(format!("video frame invalid: {error}"));
        } else {
            self.video_ok.fetch_add(1, Ordering::SeqCst);
        }
        self.video.lock().unwrap().push(Seen {
            timestamp: frame.timestamp,
            at: Instant::now(),
        });
        {
            let mut sizes = self.sizes.lock().unwrap();
            if sizes.last() != Some(&(frame.width, frame.height)) {
                sizes.push((frame.width, frame.height));
            }
        }
        if !self.delay.is_zero() {
            std::thread::sleep(self.delay);
        }
    }

    fn audio_frame(&self, frame: RawAudioFrame<'_>) {
        self.guard_end();
        if let Err(error) = frame.validate() {
            self.note_error(format!("audio block invalid: {error}"));
        } else {
            self.audio_ok.fetch_add(1, Ordering::SeqCst);
        }
        let peak = frame
            .samples
            .iter()
            .map(|s| i32::from(*s).abs())
            .max()
            .unwrap_or(0);
        {
            let mut seen = self.peak_sample.lock().unwrap();
            *seen = (*seen).max(peak);
        }
        self.audio.lock().unwrap().push(Seen {
            timestamp: frame.timestamp,
            at: Instant::now(),
        });
        if !self.delay.is_zero() {
            std::thread::sleep(self.delay);
        }
    }

    fn ended(&self) {
        self.ended.fetch_add(1, Ordering::SeqCst);
    }
}

/// Polls `condition` every 10 ms until it holds; false on timeout.
pub fn wait_until(timeout: Duration, mut condition: impl FnMut() -> bool) -> bool {
    let start = Instant::now();
    while start.elapsed() < timeout {
        if condition() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    condition()
}

/// Microseconds on a wall clock anchored at the first call (monotonic, `Instant`-based).
pub fn wall_micros() -> i64 {
    use std::sync::OnceLock;
    static ORIGIN: OnceLock<Instant> = OnceLock::new();
    let origin = ORIGIN.get_or_init(Instant::now);
    i64::try_from(origin.elapsed().as_micros()).unwrap()
}

/// A wall-clock instant as microseconds on the scale of [`wall_micros`].
pub fn micros_of(instant: Instant) -> i64 {
    // Make sure the origin exists and is not later than `instant`'s capture (it is created by the
    // first call of `wall_micros`, which tests make before starting a stream).
    let now_micros = wall_micros();
    let back = Instant::now().saturating_duration_since(instant);
    now_micros - i64::try_from(back.as_micros()).unwrap()
}

/// A producer clock anchored on the wall clock: the best of several adjacent reads, with the
/// uncertainty of the best pair in microseconds.
pub fn anchored_map(producer_now: impl Fn() -> Micros) -> (mercury_frame_sink::ClockMap, i64) {
    let mut best: Option<(i64, Micros, i64)> = None;
    for _ in 0..8 {
        let before = wall_micros();
        let producer = producer_now();
        let after = wall_micros();
        let gap = after - before;
        if best.is_none_or(|(g, _, _)| gap < g) {
            best = Some((gap, producer, (before + after) / 2));
        }
    }
    let (gap, producer, wall) = best.unwrap();
    (
        mercury_frame_sink::ClockMap::new(producer, Micros(wall)),
        gap / 2 + 1,
    )
}

/// The median of a list (the middle element after sorting; 0 for an empty list).
pub fn median(mut values: Vec<i64>) -> i64 {
    values.sort_unstable();
    values.get(values.len() / 2).copied().unwrap_or(0)
}
