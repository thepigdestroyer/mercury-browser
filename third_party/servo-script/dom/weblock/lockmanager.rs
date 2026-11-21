/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

// Mercury: carried from servo/servo pull request #47981 "storages: implement Web Locks API"
// (draft, by xbwwj, Xubai Wang), branch `weblocks` of xbwwj/servo, head commit a900121b
// (`components/script/dom/weblock/lockmanager.rs`), MPL-2.0. The shape comes from there: the
// manager sends its requests to the lock thread in the storage threads and gets the grants back
// through a callback. How a granted lock ends (the callback's promise settling) is after
// LunarWerxs `pr/web-locks` (commit 23583d753, `components/script/dom/weblocks/lockmanager.rs`,
// L0garithmic, MPL-2.0). What differs from the pull request:
//
// - The pull request called the page's callback and dropped the promise it returned: a lock was
//   never released, and the promise `request()` returned never settled. Here a native handler on
//   the callback's promise releases the lock and settles the request's promise the same way.
// - The pull request ran `as_window()` on the global of every call, which panics in a worker.
// - Its argument checks (steal and ifAvailable, steal and mode) sat inside one branch and the
//   second one never ran; they are in `unsupported_request` (mod.rs), in the order of the
//   specification, and an opaque origin is refused with a `SecurityError`.
// - A stolen lock rejects the promise of its `request()` call with an `AbortError`.
// - An `AbortSignal` that fires after the lock was granted does nothing.
// - Nothing in the pull request ended a request when its window or worker went away. Here the
//   callback each request gives the storage thread says the client is gone when its task source
//   was cancelled (a window that was discarded, a worker that closed), the manager says so when it
//   is dropped, and the storage thread finds out by itself when it cannot reach the callback (the
//   content process died). See `servo-storage`, `weblocks/mod.rs`.
// - Every other change is marked `Mercury:`.

use std::cell::Cell;

use dom_struct::dom_struct;
use js::context::JSContext;
use js::realm::CurrentRealm;
use js::rust::HandleValue;
use script_bindings::cell::DomRefCell;
use script_bindings::inheritance::Castable;
use script_bindings::reflector::{Reflector, reflect_dom_object};
use servo_base::generic_channel::{GenericCallback, GenericSend, GenericSender, SendError};
use servo_url::ImmutableOrigin;
use storage_traits::weblocks::{
    LockEventMsg, LockManagerSnapshotMsg, LockMsg, LockRequest, LockRequestId, WebLocksThreadMsg,
};
use uuid::Uuid;

use crate::conversions::Convert;
use crate::dom::abortsignal::AbortAlgorithm;
use crate::dom::bindings::callback::{ExceptionHandling, RootedCallback, TracedCallback};
use crate::dom::bindings::codegen::Bindings::AbortSignalBinding::AbortSignalMethods;
use crate::dom::bindings::codegen::Bindings::WebLockBinding::{
    LockGrantedCallback, LockManagerMethods, LockManagerSnapshot, LockMode, LockOptions,
};
use crate::dom::bindings::codegen::Bindings::WindowBinding::WindowMethods;
use crate::dom::bindings::error::Error;
use crate::dom::bindings::refcounted::Trusted;
use crate::dom::bindings::reflector::DomGlobal;
use crate::dom::bindings::root::{Dom, DomRoot};
use crate::dom::bindings::str::DOMString;
use crate::dom::globalscope::GlobalScope;
use crate::dom::lock::Lock;
use crate::dom::promise::{Promise, RootedPromise, TracedPromise};
use crate::dom::promisenativehandler::Callback;
use crate::dom::types::{AbortSignal, PromiseNativeHandler};
use crate::dom::weblock::{ClientAction, client_action, unsupported_request};
use crate::dom::window::Window;
use crate::realms::enter_auto_realm;
use crate::routed_promise::{RoutedPromiseListener, callback_promise};

/// A request waiting for its turn: the page's callback and the promise `request()` returned.
/// Kept here, on the DOM object, so that both stay traced while the lock is contended.
#[derive(JSTraceable, MallocSizeOf)]
#[cfg_attr(crown, crown::unrooted_must_root_lint::must_root)]
struct PendingRequest {
    #[no_trace]
    id: LockRequestId,
    name: String,
    callback: TracedCallback<LockGrantedCallback>,
    promise: TracedPromise,
}

