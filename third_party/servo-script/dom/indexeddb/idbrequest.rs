/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

// Mercury: this file carries upstream code, hand-ported onto Servo 0.8, from LunarWerxs/servo, branch
// pr/indexeddb (author L0garithmic, MPL-2.0; the branch is AI-written): commit 4b07849e2 (the reply
// handler derives index records: `execute_async_indexed`) and c609fcd06 (failed requests are logged
// where they fail). Mercury's other changes to this file are marked `Mercury:` where they are.

#![cfg_attr(crown, allow(crown::jscontext_first_arg))]

use std::cell::Cell;
use std::sync::Arc;

use dom_struct::dom_struct;
use js::context::JSContext;
use js::conversions::ToJSValConvertible;
use js::jsapi::Heap;
use js::jsval::{DoubleValue, JSVal, NullValue, ObjectValue, UndefinedValue};
use js::rust::HandleValue;
use profile_traits::generic_callback::GenericCallback;
use script_bindings::cell::DomRefCell;
use script_bindings::reflector::{DomObject, reflect_dom_object};
use serde::{Deserialize, Serialize};
use servo_base::generic_channel::{GenericSend, SendError};
use storage_traits::indexeddb::{
    AsyncOperation, AsyncReadOnlyOperation, BackendError, BackendResult, IndexedDBKeyType,
    IndexedDBRecord, IndexedDBThreadMsg, IndexedDBTxnMode, PutItemResult, SyncOperation,
};
use stylo_atoms::Atom;

use crate::dom::bindings::codegen::Bindings::IDBRequestBinding::{
    IDBRequestMethods, IDBRequestReadyState,
};
use crate::dom::bindings::codegen::Bindings::IDBTransactionBinding::IDBTransactionMode;
use crate::dom::bindings::codegen::UnionTypes::IDBObjectStoreOrIDBIndexOrIDBCursor;
use crate::dom::bindings::error::{Error, Fallible, create_dom_exception};
use crate::dom::bindings::inheritance::Castable;
use crate::dom::bindings::refcounted::Trusted;
use crate::dom::bindings::reflector::DomGlobal;
use crate::dom::bindings::root::{Dom, DomRoot, MutNullableDom};
use crate::dom::bindings::structuredclone;
use crate::dom::domexception::DOMException;
use crate::dom::event::{Event, EventBubbles, EventCancelable};
use crate::dom::eventtarget::EventTarget;
use crate::dom::globalscope::GlobalScope;
use crate::dom::indexeddb::idbcursor::{IDBCursor, IterationParam, iterate_cursor};
use crate::dom::indexeddb::idbcursorwithvalue::IDBCursorWithValue;
use crate::dom::indexeddb::idbindex::{IDBIndex, IndexQueryKind, IndexQueryParam};
use crate::dom::indexeddb::idbobjectstore::IDBObjectStore;
use crate::dom::indexeddb::idbtransaction::IDBTransaction;
use crate::dom::indexeddb::key::{key_type_to_jsval, map_backend_error_to_request_error};
use crate::realms::enter_auto_realm;

/// <https://www.w3.org/TR/IndexedDB-3/#request-source>
///
/// Mercury: IDBRequest.source is an object store, an index or a cursor; it used to be an object
/// store only, so a request made through an index or a cursor reported the wrong source.
#[derive(JSTraceable, MallocSizeOf)]
#[cfg_attr(crown, crown::unrooted_must_root_lint::must_root)]
pub(crate) enum RequestSource {
    ObjectStore(Dom<IDBObjectStore>),
    Index(Dom<IDBIndex>),
    Cursor(Dom<IDBCursor>),
}

#[derive(Clone)]
struct RequestListener {
    request: Trusted<IDBRequest>,
    iteration_param: Option<IterationParam>,
    // Mercury: set when this request is an index query or a step of an index cursor, which the
    // backend cannot answer itself, see `IDBIndex::records_for`.
    index_param: Option<IndexQueryParam>,
    request_id: u64,
}

pub enum IdbResult {
    Key(IndexedDBKeyType),
    Keys(Vec<IndexedDBKeyType>),
    Value(Vec<u8>),
    Values(Vec<Vec<u8>>),
    Count(u64),
    Iterate(Vec<IndexedDBRecord>),
    Error(Error),
    None,
}

