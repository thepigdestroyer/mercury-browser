/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

// Mercury: new file for ED5 (lane txt-1). The counter pass: the value of every CSS counter at
// every place that shows one, computed in one pre-order walk of the styled DOM before boxes are
// built.
//
// The rules are CSS Lists Level 3, section 4 ("Automatic Numbering with Counters"),
// <https://drafts.csswg.org/css-lists-3/#auto-numbering>: counters are created by
// `counter-reset`, found again by name in the scope of the element that created them (the
// element, its following siblings and all their descendants), nested when an element creates a
// counter that an ancestor's scope already has, and counted by `counter-increment` and
// `counter-set`; `display: list-item` increments the implicit `list-item` counter. The
// computation of the starting value of a `reversed()` counter whose start is not given (the
// number of items counted from the end, which is what `<ol reversed>` needs), the implicit
// creation of a counter by `counter-increment` or by `counter()` and the choice of -1 as the
// increment of `list-item` in a reversed list follow Gecko's layout/base/nsCounterManager.cpp
// (MPL-2.0, https://github.com/mozilla-firefox/firefox/blob/main/layout/base/nsCounterManager.cpp;
// its `RecalcAll` and `AddCounterChanges`). Gecko's code is organised around a linked list of
// counter nodes per name and is not copied: this file keeps a stack of scopes and a stack of
// counters per name instead. Ladybird's `CountersSet` was read for the design only.
//
// Nothing in this file names a Stylo or Servo type, so it is compiled and tested on its own
// (`tests/counters.rs` includes it by path). The code that reads the styled DOM and feeds this
// pass is in `lists.rs`.

use std::collections::HashMap;
use std::hash::{BuildHasherDefault, Hasher};

use smallvec::SmallVec;

/// A fast hasher for the small keys of this file (Firefox's "Fx" hash, with the final rotation
/// that rustc-hash 2 uses so that addresses, whose low bits are zero, spread over the buckets).
/// The keys are not controlled by a page: they are addresses of nodes and counter name ids.
#[derive(Clone, Copy, Default)]
pub(crate) struct FxHasher {
    hash: u64,
}

impl FxHasher {
    const SEED: u64 = 0xf135_7aea_2e62_a9c5;

    fn add(&mut self, word: u64) {
        self.hash = (self.hash.rotate_left(5) ^ word).wrapping_mul(Self::SEED);
    }
}

impl Hasher for FxHasher {
    fn write(&mut self, bytes: &[u8]) {
        for chunk in bytes.chunks(8) {
            let mut word = [0u8; 8];
            word[..chunk.len()].copy_from_slice(chunk);
            self.add(u64::from_le_bytes(word));
        }
    }

    fn write_u8(&mut self, value: u8) {
        self.add(u64::from(value));
    }

    fn write_u32(&mut self, value: u32) {
        self.add(u64::from(value));
    }

    fn write_u64(&mut self, value: u64) {
        self.add(value);
    }

    fn write_usize(&mut self, value: usize) {
        self.add(value as u64);
    }

    fn write_isize(&mut self, value: isize) {
        self.add(value as u64);
    }

    fn finish(&self) -> u64 {
        self.hash.rotate_left(26)
    }
}

type FastMap<K, V> = HashMap<K, V, BuildHasherDefault<FxHasher>>;

/// Identifies a node of the styled DOM: the address Servo's `OpaqueNode` holds.
pub(crate) type NodeKey = usize;

/// The pseudo-elements that can have counters of their own.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum PseudoKind {
    Marker,
    Before,
    After,
}

/// Where a counter is looked up: an element, or one of its pseudo-elements (`::before`), or a
/// pseudo-element of one (`::before::marker`, the only nesting Servo has).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub(crate) struct CounterKey {
    pub node: NodeKey,
    pub primary: Option<PseudoKind>,
    pub secondary: Option<PseudoKind>,
}

impl CounterKey {
    pub(crate) fn element(node: NodeKey) -> Self {
        Self {
            node,
            primary: None,
            secondary: None,
        }
    }

    pub(crate) fn pseudo(node: NodeKey, pseudo: PseudoKind) -> Self {
        Self {
            node,
            primary: Some(pseudo),
            secondary: None,
        }
    }

