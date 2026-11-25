/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

// Mercury: this file carries upstream code, hand-ported onto Servo 0.8, from LunarWerxs/servo, branch
// pr/indexeddb (author L0garithmic, MPL-2.0; the branch is AI-written, Co-Authored-By Claude Fable 5.1):
//   4b07849e2 "indexeddb: implement IDBIndex queries, and fix an index cursor losing its position"
//   098e8a02c "indexeddb: do not copy stored values into index records that never read them"
//   c609fcd06 "indexeddb: require a key for index get/getKey, and log failed requests at the source"
// (IDBIndex.get, getKey, getAll, getAllKeys and count, the records of an index derived in script, the
// index cursor that keeps asking through its index, no value copies for key-only queries, and `get` and
// `getKey` requiring a key). Mercury's other changes to this file are marked `Mercury:` where they are.
use std::cmp::Ordering;
use std::sync::Arc;

use dom_struct::dom_struct;
use js::context::JSContext;
use js::conversions::ToJSValConvertible;
use js::gc::MutableHandleValue;
use js::jsval::{DoubleValue, JSVal, UndefinedValue};
use js::rust::HandleValue;
use script_bindings::cell::DomRefCell;
use script_bindings::codegen::GenericBindings::IDBIndexBinding::IDBIndexMethods;
use script_bindings::codegen::GenericBindings::IDBTransactionBinding::IDBTransactionMode;
use script_bindings::domstring::TracedDOMString;
use script_bindings::error::{Error, ErrorResult, Fallible};
use script_bindings::reflector::{Reflector, reflect_dom_object};
use script_bindings::str::DOMString;
use storage_traits::indexeddb::{
    AsyncOperation, AsyncReadOnlyOperation, IndexedDBKeyRange, IndexedDBKeyType, IndexedDBRecord,
    UniqueIndexKeys,
};

use crate::dom::bindings::codegen::Bindings::IDBCursorBinding::IDBCursorDirection;
use crate::dom::bindings::refcounted::Trusted;
use crate::dom::bindings::reflector::DomGlobal;
use crate::dom::bindings::root::{Dom, DomRoot};
use crate::dom::bindings::structuredclone;
use crate::dom::globalscope::GlobalScope;
use crate::dom::idbobjectstore::KeyPath;
use crate::dom::indexeddb::idbcursor::{IDBCursor, IterationParam, ObjectStoreOrIndex};
use crate::dom::indexeddb::idbcursorwithvalue::IDBCursorWithValue;
use crate::dom::indexeddb::idbobjectstore::IDBObjectStore;
use crate::dom::indexeddb::idbrequest::{IDBRequest, RequestSource};
use crate::dom::indexeddb::key::{
    ExtractionResult, clear_pending_exception, convert_value_to_key_range, extract_key,
    index_keys_of, key_type_to_jsval,
};

#[dom_struct]
pub(crate) struct IDBIndex {
    reflector_: Reflector,
    object_store: Dom<IDBObjectStore>,
    name: DomRefCell<TracedDOMString>,
    multi_entry: bool,
    unique: bool,
    key_path: KeyPath,
}

impl IDBIndex {
    pub fn new_inherited(
        object_store: &IDBObjectStore,
        name: DOMString,
        multi_entry: bool,
        unique: bool,
        key_path: KeyPath,
    ) -> IDBIndex {
        IDBIndex {
            reflector_: Reflector::new(),
            object_store: Dom::from_ref(object_store),
            name: DomRefCell::new(name.traced()),
            multi_entry,
            unique,
            key_path,
        }
    }

    pub fn new(
        cx: &mut JSContext,
        global: &GlobalScope,
        object_store: &IDBObjectStore,
        name: DOMString,
        multi_entry: bool,
        unique: bool,
        key_path: KeyPath,
    ) -> DomRoot<IDBIndex> {
        reflect_dom_object(
            cx,
            Box::new(IDBIndex::new_inherited(
                object_store,
                name,
                multi_entry,
                unique,
                key_path,
            )),
            global,
        )
    }

    pub(crate) fn object_store(&self) -> DomRoot<IDBObjectStore> {
        DomRoot::from_ref(&self.object_store)
    }

