// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

// Mercury: (MED-1) the media element's side of Media Source Extensions: attach and detach, the
// readiness from the backend, the metadata and duration steps, the end of stream and error steps, and
// the MediaSource's seeking algorithm. Written by Mercury; it is a new file so that the upstream
// `htmlmediaelement.rs` keeps only one-line hooks (the `mod` line, the calls, and the `media_source`
// field), which a Servo bump rebases in minutes.
//
// This file is declared as a child module of `htmlmediaelement.rs` (`#[path = "../../mse/mediaelement.rs"]`)
// and not in `dom/mse/mod.rs`: a child module sees the element's private fields and methods, which
// these functions read and set, so no upstream item had to change its visibility.

// Mercury (MED-1): the MSE contract types the element reads from the backend.
use servo_media::player::mse::{
    ElementReadyState, EosError, InitInfo, MediaSourceReadyState as BackendReadyState, StallAction,
    StallInput, TrackKind,
};

use super::*;

/// How long after a stall check that wanted to change state and could not (the guard keeps 250 ms
/// between two transitions) the element looks again.
const STALL_RECHECK_MS: u64 = 260;

impl HTMLMediaElement {
    //
    // While a MediaSource is attached (`media_source`) the element asks its backend for the buffered
    // and seekable ranges and for the readiness around the playback position, takes the media duration
    // from the MediaSource, and ignores the media engine's own duration and metadata. The player pulls
    // frames from the backend's feed; nothing is ever pushed to it. The hooks in the upstream functions
    // are marked `Mercury (MED-1)`; `third_party/servo-script/MERCURY.md` lists them.

    /// The MediaSource a resource names, if any: a media source object URL or `srcObject`.
    pub(super) fn media_source_for_resource(&self, resource: &Resource) -> Option<DomRoot<MediaSource>> {
        match resource {
            Resource::MediaSource(media_source) => Some(media_source.clone()),
            Resource::Object => match *self.src_object.borrow() {
                Some(SrcObject::MediaSource(ref media_source)) => {
                    Some(DomRoot::from_ref(&**media_source))
                },
                _ => None,
            },
            Resource::Url(_) => None,
        }
    }

    /// The attach steps set the delaying-the-load-event flag to false.
    /// <https://w3c.github.io/media-source/#dfn-attaching-to-a-media-element>
    pub(super) fn queue_media_source_stop_delaying_the_load_event(&self) {
        let this = Trusted::new(self);
        let generation_id = self.generation_id.get();

        self.owner_global()
            .task_manager()
            .media_element_task_source()
            .queue(task!(media_source_stop_delaying_the_load_event: move |cx| {
                let this = this.root();
                if generation_id != this.generation_id.get() {
                    return;
                }

                this.delay_load_event(cx, false);
            }));
    }

    /// <https://w3c.github.io/media-source/#dfn-detaching-from-a-media-element>
    pub(super) fn detach_media_source(&self) {
        if let Some(media_source) = self.media_source.take() {
            media_source.detach();
        }
    }

    /// `(total, dropped)` video frames of the element's player (lane med-1b, decision D5), for
    /// `getVideoPlaybackQuality()`. `None` without a player or when the backend does not count.
    pub(crate) fn player_video_playback_quality(&self) -> Option<(u64, u64)> {
        let player = self.player.borrow();
        let player = player.as_ref()?.lock().unwrap();
        player.video_playback_quality()
    }

    /// The backend learns every playback position (coded frame eviction, readiness).
    pub(super) fn media_source_position_changed(&self, position: f64) {
        if let Some(media_source) = self.media_source.get() {
            media_source.backend().set_position(position);
        }
    }

