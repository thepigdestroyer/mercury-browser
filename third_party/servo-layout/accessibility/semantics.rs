/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

//! Mercury: a11y-1 item 4 (`layout-glue`, EE2-EE4). New file: the one function that puts
//! `mercury_a11y`'s answer onto an [`AccessibilityNode`], and the follow-up that keeps the names of
//! other nodes current.
//!
//! [`update_semantics`] replaces what upstream does in `update_properties_from_dom_node` (a role from
//! a 16-entry table, plus a name from the node's descendants for headings and links):
//!
//! 1. an element is handed to `mercury_a11y::compute` through the adapter in `dom_adapter.rs`, which
//!    returns its role, name, description, states, value, advertised actions, relations and whether
//!    its semantics hide it (`aria-hidden`, `inert`, ...);
//! 2. [`merge_computed`] puts that onto the node's accesskit node while keeping what layout owns
//!    (children, bounds, transform, the hidden flag, the HTML tag) and adding what only this crate
//!    knows (the graft of an `<iframe>`, the relations as accesskit node ids), and says whether
//!    anything changed and whether the role or a text property did;
//! 3. a relation to a node that is not in the tree yet is retried at the end of the same update (the
//!    node may simply come later in the walk); a relation that is still unresolved then is dropped,
//!    never a panic.
//!
//! Names come from the DOM, not from the accessibility tree, so a change in one node can change the
//! name of another that has no damage of its own: a link whose text changes, a control whose `<label>`
//! changes, an element that another names with `aria-labelledby`. [`apply_followups`] runs after the
//! walk and recomputes those nodes (what the brief calls the dependants pass). It is skipped when the
//! whole tree was built in the update, because every node was then computed from the final DOM.
//!
//! The id index of the document (`accname::IdIndex`) is built once per update, lazily, and only when
//! the node being computed can look other elements up by id (see
//! `dom_adapter::needs_document_index`); see `SemanticsCache`. A document that never uses an IDREF
//! attribute or a `<label for>` does not pay for the reverse maps either (`AccessibilityTree::uses_idrefs`).
//!
//! # Known gaps
//!
//! * Removing a node does not recompute the nodes whose names it contributed to (a `<label>` taken
//!   out of the document leaves its control's name stale until the control has damage of its own).
//! * The dependants of a node are over-approximated on purpose (every ancestor that may take its name
//!   from content, sixteen levels at most): an unneeded recompute is cheap, a missed one is a stale name.

use a11y_dom::{Node as _, NodeId as DomNodeId, NodeKind};
use accesskit::{NodeId, Role, TreeId};
use layout_api::LayoutNode;
use mercury_a11y::names::DependantsIndex;
use mercury_a11y::{IdIndex, Relations};
use rustc_hash::{FxHashMap, FxHashSet};
use script::layout_dom::ServoLayoutNode;
use style::dom::{NodeInfo, OpaqueNode};

use super::accessibility_node::{AccessibilityNode, LocalAccessibilityDamage};
use super::accessibility_tree::{AccessibilityTree, AccessibilityUpdate};
use super::dom_adapter::{
    AccDocument, AccNode, document_root, first_leaf, has_idref_attribute, is_frame_element,
    may_name_from_content, needs_document_index, widget_host,
};

/// The most ancestors of a changed node that are looked at for names taken from content.
const ANCESTOR_LIMIT: usize = 16;

/// The tags a `<label>` can name (the labelable elements that matter here).
const LABELABLE: [&str; 7] = [
    "input", "button", "select", "textarea", "meter", "output", "progress",
];

/// The relations of a node as accesskit node ids.
#[derive(Clone, Debug, Default, PartialEq)]
pub(super) struct ResolvedRelations {
    pub(super) controls: Vec<NodeId>,
    pub(super) described_by: Vec<NodeId>,
    pub(super) details: Vec<NodeId>,
    pub(super) error_message: Option<NodeId>,
    pub(super) flow_to: Vec<NodeId>,
    pub(super) labelled_by: Vec<NodeId>,
    pub(super) owns: Vec<NodeId>,
    pub(super) active_descendant: Option<NodeId>,
}

