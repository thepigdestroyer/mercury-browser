/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

// Mercury: new file (lane txt-1, EB7). Typed highlights per document, pushed to layout.

//! The typed highlights of a [`Document`]: the marked text of an IME composition inside an
//! editing host, find matches, text fragments, spelling and grammar errors.
//!
//! A [`HighlightSet`] holds live DOM [`Range`]s, each with a [`HighlightKind`]. Producers (the IME
//! code, later find in page) call [`HighlightSet::set`] and [`HighlightSet::clear`]. Once per
//! reflow, right after the document selection's flags are updated (`Window::reflow`),
//! [`HighlightSet::update`] works out which text nodes each range covers and the offsets in each,
//! gives the typed highlights of a text node to the text run of that node with
//! `CharacterData::set_text_run_highlights` (layout repaints without a new layout), and keeps the
//! result, per node, for layout to read when it builds a text run later
//! (`LayoutDom<Node>::highlights_for_text_node`). Nodes that are covered by a highlight carry
//! `NodeFlags::OVERLAPS_HIGHLIGHT`, so layout only looks up the nodes that have one.
//!
//! This is separate from the document selection (`selection.rs`), which keeps its own path to
//! layout. The text of a text control (`<input>`, `<textarea>`) is not reached by a range of the
//! document: its composition comes from the control (`composition_for_layout`).

use std::cmp::Ordering;

use atomic_refcell::AtomicRefCell;
use js::context::NoGC;
use layout_api::{HighlightKind, TextHighlight, TextHighlights};
use rustc_hash::FxHashMap;
use script_bindings::cell::DomRefCell;
use script_bindings::dom::UnrootedDom;
use servo_base::text::{RangeAny, Utf16CodeUnits};
use style::dom::OpaqueNode;

use crate::dom::abstractrange::bp_position;
use crate::dom::bindings::inheritance::Castable;
use crate::dom::bindings::root::{Dom, DomRoot, LayoutDom};
use crate::dom::document::Document;
use crate::dom::iterators::ShadowIncluding;
use crate::dom::range::Range;
use crate::dom::text::Text;
use crate::dom::{CharacterData, Node, NodeDamage, NodeFlags, NodeTraits};

/// One range with the kind of highlight it stands for.
#[derive(JSTraceable, MallocSizeOf)]
#[cfg_attr(crown, crown::unrooted_must_root_lint::must_root)]
struct HighlightEntry {
    #[no_trace]
    kind: HighlightKind,
    range: Dom<Range>,
}

/// The typed highlights of one [`Document`]. See the module documentation.
#[derive(JSTraceable, MallocSizeOf)]
#[cfg_attr(crown, crown::unrooted_must_root_lint::must_root)]
pub(crate) struct HighlightSet {
    /// The ranges of every producer, in the order they were set.
    entries: DomRefCell<Vec<HighlightEntry>>,
    /// The text nodes that got highlights at the last [`HighlightSet::update`] (the nodes that
    /// have `NodeFlags::OVERLAPS_HIGHLIGHT`), so that a node which loses its last highlight is
    /// found and cleared.
    touched: DomRefCell<Vec<Dom<Node>>>,
    /// What layout reads: the highlights of each of the `touched` nodes as of the last update.
    /// Plain data, replaced only by [`HighlightSet::update`], never while layout runs.
    #[no_trace]
    #[ignore_malloc_size_of = "plain data for layout, bounded by the highlighted text nodes"]
    resolved: AtomicRefCell<FxHashMap<OpaqueNode, TextHighlights>>,
}

impl Default for HighlightSet {
    fn default() -> Self {
        Self {
            entries: DomRefCell::new(Vec::new()),
            touched: DomRefCell::new(Vec::new()),
            resolved: AtomicRefCell::new(FxHashMap::default()),
        }
    }
}

