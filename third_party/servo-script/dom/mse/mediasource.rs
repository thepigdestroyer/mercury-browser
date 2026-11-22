/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

// Mercury: (MED-1) the structure, the attach and detach steps, the event queuing and the method list are
// carried from servo/servo#47269 by decomoraes (MPL-2.0). The spec algorithms are not: the readyState,
// the duration rules, the end of stream algorithm, the buffered intersection and the seekable ranges are
// Mercury's own and live in `crates/mercury-mse` (`MediaSourceBackend`), which this object calls and
// whose `MseEvent`s it turns into DOM events and media element notifications. The backend is created
// with a `GenericCallback` that queues a media element task, like the media element's own player
// callback; the weak reference in that callback keeps the object collectable.

use std::cell::{Cell, RefCell};
use std::ffi::CString;
use std::rc::Rc;
use std::sync::{Arc, Mutex, OnceLock, PoisonError, Weak};

use dom_struct::dom_struct;
use js::context::JSContext;
use js::rust::HandleObject;
use script_bindings::assert::assert_in_script;
use script_bindings::reflector::reflect_weak_referenceable_dom_object_with_proto;
use script_bindings::weakref::WeakRef;
use servo_base::generic_channel;
use servo_media::ServoMedia;
use servo_media::player::mse::{
    EosError, MediaSourceBackend, MediaSourceReadyState as BackendReadyState, MseError, MseEvent,
    SourceBufferId, TimeRange, TrackKind,
};
use stylo_atoms::Atom;

use crate::dom::bindings::codegen::Bindings::MediaSourceBinding::{
    EndOfStreamError, MediaSourceMethods, ReadyState,
};
use crate::dom::bindings::error::{Error, ErrorResult, Fallible};
use crate::dom::bindings::inheritance::Castable;
use crate::dom::bindings::num::Finite;
use crate::dom::bindings::refcounted::Trusted;
use crate::dom::bindings::reflector::DomGlobal;
use crate::dom::bindings::root::{Dom, DomRoot, MutNullableDom};
use crate::dom::bindings::str::DOMString;
use crate::dom::eventtarget::EventTarget;
use crate::dom::globalscope::GlobalScope;
use crate::dom::html::htmlmediaelement::HTMLMediaElement;
use crate::dom::mse::sourcebuffer::SourceBuffer;
use crate::dom::mse::sourcebufferlist::SourceBufferList;
use crate::dom::mse::stall::StallState;
use crate::dom::timeranges::TimeRangesContainer;
use crate::dom::window::Window;

/// Maps what the backend refuses with to the exception the specification names (a JavaScript
/// `TypeError` for `MseError::Type`).
pub(crate) fn dom_error(error: MseError) -> Error {
    match error {
        MseError::InvalidState(message) | MseError::Other(message) => {
            Error::InvalidState(Some(message))
        },
        MseError::Type(message) => {
            Error::Type(CString::new(message.replace('\0', " ")).unwrap_or_default())
        },
        MseError::NotSupported(message) => Error::NotSupported(Some(message)),
        MseError::NotFound(message) => Error::NotFound(Some(message)),
        MseError::QuotaExceeded(_) => Error::QuotaExceeded {
            quota: None,
            requested: None,
        },
    }
}

/// The ranges the backend reports (nanoseconds) as a `TimeRanges` container (seconds).
pub(crate) fn ranges_to_container(ranges: &[TimeRange]) -> TimeRangesContainer {
    let mut container = TimeRangesContainer::default();
    for range in ranges {
        let _ = container.add(range.start_secs(), range.end_secs());
    }
    container
}

/// <https://w3c.github.io/media-source/#mediasource>
#[dom_struct]
pub(crate) struct MediaSource {
    eventtarget: EventTarget,

    /// <https://w3c.github.io/media-source/#dom-mediasource-sourcebuffers>
    source_buffers: Dom<SourceBufferList>,