impl From<IndexedDBKeyType> for IdbResult {
    fn from(value: IndexedDBKeyType) -> Self {
        IdbResult::Key(value)
    }
}

impl From<Vec<IndexedDBKeyType>> for IdbResult {
    fn from(value: Vec<IndexedDBKeyType>) -> Self {
        IdbResult::Keys(value)
    }
}

impl From<Vec<u8>> for IdbResult {
    fn from(value: Vec<u8>) -> Self {
        IdbResult::Value(value)
    }
}

impl From<Vec<Vec<u8>>> for IdbResult {
    fn from(value: Vec<Vec<u8>>) -> Self {
        IdbResult::Values(value)
    }
}

impl From<PutItemResult> for IdbResult {
    fn from(value: PutItemResult) -> Self {
        match value {
            PutItemResult::Key(key) => Self::Key(key),
            PutItemResult::CannotOverwrite => Self::Error(Error::Constraint(None)),
        }
    }
}

impl From<Vec<IndexedDBRecord>> for IdbResult {
    fn from(value: Vec<IndexedDBRecord>) -> Self {
        Self::Iterate(value)
    }
}

impl From<()> for IdbResult {
    fn from(_value: ()) -> Self {
        Self::None
    }
}

impl<T> From<Option<T>> for IdbResult
where
    T: Into<IdbResult>,
{
    fn from(value: Option<T>) -> Self {
        match value {
            Some(value) => value.into(),
            None => IdbResult::None,
        }
    }
}

impl From<u64> for IdbResult {
    fn from(value: u64) -> Self {
        IdbResult::Count(value)
    }
}

impl RequestListener {
    fn send_request_handled(cx: &mut JSContext, transaction: &IDBTransaction, request_id: u64) {
        let global = transaction.global();
        // https://w3c.github.io/IndexedDB/#transaction-lifecycle
        // A transaction is inactive after control returns to the event loop and
        // when events are not being dispatched. We call this after dispatching
        // the request event, so the backend can reevaluate commit eligibility.
        let send_result = global.storage_threads().send(IndexedDBThreadMsg::Sync(
            SyncOperation::RequestHandled {
                origin: global.origin().immutable().clone(),
                db_name: String::from(transaction.get_db_name()),
                txn: transaction.get_serial_number(),
                request_id,
            },
        ));
        if send_result.is_err() {
            error!("Failed to send SyncOperation::RequestHandled");
        }
        transaction.mark_request_handled(request_id);

        // This request's result has been handled by script, the
        // transaction might finally be ready to auto-commit.
        transaction.maybe_commit(cx);
    }