impl ResolvedRelations {
    /// Set the relations on the node; an empty relation sets nothing.
    fn apply(&self, node: &mut accesskit::Node) {
        if !self.controls.is_empty() {
            node.set_controls(self.controls.clone());
        }
        if !self.described_by.is_empty() {
            node.set_described_by(self.described_by.clone());
        }
        if !self.details.is_empty() {
            node.set_details(self.details.clone());
        }
        if let Some(error_message) = self.error_message {
            node.set_error_message(error_message);
        }
        if !self.flow_to.is_empty() {
            node.set_flow_to(self.flow_to.clone());
        }
        if !self.labelled_by.is_empty() {
            node.set_labelled_by(self.labelled_by.clone());
        }
        if !self.owns.is_empty() {
            node.set_owns(self.owns.clone());
        }
        if let Some(active_descendant) = self.active_descendant {
            node.set_active_descendant(active_descendant);
        }
    }
}

/// Put what `mercury_a11y` computed onto a node, and say what changed.
///
/// `old` is the node as it is in the tree; `computed` is `NodeSemantics::node`, which has no
/// children, bounds, transform, graft or relation lists. The result keeps from `old` everything
/// layout owns (children, bounds, transform, the hidden flag, the HTML tag if `computed` has none),
/// takes the graft from `tree_id` (`None` removes it) and the relations from `relations`.
///
/// Returns `None` when the result equals `old`, else the new node and the local damage: `RoleChanged`
/// if the role changed, `TextChanged` if the label, description or value did (the same bits
/// `set_role` and `set_label` return upstream). A change of a state only is a change with no damage
/// bit: the node must be sent again, nothing else needs recomputing.
pub(super) fn merge_computed(
    old: &accesskit::Node,
    computed: accesskit::Node,
    tree_id: Option<TreeId>,
    relations: &ResolvedRelations,
) -> Option<(accesskit::Node, LocalAccessibilityDamage)> {
    let mut new = computed;
    if !old.children().is_empty() {
        new.set_children(old.children().to_vec());
    }
    if let Some(bounds) = old.bounds() {
        new.set_bounds(bounds);
    }
    if let Some(transform) = old.transform() {
        new.set_transform(*transform);
    }
    if old.is_hidden() {
        new.set_hidden();
    }
    if new.html_tag().is_none() &&
        let Some(html_tag) = old.html_tag()
    {
        new.set_html_tag(html_tag);
    }
    if let Some(tree_id) = tree_id {
        // A graft node cannot have children (AccessKit panics on a node with both), and a frame
        // element never has any in the accessibility tree; make sure of it.
        new.clear_children();
        new.set_tree_id(tree_id);
    }
    relations.apply(&mut new);

    if new == *old {
        return None;
    }
    let mut damage = LocalAccessibilityDamage::empty();
    if new.role() != old.role() {
        damage.insert(LocalAccessibilityDamage::RoleChanged);
    }
    if new.label() != old.label() ||
        new.description() != old.description() ||
        new.value() != old.value()
    {
        damage.insert(LocalAccessibilityDamage::TextChanged);
    }
    Some((new, damage))
}

/// Why a node is a reason to recompute other nodes.
struct Source<'update> {
    node: ServoLayoutNode<'update>,
    /// The node's children changed (so the node itself, if it takes its name from content, is stale
    /// too), as opposed to the node itself having been recomputed.
    include_self: bool,
}

/// What an id index needs: the document's elements by their DOM id, and the nodes by the ids the
/// adapter handed out.
struct Session<'update> {
    ids: IdIndex<'update, AccNode<'update>>,
    by_id: FxHashMap<u32, AccNode<'update>>,
}

impl<'update> Session<'update> {
    fn build(root: AccNode<'update>) -> Self {
        let by_id = a11y_dom::self_and_descendants(root)
            .map(|node| (node.id().0, node))
            .collect();
        Session {
            ids: IdIndex::build(root),
            by_id,
        }
    }
}