    /// <https://w3c.github.io/media-source/#dom-mediasource-activesourcebuffers>
    active_source_buffers: Dom<SourceBufferList>,

    /// The media element this media source is attached to, if any.
    ///
    /// <https://w3c.github.io/media-source/#dfn-attaching-to-a-media-element>
    media_element: MutNullableDom<HTMLMediaElement>,

    /// <https://w3c.github.io/media-source/#dfn-has-ever-been-attached>
    has_ever_been_attached: Cell<bool>,

    /// The backend that owns the readyState, the duration, the source buffers' data and the feed the
    /// media player pulls from (`crates/mercury-mse`).
    #[ignore_malloc_size_of = "mercury-mse"]
    #[no_trace]
    backend: Arc<dyn MediaSourceBackend>,

    /// Receives the backend's events on the script thread. Owned here so that it is dropped on the
    /// script thread with this object; the backend's callback holds a weak reference only.
    #[conditional_malloc_size_of]
    #[no_trace]
    event_handler: RefCell<Option<Arc<Mutex<MediaSourceEventHandler>>>>,

    /// The underrun decision of the element this media source is attached to (lane med-1b, D4).
    #[ignore_malloc_size_of = "mercury-mse"]
    #[no_trace]
    stall: RefCell<StallState>,
}

impl MediaSource {
    fn new_inherited(
        source_buffers: &SourceBufferList,
        active_source_buffers: &SourceBufferList,
        backend: Arc<dyn MediaSourceBackend>,
    ) -> MediaSource {
        MediaSource {
            eventtarget: EventTarget::new_inherited(),
            source_buffers: Dom::from_ref(source_buffers),
            active_source_buffers: Dom::from_ref(active_source_buffers),
            media_element: Default::default(),
            has_ever_been_attached: Cell::new(false),
            backend,
            event_handler: Default::default(),
            stall: RefCell::new(StallState::new()),
        }
    }

    /// <https://w3c.github.io/media-source/#dom-mediasource-mediasource>
    pub(crate) fn new_with_proto(
        cx: &mut JSContext,
        global: &GlobalScope,
        proto: Option<HandleObject>,
    ) -> Fallible<DomRoot<MediaSource>> {
        let window = global.as_window();

        // The backend's events come back through a callback that only queues a task on the media
        // element task source (the callback runs on the backend's own thread). It reaches this object
        // through a slot filled once the object exists.
        let task_source = global
            .task_manager()
            .media_element_task_source()
            .to_sendable();
        let handler_slot: Arc<OnceLock<Weak<Mutex<MediaSourceEventHandler>>>> =
            Arc::new(OnceLock::new());
        let callback_slot = handler_slot.clone();
        let callback = generic_channel::GenericCallback::new(move |message| {
            let Ok(event) = message else {
                return;
            };
            let slot = callback_slot.clone();
            task_source.queue(task!(media_source_event: move |cx| {
                let Some(handler) = slot.get().and_then(Weak::upgrade) else {
                    return;
                };
                // A panic in an earlier event must not make every later one panic the script thread.
                handler
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .handle_event(cx, event);
            }));
        })
        .map_err(|_| {
            Error::NotSupported(Some(
                "Media Source Extensions could not create an event channel".into(),
            ))
        })?;

        // Step 1 to 5 of the constructor, spread over the backend (readyState "closed", duration NaN,
        // the live seekable range empty) and the two lists.
        let backend = ServoMedia::get()
            .create_media_source(callback)
            .map_err(dom_error)?;

        let source_buffers = SourceBufferList::new(cx, window);
        let active_source_buffers = SourceBufferList::new(cx, window);

        // `MediaSource` is weak-referenceable (Bindings.conf), so it is created in an `Rc`.
        let media_source = reflect_weak_referenceable_dom_object_with_proto(
            cx,
            Rc::new(MediaSource::new_inherited(
                &source_buffers,
                &active_source_buffers,
                backend,
            )),
            global,
            proto,
        );

        let handler = Arc::new(Mutex::new(MediaSourceEventHandler::new(&media_source)));
        let _ = handler_slot.set(Arc::downgrade(&handler));
        *media_source.event_handler.borrow_mut() = Some(handler);

        Ok(media_source)
    }