impl HighlightSet {
    /// Replace every range of `kind` with `ranges` (none clears the kind) and ask for a new
    /// display list, which is when the change reaches layout.
    pub(crate) fn set(&self, kind: HighlightKind, ranges: Vec<DomRoot<Range>>) {
        let document = {
            let mut entries = self.entries.borrow_mut();
            let document_of_previous = entries
                .iter()
                .find(|entry| entry.kind == kind)
                .map(|entry| entry.range.start_container().owner_document());
            if document_of_previous.is_none() && ranges.is_empty() {
                return;
            }
            let document = document_of_previous.or_else(|| {
                ranges
                    .first()
                    .map(|range| range.start_container().owner_document())
            });
            entries.retain(|entry| entry.kind != kind);
            entries.extend(ranges.iter().map(|range| HighlightEntry {
                kind,
                range: Dom::from_ref(&**range),
            }));
            document
        };
        if let Some(document) = document {
            Self::request_new_display_list(&document);
        }
    }

    /// Remove every range of `kind`.
    pub(crate) fn clear(&self, kind: HighlightKind) {
        self.set(kind, Vec::new());
    }

    /// Remove every range of every kind.
    #[allow(dead_code)] // Mercury: for producers of later lanes that reset everything at once.
    pub(crate) fn clear_all(&self) {
        let document = {
            let mut entries = self.entries.borrow_mut();
            let document = entries
                .first()
                .map(|entry| entry.range.start_container().owner_document());
            entries.clear();
            document
        };
        if let Some(document) = document {
            Self::request_new_display_list(&document);
        }
    }

    /// A reflow, and with it [`HighlightSet::update`], is needed for a change of the set to show.
    /// A request for a new display list is what makes the document ask for a rendering update.
    fn request_new_display_list(document: &Document) {
        document.window().layout().set_needs_new_display_list();
    }

    /// The highlights that [`HighlightSet::update`] resolved for `node`, for layout.
    fn resolved_for(&self, node: OpaqueNode) -> TextHighlights {
        self.resolved
            .borrow()
            .get(&node)
            .cloned()
            .unwrap_or_default()
    }

    /// Work out the highlights of every text node the ranges of the set cover and give them to
    /// layout. Called once per reflow, before layout runs. A set with no range and no text node
    /// left to clear costs nothing.
    ///
    /// The result is recomputed every time and only pushed where it differs from the last one,
    /// so there is no need to be told when a live range moves because the DOM changed.
    pub(crate) fn update(&self, no_gc: &NoGC, document: &Document) {
        let entries = self.entries.borrow();
        let mut touched = self.touched.borrow_mut();
        if entries.is_empty() && touched.is_empty() {
            return;
        }

        let mut per_node: FxHashMap<UnrootedDom<'_, Node>, TextHighlights> = Default::default();
        for entry in entries.iter() {
            for_each_text_node_in_range(no_gc, &entry.range, |node, start, end| {
                let Some(character_data) = node.downcast::<CharacterData>() else {
                    return;
                };
                let (start, end) = {
                    let text = character_data.data();
                    (
                        start.map(|offset| offset.to_utf32_code_units_in(&text)),
                        end.map(|offset| offset.to_utf32_code_units_in(&text)),
                    )
                };
                if let (Some(start), Some(end)) = (start, end) &&
                    start >= end
                {
                    // Nothing of this node is in the range (an offset in the middle of an
                    // astral character rounds towards the start).
                    return;
                }
                per_node.entry(node).or_default().push(TextHighlight {
                    kind: entry.kind,
                    range: RangeAny::new(start, end),
                });
            });
        }

        let mut resolved = self.resolved.borrow_mut();
        let mut new_resolved: FxHashMap<OpaqueNode, TextHighlights> =
            FxHashMap::with_capacity_and_hasher(per_node.len(), Default::default());
        let mut new_touched: Vec<Dom<Node>> = Vec::with_capacity(per_node.len());
        let mut needs_new_display_list = false;

        for (node, highlights) in per_node {
            let key = node.to_opaque();
            node.set_flag(NodeFlags::OVERLAPS_HIGHLIGHT, true);
            new_touched.push(Dom::from_ref(&*node));
            if resolved.get(&key) != Some(&highlights) {
                needs_new_display_list |= push_to_layout(no_gc, &node, highlights.clone());
            }
            new_resolved.insert(key, highlights);
        }

        // Text nodes that had highlights and have none any more.
        for node in touched.iter() {
            if new_resolved.contains_key(&node.to_opaque()) {
                continue;
            }
            node.set_flag(NodeFlags::OVERLAPS_HIGHLIGHT, false);
            needs_new_display_list |= push_to_layout(no_gc, node, TextHighlights::new());
        }

        *resolved = new_resolved;
        *touched = new_touched;
        if needs_new_display_list {
            document.window().layout().set_needs_new_display_list();
        }
    }
}