    pub(crate) fn verify_not_deleted(&self) -> ErrorResult {
        let stored_name = self.name.borrow().clone().root();
        let transaction = self.object_store.transaction();
        if !self.object_store.has_index(&stored_name) ||
            !transaction
                .get_db()
                .object_store_exists(&self.object_store.get_name())
        {
            return Err(Error::InvalidState(Some(
                "Index or its object store has been deleted".to_owned(),
            )));
        }
        Ok(())
    }

    /// <https://www.w3.org/TR/IndexedDB-3/#dom-idbindex-opencursor>
    /// <https://www.w3.org/TR/IndexedDB-3/#dom-idbindex-openkeycursor>
    fn open_cursor(
        &self,
        cx: &mut JSContext,
        query: HandleValue,
        direction: IDBCursorDirection,
        key_only: bool,
    ) -> Fallible<DomRoot<IDBRequest>> {
        // Step 1. Let transaction be this’s object store's transaction.
        let transaction = self.object_store.transaction();

        // Step 2. If index or its object store has been deleted, throw an "InvalidStateError" DOMException.
        self.verify_not_deleted()?;

        // Step 3. If transaction is not active, throw a "TransactionInactiveError" DOMException.
        if !transaction.is_active() || !transaction.is_usable() {
            return Err(Error::TransactionInactive(Some(
                "Transaction is not active".to_owned(),
            )));
        }

        // Step 4. Let range be the result of running the steps to convert a value to a key range with query.
        let range = convert_value_to_key_range(cx, query, Some(false))?;

        // Step 5. Let cursor be a new cursor with transaction, direction, source: Index, range.
        let cursor = if key_only {
            IDBCursor::new(
                cx,
                &self.global(),
                &transaction,
                direction,
                false,
                ObjectStoreOrIndex::Index(Dom::from_ref(self)),
                range.clone(),
                key_only,
            )
        } else {
            DomRoot::upcast(IDBCursorWithValue::new(
                cx,
                &self.global(),
                &transaction,
                direction,
                false,
                ObjectStoreOrIndex::Index(Dom::from_ref(self)),
                range.clone(),
                key_only,
            ))
        };

        // Step 6. Asynchronously execute a request with iteration_param.
        let iteration_param = IterationParam {
            cursor: Trusted::new(&cursor),
            key: None,
            primary_key: None,
            count: None,
        };

        // Mercury: the records of an index are not in the backend (see `records_for`), so it is asked
        // for the whole object store and the reply handler of the request derives the index's records
        // and applies `range` to them. The cursor does the same on every step, see `IDBCursor`.
        // A new cursor has derived nothing yet: this first step scans the store.
        let index_param = IndexQueryParam {
            index: Trusted::new(self),
            range,
            count: None,
            kind: IndexQueryKind::Cursor,
            needs_values: !key_only,
            generation: self.object_store.write_generation(),
            snapshot: None,
        };

        IDBRequest::execute_async_indexed(
            cx,
            &self.object_store,
            |callback| {
                AsyncOperation::ReadOnly(AsyncReadOnlyOperation::Iterate {
                    callback,
                    key_range: IndexedDBKeyRange::default(),
                })
            },
            None,
            Some(iteration_param),
            Some(index_param),
        )
        .inspect(|request| {
            // Mercury: the source of the request is this index.
            request.set_source(Some(RequestSource::Index(Dom::from_ref(self))));
            cursor.set_request(request)
        })
    }

    /// The checks every index query makes before it creates a request.
    ///
    /// Mercury: shared by get, getKey, getAll, getAllKeys and count.
    fn check_usable(&self) -> ErrorResult {
        // Step: If index or its object store has been deleted, throw an "InvalidStateError" DOMException.
        self.verify_not_deleted()?;

        // Step: If transaction is not active, throw a "TransactionInactiveError" DOMException.
        let transaction = self.object_store.transaction();
        if !transaction.is_active() || !transaction.is_usable() {
            return Err(Error::TransactionInactive(Some(
                "Transaction is not active".to_owned(),
            )));
        }
        Ok(())
    }

