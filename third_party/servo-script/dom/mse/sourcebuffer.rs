/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

// Mercury: (MED-1) the structure, the method list and the event plumbing are carried from
// servo/servo#47269 by decomoraes (MPL-2.0). The data plane and the spec algorithms are not: they are
// Mercury's own and live in `crates/mercury-mse`. This object keeps its JavaScript-visible state (the
// `updating` flag as the specification's tasks publish it, the track lists, the last known values of the
// attributes once the buffer has been removed) and forwards everything else to
// `MediaSourceBackend`, mapping `MseError` to the DOMException the specification names.
//
// Left out on purpose (listed in `third_party/servo-script/MERCURY.md`): `textTracks` (this Servo's
// `TextTrackList` needs a media element), and the contents of `audioTracks` and `videoTracks`.

use std::cell::Cell;

use dom_struct::dom_struct;
use js::context::JSContext;
use script_bindings::reflector::reflect_dom_object;
use servo_media::player::mse::{
    AppendMode as BackendAppendMode, MediaSourceReadyState as BackendReadyState, MseError,
    SourceBufferId,
};
use stylo_atoms::Atom;

use crate::dom::audio::audiotracklist::AudioTrackList;
use crate::dom::bindings::buffer_source::get_buffer_source_copy;
use crate::dom::bindings::codegen::Bindings::SourceBufferBinding::{
    AppendMode, SourceBufferMethods,
};
use crate::dom::bindings::codegen::UnionTypes::ArrayBufferViewOrArrayBuffer;
use crate::dom::bindings::error::{Error, ErrorResult, Fallible};
use crate::dom::bindings::inheritance::Castable;
use crate::dom::bindings::num::Finite;
use crate::dom::bindings::refcounted::Trusted;
use crate::dom::bindings::reflector::DomGlobal;
use crate::dom::bindings::root::{Dom, DomRoot, MutNullableDom};
use crate::dom::bindings::str::DOMString;
use crate::dom::eventtarget::EventTarget;
use crate::dom::mse::mediasource::{MediaSource, dom_error, ranges_to_container};
use crate::dom::timeranges::TimeRanges;
use crate::dom::videotracklist::VideoTrackList;

/// <https://w3c.github.io/media-source/#sourcebuffer>
#[dom_struct]
pub(crate) struct SourceBuffer {
    eventtarget: EventTarget,

    /// The media source this object belongs to, cleared once it is removed from the `sourceBuffers`
    /// attribute of its parent media source.
    media_source: MutNullableDom<MediaSource>,

    /// The backend's handle of this source buffer.
    id: SourceBufferId,

    /// <https://w3c.github.io/media-source/#dom-sourcebuffer-updating>
    ///
    /// The JavaScript-visible flag. It follows the specification's tasks: set when an append or a
    /// removal is accepted, cleared by the task that handles the backend's `AppendDone`,
    /// `AppendError` or `RemoveDone` (and by `abort()` and removal), so a page never sees it false
    /// before the `update` and `updateend` events are queued. The backend keeps its own flag for its
    /// own checks.
    updating: Cell<bool>,

    /// Whether this source buffer belongs in `activeSourceBuffers`, which the specification derives
    /// from the enabled and selected tracks: set by the first initialization segment that has an audio
    /// or video track.
    ///
    /// <https://w3c.github.io/media-source/#dfn-active-track-flag>
    active: Cell<bool>,

    /// The last known values of the attributes the backend owns, returned once this source buffer has
    /// been removed (the specification keeps the attributes readable).
    mode: Cell<AppendMode>,
    timestamp_offset: Cell<f64>,
    append_window_start: Cell<f64>,
    append_window_end: Cell<f64>,

    /// <https://w3c.github.io/media-source/#dom-sourcebuffer-audiotracks>
    audio_tracks: Dom<AudioTrackList>,

    /// <https://w3c.github.io/media-source/#dom-sourcebuffer-videotracks>
    video_tracks: Dom<VideoTrackList>,
}

fn to_backend_mode(mode: AppendMode) -> BackendAppendMode {
    match mode {
        AppendMode::Segments => BackendAppendMode::Segments,
        AppendMode::Sequence => BackendAppendMode::Sequence,
    }
}