/// The per-update state of the semantics computation, kept in `AccessibilityUpdate`.
#[derive(Default)]
pub(super) struct SemanticsCache<'update> {
    /// The root of the flat tree, found from the first node computed.
    root: Option<AccNode<'update>>,
    /// The id index of the document, built the first time a node needs it.
    session: Option<Session<'update>>,
    /// The nodes whose recomputation may change other nodes (see [`apply_followups`]).
    sources: Vec<Source<'update>>,
    /// The elements computed in this update: their semantics are final, they need no recompute.
    computed: FxHashSet<OpaqueNode>,
    /// The nodes with a relation to a node that was not in the tree when they were computed.
    retries: Vec<ServoLayoutNode<'update>>,
    /// A node had an `id` (or a label's `for`) that is not there any more: elements that referred
    /// to it must be computed again.
    ids_lost: bool,
}

impl<'update> SemanticsCache<'update> {
    fn root_for(&mut self, node: AccNode<'update>) -> AccNode<'update> {
        *self.root.get_or_insert_with(|| document_root(node))
    }

    fn session_for(&mut self, node: AccNode<'update>) -> &Session<'update> {
        let root = self.root_for(node);
        self.session.get_or_insert_with(|| Session::build(root))
    }

    /// The id index, if some node was computed in this update (so the root is known).
    fn session_if_rooted(&mut self) -> Option<&Session<'update>> {
        let root = self.root?;
        Some(self.session.get_or_insert_with(|| Session::build(root)))
    }

    fn note_changed(&mut self, node: ServoLayoutNode<'update>) {
        self.sources.push(Source {
            node,
            include_self: false,
        });
    }

    /// The children of this node changed in the update.
    pub(super) fn note_children_changed(&mut self, node: ServoLayoutNode<'update>) {
        self.sources.push(Source {
            node,
            include_self: true,
        });
    }
}

/// Compute a node's role, name, description, states, value, relations and actions and put them on
/// the node, replacing what upstream does in `update_properties_from_dom_node`.
///
/// Text nodes keep the role `TextRun` (their text is set by the caller); nodes that are neither
/// elements nor text (comments, ...) are generic containers, as before.
pub(super) fn update_semantics<'update>(
    node: &mut AccessibilityNode,
    dom_node: &ServoLayoutNode<'update>,
    tree: &mut AccessibilityTree,
    update: &mut AccessibilityUpdate<'update>,
) -> LocalAccessibilityDamage {
    if dom_node.is_text_node() {
        if !update.is_full_rebuild() {
            update.semantics.note_changed(*dom_node);
        }
        return node.set_role(Role::TextRun);
    }
    if !dom_node.is_element() {
        return node.set_role(Role::GenericContainer);
    }
    if !update.is_full_rebuild() {
        update.semantics.note_changed(*dom_node);
    }
    compute_element(node, dom_node, tree, update, false)
}

/// The accesskit ids of the DOM nodes a relation names, and whether some were not in the tree yet.
fn resolve_relations(
    relations: &Relations,
    session: &Session<'_>,
    tree: &AccessibilityTree,
) -> (ResolvedRelations, bool) {
    let mut unresolved = false;
    let mut resolve = |ids: &[DomNodeId]| -> Vec<NodeId> {
        ids.iter()
            .filter_map(|id| {
                // Not in the document's flat tree: nothing to link to, nothing to retry.
                let target = session.by_id.get(&id.0)?;
                let resolved = tree.existing_id_for_opaque(target.dom_node().opaque());
                unresolved |= resolved.is_none();
                resolved
            })
            .collect()
    };
    let resolved = ResolvedRelations {
        controls: resolve(&relations.controls),
        described_by: resolve(&relations.described_by),
        details: resolve(&relations.details),
        error_message: resolve(&relations.error_message).into_iter().next(),
        flow_to: resolve(&relations.flow_to),
        labelled_by: resolve(&relations.labelled_by),
        owns: resolve(&relations.owns),
        active_descendant: relations
            .active_descendant
            .and_then(|id| resolve(&[id]).into_iter().next()),
    };
    (resolved, unresolved)
}