    /// Runs an index query that is not a cursor: get, getKey, getAll, getAllKeys and count.
    ///
    /// Mercury: Servo's IndexedDB backend keeps no index entries (its `index_data` and
    /// `unique_index_data` tables are created and cleared but never written) and cannot derive them,
    /// because it stores values as opaque structured-clone blobs. Until it maintains real indexes the
    /// records are derived here, in the reply handler of the request (see `records_for`): the backend
    /// is asked for the whole object store. That is correct but linear in the size of the store.
    fn query(
        &self,
        cx: &mut JSContext,
        query: HandleValue,
        kind: IndexQueryKind,
        count: Option<u32>,
        needs_values: bool,
    ) -> Fallible<DomRoot<IDBRequest>> {
        self.check_usable()?;

        // The range applies to the keys of this index, not to those of the object store. get() and
        // getKey() ask for one record and so require a key (a "DataError" DOMException otherwise);
        // the others treat an absent query as the whole index.
        let null_disallowed = matches!(kind, IndexQueryKind::Get | IndexQueryKind::GetKey);
        let range = convert_value_to_key_range(cx, query, Some(null_disallowed))?;

        let index_param = IndexQueryParam {
            index: Trusted::new(self),
            range,
            count,
            kind,
            needs_values,
            generation: self.object_store.write_generation(),
            snapshot: None,
        };

        IDBRequest::execute_async_indexed(
            cx,
            &self.object_store,
            |callback| {
                AsyncOperation::ReadOnly(AsyncReadOnlyOperation::Iterate {
                    callback,
                    key_range: IndexedDBKeyRange::default(),
                })
            },
            None,
            None,
            Some(index_param),
        )
        .inspect(|request| {
            // Mercury: the source of the request is this index.
            request.set_source(Some(RequestSource::Index(Dom::from_ref(self))));
        })
    }

    // Mercury: the keys that `value` has in this index, when the index is unique (net-1 review, finding 3): one
    // key, one per member of a multiEntry key, or none when the value has no valid key for it. A value whose key
    // extraction throws has none either (the exception is not the put's).
    pub(crate) fn unique_index_keys_of(
        &self,
        cx: &mut JSContext,
        value: HandleValue,
    ) -> Option<UniqueIndexKeys> {
        if !self.unique {
            return None;
        }
        let keys = match extract_key(cx, value, &self.key_path, Some(self.multi_entry)) {
            Ok(ExtractionResult::Key(key)) => index_keys_of(key, self.multi_entry),
            Ok(ExtractionResult::Invalid | ExtractionResult::Failure) => Vec::new(),
            Err(_) => {
                clear_pending_exception(cx);
                Vec::new()
            },
        };
        Some(UniqueIndexKeys {
            index_name: String::from(self.Name()),
            keys,
        })
    }

    /// The records of this index, derived from the records of its object store: the key path is
    /// evaluated over each stored value, a record whose value has no valid key for the index is
    /// not in it, a multiEntry key gives one record per member, and the result is sorted into
    /// index order (key, then primary key). A unique index keeps the first record of a key.
    ///
    /// `needs_values` is false when only keys are wanted: stored values are then not copied into
    /// records that nobody reads.
    ///
    /// <https://www.w3.org/TR/IndexedDB-3/#index-referenced-value>
    pub(crate) fn records_for(
        &self,
        cx: &mut JSContext,
        global: &GlobalScope,
        store_records: Vec<IndexedDBRecord>,
        needs_values: bool,
    ) -> Vec<IndexedDBRecord> {
        let mut entries = Vec::with_capacity(store_records.len());

        for record in store_records {
            rooted!(&in(cx) let mut value = UndefinedValue());
            let read = postcard::from_bytes(&record.value)
                .map_err(|_| ())
                .and_then(|data| {
                    structuredclone::read(cx, global, data, value.handle_mut()).map_err(|_| ())
                });
            if read.is_err() {
                warn!("indexeddb: a stored value could not be read to build an index");
                clear_pending_exception(cx);
                continue;
            }

            // If extracting the key throws, or the key is invalid, or the value has none, the record
            // is not in the index (the exception does not propagate).
            let extracted = extract_key(cx, value.handle(), &self.key_path, Some(self.multi_entry));
            let keys = match extracted {
                Ok(ExtractionResult::Key(key)) => index_keys_of(key, self.multi_entry),
                Ok(ExtractionResult::Invalid | ExtractionResult::Failure) => continue,
                Err(_) => {
                    clear_pending_exception(cx);
                    continue;
                },
            };
            entries.push((record, keys));
        }

        build_index_records(entries, self.unique, needs_values)
    }
}