    /// The backend, for the source buffers and the media element.
    pub(crate) fn backend(&self) -> Arc<dyn MediaSourceBackend> {
        self.backend.clone()
    }

    pub(crate) fn media_element(&self) -> Option<DomRoot<HTMLMediaElement>> {
        self.media_element.get()
    }

    /// The element's underrun state. Do not hold the borrow across a call into the element.
    pub(crate) fn stall(&self) -> &RefCell<StallState> {
        &self.stall
    }

    /// <https://w3c.github.io/media-source/#dfn-attaching-to-a-media-element>
    ///
    /// Returns whether the media source could be attached. A media source that is not closed, or that
    /// has been attached before, makes the resource fetch algorithm fail (the caller runs those
    /// steps); the caller also sets the element's delaying-the-load-event flag to false.
    pub(crate) fn attach(&self, media_element: &HTMLMediaElement) -> bool {
        // If readyState is NOT set to "closed", then run the "If the media data cannot be fetched at
        // all, due to network errors, causing the user agent to give up trying to fetch the
        // resource" steps of the resource fetch algorithm.
        if self.backend.ready_state() != BackendReadyState::Closed ||
            self.has_ever_been_attached.get()
        {
            return false;
        }

        self.has_ever_been_attached.set(true);
        self.media_element.set(Some(media_element));

        // Set the readyState attribute to "open" and queue a task to fire sourceopen.
        self.backend.open();
        self.queue_event(Atom::from("sourceopen"));

        true
    }

    /// <https://w3c.github.io/media-source/#dfn-detaching-from-a-media-element>
    pub(crate) fn detach(&self) {
        if self.media_element.get().is_none() {
            return;
        }

        // Step 1. Set the readyState attribute to "closed". Step 2. Update duration to NaN.
        // The backend also drops its source buffers (steps 3 and 5).
        self.backend.close();

        // Step 3. Remove all the SourceBuffer objects from activeSourceBuffers.
        // Step 4. Queue a task to fire an event named removesourcebuffer at activeSourceBuffers.
        let had_active_source_buffers = !self.active_source_buffers.is_empty();
        self.active_source_buffers.clear();
        if had_active_source_buffers {
            self.active_source_buffers.queue_removesourcebuffer_event();
        }

        // Step 5. Remove all the SourceBuffer objects from sourceBuffers.
        // Step 6. Queue a task to fire an event named removesourcebuffer at sourceBuffers.
        let had_source_buffers = !self.source_buffers.is_empty();
        for source_buffer in self.source_buffers.buffers() {
            source_buffer.detach();
        }
        self.source_buffers.clear();
        if had_source_buffers {
            self.source_buffers.queue_removesourcebuffer_event();
        }

        // Step 7. Queue a task to fire an event named sourceclose at the MediaSource.
        self.queue_event(Atom::from("sourceclose"));

        self.media_element.set(None);
    }

    /// The seeking algorithm's MediaSource steps: an ended source goes back to "open" (the backend
    /// reports it) and `sourceopen` is queued.
    ///
    /// <https://w3c.github.io/media-source/#mediasource-seeking>
    pub(crate) fn seeking(&self, time: f64) {
        if self.backend.seeking(time) {
            self.queue_event(Atom::from("sourceopen"));
        }
    }