    /// The MediaSource's seeking algorithm, run by the seek algorithm once the new playback position is
    /// set: an ended MediaSource goes back to "open" (and `sourceopen` is queued) and the readiness is
    /// re-evaluated at the new position (HAVE_METADATA when it is not buffered).
    /// <https://w3c.github.io/media-source/#mediasource-seeking>
    pub(super) fn media_source_seeking(&self, time: f64) {
        let Some(media_source) = self.media_source.get() else {
            return;
        };
        media_source.backend().set_position(time);
        media_source.seeking(time);
        // The seek algorithm has no JavaScript context, so the readiness is re-evaluated in a task.
        let this = Trusted::new(self);
        let generation_id = self.generation_id.get();
        self.owner_global()
            .task_manager()
            .media_element_task_source()
            .queue(task!(media_source_seek_ready_state: move |cx| {
                let this = this.root();
                if generation_id != this.generation_id.get() {
                    return;
                }

                this.media_source_update_ready_state(cx);
            }));
    }

    /// Sets the element's readyState from what the backend says about the position (decision D13 of
    /// the lane): HAVE_METADATA once every source buffer has its initialization segment and outside
    /// the buffered ranges, HAVE_CURRENT_DATA, HAVE_FUTURE_DATA and HAVE_ENOUGH_DATA by how much is
    /// buffered ahead, HAVE_ENOUGH_DATA once the stream has ended. The events follow from
    /// `change_ready_state`. Called on every initialization segment, append, removal, eviction,
    /// position and seek. A no-op without a MediaSource.
    pub(crate) fn media_source_update_ready_state(&self, cx: &mut JSContext) {
        self.media_source_apply_ready_state(cx);
        self.media_source_check_stall();
    }

    /// Whether the underrun guard has paused the pipeline (decision D4 of lane med-1b). The upstream
    /// `update_media_state` reads it: while it is true the pipeline is not resumed, whatever the
    /// readiness says, until the guard's hysteresis has passed.
    pub(super) fn media_source_stalled(&self) -> bool {
        self.media_source
            .get()
            .is_some_and(|media_source| media_source.stall().borrow().guard.is_stalled())
    }

    /// The underrun decision (lane med-1b, decision D4): asks the `StallGuard` what to do with the
    /// position, the readiness and the state of playback, and has `update_media_state` pause or resume
    /// the pipeline when the stall begins or ends. The `waiting` and `playing` events come from the
    /// readiness change that goes with it (`change_ready_state`); the page's `paused` is not touched.
    /// Called after every readiness update: position updates (every 100 ms for a MediaSource player),
    /// appends, removals, seeks, end of stream.
    fn media_source_check_stall(&self) {
        let Some(media_source) = self.media_source.get() else {
            return;
        };
        let backend = media_source.backend();
        let position = self.current_playback_position.get();
        let ready = backend.element_ready_state(position);
        let ended = backend.ready_state() == BackendReadyState::Ended;
        let playing = !self.paused.get() &&
            self.error.get().is_none() &&
            self.network_state.get() != NetworkState::Empty &&
            self.ready_state.get() != ReadyState::HaveNothing &&
            !self.ended_playback(LoopCondition::Included);
        let seeking = self.seeking.get();

        let (action, was_stalled, is_stalled, wants_recheck) = {
            let mut state = media_source.stall().borrow_mut();
            let was_stalled = state.guard.is_stalled();
            let input = StallInput {
                position,
                ready,
                playing,
                seeking,
                ended,
                now_ms: state.now_ms(),
            };
            let action = state.guard.update(input);
            let is_stalled = state.guard.is_stalled();
            // The guard keeps a quarter of a second between two transitions: when it wants to change
            // state and refused for that reason, nothing may be left waiting for the next position
            // update (the pipeline is paused during a stall, so none comes).
            let wants_change = if is_stalled {
                ready >= ElementReadyState::HaveFutureData || ended
            } else {
                playing && !seeking && !ended && ready <= ElementReadyState::HaveCurrentData
            };
            let wants_recheck =
                action == StallAction::None && wants_change && !state.recheck_pending;
            if wants_recheck {
                state.recheck_pending = true;
            }
            (action, was_stalled, is_stalled, wants_recheck)
        };

        if action != StallAction::None || was_stalled != is_stalled {
            // Pauses the pipeline during a stall and resumes it when the stall is over (a seek ends
            // it without an action).
            self.update_media_state();
        }
        if wants_recheck {
            self.schedule_media_source_stall_recheck();
        }
    }