    /// The key of a pseudo-element of this element or pseudo-element; `None` below the second
    /// level, which Servo does not have.
    pub(crate) fn with_pseudo(self, pseudo: PseudoKind) -> Option<Self> {
        match (self.primary, self.secondary) {
            (None, _) => Some(Self {
                primary: Some(pseudo),
                ..self
            }),
            (Some(_), None) => Some(Self {
                secondary: Some(pseudo),
                ..self
            }),
            (Some(_), Some(_)) => None,
        }
    }
}

/// No value was given for a `reversed()` counter; the pass computes the start.
pub(crate) const NO_VALUE: i32 = i32::MIN;

/// The name of the implicit counter of list items.
pub(crate) const LIST_ITEM: &str = "list-item";

/// One entry of `counter-reset`, `counter-increment` or `counter-set`.
#[derive(Clone, Copy, Debug)]
pub(crate) struct CounterOp<'a> {
    pub name: &'a str,
    /// The integer of the entry (1 for an increment, 0 for a reset or a set that gave none:
    /// the style system has already filled that in), or [`NO_VALUE`] for `reversed(name)`.
    pub value: i32,
    /// `reversed(name)`: only `counter-reset` has it.
    pub reversed: bool,
}

/// What an HTML element contributes besides its style. The user agent style sheet of the HTML
/// standard maps these attributes to counter properties; the pass reads them directly.
/// <https://html.spec.whatwg.org/multipage/rendering.html#lists>
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum HtmlHint {
    #[default]
    None,
    /// `<ol>`: the `start` and `reversed` attributes (`start` is already parsed).
    OrderedList { start: Option<i32>, reversed: bool },
    /// `<ul>`, `<menu>` and `<dir>`.
    UnorderedList,
    /// `<li>`: the `value` attribute.
    ListItem { value: Option<i32> },
}

/// A `counter()` or `counters()` item of a `content` property.
#[derive(Clone, Copy, Debug)]
pub(crate) struct CounterUse<'a> {
    pub name: &'a str,
    /// `counters()`: every counter of that name in scope is wanted, not only the innermost.
    pub all: bool,
}

/// What the pass needs to know about one element or pseudo-element that generates a box.
#[derive(Clone, Debug, Default)]
pub(crate) struct ElementInput<'a> {
    pub key: CounterKey,
    /// `display: list-item`.
    pub is_list_item: bool,
    pub reset: Vec<CounterOp<'a>>,
    pub increment: Vec<CounterOp<'a>>,
    pub set: Vec<CounterOp<'a>>,
    pub html: HtmlHint,
    /// The counters that this element's `content` shows.
    pub uses: Vec<CounterUse<'a>>,
    /// Record the value of the `list-item` counter as it is after this element's own changes
    /// (the marker of a list item shows it).
    pub observe_list_item: bool,
}

/// The HTML "rules for parsing integers" (without the range of a `long`: a value that does not
/// fit an `i32` is an error here). <https://html.spec.whatwg.org/multipage/common-microsyntaxes.html#rules-for-parsing-integers>
pub(crate) fn parse_html_integer(input: &str) -> Option<i32> {
    let trimmed = input.trim_start_matches([' ', '\t', '\n', '\x0C', '\r']);
    let (negative, rest) = match trimmed.as_bytes().first()? {
        b'-' => (true, &trimmed[1..]),
        b'+' => (false, &trimmed[1..]),
        _ => (false, trimmed),
    };
    let digits = rest.bytes().take_while(u8::is_ascii_digit).count();
    if digits == 0 {
        return None;
    }
    let magnitude: i64 = rest[..digits].parse().ok()?;
    i32::try_from(if negative { -magnitude } else { magnitude }).ok()
}

type NameId = usize;

/// The value of a counter at some point of the walk. A `reversed()` counter without a start has
/// no value until every item that counts it has been seen, so what is known meanwhile is the
/// distance from that start.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Value {
    Known(i32),
    FromBase { base: usize, offset: i32 },
}

/// A counter, as created by `counter-reset` (or implicitly) on some element.
#[derive(Clone, Copy, Debug)]
struct Instance {
    /// The scope that holds it: the index in `CounterPass::frames` of the parent of the element
    /// that created it.
    frame: usize,
    /// The value, or the offset from the base while `base` is set.
    value: i32,
    reversed: bool,
    /// `Some(index in CounterPass::bases)` while the start of a `reversed()` counter is unknown.
    base: Option<usize>,
    /// The last increment that was not zero (needed to find the start).
    last_increment: i32,
}