/// What the element can be referred to by (`id`), and for a `<label>` what it refers to (`for`).
fn id_signature(node: AccNode<'_>) -> Option<String> {
    let id = node.attr("id").filter(|id| !id.is_empty());
    let target = node
        .is_element("label")
        .then(|| node.attr("for"))
        .flatten()
        .filter(|target| !target.is_empty());
    if id.is_none() && target.is_none() {
        return None;
    }
    Some(format!("{}\0{}", id.unwrap_or(""), target.unwrap_or("")))
}

/// [`update_semantics`] for an element. `last_try` is set when this is already the retry for
/// unresolved relations (they are then dropped).
fn compute_element<'update>(
    node: &mut AccessibilityNode,
    dom_node: &ServoLayoutNode<'update>,
    tree: &mut AccessibilityTree,
    update: &mut AccessibilityUpdate<'update>,
    last_try: bool,
) -> LocalAccessibilityDamage {
    let acc = AccNode::new(*dom_node);
    let cache = &mut update.semantics;
    cache.computed.insert(dom_node.opaque());
    if has_idref_attribute(acc) {
        tree.note_idrefs_used();
    }
    if node.replace_id_signature(id_signature(acc)) {
        cache.ids_lost = true;
    }
    let root = cache.root_for(acc);
    let doc = AccDocument::new(root);

    let (semantics, resolved, unresolved) = if needs_document_index(acc) {
        let session = cache.session_for(acc);
        let semantics = mercury_a11y::compute(&doc, acc, &session.ids);
        let (resolved, unresolved) = resolve_relations(&semantics.relations, session, tree);
        (semantics, resolved, unresolved)
    } else {
        // Nothing this node reads is looked up by id: an index that holds at most one node does.
        let ids = IdIndex::build(first_leaf(acc));
        let semantics = mercury_a11y::compute(&doc, acc, &ids);
        (semantics, ResolvedRelations::default(), false)
    };
    let hidden = semantics.hidden;
    let computed = semantics.node;

    // `child_tree_id` answers `None` for anything but an iframe; the check keeps a node that is not
    // a frame element from ever becoming a graft (a graft cannot have children).
    let tree_id = if is_frame_element(dom_node) {
        super::frames::child_tree_id(dom_node)
    } else {
        None
    };

    let mut damage = node.set_semantic_hidden(hidden);
    if let Some((merged, merged_damage)) =
        merge_computed(node.accesskit_node(), computed, tree_id, &resolved)
    {
        node.replace_accesskit_node(merged);
        damage.insert(merged_damage);
    }
    if unresolved && !last_try {
        update.semantics.retries.push(*dom_node);
    }
    damage
}

/// Recompute the semantics of nodes that other nodes' changes may have made stale; see the module
/// documentation. Runs after the walk of the damaged nodes, before the update is finalized.
pub(super) fn apply_followups<'update>(
    tree: &mut AccessibilityTree,
    update: &mut AccessibilityUpdate<'update>,
) {
    let rebuilt = update.is_full_rebuild();
    let sources = std::mem::take(&mut update.semantics.sources);
    let retries = std::mem::take(&mut update.semantics.retries);

    let mut queue: Vec<ServoLayoutNode<'update>> = Vec::new();
    let mut retried: FxHashSet<OpaqueNode> = FxHashSet::default();
    for node in retries {
        retried.insert(node.opaque());
        queue.push(node);
    }
    if !rebuilt && !sources.is_empty() {
        collect_dependants(tree, update, &sources, &mut queue);
    }
    if !rebuilt && update.semantics.ids_lost {
        // An id (or a label's `for`) went away or changed: whoever referred to the old value
        // (`aria-labelledby`, `<label for>`, ...) has a stale name. Every element that looks others up by
        // id is computed again; this happens once per update, not once per changed id.
        if let Some(session) = update.semantics.session_if_rooted() {
            queue.extend(
                session
                    .by_id
                    .values()
                    .filter(|node| {
                        node.kind() == NodeKind::Element &&
                            (has_idref_attribute(**node) ||
                                LABELABLE.iter().any(|tag| *tag == node.local_name()))
                    })
                    .map(|node| node.dom_node()),
            );
        }
        update.semantics.ids_lost = false;
    }

    let mut seen: FxHashSet<OpaqueNode> = FxHashSet::default();
    for dom_node in queue {
        let opaque = dom_node.opaque();
        if !seen.insert(opaque) {
            continue;
        }
        let retry = retried.contains(&opaque);
        if !retry && update.semantics.computed.contains(&opaque) {
            // Computed in this update, from the final DOM.
            continue;
        }
        recompute(tree, update, &dom_node, retry);
    }
}

