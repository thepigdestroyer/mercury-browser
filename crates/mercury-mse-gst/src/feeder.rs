// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! The feeder: the one thread of an `MseSrc` that pulls from the [`FrameFeed`] and pushes into the
//! appsrc of each track.
//!
//! It sleeps on a condition variable, never on a timer, and wakes for: the backend's `wake` closure
//! (new data, a new configuration, the end of the stream), an appsrc's `need-data` (after it said
//! `enough-data`, or after a seek), a seek, and the request to stop. It pulls a batch of frames of the
//! kind that needs data, turns it into buffers, and pushes them without blocking (the appsrc's `block`
//! is off; it tells us to stop with `enough-data`). `WaitingForData` parks the track until the next
//! wake; `Ended` queues the end of stream after the last frame.
//!
//! The thread is joined when the element is disposed (`element.rs`); nothing in here sleeps, so the
//! join is immediate.

use std::sync::Arc;

use gstreamer as gst;
use gstreamer::prelude::*;
use gstreamer_app as gst_app;
use gstreamer_audio::AudioClippingMeta;
use mercury_mse::{CodecConfig, FeedItem, FeedState, FrameFeed, TrackConfig, TrackKind};

use crate::buffers::buffer_for;
use crate::caps::{caps_for, opus_packet_samples, opus_pre_skip};
use crate::element::MseSrc;
use crate::shared::{Shared, State, index};

/// Frames asked of the feed in one pull, by kind.
const VIDEO_BATCH: usize = 8;
const AUDIO_BATCH: usize = 48;
/// Pulls in a row that returned no item and said `More`: a feed that does this is treated as waiting,
/// so a faulty feed cannot make the thread spin.
const EMPTY_MORE_LIMIT: u32 = 3;

/// One track's appsrc and what the feeder keeps with it.
pub(crate) struct TrackOut {
    pub kind: TrackKind,
    pub appsrc: gst_app::AppSrc,
    /// A configuration change seen in the feed, applied to the appsrc just before the next frame.
    pub pending_caps: Option<gst::Caps>,
    /// The `OpusHead` the decoder was last configured with (Opus tracks only).
    opus_head: Option<Vec<u8>>,
    /// Samples (48 kHz) still to be clipped off the start of the next frames (decision D18): set to the
    /// head's pre-skip when the decoder is new, that is at the announcement and when the head changes, and
    /// reduced by each frame pushed with a clip, so a pre-skip longer than one packet (3,840 samples are
    /// 80 ms, four packets of 20 ms) is taken off as many frames as it covers. After a flushing seek the
    /// same head is not new, so the decoder's pre-roll there is not clipped again.
    pre_skip: u32,
}

impl TrackOut {
    /// A track that starts with `config` (the one its appsrc has as caps).
    pub(crate) fn new(kind: TrackKind, appsrc: gst_app::AppSrc, config: &TrackConfig) -> Self {
        let mut track = TrackOut {
            kind,
            appsrc,
            pending_caps: None,
            opus_head: None,
            pre_skip: 0,
        };
        track.note_config(config);
        track
    }

    /// A configuration the feed handed over: a new `OpusHead` is a new decoder, whose first samples are
    /// the pre-skip. The same head again (the feed repeats it after a seek) is not.
    fn note_config(&mut self, config: &TrackConfig) {
        if let CodecConfig::Opus { head } = &config.codec
            && self.opus_head.as_deref() != Some(head.as_slice())
        {
            self.opus_head = Some(head.clone());
            self.pre_skip = opus_pre_skip(head).unwrap_or(0);
        }
    }

    /// The samples the buffer of this packet is to have clipped off its start: the packet's own length
    /// at most (a longer rest goes on to the next packets). A packet whose length cannot be read takes
    /// all that is left, as the first frame did before the count was carried over.
    fn clip_for(&self, packet: &[u8]) -> u32 {
        match opus_packet_samples(packet) {
            Some(samples) => self.pre_skip.min(samples),
            None => self.pre_skip,
        }
    }

    /// The packet was pushed with `clip` samples clipped off: they are no longer owed.
    fn clipped(&mut self, clip: u32) {
        self.pre_skip = self.pre_skip.saturating_sub(clip);
    }
}

/// What `wait_for_work` found.
struct Work {
    kick: bool,
    seek: Option<u64>,
}