impl Instance {
    fn value(&self) -> Value {
        match self.base {
            None => Value::Known(self.value),
            Some(base) => Value::FromBase {
                base,
                offset: self.value,
            },
        }
    }
}

/// What is recorded for one place.
#[derive(Debug, Default)]
struct Entry {
    list_item: Option<Value>,
    uses: Vec<Use>,
}

#[derive(Debug)]
struct Use {
    name: NameId,
    value: Value,
    /// `counters()`: every value of that name in scope, outermost first.
    all: Option<Box<[Value]>>,
}

/// The walk. Feed it `enter` and `leave` for every element (and pseudo-element) that generates
/// a box, in tree order, and take the result with `finish`. Elements without a box
/// (`display: none`) are not fed, nor is anything inside them; `display: contents` elements are
/// not fed either, but their children are.
pub(crate) struct CounterPass {
    names: HashMap<Box<str>, NameId>,
    /// The counters of each name that are in scope now, outermost first.
    stacks: Vec<Vec<Instance>>,
    /// One per open element, and the document at index 0: the names of the counters that its
    /// children (including its pseudo-elements) created.
    frames: Vec<Vec<NameId>>,
    /// The start of every `reversed()` counter that did not have one, once known.
    bases: Vec<Option<i32>>,
    entries: FastMap<CounterKey, Entry>,
    list_item: NameId,
}

impl Default for CounterPass {
    fn default() -> Self {
        Self::new()
    }
}

impl CounterPass {
    pub(crate) fn new() -> Self {
        let mut pass = Self {
            names: HashMap::new(),
            stacks: Vec::new(),
            frames: vec![Vec::new()],
            bases: Vec::new(),
            entries: FastMap::default(),
            list_item: 0,
        };
        pass.list_item = pass.name_id(LIST_ITEM);
        pass
    }

    fn name_id(&mut self, name: &str) -> NameId {
        if let Some(id) = self.names.get(name) {
            return *id;
        }
        let id = self.stacks.len();
        self.stacks.push(Vec::new());
        self.names.insert(name.into(), id);
        id
    }

    /// An element or pseudo-element starts: its counters are created and changed, and the
    /// places that show a counter are recorded. Its children come next, then `leave`.
    pub(crate) fn enter(&mut self, input: &ElementInput<'_>) {
        self.apply_resets(input);
        self.apply_increments(input);
        self.apply_sets(input);
        self.record(input);
        self.frames.push(Vec::new());
    }

    /// The element that was entered last ends.
    pub(crate) fn leave(&mut self) {
        debug_assert!(self.frames.len() > 1, "leave without enter");
        if self.frames.len() > 1 {
            self.close_frame();
        }
    }

    fn close_frame(&mut self) {
        let created = self.frames.pop().expect("a frame to close");
        for name in created.into_iter().rev() {
            if let Some(instance) = self.stacks[name].pop() {
                self.finalize(instance);
            }
        }
    }

    fn apply_resets(&mut self, input: &ElementInput<'_>) {
        // "If multiple instances of the same counter-name occur in the property value, only the
        // last one is honored."
        for (index, op) in input.reset.iter().enumerate() {
            if input.reset[index + 1..]
                .iter()
                .any(|later| later.name == op.name)
            {
                continue;
            }
            self.instantiate(op.name, op.value, op.reversed);
        }

        // The HTML lists create the implicit counter, unless the author's `counter-reset`
        // names it.
        if input.reset.iter().any(|op| op.name == LIST_ITEM) {
            return;
        }
        match input.html {
            HtmlHint::OrderedList {
                start,
                reversed: false,
            } => {
                self.instantiate(LIST_ITEM, start.map_or(0, |s| s.saturating_sub(1)), false);
            },
            HtmlHint::OrderedList {
                start,
                reversed: true,
            } => {
                // The first item of a reversed list that starts at 5 shows 5, and each item
                // counts by -1, so the counter starts at 6.
                self.instantiate(
                    LIST_ITEM,
                    start.map_or(NO_VALUE, |s| s.saturating_add(1)),
                    true,
                );
            },
            HtmlHint::UnorderedList => self.instantiate(LIST_ITEM, 0, false),
            HtmlHint::ListItem { .. } | HtmlHint::None => {},
        }
    }