    // https://www.w3.org/TR/IndexedDB-3/#async-execute-request
    // Implements Step 5.4
    fn handle_async_request_finished(&self, cx: &mut JSContext, result: BackendResult<IdbResult>) {
        let request = self.request.root();
        let global = request.global();

        // Mercury: a reply for a request that lost its transaction is dropped, not a panic of the
        // content process.
        let Some(transaction) = request.transaction.get() else {
            warn!("indexeddb: the reply to a request arrived after its transaction was cleared");
            return;
        };
        // Substep 1: Set the result of request to result.
        request.set_ready_state_done();

        let mut realm = enter_auto_realm(cx, &*request);
        let cx: &mut JSContext = &mut realm;
        rooted!(&in(cx) let mut answer = UndefinedValue());

        // Mercury: the request fails with the error the backend reported (a quota error, a
        // constraint error, an abort, any other failure of the backend), where it used to fail
        // with a DataError whatever went wrong.
        let data = match result {
            Ok(data) => data,
            Err(error) => {
                Self::handle_async_request_error(
                    &global,
                    cx,
                    request,
                    map_backend_error_to_request_error(error),
                    self.request_id,
                );
                return;
            },
        };

        {
            match data {
                IdbResult::Key(key) => key_type_to_jsval(cx, &key, answer.handle_mut()),
                IdbResult::Keys(keys) => {
                    rooted!(&in(cx) let mut array = vec![JSVal::default(); keys.len()]);
                    for (i, key) in keys.into_iter().enumerate() {
                        key_type_to_jsval(cx, &key, array.handle_mut_at(i));
                    }
                    array.to_jsval(cx, answer.handle_mut());
                },
                IdbResult::Value(serialized_data) => {
                    let result = postcard::from_bytes(&serialized_data)
                        .map_err(|_| Error::Data(None))
                        .and_then(|data| {
                            structuredclone::read(cx, &global, data, answer.handle_mut())
                        });
                    if let Err(e) = result {
                        warn!("Error reading structuredclone data");
                        Self::handle_async_request_error(&global, cx, request, e, self.request_id);
                        return;
                    };
                },
                IdbResult::Values(serialized_values) => {
                    rooted!(&in(cx) let mut values = vec![JSVal::default(); serialized_values.len()]);
                    for (i, serialized_data) in serialized_values.into_iter().enumerate() {
                        let result = postcard::from_bytes(&serialized_data)
                            .map_err(|_| Error::Data(None))
                            .and_then(|data| {
                                structuredclone::read(cx, &global, data, values.handle_mut_at(i))
                            });
                        if let Err(e) = result {
                            warn!("Error reading structuredclone data");
                            Self::handle_async_request_error(
                                &global,
                                cx,
                                request,
                                e,
                                self.request_id,
                            );
                            return;
                        };
                    }
                    values.to_jsval(cx, answer.handle_mut());
                },
                IdbResult::Count(count) => {
                    answer.handle_mut().set(DoubleValue(count as f64));
                },
                IdbResult::Iterate(records) => {
                    // Mercury: LunarWerxs pr/indexeddb 4b07849e2. An index query arrives here as
                    // the scan of the whole object store that owns the index, because the backend
                    // keeps no index entries: derive the records of the index first (see
                    // `IDBIndex::records_for`). A query that is not a cursor is answered from them
                    // directly; the records of a cursor step go to `iterate_cursor` below.
                    let mut answered_directly = false;
                    let records: Arc<Vec<IndexedDBRecord>> = match self.index_param.as_ref() {
                        Some(index_param) => {
                            // Mercury: a cursor step that has its records already (see
                            // `IDBCursor::execute_iteration`) does not derive them again; the first step
                            // derives them and the cursor keeps them (net-1 review, finding 3).
                            let records = match index_param.snapshot.as_ref() {
                                Some(snapshot) => snapshot.clone(),
                                None => {
                                    let index = index_param.index.root();
                                    let mut derived = index.records_for(
                                        cx,
                                        &global,
                                        records,
                                        index_param.needs_values,
                                    );
                                    derived
                                        .retain(|record| index_param.range.contains(&record.key));
                                    let derived = Arc::new(derived);
                                    if index_param.kind == IndexQueryKind::Cursor &&
                                        let Some(param) = self.iteration_param.as_ref()
                                    {
                                        param
                                            .cursor
                                            .root()
                                            .keep_snapshot(index_param.generation, derived.clone());
                                    }
                                    derived
                                },
                            };
                            if index_param.kind != IndexQueryKind::Cursor {
                                if let Err(e) =
                                    index_param.answer(cx, &global, &records, answer.handle_mut())
                                {
                                    Self::handle_async_request_error(
                                        &global,
                                        cx,
                                        request,
                                        e,
                                        self.request_id,
                                    );
                                    return;
                                }
                                answered_directly = true;
                            }
                            records
                        },
                        None => Arc::new(records),
                    };

                    if !answered_directly {
                        // Mercury: execute_async_indexed asserted that a cursor step has one.
                        let Some(param) = self.iteration_param.as_ref() else {
                            Self::handle_async_request_error(
                                &global,
                                cx,
                                request,
                                Error::InvalidState(Some(
                                    "A cursor step lost its iteration parameters".to_owned(),
                                )),
                                self.request_id,
                            );
                            return;
                        };
                        let cursor = match iterate_cursor(&global, cx, param, &records) {
                            Ok(cursor) => cursor,
                            Err(e) => {
                                warn!("Error reading structuredclone data");
                                Self::handle_async_request_error(
                                    &global,
                                    cx,
                                    request,
                                    e,
                                    self.request_id,
                                );
                                return;
                            },
                        };
                        if let Some(cursor) = cursor {
                            match cursor.downcast::<IDBCursorWithValue>() {
                                Some(cursor_with_value) => {
                                    answer.handle_mut().set(ObjectValue(
                                        *cursor_with_value.reflector().get_jsobject(),
                                    ));
                                },
                                None => {
                                    answer
                                        .handle_mut()
                                        .set(ObjectValue(*cursor.reflector().get_jsobject()));
                                },
                            }
                        } else {
                            answer.handle_mut().set(NullValue());
                        }
                    }
                },
                IdbResult::None => {
                    // no-op
                },
                IdbResult::Error(error) => {
                    // Substep 2
                    Self::handle_async_request_error(&global, cx, request, error, self.request_id);
                    return;
                },
            }

            // Substep 3.1: Set the result of request to answer.
            request.set_result(answer.handle());

            // Substep 3.2: Set the error of request to undefined
            request.set_error(cx, None);

            // https://w3c.github.io/IndexedDB/#fire-success-event
            // Step 1: Let event be the result of creating an event using Event.
            // Step 2: Set event’s type attribute to "success".
            // Step 3: Set event’s bubbles and cancelable attributes to false.
            let event = Event::new(
                cx,
                &global,
                Atom::from("success"),
                EventBubbles::DoesNotBubble,
                EventCancelable::NotCancelable,
            );

            // Step 5: Let legacyOutputDidListenersThrowFlag be initially false.
            let did_listeners_throw = Cell::new(false);
            // Step 6: If transaction’s state is inactive, then set transaction’s state to active.
            if transaction.is_inactive() {
                transaction.set_active_flag(true);
            }
            // Step 7: Dispatch event at request with legacyOutputDidListenersThrowFlag.
            event
                .upcast::<Event>()
                .fire_with_legacy_output_did_listeners_throw(
                    cx,
                    request.upcast(),
                    &did_listeners_throw,
                );
            // Step 8: If transaction’s state is active, then:
            if transaction.is_active() {
                // Step 8.1: Set transaction’s state to inactive.
                transaction.set_active_flag(false);
                // Step 8.2: If legacyOutputDidListenersThrowFlag is true, then run abort a
                // transaction with transaction and a newly created "AbortError" DOMException.
                if did_listeners_throw.get() {
                    transaction.initiate_abort(cx, Error::Abort(None));
                    transaction.request_backend_abort();
                }
            }
            transaction.request_finished();

            Self::send_request_handled(cx, &transaction, self.request_id);
        }
    }