fn from_backend_mode(mode: BackendAppendMode) -> AppendMode {
    match mode {
        BackendAppendMode::Segments => AppendMode::Segments,
        BackendAppendMode::Sequence => AppendMode::Sequence,
    }
}

impl SourceBuffer {
    fn new_inherited(
        media_source: &MediaSource,
        id: SourceBufferId,
        audio_tracks: &AudioTrackList,
        video_tracks: &VideoTrackList,
    ) -> SourceBuffer {
        SourceBuffer {
            eventtarget: EventTarget::new_inherited(),
            media_source: MutNullableDom::new(Some(media_source)),
            id,
            updating: Cell::new(false),
            active: Cell::new(false),
            mode: Cell::new(AppendMode::Segments),
            timestamp_offset: Cell::new(0.),
            append_window_start: Cell::new(0.),
            append_window_end: Cell::new(f64::INFINITY),
            audio_tracks: Dom::from_ref(audio_tracks),
            video_tracks: Dom::from_ref(video_tracks),
        }
    }

    pub(crate) fn new(
        cx: &mut JSContext,
        media_source: &MediaSource,
        id: SourceBufferId,
    ) -> DomRoot<SourceBuffer> {
        let global = media_source.global();
        let window = global.as_window();
        let audio_tracks = AudioTrackList::new(cx, window, &[], None);
        let video_tracks = VideoTrackList::new(cx, window, &[], None);

        let source_buffer = reflect_dom_object(
            cx,
            Box::new(SourceBuffer::new_inherited(
                media_source,
                id,
                &audio_tracks,
                &video_tracks,
            )),
            window,
        );
        // Seed the cached attributes from the backend (a mode other than "segments" would be a
        // generate-timestamps format).
        if let Ok(mode) = media_source.backend().mode(id) {
            source_buffer.mode.set(from_backend_mode(mode));
        }
        source_buffer
    }

    pub(crate) fn id(&self) -> SourceBufferId {
        self.id
    }

    pub(crate) fn updating(&self) -> bool {
        self.updating.get()
    }

    /// Whether this source buffer belongs in `activeSourceBuffers`.
    pub(crate) fn is_active(&self) -> bool {
        self.active.get()
    }

    pub(crate) fn set_active(&self, active: bool) {
        self.active.set(active);
    }

    /// Detaches this source buffer from its media source, which the specification describes as
    /// removing it from the `sourceBuffers` attribute. The backend has already dropped its half.
    pub(crate) fn detach(&self) {
        self.updating.set(false);
        self.active.set(false);
        self.media_source.set(None);
    }

    /// <https://w3c.github.io/media-source/#dom-mediasource-removesourcebuffer> step 2: the append
    /// that was running ends with `abort` and `updateend` events.
    pub(crate) fn abort_pending(&self) {
        if !self.updating.get() {
            return;
        }
        self.updating.set(false);
        self.queue_event(Atom::from("abort"));
        self.queue_event(Atom::from("updateend"));
    }

    /// The backend finished an append: the buffer append algorithm's steps 7 to 9 (and the same
    /// steps of the range removal algorithm). Ignored when no append is pending (it was aborted or
    /// the buffer removed while the event was in flight).
    pub(crate) fn operation_finished(&self) {
        if !self.updating.get() {
            return;
        }
        // Step 7. Set the updating attribute to false.
        self.updating.set(false);
        // Step 8. Queue a task to fire an event named update at this SourceBuffer object.
        self.queue_event(Atom::from("update"));
        // Step 9. Queue a task to fire an event named updateend at this SourceBuffer object.
        self.queue_event(Atom::from("updateend"));
    }

    /// <https://w3c.github.io/media-source/#dfn-append-error> steps 2 to 4 (the end of stream with a
    /// decode error, step 5, comes as the backend's `SourceEnded` event).
    pub(crate) fn operation_failed(&self) {
        if !self.updating.get() {
            return;
        }
        // Step 2. Set the updating attribute to false.
        self.updating.set(false);
        // Step 3. Queue a task to fire an event named error at this SourceBuffer object.
        self.queue_event(Atom::from("error"));
        // Step 4. Queue a task to fire an event named updateend at this SourceBuffer object.
        self.queue_event(Atom::from("updateend"));
    }