pub(crate) struct Feeder {
    shared: Arc<Shared>,
    feed: Arc<dyn FrameFeed>,
    element: glib::WeakRef<MseSrc>,
    tracks: Vec<TrackOut>,
    announced: bool,
    duration: Option<u64>,
    /// Kinds whose late track could not be added (unusable configuration): not tried again.
    refused: Vec<TrackKind>,
}

impl Feeder {
    pub(crate) fn new(
        shared: Arc<Shared>,
        feed: Arc<dyn FrameFeed>,
        element: glib::WeakRef<MseSrc>,
    ) -> Self {
        Feeder {
            shared,
            feed,
            element,
            tracks: Vec::new(),
            announced: false,
            duration: None,
            refused: Vec::new(),
        }
    }

    /// The thread's body: returns when the element asks it to stop.
    pub(crate) fn run(mut self) {
        let _ = self.shared.feeder_thread.set(std::thread::current().id());
        while let Some(work) = self.wait_for_work() {
            if work.kick {
                if !self.announced {
                    self.try_announce();
                } else {
                    self.sync_tracks();
                }
                self.sync_duration();
            }
            if let Some(position) = work.seek {
                // The one call per seek: both tracks' seek-data callbacks recorded the same position.
                self.feed.seek(i64::try_from(position).unwrap_or(i64::MAX));
            }
            for track in 0..self.tracks.len() {
                self.pump(track);
            }
        }
        log::debug!("mercury-mse-gst: feeder thread ends");
    }

    fn any_runnable(&self, state: &State) -> bool {
        self.tracks.iter().any(|track| {
            let flags = &state.tracks[index(track.kind)];
            flags.present
                && flags.need
                && !flags.flushing
                && !flags.ended
                && flags.waiting_seq != Some(state.wake_seq)
        })
    }

    /// Blocks until there is something to do; `None` when asked to stop.
    fn wait_for_work(&self) -> Option<Work> {
        let mut state = self.shared.lock();
        loop {
            if state.stop {
                return None;
            }
            if state.seek.is_some() || state.kick || self.any_runnable(&state) {
                return Some(Work {
                    kick: std::mem::take(&mut state.kick),
                    seek: state.seek.take(),
                });
            }
            state = self
                .shared
                .cond
                .wait(state)
                .unwrap_or_else(std::sync::PoisonError::into_inner);
        }
    }

    /// Announces the streams once the feed knows its tracks.
    fn try_announce(&mut self) {
        if !self.feed.all_initialized() {
            return;
        }
        let configs = self.feed.tracks();
        if configs.is_empty() {
            return;
        }
        let Some(element) = self.element.upgrade() else {
            return;
        };
        let duration = self.read_duration();
        // Whatever the outcome, the feed's tracks are announced once: a retry would add the pads twice.
        self.announced = true;
        match element.announce(&self.shared, &configs, duration) {
            Ok(tracks) => {
                self.duration = duration;
                self.shared.set_duration(duration);
                self.tracks = tracks;
            }
            Err(message) => element.fail(&message),
        }
    }

    /// Adds the tracks the feed has gained since the announcement (decision D17): a source buffer that
    /// was added or initialized after the streams were announced. A kind that is already announced is
    /// not a new track (a quality switch changes its configuration, which `pump` applies).
    fn sync_tracks(&mut self) {
        if self.tracks.is_empty() {
            return;
        }
        for config in self.feed.tracks() {
            let kind = config.kind;
            if kind == TrackKind::Text
                || self.refused.contains(&kind)
                || self.tracks.iter().any(|track| track.kind == kind)
            {
                continue;
            }
            let Some(element) = self.element.upgrade() else {
                return;
            };
            match element.add_track(&self.shared, &config, self.duration) {
                Ok(Some(track)) => self.tracks.push(track),
                Ok(None) => {}
                Err(message) => {
                    // The tracks that play go on: a late track that cannot is only left out.
                    log::warn!(
                        "mercury-mse-gst: the late {} track is not played: {message}",
                        kind.name()
                    );
                    self.refused.push(kind);
                }
            }
        }
    }

    fn read_duration(&self) -> Option<u64> {
        // Infinity and absurd values mean "no duration" for GStreamer.
        self.feed
            .duration()
            .and_then(|ns| u64::try_from(ns).ok())
            .filter(|&ns| ns < u64::MAX / 4)
    }