/// A request whose callback runs: the promise to reject when another request steals its lock.
#[derive(JSTraceable, MallocSizeOf)]
#[cfg_attr(crown, crown::unrooted_must_root_lint::must_root)]
struct HeldRequest {
    #[no_trace]
    id: LockRequestId,
    promise: TracedPromise,
}

/// Tells the storage thread that this global scope has gone away, so that it releases what the
/// global holds. It runs when the `LockManager` is finalized: the garbage collector of a window,
/// the end of the script thread, the end of a worker's thread. A global that is discarded earlier
/// is found by the probes (see `LockEventMsg::Probe`), and a global whose process died by the
/// storage thread itself.
#[derive(JSTraceable, MallocSizeOf)]
struct DroppableLockManager {
    /// Names this global in `query()` results and in `WebLocksThreadMsg::ClientGone`.
    client_id: String,
    #[no_trace]
    storage: GenericSender<WebLocksThreadMsg>,
    /// Whether this global ever asked for a lock. If it did not, there is nothing to release.
    used: Cell<bool>,
}

impl Drop for DroppableLockManager {
    fn drop(&mut self) {
        if self.used.get() {
            // The storage thread may be gone already, at the end of the process.
            let _ = self
                .storage
                .send(WebLocksThreadMsg::ClientGone(self.client_id.clone()));
        }
    }
}

/// What `request()` was given, after the dictionary is read.
struct RequestOptions<'a> {
    mode: LockMode,
    if_available: bool,
    steal: bool,
    signal: Option<&'a DomRoot<AbortSignal>>,
}

/// <https://w3c.github.io/web-locks/#api-lock-manager>
#[dom_struct]
pub(crate) struct LockManager {
    reflector_: Reflector,
    /// Requests waiting for a lock.
    pending: DomRefCell<Vec<PendingRequest>>,
    /// Requests that hold a lock (the callback's promise is not settled yet).
    held: DomRefCell<Vec<HeldRequest>>,
    droppable: DroppableLockManager,
}

impl LockManager {
    fn new_inherited(storage: GenericSender<WebLocksThreadMsg>) -> LockManager {
        LockManager {
            reflector_: Reflector::new(),
            pending: Default::default(),
            held: Default::default(),
            droppable: DroppableLockManager {
                client_id: Uuid::new_v4().to_string(),
                storage,
                used: Cell::new(false),
            },
        }
    }

    pub(crate) fn new(cx: &mut JSContext, global: &GlobalScope) -> DomRoot<LockManager> {
        let storage: GenericSender<WebLocksThreadMsg> =
            GenericSend::sender(global.storage_threads());
        reflect_dom_object(cx, Box::new(LockManager::new_inherited(storage)), global)
    }

    fn storage(&self) -> &GenericSender<WebLocksThreadMsg> {
        &self.droppable.storage
    }

    fn origin(&self) -> ImmutableOrigin {
        self.global().origin().immutable().clone()
    }

    /// The checks at the start of `request()` and `query()`, which refuse with an exception:
    /// the document of a window must be fully active (Step 3) and the origin must not be opaque
    /// (Step 4). Returns the exception.
    fn refusal(&self) -> Option<Error> {
        let global = self.global();
        // Mercury: the pull request called `as_window()`, which panics in a worker.
        if let Some(window) = global.downcast::<Window>()
            && !window.Document().is_fully_active()
        {
            return Some(Error::InvalidState(Some(
                "The document is not fully active".to_owned(),
            )));
        }
        if !global.origin().is_tuple() {
            return Some(Error::Security(Some(
                "Web Locks are not available to an opaque origin".to_owned(),
            )));
        }
        None
    }