/// What an index query hands back once the records of the index have been derived.
///
/// Mercury: from LunarWerxs pr/indexeddb 4b07849e2.
#[derive(Clone, Copy, PartialEq)]
pub(crate) enum IndexQueryKind {
    /// The referenced value of the first record in the range.
    Get,
    /// The primary key of the first record in the range.
    GetKey,
    /// The referenced value of every record in the range.
    GetAll,
    /// The primary key of every record in the range.
    GetAllKeys,
    /// How many records are in the range.
    Count,
    /// The records go to a cursor instead of being the answer.
    Cursor,
}

/// Everything the reply handler of an `IDBRequest` needs to turn the scan of an object store into
/// the answer to one index query (or into the records one cursor step chooses from).
///
/// Mercury: from LunarWerxs pr/indexeddb 4b07849e2 and 098e8a02c.
#[derive(Clone)]
pub(crate) struct IndexQueryParam {
    pub(crate) index: Trusted<IDBIndex>,
    pub(crate) range: IndexedDBKeyRange,
    pub(crate) count: Option<u32>,
    pub(crate) kind: IndexQueryKind,
    /// False when only keys are wanted (getKey, getAllKeys, count, a key cursor). An index query
    /// rebuilds its records from a full scan of the object store, and copying every stored value
    /// into records nobody reads is the difference between a cheap query and an expensive one.
    pub(crate) needs_values: bool,
    /// The write generation of the object store when the request was made (see
    /// `IDBObjectStore::write_generation`).
    ///
    /// Mercury: net-1 review, finding 3.
    pub(crate) generation: u64,
    /// For a cursor step: the records of the index that the cursor derived at the generation above, when
    /// it has them. The step then asks the backend for nothing (see `execute_iteration`) and reads them
    /// instead of deriving the index again from the whole object store.
    pub(crate) snapshot: Option<Arc<Vec<IndexedDBRecord>>>,
}

impl IndexQueryParam {
    /// Builds the answer of a query that is not a cursor from the records of the index that are in
    /// its range, honouring the `count` limit.
    pub(crate) fn answer(
        &self,
        cx: &mut JSContext,
        global: &GlobalScope,
        records: &[IndexedDBRecord],
        mut answer: MutableHandleValue,
    ) -> Result<(), Error> {
        let limit = match self.count {
            Some(0) | None => records.len(),
            Some(count) => (count as usize).min(records.len()),
        };

        match self.kind {
            IndexQueryKind::Get => match records.first() {
                Some(record) => read_value(cx, global, record, answer)?,
                // A query that matches nothing resolves with undefined, not with an error.
                None => answer.set(UndefinedValue()),
            },
            IndexQueryKind::GetKey => match records.first() {
                Some(record) => key_type_to_jsval(cx, &record.primary_key, answer),
                None => answer.set(UndefinedValue()),
            },
            IndexQueryKind::GetAll => {
                rooted!(&in(cx) let mut values = vec![JSVal::default(); limit]);
                for (i, record) in records.iter().take(limit).enumerate() {
                    read_value(cx, global, record, values.handle_mut_at(i))?;
                }
                values.to_jsval(cx, answer);
            },
            IndexQueryKind::GetAllKeys => {
                rooted!(&in(cx) let mut keys = vec![JSVal::default(); limit]);
                for (i, record) in records.iter().take(limit).enumerate() {
                    key_type_to_jsval(cx, &record.primary_key, keys.handle_mut_at(i));
                }
                keys.to_jsval(cx, answer);
            },
            IndexQueryKind::Count => answer.set(DoubleValue(records.len() as f64)),
            IndexQueryKind::Cursor => {
                return Err(Error::InvalidState(Some(
                    "A cursor query does not answer directly".to_owned(),
                )));
            },
        }
        Ok(())
    }
}

