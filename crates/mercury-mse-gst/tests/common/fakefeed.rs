// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! A [`FrameFeed`] for the playback tests, until (and next to) the real `MseBackend`: frames from the
//! fixtures that a test releases the way appends would, a cursor that serves them in order from the random
//! access point at or before a seek position, `Config` items when the rendition changes, and the three
//! feed states. It is deliberately strict where the real backend may be careless: the wake closure is
//! called **with the feed's lock held** (the worst case: a feed that wakes from inside its own critical
//! section), so a source element that called back into the feed from `wake` would deadlock here.

use std::sync::{Arc, Mutex, MutexGuard};

use mercury_mse::{
    CodedFrame, FeedBatch, FeedItem, FeedState, FrameFeed, Ns, TrackConfig, TrackKind,
};

use super::frames::Extracted;

struct Track {
    configs: Vec<TrackConfig>,
    /// Every frame of the track in decode order, with the index of its configuration and whether an
    /// append has delivered it.
    frames: Vec<(usize, CodedFrame, bool)>,
    cursor: usize,
    last_config: Option<usize>,
    discont: bool,
    /// `tracks()` lists it (its initialization segment has been appended).
    visible: bool,
}

struct Inner {
    tracks: [Option<Track>; 3],
    initialized: bool,
    duration: Option<Ns>,
    ended: bool,
    /// A seek whose target has no available frame yet: resolved by the next pull that finds data.
    pending_seek: Option<Ns>,
    seeks: Vec<Ns>,
    pulls: u64,
    wake_under_lock: bool,
}

pub struct FakeFeed {
    inner: Mutex<Inner>,
    wake: Mutex<Option<Arc<dyn Fn() + Send + Sync>>>,
    /// The names of the threads that called into the feed (T01: only the element's feeder thread may).
    callers: Mutex<std::collections::BTreeSet<String>>,
    /// Call the wake closure from inside `pull` and `seek`, with the lock held (T02).
    wake_in_calls: std::sync::atomic::AtomicBool,
}

fn slot(kind: TrackKind) -> usize {
    kind as usize
}

impl FakeFeed {
    pub fn new() -> Arc<Self> {
        Arc::new(FakeFeed {
            inner: Mutex::new(Inner {
                tracks: [None, None, None],
                initialized: false,
                duration: None,
                ended: false,
                pending_seek: None,
                seeks: Vec::new(),
                pulls: 0,
                wake_under_lock: true,
            }),
            wake: Mutex::new(None),
            callers: Mutex::default(),
            wake_in_calls: std::sync::atomic::AtomicBool::new(false),
        })
    }