    // https://www.w3.org/TR/IndexedDB-3/#async-execute-request
    // Implements Step 5.4.2
    fn handle_async_request_error(
        global: &GlobalScope,
        cx: &mut JSContext,
        request: DomRoot<IDBRequest>,
        error: Error,
        request_id: u64,
    ) {
        // Mercury: LunarWerxs pr/indexeddb c609fcd06. Log every failed request where it fails: a
        // DOMException carries no JavaScript stack, so a page that swallows the rejection leaves
        // nothing behind to find the cause by.
        warn!("indexeddb: request {request_id} failed: {error:?}");
        // Mercury: a request that lost its transaction has nothing left to abort or to report to.
        let Some(transaction) = request.transaction.get() else {
            return;
        };
        // Substep 1: Set the result of request to undefined.
        rooted!(&in(cx) let undefined = UndefinedValue());
        request.set_result(undefined.handle());

        // Substep 2: Set the error of request to result.
        request.set_error(cx, Some(error.clone()));

        // https://w3c.github.io/IndexedDB/#fire-error-event
        // Step 1: Let event be the result of creating an event using Event.
        // Step 2: Set event’s type attribute to "error".
        // Step 3: Set event’s bubbles and cancelable attributes to true.
        let event = Event::new(
            cx,
            global,
            Atom::from("error"),
            EventBubbles::Bubbles,
            EventCancelable::Cancelable,
        );

        // If result is an error and transaction’s state is committing, then run abort a
        // transaction with transaction and result, and terminate these steps.
        if transaction.is_committing() {
            transaction.initiate_abort(cx, error.clone());
            transaction.request_backend_abort();
        }
        // Step 5: Let legacyOutputDidListenersThrowFlag be initially false.
        let did_listeners_throw = Cell::new(false);
        // Step 6: If transaction’s state is inactive, then set transaction’s state to active.
        if transaction.is_inactive() {
            transaction.set_active_flag(true);
        }
        // Step 7: Dispatch event at request with legacyOutputDidListenersThrowFlag.
        let default_not_prevented = event
            .upcast::<Event>()
            .fire_with_legacy_output_did_listeners_throw(
                cx,
                request.upcast(),
                &did_listeners_throw,
            );
        // Step 8: If transaction’s state is active, then:
        if transaction.is_active() {
            // Step 8.1: Set transaction’s state to inactive.
            transaction.set_active_flag(false);
            // Step 8.2: If legacyOutputDidListenersThrowFlag is true, then run abort a transaction
            // with transaction and a newly created "AbortError" DOMException and terminate these steps.
            // NOTE: This is done even if event’s canceled flag is false.
            // NOTE: This means that if an error event is fired and any of the event handlers throw an
            // exception, transaction’s error property is set to an AbortError rather than request’s
            // error, even if preventDefault() is never called.
            if did_listeners_throw.get() {
                transaction.initiate_abort(cx, Error::Abort(None));
                transaction.request_backend_abort();
            } else if default_not_prevented {
                // Step 8.3: If event’s canceled flag is false, then run abort a transaction
                // using transaction and request’s error, and terminate these steps.
                transaction.initiate_abort(cx, error);
                transaction.request_backend_abort();
            }
        }
        transaction.request_finished();
        Self::send_request_handled(cx, &transaction, request_id);
    }
}