/// Reads the stored value of a record into a JavaScript value.
fn read_value(
    cx: &mut JSContext,
    global: &GlobalScope,
    record: &IndexedDBRecord,
    answer: MutableHandleValue,
) -> Result<(), Error> {
    let data = postcard::from_bytes(&record.value).map_err(|_| {
        warn!(
            "indexeddb: index record value failed to decode ({} bytes)",
            record.value.len()
        );
        Error::Data(None)
    })?;
    structuredclone::read(cx, global, data, answer)
        .map(|_ports| ())
        .inspect_err(|_| warn!("indexeddb: index record value failed structured deserialize"))
}

/// The records of an index, from the stored records and the index keys extracted from each of them,
/// in index order: by key, then by primary key. A record with no keys is not in the index. A unique
/// index keeps the first record of each key.
///
/// With `needs_values` false the records carry no value; otherwise each carries a copy of the stored
/// value (the last key of a record takes the stored value itself).
///
/// Mercury: the pure part of `IDBIndex::records_for`, split out so that it can be tested without a
/// JavaScript context.
fn build_index_records(
    entries: Vec<(IndexedDBRecord, Vec<IndexedDBKeyType>)>,
    unique: bool,
    needs_values: bool,
) -> Vec<IndexedDBRecord> {
    let mut out: Vec<IndexedDBRecord> = Vec::new();

    for (record, keys) in entries {
        let IndexedDBRecord {
            primary_key, value, ..
        } = record;
        let mut stored_value = needs_values.then_some(value);
        let mut keys = keys.into_iter().peekable();
        while let Some(key) = keys.next() {
            let copy = if keys.peek().is_some() {
                stored_value.clone()
            } else {
                stored_value.take()
            };
            out.push(IndexedDBRecord {
                key,
                primary_key: primary_key.clone(),
                value: copy.unwrap_or_default(),
            });
        }
    }

    // Index order is by index key, then by primary key.
    out.sort_by(|a, b| {
        a.key
            .partial_cmp(&b.key)
            .unwrap_or(Ordering::Equal)
            .then_with(|| {
                a.primary_key
                    .partial_cmp(&b.primary_key)
                    .unwrap_or(Ordering::Equal)
            })
    });

    // A unique index keeps only the first record for each key.
    if unique {
        out.dedup_by(|later, earlier| later.key == earlier.key);
    }
    out
}

impl IDBIndexMethods<crate::DomTypeHolder> for IDBIndex {
    /// <https://www.w3.org/TR/IndexedDB/#dom-idbindex-name>
    fn Name(&self) -> DOMString {
        self.name.borrow().clone().root()
    }

    /// <https://www.w3.org/TR/IndexedDB/#ref-for-dom-idbindex-name%E2%91%A2>
    fn SetName(&self, name: DOMString) -> ErrorResult {
        // Step 1: Let name be the given value.
        // Step 2: Let transaction be this’s transaction.
        let transaction = self.object_store.transaction();

        // Step 3: Let index be this’s index.
        // We do not have an explicit object representing the underlying index.

        // Step 4: If transaction is not an upgrade transaction, throw an "InvalidStateError" DOMException.
        if transaction.get_mode() != IDBTransactionMode::Versionchange {
            return Err(Error::InvalidState(Some(
                "Transaction is not an upgrade transaction".to_owned(),
            )));
        }

        // Step 5: If transaction’s state is not active, then throw a "TransactionInactiveError" DOMException.
        if !transaction.is_active() {
            return Err(Error::TransactionInactive(Some(
                "Transaction is not active while updating index name".to_owned(),
            )));
        }

        // Step 6: If index or index’s object store has been deleted, throw an "InvalidStateError" DOMException.
        let mut stored_name = self.name.borrow_mut();
        if !self.object_store.has_index(&stored_name) ||
            !transaction
                .get_db()
                .object_store_exists(&self.object_store.get_name())
        {
            return Err(Error::InvalidState(Some(
                "Index or its object store has been deleted".to_owned(),
            )));
        }

        // Step 7: If index’s name is equal to name, terminate these steps.
        if *stored_name == *name {
            return Ok(());
        }

        // Step 8: If an index named name already exists in index’s object store, throw a "ConstraintError" DOMException.
        if self.object_store.has_index(&name) {
            return Err(Error::Constraint(Some(
                "An index with the given name already exists".to_owned(),
            )));
        }

        // Step 9: Set index’s name to name.
        // Mercury: `?`: the rename can fail now, with nothing changed, instead of going on locally.
        self.object_store.rename_index(&stored_name, &name)?;

        // Step 10: Set this’s name to name.
        *stored_name = name.traced();
        Ok(())
    }