    /// Queues the event the backend's readyState transition across a call implies: an ended source
    /// reopened by an append, a removal, a mode or offset change (`sourceopen`), or ended by it
    /// (`sourceended`). `before` is `ready_state()` read before the call.
    pub(crate) fn note_ready_state(&self, before: BackendReadyState) {
        match (before, self.backend.ready_state()) {
            (BackendReadyState::Ended, BackendReadyState::Open) => {
                self.queue_event(Atom::from("sourceopen"))
            },
            (BackendReadyState::Open, BackendReadyState::Ended) => {
                self.queue_event(Atom::from("sourceended"))
            },
            _ => {},
        }
    }

    /// Recomputes `activeSourceBuffers`, which the specification keeps in the same order as
    /// `sourceBuffers`.
    ///
    /// <https://w3c.github.io/media-source/#dom-mediasource-activesourcebuffers>
    fn update_active_source_buffers(&self) {
        let active: Vec<_> = self
            .source_buffers
            .buffers()
            .into_iter()
            .filter(|source_buffer| source_buffer.is_active())
            .collect();
        self.active_source_buffers.update(&active);
    }

    /// Whether an append or a removal is in flight on any of the source buffers (the JavaScript-visible
    /// `updating` flags, which the specification's checks read).
    fn is_updating(&self) -> bool {
        self.source_buffers
            .buffers()
            .into_iter()
            .any(|source_buffer| source_buffer.updating())
    }

    fn source_buffer(&self, id: SourceBufferId) -> Option<DomRoot<SourceBuffer>> {
        self.source_buffers
            .buffers()
            .into_iter()
            .find(|source_buffer| source_buffer.id() == id)
    }

    pub(crate) fn queue_event(&self, name: Atom) {
        let this = Trusted::new(self);
        self.global()
            .task_manager()
            .media_element_task_source()
            .queue(task!(fire_media_source_event: move |cx| {
                let this = this.root();
                this.upcast::<EventTarget>().fire_event(cx, name);
            }));
    }

    /// What the backend tells the DOM (decisions D9 and D10 of the lane): the events that end an
    /// append or a removal, the duration, the end of the stream, an initialization segment, a change
    /// of what is buffered. Runs in a media element task on the script thread.
    fn handle_backend_event(&self, cx: &mut JSContext, event: MseEvent) {
        match event {
            // The initialization segment received algorithm's DOM steps. The backend has already run
            // the rest (the duration, the track checks, the track buffers).
            MseEvent::InitSegment { id, init } => {
                let Some(source_buffer) = self.source_buffer(id) else {
                    return;
                };
                // Steps 4 and 5 (lane med-1b, D6): the tracks of the segment join the source buffer's
                // and the element's track lists, with an `addtrack` event each.
                source_buffer.init_received(cx, &init);
                // Step 6. If the active track flag equals true, then add this SourceBuffer to
                // activeSourceBuffers. The first initialization segment with an audio or video track
                // enables the default track of its kind, which sets the flag.
                if init.first &&
                    init.tracks
                        .iter()
                        .any(|track| matches!(track.kind, TrackKind::Video | TrackKind::Audio))
                {
                    source_buffer.set_active(true);
                    self.update_active_source_buffers();
                }
                if let Some(media_element) = self.media_element.get() {
                    media_element.media_source_init_received(cx, &init);
                }
            },
            // The buffer append algorithm's steps 7 to 9 (and the range removal algorithm's 4 to 6).
            // The source buffer drops an event whose operation is not the one it is waiting for (one
            // queued before `abort()` or the removal of the buffer).
            MseEvent::AppendDone { id, op } | MseEvent::RemoveDone { id, op } => {
                if let Some(source_buffer) = self.source_buffer(id) {
                    source_buffer.operation_finished(op);
                }
                self.media_data_changed(cx);
            },
            // The append error algorithm's steps 1 to 4; step 5 (the end of stream with "decode")
            // arrives as `SourceEnded`.
            MseEvent::AppendError { id, op, kind } => {
                warn!("An append to MSE source buffer {id} failed: {kind:?}");
                if let Some(source_buffer) = self.source_buffer(id) {
                    source_buffer.operation_failed(op);
                }
            },
            MseEvent::DurationChanged(duration) => {
                if let Some(media_element) = self.media_element.get() {
                    media_element.media_source_duration_changed(duration.unwrap_or(f64::NAN));
                }
            },
            // The model ended the stream itself (an append error).
            MseEvent::SourceEnded { error } => {
                self.queue_event(Atom::from("sourceended"));
                if let Some(media_element) = self.media_element.get() {
                    match error {
                        Some(error) => media_element.media_source_error(cx, error),
                        None => media_element.media_source_end_of_stream(cx),
                    }
                }
            },
            // Data left `buffered` without a call from the page (eviction).
            MseEvent::BufferedChanged { .. } => self.media_data_changed(cx),
        }
    }