/// Give `highlights` to the text run of `node`. Returns whether a new display list is needed.
///
/// When there is no text run to take them (the node has no box yet, or `::first-letter` makes
/// several), the highlights are not lost: layout reads them from the set when it builds the
/// run, so the node is dirtied to make it do so. That only happens when a node's highlights
/// change, never on a reflow that changes nothing.
fn push_to_layout(no_gc: &NoGC, node: &Node, highlights: TextHighlights) -> bool {
    let Some(character_data) = node.downcast::<CharacterData>() else {
        return false;
    };
    if character_data.set_text_run_highlights(highlights) {
        true
    } else {
        node.dirty(no_gc, NodeDamage::ContentOrHeritage);
        false
    }
}

/// Call `callback` for each text node that `range` covers any part of, in tree order, with the
/// start and the end of the covered part in UTF-16 code units of the node's text. A bound is
/// `None` when the range goes on past it (the range starts or ends in another node).
///
/// A collapsed range covers nothing.
fn for_each_text_node_in_range<'no_gc>(
    no_gc: &'no_gc NoGC,
    range: &Range,
    mut callback: impl FnMut(UnrootedDom<'no_gc, Node>, Option<Utf16CodeUnits>, Option<Utf16CodeUnits>),
) {
    let start_container = range.start_container();
    let start_offset = range.start_offset();
    let end_container = range.end_container();
    let end_offset = range.end_offset();
    let same_container = std::ptr::eq::<Node>(&*start_container, &*end_container);
    if same_container && start_offset == end_offset {
        return;
    }

    // The common case: a composition or a find match inside one text node.
    if same_container && start_container.is::<Text>() {
        callback(
            start_container.as_unrooted(no_gc),
            Some(Utf16CodeUnits(start_offset)),
            Some(Utf16CodeUnits(end_offset)),
        );
        return;
    }

    let Some(root) = end_container.common_ancestor(&start_container, ShadowIncluding::No) else {
        return;
    };
    for node in root.traverse_preorder_unrooted(no_gc, ShadowIncluding::No) {
        if !node.is::<Text>() {
            continue;
        }
        // A text node that starts at or after the end of the range, and so does every text node
        // after it in tree order.
        if bp_position(no_gc, &node, 0, &end_container, end_offset) != Ordering::Less {
            break;
        }
        // A text node that ends at or before the start of the range.
        if bp_position(no_gc, &node, node.len(), &start_container, start_offset) !=
            Ordering::Greater
        {
            continue;
        }
        let is_start = std::ptr::eq::<Node>(&**node, &*start_container);
        let is_end = std::ptr::eq::<Node>(&**node, &*end_container);
        callback(
            node,
            is_start.then_some(Utf16CodeUnits(start_offset)),
            is_end.then_some(Utf16CodeUnits(end_offset)),
        );
    }
}

impl Document {
    // Mercury: lane txt-1 (EB7). The hook of `Window::reflow`.
    /// Push the typed highlights of this document to layout. Called once per reflow, right after
    /// the document selection's flags are updated and before layout runs.
    pub(crate) fn update_highlights(&self, no_gc: &NoGC) {
        self.highlights().update(no_gc, self);
    }
}

impl<'dom> LayoutDom<'dom, Document> {
    /// The typed highlights the set of this document resolved for the text node `node` at the
    /// last reflow. Layout asks for it when it builds a text run of a node that has
    /// `NodeFlags::OVERLAPS_HIGHLIGHT`.
    pub(crate) fn highlights_for_node_for_layout(self, node: OpaqueNode) -> TextHighlights {
        self.unsafe_get().highlights().resolved_for(node)
    }
}