    /// <https://w3c.github.io/web-locks/#dom-lockmanager-request>
    fn request(
        &self,
        cx: &mut CurrentRealm,
        name: DOMString,
        options: RequestOptions<'_>,
        callback: RootedCallback<LockGrantedCallback>,
    ) -> RootedPromise {
        // Step 10. Let promise be a new promise.
        // (Made first: the steps that refuse return a rejected promise.)
        let promise = Promise::new_in_realm(cx);

        // Step 3. If the document is not fully active, reject with "InvalidStateError".
        // Step 4. If origin is an opaque origin, reject with "SecurityError".
        if let Some(error) = self.refusal() {
            promise.reject_error(cx, error);
            return promise;
        }

        // Step 5-8. Refuse the options that do not go together with "NotSupportedError".
        let name = name.to_string();
        if let Some(message) = unsupported_request(
            &name,
            options.mode,
            options.steal,
            options.if_available,
            options.signal.is_some(),
        ) {
            promise.reject_error(cx, Error::NotSupported(Some(message.to_owned())));
            return promise;
        }

        // Step 9. If options’ signal dictionary member is present and is aborted, then return a
        // promise rejected with options’ signal’s abort reason.
        if let Some(signal) = options.signal
            && signal.aborted()
        {
            rooted!(&in(cx) let mut reason = js::jsval::UndefinedValue());
            signal.Reason(reason.handle_mut());
            promise.reject(cx, reason.handle());
            return promise;
        }

        // Step 11. Request a lock with promise, the current agent’s event loop, environment’s id,
        // origin, callback, name, options’ mode dictionary member, options’ ifAvailable
        // dictionary member, and options’ steal dictionary member, and options’ signal
        // dictionary member.
        self.request_lock(cx, &promise, name, callback, options);

        // Step 12. Return promise.
        promise
    }

    /// <https://w3c.github.io/web-locks/#request-a-lock>
    fn request_lock(
        &self,
        cx: &mut CurrentRealm,
        promise: &RootedPromise,
        name: String,
        callback: RootedCallback<LockGrantedCallback>,
        options: RequestOptions<'_>,
    ) {
        let global = self.global();
        let request_id = LockRequestId::next();

        // What the storage thread says about this request comes back to this callback. It may
        // run on the router thread (several processes) or on the storage thread (one process).
        let task_source = global.task_manager().weblocks_task_source().to_sendable();
        let manager = Trusted::new(self);
        let storage = self.storage().clone();
        let client_id = self.droppable.client_id.clone();
        let event_callback = GenericCallback::new(move |event: Result<LockEventMsg, SendError>| {
            let Ok(event) = event else {
                return;
            };
            // Mercury: a global whose task source is cancelled is gone: its window was
            // discarded or its worker closed. Whatever the event, its locks are released.
            let global_closed = task_source.canceller.cancelled();
            match client_action(matches!(event, LockEventMsg::Probe), global_closed) {
                ClientAction::Ignore => {},
                ClientAction::ReportGone => {
                    let _ = storage.send(WebLocksThreadMsg::ClientGone(client_id.clone()));
                },
                ClientAction::Deliver => {
                    let manager = manager.clone();
                    match event {
                        LockEventMsg::Granted(lock) => {
                            task_source.queue(task!(weblocks_lock_granted: move |cx| {
                                manager.root().on_granted(cx, request_id, lock);
                            }));
                        },
                        LockEventMsg::Stolen => {
                            task_source.queue(task!(weblocks_lock_stolen: move |cx| {
                                manager.root().on_stolen(cx, request_id);
                            }));
                        },
                        LockEventMsg::Probe => {},
                    }
                },
            }
        });
        let event_callback = match event_callback {
            Ok(event_callback) => event_callback,
            Err(error) => {
                warn!("Could not create a Web Locks callback: {error}");
                promise.reject_error(
                    cx,
                    Error::InvalidState(Some("Web Locks are not available".to_owned())),
                );
                return;
            },
        };

        // Step 2. If signal is present, add the abort steps to signal.
        // Mercury: the steps ask this manager, which knows whether the request still waits.
        if let Some(signal) = options.signal {
            signal.add(&AbortAlgorithm::AbortLockRequest(
                Dom::from_ref(self),
                request_id,
            ));
        }

        self.pending.borrow_mut().push(PendingRequest {
            id: request_id,
            name: name.clone(),
            callback: callback.to_traced(),
            promise: promise.to_traced(),
        });

        // Step 3. Enqueue the following steps to the lock task queue.
        // The inner steps (3.1 to 3.6) continue on the storage thread.
        self.droppable.used.set(true);
        let request = LockRequest {
            id: request_id,
            client_id: self.droppable.client_id.clone(),
            name,
            mode: options.mode.convert(),
            callback: event_callback,
            if_available: options.if_available,
            steal: options.steal,
        };
        if let Err(error) = self
            .storage()
            .send(WebLocksThreadMsg::Request(request, self.origin()))
        {
            warn!("A Web Locks request could not be sent: {error}");
            self.pending
                .borrow_mut()
                .retain(|pending| pending.id != request_id);
            promise.reject_error(
                cx,
                Error::InvalidState(Some("Web Locks are not available".to_owned())),
            );
        }

        // Step 4. Return request.
    }