/// Compute an element again and send it if it changed.
fn recompute<'update>(
    tree: &mut AccessibilityTree,
    update: &mut AccessibilityUpdate<'update>,
    dom_node: &ServoLayoutNode<'update>,
    last_try: bool,
) {
    if !dom_node.is_element() {
        return;
    }
    let Some(node_ref) = tree.node_for_opaque(dom_node.opaque()) else {
        return;
    };
    let mut node = node_ref.borrow_mut();
    compute_element(&mut node, dom_node, tree, update, last_try);
    if node.dirty_state().updated() {
        update.add(&mut node);
    }
}

/// The nodes to recompute because of what happened to `sources`.
fn collect_dependants<'update>(
    tree: &AccessibilityTree,
    update: &mut AccessibilityUpdate<'update>,
    sources: &[Source<'update>],
    out: &mut Vec<ServoLayoutNode<'update>>,
) {
    for source in sources {
        structural_dependants(source, out);
    }
    // Elements that name or describe themselves by id (`aria-labelledby`, `aria-describedby`, a
    // `<label for>`): only a document that uses ids at all has any, and only then is the reverse map
    // worth building.
    if !tree.uses_idrefs() {
        return;
    }
    let Some(first) = sources.first() else {
        return;
    };
    let first = AccNode::new(first.node);
    let cache = &mut update.semantics;
    let root = cache.root_for(first);
    let session = cache.session_for(first);
    let doc = AccDocument::new(root);
    let index = DependantsIndex::build(&doc, &session.ids);
    for source in sources {
        let node = AccNode::new(source.node);
        for id in index.dependants(node, &session.ids) {
            if let Some(dependant) = session.by_id.get(&id.0) {
                out.push(dependant.dom_node());
            }
        }
    }
}

/// The nodes that depend on `source` by where they are, not by an id: the ancestors that may take
/// their name from content, the controls of a wrapping `<label>`, the element that owns the
/// user-agent widget the node is in (the text of a text field, the thumb of a slider), and the
/// `<select>` of an `<option>`.
fn structural_dependants<'update>(
    source: &Source<'update>,
    out: &mut Vec<ServoLayoutNode<'update>>,
) {
    let node = AccNode::new(source.node);
    if source.include_self && may_name_from_content(node) {
        out.push(source.node);
    }
    if node.is_element("label") {
        wrapped_controls(node, out);
    }
    let mut ancestor = node.parent();
    let mut depth = 0;
    while let Some(current) = ancestor {
        if depth >= ANCESTOR_LIMIT {
            break;
        }
        if may_name_from_content(current) {
            out.push(current.dom_node());
            if current.is_element("label") {
                wrapped_controls(current, out);
            }
        }
        if current.is_element("select") &&
            (node.is_element("option") || node.is_element("optgroup"))
        {
            out.push(current.dom_node());
        }
        ancestor = current.parent();
        depth += 1;
    }
    if let Some(host) = widget_host(source.node) {
        out.push(host);
    }
}

/// The controls inside a `<label>` (which the label names without a `for`).
fn wrapped_controls<'update>(label: AccNode<'update>, out: &mut Vec<ServoLayoutNode<'update>>) {
    out.extend(
        a11y_dom::descendants(label)
            .filter(|descendant| LABELABLE.iter().any(|tag| *tag == descendant.local_name()))
            .map(AccNode::dom_node),
    );
}

#[cfg(test)]
mod tests {
    use accesskit::{Affine, Rect, Toggled};

