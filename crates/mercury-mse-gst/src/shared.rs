// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! The state the feeder thread, the appsrc callbacks and the seek handler of one `MseSrc` share.
//!
//! The rules that keep it deadlock-free (brief risk 1, R17 section 11.2):
//!
//! * No callback of GStreamer and no closure the backend calls (`wake`) ever calls into the
//!   [`FrameFeed`](mercury_mse::FrameFeed): they only record what happened here and notify the feeder
//!   thread, which is the one place the feed is pulled from. The backend may hold its own lock while it
//!   calls `wake`, so `wake` must not wait for anything but this module's short mutex.
//! * This module's lock is never held across a call into the feed, into GStreamer, or while taking a push
//!   lock. The order where two are held is: push locks (in kind order), then the state lock.
//! * A **seek epoch** (an atomic counter) is bumped, with every push lock held, when a seek begins. The
//!   feeder checks it under the push lock of a track before each `push_buffer`, so a frame it pulled
//!   before the seek can never reach the appsrc after the seek's flush.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};

use gstreamer as gst;
use mercury_mse::TrackKind;

/// Video, audio, text: the index of a [`TrackKind`] in the arrays below.
pub(crate) const KINDS: usize = 3;

pub(crate) fn index(kind: TrackKind) -> usize {
    match kind {
        TrackKind::Video => 0,
        TrackKind::Audio => 1,
        TrackKind::Text => 2,
    }
}

/// What the callbacks of one track's appsrc have told the feeder.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct TrackFlags {
    /// The track has an appsrc (announced).
    pub present: bool,
    /// The appsrc has asked for data at least once: from then on a `seek-data` is a seek and not the one
    /// every appsrc runs at start-up.
    pub ever_needed: bool,
    /// The appsrc asked for data (`need-data`) and has not said `enough-data` since.
    pub need: bool,
    /// A seek began and the appsrc has not restarted its streaming thread since: nothing may be pushed.
    pub flushing: bool,
    /// The seek-data callback of the current seek has run.
    pub seek_data_seen: bool,
    /// `end_of_stream` was called on the appsrc; cleared by a seek.
    pub ended: bool,
    /// The feed said `WaitingForData` when the wake counter was this; runnable again when it moved.
    pub waiting_seq: Option<u64>,
}

#[derive(Debug)]
pub(crate) struct State {
    pub stop: bool,
    /// The feeder must look again at the tracks and the duration (a wake, an attach).
    pub kick: bool,
    /// Counts `wake` calls.
    pub wake_seq: u64,
    pub tracks: [TrackFlags; KINDS],
    /// A seek position (ns) the feeder has not handed to the feed yet.
    pub seek: Option<u64>,
    /// The epoch whose position `seek` recorded: the first seek-data callback of an epoch wins, the
    /// second appsrc's one (same position) is ignored, so the feed is seeked once per seek.
    pub seek_epoch: Option<u64>,
    pub last_seek_seqnum: Option<gst::Seqnum>,
    /// The feed's duration in ns, for the duration and seeking queries.
    pub duration: Option<u64>,
}

pub(crate) struct Shared {
    /// The feeder thread, once it runs: a wake that comes from it (a feed that wakes from inside `pull`
    /// or `seek`) reports nothing the call itself does not already return, and is ignored.
    pub feeder_thread: std::sync::OnceLock<std::thread::ThreadId>,
    pub state: Mutex<State>,
    pub cond: Condvar,
    pub epoch: AtomicU64,
    pub push_locks: [Mutex<()>; KINDS],
}

impl Shared {
    pub(crate) fn new() -> Arc<Self> {
        Arc::new(Shared {
            feeder_thread: std::sync::OnceLock::new(),
            state: Mutex::new(State {
                stop: false,
                kick: true,
                wake_seq: 0,
                tracks: [TrackFlags::default(); KINDS],
                seek: None,
                seek_epoch: None,
                last_seek_seqnum: None,
                duration: None,
            }),
            cond: Condvar::new(),
            epoch: AtomicU64::new(0),
            push_locks: [Mutex::new(()), Mutex::new(()), Mutex::new(())],
        })
    }

    pub(crate) fn lock(&self) -> MutexGuard<'_, State> {
        // A panic while the lock was held leaves plain flags behind: carry on with them.
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    pub(crate) fn epoch(&self) -> u64 {
        self.epoch.load(Ordering::Acquire)
    }