    /// Looks at the stall again once the guard's hysteresis has passed.
    fn schedule_media_source_stall_recheck(&self) {
        let this = Trusted::new(self);
        let generation_id = self.generation_id.get();
        let task_source = self
            .owner_global()
            .task_manager()
            .media_element_task_source()
            .to_sendable();
        let spawned = std::thread::Builder::new()
            .name("MseStallRecheck".to_owned())
            .spawn(move || {
                std::thread::sleep(std::time::Duration::from_millis(STALL_RECHECK_MS));
                task_source.queue(task!(media_source_stall_recheck: move |cx| {
                    let this = this.root();
                    if generation_id != this.generation_id.get() {
                        return;
                    }
                    if let Some(media_source) = this.media_source.get() {
                        media_source.stall().borrow_mut().recheck_pending = false;
                    }
                    this.media_source_update_ready_state(cx);
                }));
            });
        if spawned.is_err() {
            warn!("Could not start the MediaSource stall re-check thread");
            if let Some(media_source) = self.media_source.get() {
                media_source.stall().borrow_mut().recheck_pending = false;
            }
        }
    }

    fn media_source_apply_ready_state(&self, cx: &mut JSContext) {
        let Some(media_source) = self.media_source.get() else {
            return;
        };
        let current = self.ready_state.get();
        let mut target = match media_source
            .backend()
            .element_ready_state(self.current_playback_position.get())
        {
            ElementReadyState::HaveNothing => ReadyState::HaveNothing,
            ElementReadyState::HaveMetadata => ReadyState::HaveMetadata,
            ElementReadyState::HaveCurrentData => ReadyState::HaveCurrentData,
            ElementReadyState::HaveFutureData => ReadyState::HaveFutureData,
            ElementReadyState::HaveEnoughData => ReadyState::HaveEnoughData,
        };

        if current != ReadyState::HaveNothing {
            // Once the metadata is known the readyState does not go back to HAVE_NOTHING.
            if target < ReadyState::HaveMetadata {
                target = ReadyState::HaveMetadata;
            }
            // At the end of the media resource the element is at HAVE_CURRENT_DATA, as the "reaches
            // the end" steps leave it.
            if self.ended_playback(LoopCondition::Ignored) {
                target = ReadyState::HaveCurrentData;
            }
        }

        if target == current {
            return;
        }

        if current == ReadyState::HaveNothing {
            // The first transition runs the media data processing steps for the metadata, which set
            // HAVE_METADATA (and fire `loadedmetadata`), then go on to the target.
            self.media_source_metadata_received(cx);
            if target > ReadyState::HaveMetadata {
                self.change_ready_state(cx, target);
            }
            return;
        }

        self.change_ready_state(cx, target);
    }

    /// The media data processing steps "once enough of the media data has been fetched to determine
    /// the duration", for the MediaSource's initialization segments.
    /// <https://html.spec.whatwg.org/multipage/#media-data-processing-steps-list>
    pub(super) fn media_source_metadata_received(&self, cx: &mut JSContext) {
        let Some(media_source) = self.media_source.get() else {
            return;
        };

        // Step 3. Set the current playback position and the official playback position to the
        // earliest possible position.
        let earliest_possible_position = self.earliest_possible_position();
        self.current_playback_position
            .set(earliest_possible_position);
        self.official_playback_position
            .set(earliest_possible_position);

        // Step 4. Update the duration attribute with the time of the last frame of the resource (the
        // MediaSource's duration), which queues `durationchange`.
        self.duration
            .set(media_source.backend().duration().unwrap_or(f64::NAN));
        self.queue_media_element_task_to_fire_event(atom!("durationchange"));

        // Step 5. For video elements, queue `resize` (the natural dimensions were set from the
        // initialization segment).
        if self.downcast::<HTMLVideoElement>().is_some() {
            self.queue_media_element_task_to_fire_event(atom!("resize"));
        }

        // Step 6. Set the readyState attribute to HAVE_METADATA.
        self.change_ready_state(cx, ReadyState::HaveMetadata);

        // Steps 7 to 9. If the media element's default playback start position is greater than zero,
        // then seek to that time; set it to zero.
        if self.default_playback_start_position.get() > 0. {
            self.seek(
                self.default_playback_start_position.get(),
                /* approximate_for_speed */ false,
            );
        }
        self.default_playback_start_position.set(0.);
    }