    use super::*;

    fn button(label: &str) -> accesskit::Node {
        let mut node = accesskit::Node::new(Role::Button);
        node.set_label(label);
        node
    }

    #[test]
    fn same_node_is_no_change() {
        let old = button("Save");
        assert_eq!(
            merge_computed(&old, button("Save"), None, &ResolvedRelations::default()),
            None
        );
    }

    #[test]
    fn a_new_label_is_a_text_change() {
        let old = button("Save");
        let (new, damage) = merge_computed(
            &old,
            button("Save all"),
            None,
            &ResolvedRelations::default(),
        )
        .expect("a changed label is a change");
        assert_eq!(new.label(), Some("Save all"));
        assert_eq!(damage, LocalAccessibilityDamage::TextChanged);
    }

    #[test]
    fn a_label_that_goes_away_is_a_text_change() {
        let old = button("Save");
        let (new, damage) = merge_computed(
            &old,
            accesskit::Node::new(Role::Button),
            None,
            &ResolvedRelations::default(),
        )
        .expect("a removed label is a change");
        assert_eq!(new.label(), None);
        assert_eq!(damage, LocalAccessibilityDamage::TextChanged);
    }

    #[test]
    fn description_and_value_count_as_text() {
        let old = accesskit::Node::new(Role::TextInput);
        let mut with_description = accesskit::Node::new(Role::TextInput);
        with_description.set_description("Your name");
        let (_, damage) =
            merge_computed(&old, with_description, None, &ResolvedRelations::default()).unwrap();
        assert_eq!(damage, LocalAccessibilityDamage::TextChanged);

        let mut with_value = accesskit::Node::new(Role::TextInput);
        with_value.set_value("Ada");
        let (_, damage) =
            merge_computed(&old, with_value, None, &ResolvedRelations::default()).unwrap();
        assert_eq!(damage, LocalAccessibilityDamage::TextChanged);
    }

    #[test]
    fn a_new_role_is_a_role_change() {
        let old = accesskit::Node::new(Role::GenericContainer);
        let (new, damage) = merge_computed(
            &old,
            accesskit::Node::new(Role::Heading),
            None,
            &ResolvedRelations::default(),
        )
        .unwrap();
        assert_eq!(new.role(), Role::Heading);
        assert_eq!(damage, LocalAccessibilityDamage::RoleChanged);
    }

    #[test]
    fn a_role_and_a_label_change_together() {
        let old = accesskit::Node::new(Role::GenericContainer);
        let (_, damage) =
            merge_computed(&old, button("Go"), None, &ResolvedRelations::default()).unwrap();
        assert_eq!(
            damage,
            LocalAccessibilityDamage::RoleChanged | LocalAccessibilityDamage::TextChanged
        );
    }

    #[test]
    fn a_state_change_is_a_change_without_damage_bits() {
        let old = accesskit::Node::new(Role::CheckBox);
        let mut checked = accesskit::Node::new(Role::CheckBox);
        checked.set_toggled(Toggled::True);
        let (new, damage) =
            merge_computed(&old, checked, None, &ResolvedRelations::default()).unwrap();
        assert_eq!(new.toggled(), Some(Toggled::True));
        assert!(damage.is_empty());
    }

    #[test]
    fn children_bounds_transform_hidden_and_tag_survive() {
        let mut old = button("Save");
        old.set_children(vec![NodeId(7), NodeId(8)]);
        old.set_bounds(Rect::new(1.0, 2.0, 30.0, 40.0));
        old.set_transform(Affine::translate((0.0, -5.0)));
        old.set_hidden();
        old.set_html_tag("button");

        // Same computed semantics: nothing changed, though the computed node has none of those.
        assert_eq!(
            merge_computed(&old, button("Save"), None, &ResolvedRelations::default()),
            None
        );

        let (new, damage) =
            merge_computed(&old, button("Store"), None, &ResolvedRelations::default()).unwrap();
        assert_eq!(new.children(), &[NodeId(7), NodeId(8)]);
        assert_eq!(new.bounds(), Some(Rect::new(1.0, 2.0, 30.0, 40.0)));
        assert_eq!(new.transform(), Some(&Affine::translate((0.0, -5.0))));
        assert!(new.is_hidden());
        assert_eq!(new.html_tag(), Some("button"));
        assert_eq!(damage, LocalAccessibilityDamage::TextChanged);
    }