    /// What is buffered changed: the media element re-evaluates its readyState.
    fn media_data_changed(&self, cx: &mut JSContext) {
        if let Some(media_element) = self.media_element.get() {
            media_element.media_source_update_ready_state(cx);
        }
    }
}

/// Receives the backend's `MseEvent`s, queued as media element tasks, and passes them to the
/// `MediaSource` if it is still alive. A copy of the media element's `HTMLMediaElementEventHandler`:
/// it exists to decouple the ownership of the object from the callback, which runs on another thread.
#[derive(JSTraceable, MallocSizeOf)]
struct MediaSourceEventHandler {
    media_source: WeakRef<MediaSource>,
}

#[expect(unsafe_code)]
unsafe impl Send for MediaSourceEventHandler {}

impl MediaSourceEventHandler {
    fn new(media_source: &MediaSource) -> Self {
        Self {
            media_source: WeakRef::new(media_source),
        }
    }

    fn handle_event(&self, cx: &mut JSContext, event: MseEvent) {
        let Some(media_source) = self.media_source.root() else {
            return;
        };
        media_source.handle_backend_event(cx, event);
    }
}

impl Drop for MediaSourceEventHandler {
    fn drop(&mut self) {
        // The weak reference is not thread-safe and MUST be deleted on the script thread, which is
        // guaranteed by ownership of the handler by the MediaSource and by the queued tasks.
        assert_in_script();
    }
}

impl MediaSourceMethods<crate::DomTypeHolder> for MediaSource {
    /// <https://w3c.github.io/media-source/#dom-mediasource-mediasource>
    fn Constructor(
        cx: &mut JSContext,
        window: &Window,
        proto: Option<HandleObject>,
    ) -> Fallible<DomRoot<MediaSource>> {
        MediaSource::new_with_proto(cx, &window.global(), proto)
    }

    /// <https://w3c.github.io/media-source/#dom-mediasource-sourcebuffers>
    fn SourceBuffers(&self) -> DomRoot<SourceBufferList> {
        DomRoot::from_ref(&*self.source_buffers)
    }

    /// <https://w3c.github.io/media-source/#dom-mediasource-activesourcebuffers>
    fn ActiveSourceBuffers(&self) -> DomRoot<SourceBufferList> {
        DomRoot::from_ref(&*self.active_source_buffers)
    }

    /// <https://w3c.github.io/media-source/#dom-mediasource-readystate>
    fn ReadyState(&self) -> ReadyState {
        match self.backend.ready_state() {
            BackendReadyState::Closed => ReadyState::Closed,
            BackendReadyState::Open => ReadyState::Open,
            BackendReadyState::Ended => ReadyState::Ended,
        }
    }

    /// <https://w3c.github.io/media-source/#dom-mediasource-duration>
    fn Duration(&self) -> f64 {
        // If readyState equals "closed" then return NaN and abort these steps.
        if self.backend.ready_state() == BackendReadyState::Closed {
            return f64::NAN;
        }

        self.backend.duration().unwrap_or(f64::NAN)
    }