    fn apply_increments(&mut self, input: &ElementInput<'_>) {
        for op in &input.increment {
            self.increment(op.name, op.value);
        }
        // https://drafts.csswg.org/css-lists-3/#list-item-counter: "unless the
        // 'counter-increment' property explicitly specifies a different increment for the
        // list-item counter, it must be incremented by 1 on every list item, or if the counter is
        // reversed, by -1".
        if input.is_list_item && !input.increment.iter().any(|op| op.name == LIST_ITEM) {
            let id = self.list_item;
            let reversed = self.innermost(id).is_some_and(|counter| counter.reversed);
            self.increment(LIST_ITEM, if reversed { -1 } else { 1 });
        }
    }

    fn apply_sets(&mut self, input: &ElementInput<'_>) {
        for op in &input.set {
            self.set(op.name, op.value);
        }
        // `<li value=N>`: "counter-set: list-item N".
        if let HtmlHint::ListItem { value: Some(value) } = input.html {
            self.set(LIST_ITEM, value);
        }
    }

    fn record(&mut self, input: &ElementInput<'_>) {
        if !input.observe_list_item && input.uses.is_empty() {
            return;
        }
        let mut entry = Entry::default();
        if input.observe_list_item {
            let id = self.list_item;
            entry.list_item = Some(self.ensure(id).value());
        }
        for used in &input.uses {
            let id = self.name_id(used.name);
            let value = self.ensure(id).value();
            let all = used
                .all
                .then(|| self.stacks[id].iter().map(Instance::value).collect());
            entry.uses.push(Use {
                name: id,
                value,
                all,
            });
        }
        self.entries.insert(input.key, entry);
    }

    fn innermost(&self, name: NameId) -> Option<&Instance> {
        self.stacks[name].last()
    }

    /// The innermost counter of this name; "when not otherwise present" (`counter-increment`,
    /// `counter-set`, `counter()`) it is instantiated first, with a starting value of 0.
    fn ensure(&mut self, name: NameId) -> &mut Instance {
        if self.stacks[name].is_empty() {
            self.instantiate_id(name, 0, false);
        }
        self.stacks[name].last_mut().expect("a counter was created")
    }

    /// https://drafts.csswg.org/css-lists-3/#instantiate-a-counter
    fn instantiate(&mut self, name: &str, value: i32, reversed: bool) {
        let id = self.name_id(name);
        self.instantiate_id(id, value, reversed);
    }

    fn instantiate_id(&mut self, name: NameId, value: i32, reversed: bool) {
        let frame = self.frames.len() - 1;
        let pending = reversed && value == NO_VALUE;
        let base = pending.then(|| {
            self.bases.push(None);
            self.bases.len() - 1
        });
        let instance = Instance {
            frame,
            value: if pending { 0 } else { value },
            reversed,
            base,
            last_increment: 0,
        };

        // A counter of an ancestor's scope is nested in; one that the element itself or a
        // previous sibling created is replaced.
        let replaces = self.stacks[name]
            .last()
            .is_some_and(|innermost| innermost.frame == frame);
        if replaces {
            let old = self.stacks[name].pop().expect("the innermost counter");
            self.finalize(old);
        } else {
            self.frames[frame].push(name);
        }
        self.stacks[name].push(instance);
    }

    fn increment(&mut self, name: &str, by: i32) {
        let id = self.name_id(name);
        let counter = self.ensure(id);
        counter.value = counter.value.saturating_add(by);
        if counter.base.is_some() && by != 0 {
            counter.last_increment = by;
        }
    }

    fn set(&mut self, name: &str, value: i32) {
        let id = self.name_id(name);
        let counter = self.ensure(id);
        let offset = counter.value;
        let unknown_start = counter.base.take();
        counter.value = value;
        if let Some(base) = unknown_start {
            // The counter counts down to this value: the start is as far above it as the
            // increments so far have moved it.
            self.bases[base] = Some(value.saturating_sub(offset));
        }
    }

    /// A counter goes out of scope: the start of a `reversed()` counter that had none is the
    /// number of items it counted, plus one more increment.
    /// https://drafts.csswg.org/css-lists-3/#instantiating-counters
    fn finalize(&mut self, counter: Instance) {
        if let Some(base) = counter.base {
            self.bases[base] = Some(
                0i32.saturating_sub(counter.last_increment)
                    .saturating_sub(counter.value),
            );
        }
    }