    fn take_pending(&self, request_id: LockRequestId) -> Option<PendingRequest> {
        let mut pending = self.pending.borrow_mut();
        let index = pending
            .iter()
            .position(|pending| pending.id == request_id)?;
        Some(pending.remove(index))
    }

    fn send_release(&self, lock: &LockMsg) {
        _ = self.storage().send(WebLocksThreadMsg::Release(
            lock.id,
            lock.name.clone(),
            self.origin(),
        ));
    }

    /// <https://w3c.github.io/web-locks/#process-the-lock-request-queue>, Step 14: the request was
    /// granted (`Some`), or it asked for `ifAvailable` and the lock is not available (`None`).
    fn on_granted(&self, cx: &mut JSContext, request_id: LockRequestId, lock: Option<LockMsg>) {
        let Some(pending) = self.take_pending(request_id) else {
            // The request was aborted while the storage thread granted it. Give the lock back.
            if let Some(lock) = lock {
                self.send_release(&lock);
            }
            return;
        };

        let global = self.global();
        let promise = pending.promise.root(cx);
        rooted!(&in(cx) let callback = pending.callback);

        // Step 14.1. Let r be the result of invoking callback with a new Lock object (or null)
        // and "rethrow".
        let lock_object = lock.clone().map(|msg| Lock::new(cx, &global, msg));
        let result = callback.Call__(cx, lock_object.as_deref(), ExceptionHandling::Rethrow);
        let waiting = match result {
            Ok(waiting) => waiting,
            Err(error) => {
                // The callback threw: nothing holds the lock now. The request is rejected.
                if let Some(lock) = &lock {
                    self.send_release(lock);
                }
                promise.reject_error(cx, error);
                return;
            },
        };

        // Step 14.2 to 14.5. The lock is held until the promise the callback returned settles,
        // and the request settles the way that promise does.
        if lock.is_some() {
            self.held.borrow_mut().push(HeldRequest {
                id: request_id,
                promise: promise.to_traced(),
            });
        }
        let handler = PromiseNativeHandler::new(
            cx,
            &global,
            Some(Box::new(SettleRequest {
                manager: Dom::from_ref(self),
                promise: promise.to_traced(),
                request_id,
                lock: lock.clone(),
                rejected: false,
            })),
            Some(Box::new(SettleRequest {
                manager: Dom::from_ref(self),
                promise: promise.to_traced(),
                request_id,
                lock,
                rejected: true,
            })),
        );
        let mut realm = enter_auto_realm(cx, &*global);
        waiting.append_native_handler(&mut realm.current_realm(), &handler);
    }

    /// <https://w3c.github.io/web-locks/#algorithm-request-lock>, Step 3.4.1: another request
    /// stole the lock this request holds. The request is rejected with an `AbortError`.
    fn on_stolen(&self, cx: &mut JSContext, request_id: LockRequestId) {
        let held = {
            let mut held = self.held.borrow_mut();
            held.iter()
                .position(|held| held.id == request_id)
                .map(|index| held.remove(index))
        };
        if let Some(held) = held {
            let promise = held.promise.root(cx);
            promise.reject_error(
                cx,
                Error::Abort(Some("The lock was stolen by another request".to_owned())),
            );
        }
    }

    /// The promise the callback returned settled: the lock is released, and the request settles.
    fn on_settled(&self, request_id: LockRequestId, lock: Option<&LockMsg>) {
        self.held.borrow_mut().retain(|held| held.id != request_id);
        if let Some(lock) = lock {
            // A lock that was stolen is not held any more; the storage thread ignores this.
            self.send_release(lock);
        }
    }