#[dom_struct]
pub struct IDBRequest {
    eventtarget: EventTarget,
    #[ignore_malloc_size_of = "mozjs"]
    result: Heap<JSVal>,
    error: MutNullableDom<DOMException>,
    // Mercury: an object store, an index or a cursor (it was an object store only).
    source: DomRefCell<Option<RequestSource>>,
    transaction: MutNullableDom<IDBTransaction>,
    ready_state: Cell<IDBRequestReadyState>,
}

impl IDBRequest {
    #[cfg_attr(crown, expect(crown::unrooted_must_root))]
    pub fn new_inherited() -> IDBRequest {
        IDBRequest {
            eventtarget: EventTarget::new_inherited(),

            result: Heap::default(),
            error: Default::default(),
            source: DomRefCell::new(None),
            transaction: Default::default(),
            ready_state: Cell::new(IDBRequestReadyState::Pending),
        }
    }

    pub fn new(cx: &mut JSContext, global: &GlobalScope) -> DomRoot<IDBRequest> {
        reflect_dom_object(cx, Box::new(IDBRequest::new_inherited()), global)
    }

    /// <https://www.w3.org/TR/IndexedDB-3/#request-source>
    ///
    /// Mercury: takes the source of any kind; the request of an index query has the index as
    /// its source and the request of `IDBCursor.update()` or `delete()` the cursor.
    pub(crate) fn set_source(&self, source: Option<RequestSource>) {
        *self.source.borrow_mut() = source;
    }

    pub fn set_ready_state_done(&self) {
        self.ready_state.set(IDBRequestReadyState::Done);
    }

    pub fn set_result(&self, result: HandleValue) {
        self.result.set(result.get());
    }

    pub fn set_error(&self, cx: &mut JSContext, error: Option<Error>) {
        if let Some(error) = error {
            if let Ok(exception) = create_dom_exception(cx, &self.global(), error) {
                self.error.set(Some(&exception));
            }
        } else {
            self.error.set(None);
        }
    }

    pub fn set_transaction(&self, transaction: &IDBTransaction) {
        self.transaction.set(Some(transaction));
    }

    pub fn clear_transaction(&self) {
        self.transaction.set(None);
    }

    pub fn unset_done_flag(&self) {
        self.ready_state.set(IDBRequestReadyState::Pending);
    }

    fn is_done(&self) -> bool {
        self.ready_state.get() == IDBRequestReadyState::Done
    }

    pub(crate) fn transaction(&self) -> Option<DomRoot<IDBTransaction>> {
        self.transaction.get()
    }