    /// The walk is over: the table of what every recorded place shows.
    pub(crate) fn finish(mut self) -> CounterTable {
        while self.frames.len() > 1 {
            self.close_frame();
        }
        // The counters of the document itself (the outermost ones).
        let remaining: Vec<Instance> = self.stacks.iter_mut().flat_map(|s| s.drain(..)).collect();
        for counter in remaining {
            self.finalize(counter);
        }

        let mut name_list = vec![Box::<str>::from(""); self.names.len()];
        for (name, id) in &self.names {
            name_list[*id] = name.clone();
        }
        CounterTable {
            entries: self.entries,
            bases: self.bases,
            names: self.names,
            name_list,
        }
    }
}

/// What the pass found: the value that each recorded place shows.
#[derive(Debug, Default)]
pub(crate) struct CounterTable {
    entries: FastMap<CounterKey, Entry>,
    /// The start of every `reversed()` counter that had none (see [`Value::FromBase`]).
    bases: Vec<Option<i32>>,
    names: HashMap<Box<str>, NameId>,
    name_list: Vec<Box<str>>,
}

impl CounterTable {
    /// A table without any entry: every lookup answers with its default.
    pub(crate) fn empty() -> Self {
        Self::default()
    }

    fn resolve(&self, value: Value) -> i32 {
        match value {
            Value::Known(value) => value,
            Value::FromBase { base, offset } => {
                self.bases[base].unwrap_or(0).saturating_add(offset)
            },
        }
    }

    /// The value of the `list-item` counter after the element's own changes, for an element
    /// that was entered with `observe_list_item`.
    pub(crate) fn list_item_value(&self, key: CounterKey) -> Option<i32> {
        Some(self.resolve(self.entries.get(&key)?.list_item?))
    }

    /// What `counter(name)` shows in this element or pseudo-element: 0 when the pass did not see
    /// a `counter()` of that name there (the spec instantiates a counter with that value).
    pub(crate) fn counter_value(&self, key: CounterKey, name: &str) -> i32 {
        self.find(key, name)
            .map_or(0, |used| self.resolve(used.value))
    }

    /// What `counters(name, ...)` joins in this element or pseudo-element, outermost first.
    pub(crate) fn counters_values(&self, key: CounterKey, name: &str) -> SmallVec<[i32; 4]> {
        match self.find(key, name) {
            Some(Use { all: Some(all), .. }) => {
                all.iter().map(|value| self.resolve(*value)).collect()
            },
            Some(used) => SmallVec::from_slice(&[self.resolve(used.value)]),
            None => SmallVec::from_slice(&[0]),
        }
    }

    fn find(&self, key: CounterKey, name: &str) -> Option<&Use> {
        let id = *self.names.get(name)?;
        // A `counters()` entry also answers `counter()`.
        self.entries
            .get(&key)?
            .uses
            .iter()
            .find(|used| used.name == id)
    }

    /// Whether any place shows a counter. A document without a numbered list or a `counter()`
    /// does not need the pass to run again for changes that do not touch counters.
    pub(crate) fn has_observers(&self) -> bool {
        !self.entries.is_empty()
    }

    /// The number of places that were recorded (the tests count them).
    #[allow(dead_code)]
    pub(crate) fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether a place that both tables know shows something different in this table: the
    /// boxes built from the old one are out of date.
    pub(crate) fn changed_from(&self, old: &CounterTable) -> bool {
        self.entries.iter().any(|(key, entry)| {
            old.entries
                .get(key)
                .is_some_and(|before| !self.same_entry(entry, old, before))
        })
    }

    fn same_entry(&self, entry: &Entry, other: &CounterTable, before: &Entry) -> bool {
        let resolve_all = |table: &CounterTable, all: &Option<Box<[Value]>>| {
            all.as_ref()
                .map(|all| all.iter().map(|v| table.resolve(*v)).collect::<Vec<_>>())
        };
        entry.list_item.map(|value| self.resolve(value))
            == before.list_item.map(|value| other.resolve(value))
            && entry.uses.len() == before.uses.len()
            && entry.uses.iter().zip(&before.uses).all(|(now, was)| {
                self.name_list[now.name] == other.name_list[was.name]
                    && self.resolve(now.value) == other.resolve(was.value)
                    && resolve_all(self, &now.all) == resolve_all(other, &was.all)
            })
    }
}