    /// An initialization segment was parsed: the video's natural dimensions, then the readiness (the
    /// MediaSource's initialization segment received algorithm, step 7 onwards).
    /// <https://w3c.github.io/media-source/#dfn-initialization-segment-received>
    pub(crate) fn media_source_init_received(&self, cx: &mut JSContext, init: &InitInfo) {
        if let Some(video_element) = self.downcast::<HTMLVideoElement>() &&
            let Some(track) = init
                .tracks
                .iter()
                .find(|track| track.kind == TrackKind::Video && track.width > 0 && track.height > 0) &&
            video_element.set_natural_dimensions(
                cx.no_gc(),
                Some(track.width),
                Some(track.height),
            ) &&
            self.ready_state.get() != ReadyState::HaveNothing
        {
            self.queue_media_element_task_to_fire_event(atom!("resize"));
        }

        self.media_source_update_ready_state(cx);
    }

    /// <https://w3c.github.io/media-source/#dfn-duration-change> step 6: update the media duration
    /// and run the media element's duration change algorithm. Idempotent: the backend reports a
    /// change again by an event after the DOM called for it. Before HAVE_METADATA the media duration
    /// stays NaN (the metadata steps publish it).
    pub(crate) fn media_source_duration_changed(&self, duration: f64) {
        if self.ready_state.get() == ReadyState::HaveNothing {
            return;
        }
        let old_duration = self.duration.get();
        if old_duration == duration || (old_duration.is_nan() && duration.is_nan()) {
            return;
        }

        self.duration.set(duration);
        self.queue_media_element_task_to_fire_event(atom!("durationchange"));

        // If the duration is changed such that the current playback position ends up being greater
        // than the time of the end of the media resource, then the user agent must also seek to the
        // time of the end of the media resource.
        if self.current_playback_position.get() > duration {
            self.seek(duration, /* approximate_for_speed */ false);
        }
    }

    /// <https://w3c.github.io/media-source/#dfn-end-of-stream> step 3.2: notify the media element
    /// that it now has all of the media data. The duration may have changed with the stream's end, and
    /// an ended stream has enough data.
    pub(crate) fn media_source_end_of_stream(&self, cx: &mut JSContext) {
        if let Some(media_source) = self.media_source.get() {
            self.media_source_duration_changed(media_source.backend().duration().unwrap_or(f64::NAN));
        }
        self.media_source_update_ready_state(cx);
    }

    /// <https://w3c.github.io/media-source/#dfn-end-of-stream> steps 4 and 5: the media data
    /// processing steps for a network or a decode error (`endOfStream("network")`, `("decode")`, or a
    /// failed append).
    pub(crate) fn media_source_error(&self, cx: &mut JSContext, error: EosError) {
        if self.ready_state.get() == ReadyState::HaveNothing {
            // => "If the media data cannot be fetched at all, due to network errors, causing the
            // user agent to give up trying to fetch the resource" (and the unsupported-format case:
            // both end in the failure steps).
            self.media_data_processing_failure_steps(cx);
            return;
        }

        // => "If the connection is interrupted after some media data has been received" and "If the
        // media data is corrupted".
        let error = match error {
            EosError::Network => MEDIA_ERR_NETWORK,
            EosError::Decode => MEDIA_ERR_DECODE,
        };
        self.media_data_processing_fatal_steps(cx, error);
    }
}