    #[test]
    fn a_tag_the_computation_sets_wins() {
        let mut old = button("Save");
        old.set_html_tag("button");
        let mut computed = button("Save");
        computed.set_html_tag("input");
        let (new, _) = merge_computed(&old, computed, None, &ResolvedRelations::default()).unwrap();
        assert_eq!(new.html_tag(), Some("input"));
    }

    #[test]
    fn an_unhidden_node_stays_unhidden() {
        let old = button("Save");
        let (new, _) =
            merge_computed(&old, button("Store"), None, &ResolvedRelations::default()).unwrap();
        assert!(!new.is_hidden());
    }

    #[test]
    fn a_graft_is_set_and_removed() {
        let old = accesskit::Node::new(Role::Iframe);
        let (grafted, damage) = merge_computed(
            &old,
            accesskit::Node::new(Role::Iframe),
            Some(TreeId::ROOT),
            &ResolvedRelations::default(),
        )
        .expect("a new graft is a change");
        assert_eq!(grafted.tree_id(), Some(TreeId::ROOT));
        assert!(damage.is_empty());

        // The same graft again is no change; no graft removes it.
        assert_eq!(
            merge_computed(
                &grafted,
                accesskit::Node::new(Role::Iframe),
                Some(TreeId::ROOT),
                &ResolvedRelations::default()
            ),
            None
        );
        let (ungrafted, _) = merge_computed(
            &grafted,
            accesskit::Node::new(Role::Iframe),
            None,
            &ResolvedRelations::default(),
        )
        .expect("removing a graft is a change");
        assert_eq!(ungrafted.tree_id(), None);
    }

    #[test]
    fn a_graft_never_has_children() {
        let mut old = accesskit::Node::new(Role::Iframe);
        old.set_children(vec![NodeId(1)]);
        let (grafted, _) = merge_computed(
            &old,
            accesskit::Node::new(Role::Iframe),
            Some(TreeId::ROOT),
            &ResolvedRelations::default(),
        )
        .unwrap();
        assert!(grafted.children().is_empty());
        assert_eq!(grafted.tree_id(), Some(TreeId::ROOT));
    }

    #[test]
    fn relations_are_applied_and_compared() {
        let old = accesskit::Node::new(Role::TextInput);
        let relations = ResolvedRelations {
            labelled_by: vec![NodeId(3)],
            described_by: vec![NodeId(4), NodeId(5)],
            controls: vec![NodeId(6)],
            error_message: Some(NodeId(9)),
            active_descendant: Some(NodeId(10)),
            ..ResolvedRelations::default()
        };
        let (new, damage) = merge_computed(
            &old,
            accesskit::Node::new(Role::TextInput),
            None,
            &relations,
        )
        .unwrap();
        assert_eq!(new.labelled_by(), &[NodeId(3)]);
        assert_eq!(new.described_by(), &[NodeId(4), NodeId(5)]);
        assert_eq!(new.controls(), &[NodeId(6)]);
        assert_eq!(new.error_message(), Some(NodeId(9)));
        assert_eq!(new.active_descendant(), Some(NodeId(10)));
        assert!(damage.is_empty());

        // The same relations again are no change.
        assert_eq!(
            merge_computed(
                &new,
                accesskit::Node::new(Role::TextInput),
                None,
                &relations
            ),
            None
        );
        // Dropping them is.
        assert!(
            merge_computed(
                &new,
                accesskit::Node::new(Role::TextInput),
                None,
                &ResolvedRelations::default()
            )
            .is_some()
        );
    }

    #[test]
    fn empty_relations_set_nothing() {
        let mut node = accesskit::Node::new(Role::Button);
        ResolvedRelations::default().apply(&mut node);
        assert_eq!(node, accesskit::Node::new(Role::Button));
    }
}