    /// <https://w3c.github.io/web-locks/#signal-to-abort-the-request>
    pub(crate) fn abort_request(
        &self,
        cx: &mut JSContext,
        request_id: LockRequestId,
        reason: HandleValue,
    ) {
        // Mercury: a request that was granted or aborted already is not pending, and the signal
        // has no say any more.
        let Some(pending) = self.take_pending(request_id) else {
            return;
        };
        // Step 1. Enqueue the steps to abort the request to the lock task queue.
        _ = self.storage().send(WebLocksThreadMsg::Abort(
            request_id,
            self.origin(),
            pending.name,
        ));
        // Step 2. Reject request’s promise with signal’s abort reason.
        let promise = pending.promise.root(cx);
        promise.reject(cx, reason);
    }
}

/// Releases the lock, then settles the request's promise the way the callback's promise settled.
/// This is the whole release mechanism: a Web Lock is held for exactly as long as the promise its
/// callback returned (after LunarWerxs `pr/web-locks`).
#[derive(JSTraceable, MallocSizeOf)]
#[cfg_attr(crown, crown::unrooted_must_root_lint::must_root)]
struct SettleRequest {
    manager: Dom<LockManager>,
    promise: TracedPromise,
    #[no_trace]
    request_id: LockRequestId,
    /// `None` for an `ifAvailable` request that did not get the lock.
    #[no_trace]
    lock: Option<LockMsg>,
    rejected: bool,
}

impl Callback for SettleRequest {
    fn callback(&self, cx: &mut CurrentRealm, value: HandleValue) {
        // Release first: whatever runs after the promise settles must find the lock free.
        self.manager.on_settled(self.request_id, self.lock.as_ref());
        if self.rejected {
            self.promise.reject(cx, value);
        } else {
            self.promise.resolve(cx, value);
        }
    }
}

impl LockManagerMethods<crate::DomTypeHolder> for LockManager {
    /// <https://w3c.github.io/web-locks/#dom-lockmanager-request>
    fn Request(
        &self,
        cx: &mut CurrentRealm,
        name: DOMString,
        callback: RootedCallback<LockGrantedCallback>,
    ) -> RootedPromise {
        // Step 1. If options was not passed, set it to a default LockOptions.
        self.request(
            cx,
            name,
            RequestOptions {
                mode: LockMode::Exclusive,
                if_available: false,
                steal: false,
                signal: None,
            },
            callback,
        )
    }

    /// <https://w3c.github.io/web-locks/#dom-lockmanager-request>
    fn Request_(
        &self,
        cx: &mut CurrentRealm,
        name: DOMString,
        options: &LockOptions,
        callback: RootedCallback<LockGrantedCallback>,
    ) -> RootedPromise {
        self.request(
            cx,
            name,
            RequestOptions {
                mode: options.mode,
                if_available: options.ifAvailable,
                steal: options.steal,
                signal: options.signal.as_ref(),
            },
            callback,
        )
    }

    /// <https://w3c.github.io/web-locks/#dom-lockmanager-query>
    fn Query(&self, cx: &mut CurrentRealm) -> RootedPromise {
        // Step 4. Let promise be a new promise.
        let promise = Promise::new_in_realm(cx);

        // Step 2. If the document is not fully active, reject with "InvalidStateError".
        // Step 3. If origin is an opaque origin, reject with "SecurityError".
        if let Some(error) = self.refusal() {
            promise.reject_error(cx, error);
            return promise;
        }

        // Step 5. Enqueue the steps to snapshot the lock state to the lock task queue.
        let global = self.global();
        let task_manager = global.task_manager();
        let callback = callback_promise(&promise, self, task_manager.weblocks_task_source());
        if let Err(error) = self
            .storage()
            .send(WebLocksThreadMsg::Query(callback, self.origin()))
        {
            warn!("A Web Locks query could not be sent: {error}");
            promise.reject_error(
                cx,
                Error::InvalidState(Some("Web Locks are not available".to_owned())),
            );
        }

        // Step 6. Return promise.
        promise
    }
}

impl RoutedPromiseListener<crate::DomTypeHolder, LockManagerSnapshotMsg> for LockManager {
    fn handle_response(
        &self,
        cx: &mut JSContext,
        msg: LockManagerSnapshotMsg,
        promise: &RootedPromise,
    ) {
        let snapshot: LockManagerSnapshot = msg.convert();
        promise.resolve_native(cx, &snapshot);
    }
}
