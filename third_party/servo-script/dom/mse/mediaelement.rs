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
use servo_media::player::mse::media_state::{
    MediaStateAction, WaitingInputs, forward_position, media_state_action, waiting_fires,
};
use servo_media::player::mse::stall_controller::{
    ElementSnapshot, PipelineChange, highest_buffered_end,
};
use servo_media::player::mse::{
    ElementReadyState, EosError, InitInfo, MediaSourceReadyState as BackendReadyState, TrackKind,
};

use super::*;

/// The element's `readyState` in the contract's type (the same five names).
fn element_ready_state(state: ReadyState) -> ElementReadyState {
    match state {
        ReadyState::HaveNothing => ElementReadyState::HaveNothing,
        ReadyState::HaveMetadata => ElementReadyState::HaveMetadata,
        ReadyState::HaveCurrentData => ElementReadyState::HaveCurrentData,
        ReadyState::HaveFutureData => ElementReadyState::HaveFutureData,
        ReadyState::HaveEnoughData => ElementReadyState::HaveEnoughData,
    }
}

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

    /// The backend learns every playback position (coded frame eviction, readiness), except while a
    /// seek is pending: the player still reports the position from before the seek, and the backend
    /// already knows the target (`media_source_seeking`). A stale position would make the data around
    /// the target of a backward seek look old enough to evict (review finding 2).
    pub(super) fn media_source_position_changed(&self, position: f64) {
        if let Some(media_source) = self.media_source.get() &&
            let Some(position) = forward_position(self.seeking.get(), position)
        {
            media_source.backend().set_position(position);
        }
    }

    /// What `update_media_state` does to the player (`media_state_action` of `mercury-mse`). The
    /// MediaSource rule (a playing element that may play on is left alone, the guard's pause is not
    /// undone) applies only while a MediaSource is attached; a plain `<video>` or `<audio>` keeps the
    /// published behaviour until F6 of `sdd/deferred-hands-on.md` (a plain media window check) has
    /// passed.
    pub(super) fn media_state_decision(&self, is_playing: bool) -> MediaStateAction {
        media_state_action(
            self.is_potentially_playing(),
            is_playing,
            self.media_source_stalled(),
            self.media_source.get().is_some(),
        )
    }

    /// The `waiting` condition of the "ready state change" steps (`waiting_fires` of `mercury-mse`),
    /// for every media element: Servo 0.8 tested `!error.is_none()` where the specification says there
    /// is no error.
    pub(super) fn media_waiting_fires(
        &self,
        old: ReadyState,
        new: ReadyState,
        was_potentially_playing: bool,
    ) -> bool {
        waiting_fires(&WaitingInputs {
            old: element_ready_state(old),
            new: element_ready_state(new),
            was_potentially_playing,
            ended_playback: self.ended_playback(LoopCondition::Included),
            has_error: self.error.get().is_some(),
            paused_for_user_interaction: self.is_paused_for_user_interaction(),
            paused_for_in_band_content: self.is_paused_for_in_band_content(),
        })
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
            .is_some_and(|media_source| media_source.stall().borrow().controller.is_stalled())
    }

    /// The underrun decision (lane med-1b, decision D4): hands the element's raw facts to the
    /// `StallController` of `mercury-mse`, which builds the guard's input and answers what to do. The
    /// element keeps only the player calls (`update_media_state` pauses the pipeline when the stall
    /// begins and resumes it when it ends) and the re-check timer. The `waiting` and `playing` events
    /// come from the readiness change that goes with it (`change_ready_state`); the page's `paused` is
    /// not touched. Called after every readiness update: position updates (every 100 ms for a
    /// MediaSource player), appends, removals, seeks, end of stream.
    fn media_source_check_stall(&self) {
        let Some(media_source) = self.media_source.get() else {
            return;
        };
        let backend = media_source.backend();
        let position = self.current_playback_position.get();
        // Every fact is read before the controller is borrowed: nothing below calls into the element
        // while the borrow is held.
        let snapshot = ElementSnapshot {
            position,
            backend_ready: backend.element_ready_state(position),
            element_ready: element_ready_state(self.ready_state.get()),
            paused: self.paused.get(),
            has_error: self.error.get().is_some(),
            network_empty: self.network_state.get() == NetworkState::Empty,
            ended_playback: self.ended_playback(LoopCondition::Included),
            seeking: self.seeking.get(),
            source_ended: backend.ready_state() == BackendReadyState::Ended,
            highest_buffered_end: highest_buffered_end(&backend.element_buffered()),
            now_ms: media_source.stall().borrow().now_ms(),
        };
        let step = media_source.stall().borrow_mut().controller.step(snapshot);

        if step.pipeline != PipelineChange::Keep {
            // Pauses the pipeline during a stall and resumes it when the stall is over (a seek or a
            // page pause ends it without an action).
            self.update_media_state();
        }
        if let Some(delay_ms) = step.recheck_in_ms {
            self.schedule_media_source_stall_recheck(delay_ms);
        }
    }

    /// Looks at the stall again once the guard's hysteresis has passed (`delay_ms` is the controller's
    /// answer: the rest of the hysteresis plus a margin).
    fn schedule_media_source_stall_recheck(&self, delay_ms: u64) {
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
                std::thread::sleep(std::time::Duration::from_millis(delay_ms));
                task_source.queue(task!(media_source_stall_recheck: move |cx| {
                    let this = this.root();
                    if generation_id != this.generation_id.get() {
                        return;
                    }
                    if let Some(media_source) = this.media_source.get() {
                        media_source.stall().borrow_mut().controller.recheck_fired();
                    }
                    this.media_source_update_ready_state(cx);
                }));
            });
        if spawned.is_err() {
            warn!("Could not start the MediaSource stall re-check thread");
            if let Some(media_source) = self.media_source.get() {
                media_source.stall().borrow_mut().controller.recheck_fired();
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