    /// <https://w3c.github.io/media-source/#dom-mediasource-duration>
    fn SetDuration(&self, value: f64) -> ErrorResult {
        // Step 1. If the value being set is negative or NaN then throw a TypeError exception and
        // abort these steps.
        if value.is_nan() || value < 0. {
            return Err(Error::Type(
                c"The duration must not be negative or NaN".to_owned(),
            ));
        }

        // Step 2. If the readyState attribute is not "open" then throw an InvalidStateError
        // exception and abort these steps.
        if self.backend.ready_state() != BackendReadyState::Open {
            return Err(Error::InvalidState(Some(
                "The media source is not open".into(),
            )));
        }

        // Step 3. If the updating attribute equals true on any SourceBuffer in sourceBuffers, then
        // throw an InvalidStateError exception and abort these steps.
        if self.is_updating() {
            return Err(Error::InvalidState(Some(
                "A source buffer is still updating".into(),
            )));
        }

        // Step 4. Run the duration change algorithm with new duration set to the value being
        // assigned to this attribute (the backend's: the InvalidStateError for a value below the
        // highest buffered start, the truncation of the buffers).
        self.backend
            .set_duration(Some(value))
            .map_err(dom_error)?;

        // The duration change algorithm's step 6: update the media duration. The backend's
        // `DurationChanged` event follows and changes nothing more.
        if let Some(media_element) = self.media_element.get() {
            media_element.media_source_duration_changed(self.backend.duration().unwrap_or(f64::NAN));
        }

        Ok(())
    }

    /// <https://w3c.github.io/media-source/#dom-mediasource-canconstructindedicatedworker>
    fn CanConstructInDedicatedWorker(_window: &Window) -> bool {
        // Media sources are not exposed to dedicated workers yet.
        false
    }

    /// <https://w3c.github.io/media-source/#dom-mediasource-addsourcebuffer>
    fn AddSourceBuffer(
        &self,
        cx: &mut JSContext,
        type_: DOMString,
    ) -> Fallible<DomRoot<SourceBuffer>> {
        // Step 1. If type is an empty string then throw a TypeError exception and abort these
        // steps.
        if type_.is_empty() {
            return Err(Error::Type(c"The type must not be empty".to_owned()));
        }

        // Steps 2 to 4 (an unsupported type: NotSupportedError; no more source buffers or an
        // unsupported configuration: QuotaExceededError; the readyState is not "open":
        // InvalidStateError) and the creation of the backend's source buffer (steps 5 to 7).
        let id = self
            .backend
            .add_source_buffer(&type_.str())
            .map_err(dom_error)?;

        let source_buffer = SourceBuffer::new(cx, self, id);

        // Step 8. Add the new object to sourceBuffers and queue a task to fire an event named
        // addsourcebuffer at sourceBuffers.
        self.source_buffers.add(&source_buffer);

        // Step 9. Return the new object.
        Ok(source_buffer)
    }

    /// <https://w3c.github.io/media-source/#dom-mediasource-removesourcebuffer>
    fn RemoveSourceBuffer(&self, cx: &mut JSContext, source_buffer: &SourceBuffer) -> ErrorResult {
        // Step 1. If sourceBuffer specifies an object that is not in sourceBuffers then throw a
        // NotFoundError exception and abort these steps.
        if !self.source_buffers.contains(source_buffer) {
            return Err(Error::NotFound(Some(
                "The source buffer is not in this media source".into(),
            )));
        }

        // The backend drops the source buffer (aborting an append, destroying its resources).
        self.backend
            .remove_source_buffer(source_buffer.id())
            .map_err(dom_error)?;

        // Step 2. If the sourceBuffer.updating attribute equals true, then abort the buffer append
        // algorithm, set updating to false and queue `abort` and `updateend`.
        source_buffer.abort_pending();

        // Steps 3 to 8, and their video track equivalents, remove the tracks of the source buffer
        // from the media element's lists and its own (lane med-1b, D6), with `removetrack` events.
        source_buffer.remove_tracks(cx);
        source_buffer.detach();

        // Step 9. If sourceBuffer is in activeSourceBuffers, then remove sourceBuffer from
        // activeSourceBuffers and queue a task to fire an event named removesourcebuffer at the
        // SourceBufferList returned by activeSourceBuffers.
        self.update_active_source_buffers();

        // Step 10. Remove sourceBuffer from sourceBuffers and queue a task to fire an event named
        // removesourcebuffer at the SourceBufferList returned by sourceBuffers.
        self.source_buffers.remove(source_buffer);

        // Step 11. Destroy all resources for sourceBuffer: done by the backend; the DOM object is
        // collected when it is no longer reachable.
        self.media_data_changed(cx);

        Ok(())
    }