    /// Gives the appsrcs and the queries the feed's duration when it changed.
    fn sync_duration(&mut self) {
        let duration = self.read_duration();
        if duration == self.duration {
            return;
        }
        self.duration = duration;
        self.shared.set_duration(duration);
        for track in &self.tracks {
            track
                .appsrc
                .set_duration(duration.map(gst::ClockTime::from_nseconds));
        }
    }

    /// Feeds one track for as long as its appsrc wants data and the feed has some.
    fn pump(&mut self, track: usize) {
        let kind = self.tracks[track].kind;
        let appsrc = self.tracks[track].appsrc.clone();
        let batch_size = match kind {
            TrackKind::Video | TrackKind::Text => VIDEO_BATCH,
            TrackKind::Audio => AUDIO_BATCH,
        };
        let slot = index(kind);
        let mut empty_streak = 0u32;
        loop {
            let epoch = self.shared.epoch();
            let seq = {
                let state = self.shared.lock();
                let flags = &state.tracks[slot];
                if state.stop
                    || state.seek.is_some()
                    || !flags.need
                    || flags.flushing
                    || flags.ended
                    || flags.waiting_seq == Some(state.wake_seq)
                {
                    return;
                }
                state.wake_seq
            };

            let batch = self.feed.pull(kind, batch_size);
            let item_count = batch.items.len();
            let mut stale = false;
            let mut refused = false;
            for item in batch.items {
                match item {
                    FeedItem::Config(config) => match caps_for(&config) {
                        Ok(caps) => {
                            self.tracks[track].pending_caps = Some(caps);
                            self.tracks[track].note_config(&config);
                        }
                        Err(message) => {
                            if let Some(element) = self.element.upgrade() {
                                element.fail(&message);
                            }
                        }
                    },
                    FeedItem::Frame { frame, discont } => {
                        let mut buffer = buffer_for(&frame, discont);
                        let clip = self.tracks[track].clip_for(&frame.data);
                        if clip > 0 {
                            let start = u64::from(clip).default_format();
                            let end = 0u64.default_format();
                            AudioClippingMeta::add(
                                buffer.get_mut().expect("a new buffer is writable"),
                                start,
                                end,
                            );
                        }
                        let _push = self.shared.push_locks[slot]
                            .lock()
                            .unwrap_or_else(std::sync::PoisonError::into_inner);
                        if epoch != self.shared.epoch() {
                            // A seek began after the pull: this frame is from before it.
                            stale = true;
                            break;
                        }
                        if let Some(caps) = self.tracks[track].pending_caps.take() {
                            appsrc.set_caps(Some(&caps));
                        }
                        match appsrc.push_buffer(buffer) {
                            Ok(_) => self.tracks[track].clipped(clip),
                            Err(error) => {
                                log::debug!(
                                    "mercury-mse-gst: push on the {} track refused: {error:?}",
                                    kind.name()
                                );
                                refused = true;
                                break;
                            }
                        }
                    }
                }
            }
            if stale {
                // Back to the top: the seek's position is waiting for `wait_for_work`. A configuration seen
                // before the seek is not the one the pipeline needs after it (the feed says it again).
                self.tracks[track].pending_caps = None;
                return;
            }
            if refused {
                // The appsrc is flushing or at its end (a state change): it says need-data again when
                // it runs.
                self.shared.lock().tracks[slot].need = false;
                return;
            }

            match batch.state {
                FeedState::More => {
                    if item_count == 0 {
                        empty_streak += 1;
                        if empty_streak >= EMPTY_MORE_LIMIT {
                            self.shared.lock().tracks[slot].waiting_seq = Some(seq);
                            return;
                        }
                    } else {
                        empty_streak = 0;
                    }
                }
                FeedState::WaitingForData => {
                    // If a wake came in since the pull, the counter moved and the track stays runnable.
                    self.shared.lock().tracks[slot].waiting_seq = Some(seq);
                    return;
                }
                FeedState::Ended => {
                    let _push = self.shared.push_locks[slot]
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner);
                    if epoch == self.shared.epoch() {
                        // After the last queued buffer: the appsrc drains its queue first.
                        let _ = appsrc.end_of_stream();
                        self.shared.lock().tracks[slot].ended = true;
                    }
                    return;
                }
            }
        }
    }
}