    /// The specification tests for removal from `sourceBuffers` in almost every method.
    fn require_attached(&self) -> Fallible<DomRoot<MediaSource>> {
        self.media_source.get().ok_or_else(|| {
            Error::InvalidState(Some(
                "The source buffer has been removed from the media source".into(),
            ))
        })
    }

    /// The `updating` check the specification makes at the top of most methods.
    fn require_not_updating(&self) -> ErrorResult {
        if self.updating.get() {
            return Err(Error::InvalidState(Some(
                "The source buffer is still updating".into(),
            )));
        }
        Ok(())
    }

    fn queue_event(&self, name: Atom) {
        let this = Trusted::new(self);
        self.global()
            .task_manager()
            .media_element_task_source()
            .queue(task!(fire_source_buffer_event: move |cx| {
                let this = this.root();
                this.upcast::<EventTarget>().fire_event(cx, name);
            }));
    }
}

impl SourceBufferMethods<crate::DomTypeHolder> for SourceBuffer {
    /// <https://w3c.github.io/media-source/#dom-sourcebuffer-mode>
    fn Mode(&self) -> AppendMode {
        if let Some(media_source) = self.media_source.get() &&
            let Ok(mode) = media_source.backend().mode(self.id)
        {
            self.mode.set(from_backend_mode(mode));
        }
        self.mode.get()
    }

    /// <https://w3c.github.io/media-source/#dom-sourcebuffer-mode>
    fn SetMode(&self, value: AppendMode) -> ErrorResult {
        // Step 1. Removed from sourceBuffers: InvalidStateError. Step 2. updating: InvalidStateError.
        let media_source = self.require_attached()?;
        self.require_not_updating()?;

        // Steps 3 to 8 (the generate timestamps flag, the reopening of an ended source, the append
        // state, the group start timestamp) are the backend's.
        let before = media_source.backend().ready_state();
        let result = media_source
            .backend()
            .set_mode(self.id, to_backend_mode(value))
            .map_err(dom_error);
        media_source.note_ready_state(before);
        result?;
        self.mode.set(value);
        Ok(())
    }

    /// <https://w3c.github.io/media-source/#dom-sourcebuffer-updating>
    fn Updating(&self) -> bool {
        self.updating.get()
    }

    /// <https://w3c.github.io/media-source/#dom-sourcebuffer-buffered>
    fn GetBuffered(&self, cx: &mut JSContext) -> Fallible<DomRoot<TimeRanges>> {
        // Step 1. Removed from sourceBuffers: InvalidStateError.
        let media_source = self.require_attached()?;

        // Steps 2 to 6: the backend computes the intersection of the track buffers' ranges.
        let ranges = media_source.backend().buffered(self.id);
        let global = self.global();
        Ok(TimeRanges::new(
            cx,
            global.as_window(),
            ranges_to_container(&ranges),
        ))
    }

    /// <https://w3c.github.io/media-source/#dom-sourcebuffer-timestampoffset>
    fn TimestampOffset(&self) -> Finite<f64> {
        if let Some(media_source) = self.media_source.get() &&
            let Ok(offset) = media_source.backend().timestamp_offset(self.id)
        {
            self.timestamp_offset.set(offset);
        }
        Finite::wrap(self.timestamp_offset.get())
    }

    /// <https://w3c.github.io/media-source/#dom-sourcebuffer-timestampoffset>
    fn SetTimestampOffset(&self, value: Finite<f64>) -> ErrorResult {
        let media_source = self.require_attached()?;
        self.require_not_updating()?;

        let before = media_source.backend().ready_state();
        let result = media_source
            .backend()
            .set_timestamp_offset(self.id, *value)
            .map_err(dom_error);
        media_source.note_ready_state(before);
        result?;
        self.timestamp_offset.set(*value);
        Ok(())
    }

    /// <https://w3c.github.io/media-source/#dom-sourcebuffer-audiotracks>
    fn AudioTracks(&self) -> DomRoot<AudioTrackList> {
        DomRoot::from_ref(&*self.audio_tracks)
    }