    // https://www.w3.org/TR/IndexedDB-3/#asynchronously-execute-a-request
    pub fn execute_async<T, F>(
        cx: &mut JSContext,
        source: &IDBObjectStore,
        operation_fn: F,
        request: Option<DomRoot<IDBRequest>>,
        iteration_param: Option<IterationParam>,
    ) -> Fallible<DomRoot<IDBRequest>>
    where
        T: Into<IdbResult> + for<'a> Deserialize<'a> + Serialize + Send + Sync + 'static,
        F: FnOnce(GenericCallback<BackendResult<T>>) -> AsyncOperation,
    {
        Self::execute_async_indexed(cx, source, operation_fn, request, iteration_param, None)
    }

    /// As [`execute_async`](Self::execute_async), for a request whose reply has to be reinterpreted
    /// through an index before script sees it: `index_param` says how (see `IDBIndex::records_for`).
    ///
    /// Mercury: LunarWerxs pr/indexeddb 4b07849e2.
    pub fn execute_async_indexed<T, F>(
        cx: &mut JSContext,
        source: &IDBObjectStore,
        operation_fn: F,
        request: Option<DomRoot<IDBRequest>>,
        iteration_param: Option<IterationParam>,
        index_param: Option<IndexQueryParam>,
    ) -> Fallible<DomRoot<IDBRequest>>
    where
        T: Into<IdbResult> + for<'a> Deserialize<'a> + Serialize + Send + Sync + 'static,
        F: FnOnce(GenericCallback<BackendResult<T>>) -> AsyncOperation,
    {
        // Step 1: Let transaction be the transaction associated with source.
        let transaction = source.transaction();
        let global = transaction.global();
        // Step 2: Assert: transaction is active.
        if !transaction.is_active() || !transaction.is_usable() {
            return Err(Error::TransactionInactive(None));
        }

        let request_id = transaction.allocate_request_id();

        // Step 3: If request was not given, let request be a new request with source as source.
        let request = request.unwrap_or_else(|| {
            let new_request = IDBRequest::new(cx, &global);
            new_request.set_source(Some(RequestSource::ObjectStore(Dom::from_ref(source))));
            new_request.set_transaction(&transaction);
            new_request
        });

        // Step 4: Add request to the end of transaction’s request list.
        transaction.add_request(&request);

        // Step 5: Run the operation, and queue a returning task in parallel
        // the result will be put into `receiver`
        let transaction_mode = match transaction.get_mode() {
            IDBTransactionMode::Readonly => IndexedDBTxnMode::Readonly,
            IDBTransactionMode::Readwrite => IndexedDBTxnMode::Readwrite,
            IDBTransactionMode::Versionchange => IndexedDBTxnMode::Versionchange,
        };

        let response_listener = RequestListener {
            request: Trusted::new(&request),
            iteration_param: iteration_param.clone(),
            index_param: index_param.clone(),
            request_id,
        };

        let task_source = global
            .task_manager()
            .database_access_task_source()
            .to_sendable();

        let closure = move |message: Result<BackendResult<T>, SendError>| {
            let response_listener = response_listener.clone();
            // Mercury: a reply that cannot be received (the storage thread went away, or the
            // message did not decode) fails the request; it used to panic the content process.
            let result = match message {
                Ok(result) => result,
                Err(error) => {
                    error!("The reply to an IndexedDB request could not be received: {error:?}");
                    Err(BackendError::DbErr(
                        "The reply of the storage thread could not be received".to_owned(),
                    ))
                },
            };
            task_source.queue(task!(request_callback: move |cx| {
                response_listener.handle_async_request_finished(
                    cx,
                    result.inspect_err(|e| {
                        if let BackendError::DbErr(e) = e {
                            error!("Error in IndexedDB operation: {}", e);
                        }
                    }).map(|t| t.into()),
                );
            }));
        };
        // Mercury: no panic when the callback cannot be created. The request is taken out of the
        // books of its transaction again, so that the transaction can still finish.
        let callback = match GenericCallback::new(closure) {
            Ok(callback) => callback,
            Err(error) => {
                error!("Could not create the callback of an IndexedDB request: {error:?}");
                transaction.request_finished();
                transaction.mark_request_handled(request_id);
                return Err(Error::Operation(Some(
                    "The storage thread cannot be reached".to_owned(),
                )));
            },
        };
        let operation = operation_fn(callback);
        // Mercury: a request that writes makes the index records that a cursor kept stale (net-1 review,
        // finding 3).
        if matches!(operation, AsyncOperation::ReadWrite(_)) {
            source.note_write();
        }

        if matches!(
            operation,
            AsyncOperation::ReadOnly(AsyncReadOnlyOperation::Iterate { .. })
        ) {
            // Mercury: an Iterate is a cursor step, which needs an iteration_param, or an index
            // query, which scans the object store and derives the answer itself (index_param).
            assert!(
                iteration_param.is_some() || index_param.is_some(),
                "iteration_param or index_param must be provided for Iterate"
            );
        } else {
            assert!(
                iteration_param.is_none(),
                "iteration_param should not be provided for operation other than Iterate"
            );
        }

        // Start is a backend database task (spec). Script does not model it with a
        // separate queued task, backend scheduling decides when requests begin.
        // Mercury: no panic when the storage thread cannot be reached; same clean-up as above.
        if transaction
            .global()
            .storage_threads()
            .send(IndexedDBThreadMsg::Async(
                global.origin().immutable().clone(),
                String::from(transaction.get_db_name()),
                String::from(source.get_name()),
                transaction.get_serial_number(),
                request_id,
                transaction_mode,
                operation,
            ))
            .is_err()
        {
            error!("Could not send an IndexedDB request to the storage thread");
            transaction.request_finished();
            transaction.mark_request_handled(request_id);
            return Err(Error::Operation(Some(
                "The storage thread cannot be reached".to_owned(),
            )));
        }

        // Step 6
        Ok(request)
    }
}