    /// The backend's wake closure: new frames, a configuration or the end of the stream may be
    /// available. Called from any thread, possibly under the backend's own lock: records and notifies,
    /// nothing else.
    pub(crate) fn wake(&self) {
        if self.feeder_thread.get() == Some(&std::thread::current().id()) {
            return;
        }
        let mut state = self.lock();
        state.wake_seq = state.wake_seq.wrapping_add(1);
        state.kick = true;
        drop(state);
        self.cond.notify_all();
    }

    /// Asks the feeder thread to end. Does not wait for it.
    pub(crate) fn request_stop(&self) {
        let mut state = self.lock();
        state.stop = true;
        drop(state);
        self.cond.notify_all();
    }

    pub(crate) fn mark_present(&self, kind: TrackKind) {
        self.lock().tracks[index(kind)].present = true;
    }

    pub(crate) fn duration(&self) -> Option<u64> {
        self.lock().duration
    }

    pub(crate) fn set_duration(&self, duration: Option<u64>) {
        self.lock().duration = duration;
    }

    /// The appsrc's `need-data` callback. While a seek is flushing the track, one that comes before the
    /// seek-data callback is a leftover of the old streaming thread and is ignored; the first one after it
    /// is the restart of the streaming thread and ends the flushing state.
    pub(crate) fn on_need_data(&self, kind: TrackKind) {
        let mut state = self.lock();
        let track = &mut state.tracks[index(kind)];
        if track.flushing {
            if !track.seek_data_seen {
                return;
            }
            track.flushing = false;
            track.seek_data_seen = false;
        }
        track.need = true;
        track.ever_needed = true;
        drop(state);
        self.cond.notify_all();
    }

    pub(crate) fn on_enough_data(&self, kind: TrackKind) {
        self.lock().tracks[index(kind)].need = false;
    }

    /// The appsrc's `seek-data` callback, run inside the appsrc's seek (stream lock held, flush start
    /// sent, queue not yet flushed): records the position once per seek and notes this track did its part.
    pub(crate) fn on_seek_data(&self, kind: TrackKind, position: u64) {
        // A seek that did not come through `begin_seek` (an event sent to the appsrc directly): treat it
        // as one, so the stale frames are cut off the same way.
        let (unannounced, startup) = {
            let state = self.lock();
            let track = &state.tracks[index(kind)];
            (
                track.present && !track.flushing && track.ever_needed,
                track.present && !track.flushing && !track.ever_needed,
            )
        };
        if startup {
            // Every appsrc runs a seek to its start position when it starts, before it has asked for
            // anything: there is nothing to flush and nothing to tell the feed (its cursors start at
            // zero). Taking it for a seek would reset the other track, which may be past it already.
            return;
        }
        if unannounced {
            self.begin_seek_inner();
        }
        let epoch = self.epoch();
        let mut state = self.lock();
        state.tracks[index(kind)].seek_data_seen = true;
        if state.seek_epoch != Some(epoch) {
            state.seek_epoch = Some(epoch);
            state.seek = Some(position);
        }
        drop(state);
        self.cond.notify_all();
    }

    /// A seek event reached the element. `false` when the same seek (same sequence number) was already
    /// handled through another pad: the caller then does nothing. Otherwise every push is stopped and
    /// the epoch bumped, and the caller sends the event to every appsrc.
    pub(crate) fn begin_seek(&self, seqnum: gst::Seqnum) -> bool {
        {
            let mut state = self.lock();
            if state.last_seek_seqnum == Some(seqnum) {
                return false;
            }
            state.last_seek_seqnum = Some(seqnum);
        }
        self.begin_seek_inner();
        true
    }

    fn begin_seek_inner(&self) {
        // Every push lock, in kind order: a push in flight finishes first, and the next one sees the
        // new epoch.
        let guards: Vec<MutexGuard<'_, ()>> = self
            .push_locks
            .iter()
            .map(|lock| {
                lock.lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
            })
            .collect();
        self.epoch.fetch_add(1, Ordering::AcqRel);
        {
            let mut state = self.lock();
            for track in state.tracks.iter_mut().filter(|track| track.present) {
                track.flushing = true;
                track.need = false;
                track.seek_data_seen = false;
                track.ended = false;
                track.waiting_seq = None;
            }
        }
        drop(guards);
    }

    /// The seek could not be delivered to an appsrc: stop waiting for a restart that will not come.
    pub(crate) fn abort_seek(&self) {
        let mut state = self.lock();
        for track in state.tracks.iter_mut() {
            track.flushing = false;
            track.seek_data_seen = false;
        }
        drop(state);
        self.cond.notify_all();
    }
}