    /// A feed with the tracks of these fixtures, initialized, with nothing released yet, duration 8 s.
    pub fn from_fixtures(names: &[&str]) -> Arc<Self> {
        let feed = FakeFeed::new();
        for name in names {
            for track in super::frames::extract(name).iter() {
                feed.add_track(track);
            }
        }
        feed.set_duration(Some(8_000_000_000));
        feed.set_initialized(true);
        feed
    }

    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap()
    }

    /// Wakes the consumer: with the lock held in the strict mode, after releasing it otherwise.
    fn finish(&self, inner: MutexGuard<'_, Inner>) {
        let wake = self.wake.lock().unwrap().clone();
        if inner.wake_under_lock {
            if let Some(wake) = &wake {
                wake();
            }
            drop(inner);
        } else {
            drop(inner);
            if let Some(wake) = &wake {
                wake();
            }
        }
    }

    /// Adds a track (visible at once) with one configuration.
    pub fn add_track(&self, track: &Extracted) {
        self.add_track_with(
            track.kind,
            vec![track.config.clone()],
            track.frames.iter().map(|f| (0, f.clone())).collect(),
            true,
        );
    }

    pub fn add_track_with(
        &self,
        kind: TrackKind,
        configs: Vec<TrackConfig>,
        frames: Vec<(usize, CodedFrame)>,
        visible: bool,
    ) {
        let mut inner = self.lock();
        inner.tracks[slot(kind)] = Some(Track {
            configs,
            frames: frames.into_iter().map(|(c, f)| (c, f, false)).collect(),
            cursor: 0,
            last_config: None,
            discont: false,
            visible,
        });
    }

    fn note_caller(&self) {
        let name = std::thread::current()
            .name()
            .unwrap_or("<unnamed>")
            .to_owned();
        self.callers.lock().unwrap().insert(name);
    }

    /// The names of the threads that have called `tracks`, `all_initialized`, `duration`, `seek` or `pull`.
    pub fn callers(&self) -> Vec<String> {
        self.callers.lock().unwrap().iter().cloned().collect()
    }

    pub fn set_wake_in_calls(&self, on: bool) {
        self.wake_in_calls
            .store(on, std::sync::atomic::Ordering::SeqCst);
    }

    fn wake_from_call(&self) {
        if self.wake_in_calls.load(std::sync::atomic::Ordering::SeqCst)
            && let Some(wake) = self.wake.lock().unwrap().clone()
        {
            wake();
        }
    }

    pub fn set_wake_under_lock(&self, on: bool) {
        self.lock().wake_under_lock = on;
    }

    pub fn set_initialized(&self, on: bool) {
        let mut inner = self.lock();
        inner.initialized = on;
        self.finish(inner);
    }

    pub fn show_track(&self, kind: TrackKind) {
        let mut inner = self.lock();
        if let Some(track) = inner.tracks[slot(kind)].as_mut() {
            track.visible = true;
        }
        self.finish(inner);
    }

    /// Makes the track visible and serves it from the random access point at or before `position`, as
    /// the real feed does for a track whose initialization segment arrived late: its first frames start
    /// at the current position, not at zero.
    pub fn show_track_from(&self, kind: TrackKind, position: Ns) {
        let mut inner = self.lock();
        if let Some(track) = inner.tracks[slot(kind)].as_mut() {
            track.visible = true;
            let covering = track
                .frames
                .iter()
                .position(|(_, f, _)| f.pts + f.duration > position)
                .unwrap_or(track.frames.len().saturating_sub(1));
            track.cursor = track.frames[..=covering]
                .iter()
                .rposition(|(_, f, _)| f.keyframe)
                .unwrap_or(0);
            track.discont = true;
        }
        self.finish(inner);
    }

    pub fn set_duration(&self, duration: Option<Ns>) {
        let mut inner = self.lock();
        inner.duration = duration;
        self.finish(inner);
    }

    /// Delivers the frames of `kind` that start in `[from, to)`.
    pub fn release(&self, kind: TrackKind, from: Ns, to: Ns) {
        let mut inner = self.lock();
        if let Some(track) = inner.tracks[slot(kind)].as_mut() {
            for (_, frame, available) in track.frames.iter_mut() {
                if frame.pts >= from && frame.pts < to {
                    *available = true;
                }
            }
        }
        self.finish(inner);
    }

    /// Delivers every frame of every track from the start up to `to`.
    pub fn release_until(&self, to: Ns) {
        for kind in TrackKind::ALL {
            self.release(kind, Ns::MIN, to);
        }
    }

    pub fn release_all(&self) {
        self.release_until(Ns::MAX);
    }

    /// `endOfStream()`: nothing more will come.
    pub fn end(&self) {
        let mut inner = self.lock();
        inner.ended = true;
        self.finish(inner);
    }

    /// Replaces the frames of `kind` that start at or after `from` with `frames` (a rendition switch by
    /// overwrite), all delivered, with a new configuration. A cursor that was past `from` moves to the
    /// first new frame, as the real cursor would (it never points at a replaced frame).
    pub fn replace_from(
        &self,
        kind: TrackKind,
        from: Ns,
        config: TrackConfig,
        frames: Vec<CodedFrame>,
    ) {
        let mut inner = self.lock();
        let track = inner.tracks[slot(kind)].as_mut().expect("a track");
        let keep = track
            .frames
            .iter()
            .take_while(|(_, f, _)| f.pts < from)
            .count();
        let config_index = track.configs.len();
        track.configs.push(config);
        track.frames.truncate(keep);
        track
            .frames
            .extend(frames.into_iter().map(|f| (config_index, f, true)));
        if track.cursor > keep {
            track.cursor = keep;
            track.discont = true;
        }
        self.finish(inner);
    }

    /// The positions `seek` was called with.
    pub fn seeks(&self) -> Vec<Ns> {
        self.lock().seeks.clone()
    }

    pub fn pulls(&self) -> u64 {
        self.lock().pulls
    }

    /// The index of the next frame `pull` will serve.
    pub fn cursor(&self, kind: TrackKind) -> usize {
        self.lock().tracks[slot(kind)]
            .as_ref()
            .map_or(0, |t| t.cursor)
    }

    fn resolve_pending(inner: &mut Inner) {
        let Some(target) = inner.pending_seek else {
            return;
        };
        let mut resolved_all = true;
        for track in inner.tracks.iter_mut().flatten() {
            // The frame that covers the target must have arrived, with its random access point.
            let covering = track
                .frames
                .iter()
                .position(|(_, f, _)| f.pts + f.duration > target)
                .unwrap_or(track.frames.len().saturating_sub(1));
            let arrived = track.frames.get(covering).is_some_and(|(_, _, a)| *a);
            if !arrived {
                resolved_all = false;
                continue;
            }
            let rap = track.frames[..=covering]
                .iter()
                .rposition(|(_, f, a)| f.keyframe && *a)
                .unwrap_or(0);
            track.cursor = rap;
            track.last_config = None;
            track.discont = true;
        }
        if resolved_all {
            inner.pending_seek = None;
        }
    }
}