    /// <https://w3c.github.io/media-source/#dom-mediasource-endofstream>
    fn EndOfStream(&self, cx: &mut JSContext, error: Option<EndOfStreamError>) -> ErrorResult {
        // Step 1. If the readyState attribute is not in the "open" state then throw an
        // InvalidStateError exception and abort these steps.
        if self.backend.ready_state() != BackendReadyState::Open {
            return Err(Error::InvalidState(Some(
                "The media source is not open".into(),
            )));
        }

        // Step 2. If the updating attribute equals true on any SourceBuffer in sourceBuffers, then
        // throw an InvalidStateError exception and abort these steps.
        if self.is_updating() {
            return Err(Error::InvalidState(Some(
                "A source buffer is still updating".into(),
            )));
        }

        // Step 3. Run the end of stream algorithm with the error parameter set to error: the
        // backend changes the readyState to "ended" and, without an error, the duration.
        let error = error.map(|error| match error {
            EndOfStreamError::Network => EosError::Network,
            EndOfStreamError::Decode => EosError::Decode,
        });
        self.backend.end_of_stream(error).map_err(dom_error)?;

        // The end of stream algorithm's step 2. Queue a task to fire an event named sourceended.
        self.queue_event(Atom::from("sourceended"));

        // Steps 3.2 to 5 notify the media element: all of the media data is there, or it ran into a
        // network or a decode error.
        if let Some(media_element) = self.media_element.get() {
            match error {
                None => media_element.media_source_end_of_stream(cx),
                Some(error) => media_element.media_source_error(cx, error),
            }
        }

        Ok(())
    }

    /// <https://w3c.github.io/media-source/#dom-mediasource-setliveseekablerange>
    fn SetLiveSeekableRange(&self, start: Finite<f64>, end: Finite<f64>) -> ErrorResult {
        // Steps 1 to 3: the readyState check, the range check (a TypeError) and the new live
        // seekable range are the backend's.
        self.backend
            .set_live_seekable_range(*start, *end)
            .map_err(dom_error)
    }

    /// <https://w3c.github.io/media-source/#dom-mediasource-clearliveseekablerange>
    fn ClearLiveSeekableRange(&self) -> ErrorResult {
        self.backend.clear_live_seekable_range().map_err(dom_error)
    }

    /// <https://w3c.github.io/media-source/#dom-mediasource-istypesupported>
    fn IsTypeSupported(_window: &Window, type_: DOMString) -> bool {
        // Steps 1 to 4 (an empty type, an invalid MIME type, an unsupported media type, subtype or
        // codec) are the backend's, which also asks the platform for the decoders.
        ServoMedia::get().is_type_supported_mse(&type_.str())
    }

    // <https://w3c.github.io/media-source/#dom-mediasource-onsourceopen>
    event_handler!(sourceopen, GetOnsourceopen, SetOnsourceopen);

    // <https://w3c.github.io/media-source/#dom-mediasource-onsourceended>
    event_handler!(sourceended, GetOnsourceended, SetOnsourceended);

    // <https://w3c.github.io/media-source/#dom-mediasource-onsourceclose>
    event_handler!(sourceclose, GetOnsourceclose, SetOnsourceclose);
}