    /// <https://www.w3.org/TR/IndexedDB/#dom-idbindex-objectstore>
    fn ObjectStore(&self) -> DomRoot<IDBObjectStore> {
        self.object_store.as_rooted()
    }

    /// <https://www.w3.org/TR/IndexedDB/#dom-idbindex-multientry>
    fn MultiEntry(&self) -> bool {
        self.multi_entry
    }

    /// <https://www.w3.org/TR/IndexedDB/#dom-idbindex-unique>
    fn Unique(&self) -> bool {
        self.unique
    }

    /// <https://www.w3.org/TR/IndexedDB/#dom-idbindex-keypath>
    fn KeyPath(&self, cx: &mut JSContext, retval: MutableHandleValue) {
        match &self.key_path {
            KeyPath::String(string) => {
                string.to_jsval(cx, retval);
            },
            KeyPath::StringSequence(sequence) => {
                sequence.to_jsval(cx, retval);
            },
        }
    }

    /// <https://www.w3.org/TR/IndexedDB-3/#dom-idbindex-get>
    fn Get(&self, cx: &mut JSContext, query: HandleValue) -> Fallible<DomRoot<IDBRequest>> {
        self.query(cx, query, IndexQueryKind::Get, None, true)
    }

    /// <https://www.w3.org/TR/IndexedDB-3/#dom-idbindex-getkey>
    fn GetKey(&self, cx: &mut JSContext, query: HandleValue) -> Fallible<DomRoot<IDBRequest>> {
        self.query(cx, query, IndexQueryKind::GetKey, None, false)
    }

    /// <https://www.w3.org/TR/IndexedDB-3/#dom-idbindex-getall>
    fn GetAll(
        &self,
        cx: &mut JSContext,
        query: HandleValue,
        count: Option<u32>,
    ) -> Fallible<DomRoot<IDBRequest>> {
        self.query(cx, query, IndexQueryKind::GetAll, count, true)
    }

    /// <https://www.w3.org/TR/IndexedDB-3/#dom-idbindex-getallkeys>
    fn GetAllKeys(
        &self,
        cx: &mut JSContext,
        query: HandleValue,
        count: Option<u32>,
    ) -> Fallible<DomRoot<IDBRequest>> {
        self.query(cx, query, IndexQueryKind::GetAllKeys, count, false)
    }

    /// <https://www.w3.org/TR/IndexedDB-3/#dom-idbindex-count>
    fn Count(&self, cx: &mut JSContext, query: HandleValue) -> Fallible<DomRoot<IDBRequest>> {
        self.query(cx, query, IndexQueryKind::Count, None, false)
    }

    /// <https://www.w3.org/TR/IndexedDB-3/#dom-idbindex-opencursor>
    fn OpenCursor(
        &self,
        cx: &mut JSContext,
        query: HandleValue,
        direction: IDBCursorDirection,
    ) -> Fallible<DomRoot<IDBRequest>> {
        self.open_cursor(cx, query, direction, false)
    }

    /// <https://www.w3.org/TR/IndexedDB-3/#dom-idbindex-openkeycursor>
    fn OpenKeyCursor(
        &self,
        cx: &mut JSContext,
        query: HandleValue,
        direction: IDBCursorDirection,
    ) -> Fallible<DomRoot<IDBRequest>> {
        self.open_cursor(cx, query, direction, true)
    }
}

// Mercury: tests for the pure part of deriving the records of an index (no JavaScript context).
#[cfg(test)]
mod tests {
    use storage_traits::indexeddb::{IndexedDBKeyType, IndexedDBRecord};

    use super::build_index_records;

    fn number(n: f64) -> IndexedDBKeyType {
        IndexedDBKeyType::Number(n)
    }

    fn string(s: &str) -> IndexedDBKeyType {
        IndexedDBKeyType::String(s.to_owned())
    }

    /// A stored record with primary key `id` whose value is the bytes of `id`.
    fn stored(id: u8) -> IndexedDBRecord {
        IndexedDBRecord {
            key: number(f64::from(id)),
            primary_key: number(f64::from(id)),
            value: vec![id],
        }
    }