    /// <https://w3c.github.io/media-source/#dom-sourcebuffer-videotracks>
    fn VideoTracks(&self) -> DomRoot<VideoTrackList> {
        DomRoot::from_ref(&*self.video_tracks)
    }

    /// <https://w3c.github.io/media-source/#dom-sourcebuffer-appendwindowstart>
    fn AppendWindowStart(&self) -> Finite<f64> {
        if let Some(media_source) = self.media_source.get() &&
            let Ok(start) = media_source.backend().append_window_start(self.id)
        {
            self.append_window_start.set(start);
        }
        Finite::wrap(self.append_window_start.get())
    }

    /// <https://w3c.github.io/media-source/#dom-sourcebuffer-appendwindowstart>
    fn SetAppendWindowStart(&self, value: Finite<f64>) -> ErrorResult {
        // Steps 1 and 2 (removed, updating), then the TypeError range checks of steps 3 and 4: the
        // backend knows `appendWindowEnd`.
        let media_source = self.require_attached()?;
        self.require_not_updating()?;
        media_source
            .backend()
            .set_append_window_start(self.id, *value)
            .map_err(dom_error)?;
        self.append_window_start.set(*value);
        Ok(())
    }

    /// <https://w3c.github.io/media-source/#dom-sourcebuffer-appendwindowend>
    fn AppendWindowEnd(&self) -> f64 {
        if let Some(media_source) = self.media_source.get() &&
            let Ok(end) = media_source.backend().append_window_end(self.id)
        {
            self.append_window_end.set(end);
        }
        self.append_window_end.get()
    }

    /// <https://w3c.github.io/media-source/#dom-sourcebuffer-appendwindowend>
    fn SetAppendWindowEnd(&self, value: f64) -> ErrorResult {
        let media_source = self.require_attached()?;
        self.require_not_updating()?;
        // Step 3. NaN is a TypeError (the backend checks it too, with the comparison to the start).
        media_source
            .backend()
            .set_append_window_end(self.id, value)
            .map_err(dom_error)?;
        self.append_window_end.set(value);
        Ok(())
    }

    /// <https://w3c.github.io/media-source/#dom-sourcebuffer-appendbuffer>
    fn AppendBuffer(&self, data: ArrayBufferViewOrArrayBuffer) -> ErrorResult {
        // Step 1. Run the prepare append algorithm.
        // <https://w3c.github.io/media-source/#dfn-prepare-append>
        // Step 1.1. Removed from sourceBuffers: InvalidStateError. Step 1.2. updating:
        // InvalidStateError.
        let media_source = self.require_attached()?;
        self.require_not_updating()?;

        // Steps 1.3 and 1.4. If the media element's error attribute is not null, InvalidStateError.
        if let Some(media_element) = media_source.media_element() &&
            media_element.in_error_state()
        {
            return Err(Error::InvalidState(Some(
                "The media element is in an error state".into(),
            )));
        }

        // A buffer bigger than everything the source buffer may ever hold is refused before it is
        // copied (the quota check of prepare append, made on the length alone: the copy of a
        // multi-gigabyte buffer is itself what a page could use to exhaust memory).
        let length = match &data {
            ArrayBufferViewOrArrayBuffer::ArrayBufferView(view) => view.len(),
            ArrayBufferViewOrArrayBuffer::ArrayBuffer(buffer) => buffer.len(),
        };
        if length > media_source.backend().max_append_bytes() {
            return Err(dom_error(MseError::QuotaExceeded(format!(
                "an append of {length} bytes is bigger than the source buffer can ever hold"
            ))));
        }

        // Step 2. Add data to the end of the input buffer (a copy: the page may reuse its buffer).
        let bytes = match &data {
            ArrayBufferViewOrArrayBuffer::ArrayBufferView(view) => {
                get_buffer_source_copy(view.into())
            },
            ArrayBufferViewOrArrayBuffer::ArrayBuffer(buffer) => {
                get_buffer_source_copy(buffer.into())
            },
        };

        // Steps 1.5 to 1.7 of prepare append (an ended source reopens, the coded frame eviction, the
        // buffer full flag: QuotaExceededError) and steps 3 to 5 (updating, updatestart, the buffer
        // append algorithm on the backend's worker thread) are the backend's.
        let before = media_source.backend().ready_state();
        let result = media_source
            .backend()
            .append(self.id, bytes)
            .map_err(dom_error);
        media_source.note_ready_state(before);
        result?;

        // Step 3. Set the updating attribute to true.
        self.updating.set(true);

        // Step 4. Queue a task to fire an event named updatestart at this SourceBuffer object.
        self.queue_event(Atom::from("updatestart"));

        // Step 5. The buffer append algorithm runs on the backend; its `AppendDone` or `AppendError`
        // event ends the operation (`operation_finished`, `operation_failed`).
        Ok(())
    }