impl IDBRequestMethods<crate::DomTypeHolder> for IDBRequest {
    /// <https://www.w3.org/TR/IndexedDB-3/#dom-idbrequest-result>
    fn GetResult(
        &self,
        _cx: &mut JSContext,
        mut val: js::rust::MutableHandle<'_, js::jsapi::Value>,
    ) -> Fallible<()> {
        // Step 1. If this's done flag is false, then throw an "InvalidStateError" DOMException.
        if !self.is_done() {
            return Err(Error::InvalidState(Some(
                "Cannot get result on a request that is still pending.".into(),
            )));
        }

        // Step 2. Return this's result, or undefined if the request resulted in an error.
        val.set(self.result.get());
        Ok(())
    }

    /// <https://www.w3.org/TR/IndexedDB-3/#dom-idbrequest-error>
    fn GetError(&self) -> Fallible<Option<DomRoot<DOMException>>> {
        // Step 1. If this's done flag is false, then throw an "InvalidStateError" DOMException.
        if !self.is_done() {
            return Err(Error::InvalidState(Some(
                "Cannot get error on a request that is still pending.".into(),
            )));
        }

        // Step 2. Return this's error, or null if no error occurred.
        Ok(self.error.get())
    }

    /// <https://www.w3.org/TR/IndexedDB-3/#dom-idbrequest-source>
    fn GetSource(&self) -> Option<IDBObjectStoreOrIDBIndexOrIDBCursor> {
        // Mercury: an object store, an index or a cursor, as the specification says.
        self.source.borrow().as_ref().map(|source| match source {
            RequestSource::ObjectStore(store) => {
                IDBObjectStoreOrIDBIndexOrIDBCursor::IDBObjectStore(store.as_rooted())
            },
            RequestSource::Index(index) => {
                IDBObjectStoreOrIDBIndexOrIDBCursor::IDBIndex(index.as_rooted())
            },
            RequestSource::Cursor(cursor) => {
                IDBObjectStoreOrIDBIndexOrIDBCursor::IDBCursor(cursor.as_rooted())
            },
        })
    }

    /// <https://www.w3.org/TR/IndexedDB-3/#dom-idbrequest-transaction>
    fn GetTransaction(&self) -> Option<DomRoot<IDBTransaction>> {
        self.transaction.get()
    }

    /// <https://www.w3.org/TR/IndexedDB-3/#dom-idbrequest-readystate>
    fn ReadyState(&self) -> IDBRequestReadyState {
        self.ready_state.get()
    }

    // https://www.w3.org/TR/IndexedDB-3/#dom-idbrequest-onsuccess
    event_handler!(success, GetOnsuccess, SetOnsuccess);

    // https://www.w3.org/TR/IndexedDB-3/#dom-idbrequest-onerror
    event_handler!(error, GetOnerror, SetOnerror);
}