impl FrameFeed for FakeFeed {
    fn tracks(&self) -> Vec<TrackConfig> {
        self.note_caller();
        let inner = self.lock();
        TrackKind::ALL
            .iter()
            .filter_map(|&kind| inner.tracks[slot(kind)].as_ref())
            .filter(|track| track.visible)
            .map(|track| {
                track.configs[track.last_config.unwrap_or(0).min(track.configs.len() - 1)].clone()
            })
            .collect()
    }

    fn all_initialized(&self) -> bool {
        self.note_caller();
        self.lock().initialized
    }

    fn duration(&self) -> Option<Ns> {
        self.note_caller();
        self.lock().duration
    }

    fn seek(&self, position: Ns) {
        self.note_caller();
        let mut inner = self.lock();
        inner.seeks.push(position);
        inner.pending_seek = Some(position);
        Self::resolve_pending(&mut inner);
        self.wake_from_call();
    }

    fn pull(&self, kind: TrackKind, max_frames: usize) -> FeedBatch {
        self.note_caller();
        let mut inner = self.lock();
        inner.pulls += 1;
        self.wake_from_call();
        Self::resolve_pending(&mut inner);
        let ended = inner.ended;
        let waiting_for_seek = inner.pending_seek.is_some();
        let Some(track) = inner.tracks[slot(kind)].as_mut() else {
            return FeedBatch {
                items: Vec::new(),
                state: FeedState::WaitingForData,
            };
        };
        let mut items = Vec::new();
        let mut frames = 0;
        // A seek whose target has not arrived serves nothing.
        while !waiting_for_seek && frames < max_frames && track.cursor < track.frames.len() {
            let (config, frame, available) = &track.frames[track.cursor];
            if !available {
                break;
            }
            if track.last_config != Some(*config) {
                items.push(FeedItem::Config(track.configs[*config].clone()));
                track.last_config = Some(*config);
            }
            items.push(FeedItem::Frame {
                frame: frame.clone(),
                discont: std::mem::take(&mut track.discont),
            });
            track.cursor += 1;
            frames += 1;
        }
        let more =
            !waiting_for_seek && track.cursor < track.frames.len() && track.frames[track.cursor].2;
        let state = if more {
            FeedState::More
        } else if !waiting_for_seek && track.cursor >= track.frames.len() && ended {
            FeedState::Ended
        } else {
            FeedState::WaitingForData
        };
        FeedBatch { items, state }
    }

    fn set_wake(&self, wake: Arc<dyn Fn() + Send + Sync>) {
        *self.wake.lock().unwrap() = Some(wake);
    }
}