    /// <https://w3c.github.io/media-source/#dom-sourcebuffer-abort>
    fn Abort(&self) -> ErrorResult {
        // Step 1. Removed from sourceBuffers: InvalidStateError. Step 2. The parent is not "open":
        // InvalidStateError. Step 3 (the range removal algorithm running) never holds on the script
        // thread: the backend completes a removal before its event.
        let media_source = self.require_attached()?;
        if media_source.backend().ready_state() != BackendReadyState::Open {
            return Err(Error::InvalidState(Some(
                "The media source is not open".into(),
            )));
        }

        // Steps 4 to 7 (abort the append, reset the parser state, the append window defaults) are the
        // backend's.
        media_source
            .backend()
            .abort(self.id)
            .map_err(dom_error)?;

        // Step 4.2 to 4.4. If updating, set it to false and queue `abort` and `updateend`.
        self.abort_pending();
        self.append_window_start.set(0.);
        self.append_window_end.set(f64::INFINITY);
        Ok(())
    }

    /// <https://w3c.github.io/media-source/#dom-sourcebuffer-changetype>
    fn ChangeType(&self, type_: DOMString) -> ErrorResult {
        // Step 1. If type is an empty string then throw a TypeError exception.
        if type_.is_empty() {
            return Err(Error::Type(c"The type must not be empty".to_owned()));
        }

        // Steps 2 and 3. Removed from sourceBuffers, or updating: InvalidStateError.
        let media_source = self.require_attached()?;
        self.require_not_updating()?;

        // Steps 4 to 9 (support check: NotSupportedError, reopening an ended source, reset parser
        // state, the generate timestamps flag, the append state, the type) are the backend's.
        let before = media_source.backend().ready_state();
        let result = media_source
            .backend()
            .change_type(self.id, &type_.str())
            .map_err(dom_error);
        media_source.note_ready_state(before);
        result
    }

    /// <https://w3c.github.io/media-source/#dom-sourcebuffer-remove>
    fn Remove(&self, start: Finite<f64>, end: f64) -> ErrorResult {
        // Steps 1 and 2. Removed from sourceBuffers, or updating: InvalidStateError.
        let media_source = self.require_attached()?;
        self.require_not_updating()?;

        // Steps 3 to 6 (duration NaN, start and end range checks: TypeError; reopening an ended
        // source) and the range removal algorithm are the backend's.
        let before = media_source.backend().ready_state();
        let result = media_source
            .backend()
            .remove(self.id, *start, end)
            .map_err(dom_error);
        media_source.note_ready_state(before);
        result?;

        // Range removal step 1 and 2. Set updating to true and queue `updatestart`; step 5 and 6 come
        // with the backend's `RemoveDone` event.
        self.updating.set(true);
        self.queue_event(Atom::from("updatestart"));
        Ok(())
    }

    // <https://w3c.github.io/media-source/#dom-sourcebuffer-onupdatestart>
    event_handler!(updatestart, GetOnupdatestart, SetOnupdatestart);

    // <https://w3c.github.io/media-source/#dom-sourcebuffer-onupdate>
    event_handler!(update, GetOnupdate, SetOnupdate);

    // <https://w3c.github.io/media-source/#dom-sourcebuffer-onupdateend>
    event_handler!(updateend, GetOnupdateend, SetOnupdateend);

    // <https://w3c.github.io/media-source/#dom-sourcebuffer-onerror>
    event_handler!(error, GetOnerror, SetOnerror);

    // <https://w3c.github.io/media-source/#dom-sourcebuffer-onabort>
    event_handler!(abort, GetOnabort, SetOnabort);
}