    /// (index key, primary key) of each record, in order.
    fn pairs(records: &[IndexedDBRecord]) -> Vec<(IndexedDBKeyType, IndexedDBKeyType)> {
        records
            .iter()
            .map(|record| (record.key.clone(), record.primary_key.clone()))
            .collect()
    }

    #[test]
    fn records_come_out_in_key_then_primary_key_order() {
        let entries = vec![
            (stored(1), vec![string("oslo")]),
            (stored(2), vec![string("berlin")]),
            (stored(3), vec![string("oslo")]),
            (stored(4), vec![string("cairo")]),
        ];
        let records = build_index_records(entries, false, true);
        assert_eq!(
            pairs(&records),
            vec![
                (string("berlin"), number(2.0)),
                (string("cairo"), number(4.0)),
                (string("oslo"), number(1.0)),
                (string("oslo"), number(3.0)),
            ]
        );
    }

    #[test]
    fn a_multientry_record_is_in_the_index_once_per_key() {
        // Record 1 has the tags a and b, record 2 has b and c, record 3 has none.
        let entries = vec![
            (stored(1), vec![string("a"), string("b")]),
            (stored(2), vec![string("b"), string("c")]),
            (stored(3), vec![]),
        ];
        let records = build_index_records(entries, false, true);
        assert_eq!(
            pairs(&records),
            vec![
                (string("a"), number(1.0)),
                (string("b"), number(1.0)),
                (string("b"), number(2.0)),
                (string("c"), number(2.0)),
            ]
        );
    }

    #[test]
    fn every_key_of_a_record_carries_the_stored_value_when_values_are_wanted() {
        let entries = vec![(stored(7), vec![string("a"), string("b"), string("c")])];
        let records = build_index_records(entries, false, true);
        assert_eq!(records.len(), 3);
        assert!(records.iter().all(|record| record.value == vec![7]));
    }

    #[test]
    fn no_value_is_copied_when_only_keys_are_wanted() {
        let entries = vec![
            (stored(1), vec![string("a"), string("b")]),
            (stored(2), vec![string("a")]),
        ];
        let records = build_index_records(entries, false, false);
        assert_eq!(records.len(), 3);
        assert!(records.iter().all(|record| record.value.is_empty()));
    }

    #[test]
    fn a_unique_index_keeps_the_first_record_of_a_key() {
        let entries = vec![
            (stored(3), vec![string("x")]),
            (stored(1), vec![string("x")]),
            (stored(2), vec![string("y")]),
        ];
        let records = build_index_records(entries, true, true);
        assert_eq!(
            pairs(&records),
            vec![(string("x"), number(1.0)), (string("y"), number(2.0))]
        );
    }

    #[test]
    fn a_unique_multientry_index_keeps_one_record_per_member() {
        // Record 1 has x and y, record 2 has y and z: y belongs to the record with the smaller primary key.
        let entries = vec![
            (stored(1), vec![string("x"), string("y")]),
            (stored(2), vec![string("y"), string("z")]),
        ];
        let records = build_index_records(entries, true, true);
        assert_eq!(
            pairs(&records),
            vec![
                (string("x"), number(1.0)),
                (string("y"), number(1.0)),
                (string("z"), number(2.0)),
            ]
        );
    }

    #[test]
    fn keys_of_different_types_sort_by_the_specification_order() {
        // number < date < string < binary < array
        let entries = vec![
            (stored(1), vec![IndexedDBKeyType::Array(vec![number(1.0)])]),
            (stored(2), vec![string("s")]),
            (stored(3), vec![number(5.0)]),
            (stored(4), vec![IndexedDBKeyType::Date(1.0)]),
        ];
        let records = build_index_records(entries, false, false);
        let order: Vec<_> = records
            .iter()
            .map(|record| record.primary_key.clone())
            .collect();
        assert_eq!(
            order,
            vec![number(3.0), number(4.0), number(2.0), number(1.0)]
        );
    }

    #[test]
    fn an_empty_store_gives_no_records() {
        assert!(build_index_records(vec![], false, true).is_empty());
        assert!(build_index_records(vec![], true, false).is_empty());
    }
}
