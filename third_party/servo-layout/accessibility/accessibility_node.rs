/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

// Mercury: a11y-1 item 4: `VecDeque` and `FxHashSet` were only used by the name-from-descendants code
// that `semantics::update_semantics` replaces.
use std::fmt::Debug;
use std::sync::LazyLock;

use accesskit::{Affine, NodeId, Role};
use app_units::Au;
use bitflags::bitflags;
use euclid::Rect;
use layout_api::{
    ARIA_ROLE_TO_ACCESSKIT_ROLE, AccessibilityDamage, BoxAreaType, LayoutElement, LayoutNode,
    LayoutNodeType, SUPPORTED_ARIA_ROLES,
};
use log::trace;
use num_traits::ToPrimitive;
use rustc_hash::FxHashMap;
use script::layout_dom::{ServoLayoutElement, ServoLayoutNode};
use servo_base::print_tree::PrintTree;
use servo_config::pref;
use style::dom::{NodeInfo, OpaqueNode};
use style_traits::CSSPixel;
use web_atoms::{LocalName, local_name, ns};
use webrender_api::units::LayoutVector2D;

use super::accessibility_tree::{AccessibilityTree, AccessibilityUpdate, TreeChange};
// Mercury: a11y-1 item 4: roles, names, states and relations are computed in `semantics.rs`.
use super::{dom_adapter, semantics};
use crate::ArcRefCell;
use crate::accessibility::AccessibilityContext;
use crate::cell::WeakRefCell;
use crate::query::{BoxAreaInclusion, process_box_area_request};

/// Convert a rectangle as layout reports it into the one [`accesskit`] wants.
fn au_rect_to_accesskit_rect(rect: Rect<Au, CSSPixel>) -> accesskit::Rect {
    accesskit::Rect::new(
        rect.min_x().to_f64_px(),
        rect.min_y().to_f64_px(),
        rect.max_x().to_f64_px(),
        rect.max_y().to_f64_px(),
    )
}

fn scroll_offset_to_affine(layout_vector: LayoutVector2D) -> Affine {
    Affine::translate((
        -layout_vector.x.to_f64().unwrap_or(0.),
        -layout_vector.y.to_f64().unwrap_or(0.),
    ))
}

bitflags! {
    /// Flags tracking an [`AccessibilityNode`]'s dirty state during an update. All flags which are
    /// set during the update should be unset by the end of the update.
    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    pub(super) struct DirtyState : u16 {
        /// At least one descendant of this node has unresolved damage from the DOM tree.
        const DescendantHasDamage = 0b0001;
        /// This node has unresolved damage from the DOM tree.
        const HasDamage = 0b0010;
        /// This node's data changed, but it hasn't yet been added to the [`AccessibilityUpdate`].
        const Updated = 0b0100;
    }
}

bitflags! {
    /// Damage which was caused by changes to the accessibility tree. These changes can cause other
    /// properties to need to be re-computed based on the updated values, either on the same node or
    /// on other nodes.
    #[derive(Clone, Copy, Default, Debug, Eq, PartialEq)]
    pub(super) struct LocalAccessibilityDamage: u16 {
        /// This node's children changed, and/or any node in its subtree changed.
        const SubtreeChanged = 0b0001;
        /// This node's computed role changed.
        const RoleChanged = 0b0010;
        /// This node's computed label or text value (for a text node) changed.
        const TextChanged = 0b0100;
        /// This node's visibility changed.
        const VisibilityChanged = 0b1000;
    }
}

pub(super) struct AccessibilityNode {
    /// The unique ID for the node. This is used both as a key in [`AccessibilityTree`]'s cache of
    /// nodes, and as an identifier in [`accesskit`] datastructures: [`accesskit::Node`]s,
    /// [`accesskit::TreeUpdate`]s and [`accesskit::ActionRequest`]s.
    id: NodeId,
    /// The computed [`accesskit::Node`] data. This will be copied and serialized into a
    /// [`accesskit::TreeUpdate`] whenever it is changed during an update.
    accesskit_node: accesskit::Node,
    /// This node's parent, if any.
    parent_node: Option<WeakRefCell<AccessibilityNode>>,
    /// All this node's children.
    child_nodes: Vec<ArcRefCell<AccessibilityNode>>,
    /// The [`OpaqueNode`] for the DOM node which corresponds to this accessibility node, if any.
    /// An accessibility node may not correspond to a DOM node if it corresponds to a
    /// pseudo-element, or in a test.
    opaque_node: Option<OpaqueNode>,
    /// This node's scroll offset, if it is a scroll container which has scrolled. This is used to
    /// translate this node's children.
    scroll_offset: Option<LayoutVector2D>,
    /// Any dirty state for the current update.
    dirty_state: DirtyState,
    // Mercury: a11y-1 item 4: a node is hidden when layout hides it (`display: none`) or its semantics
    // do (`aria-hidden`, `inert`, `visibility: hidden`); two flags, so that one does not clear the other.
    layout_hidden: bool,
    semantic_hidden: bool,
    // Mercury: a11y-1 item 4: what the node's `id` (and, for a `<label>`, `for`) were when its semantics
    // were last computed, so that a change tells `semantics::apply_followups` that elements which
    // referred to the old value must be computed again.
    id_signature: Option<Box<str>>,
}

impl AccessibilityNode {
    pub(super) fn new(id: NodeId) -> Self {
        Self::new_with_role(id, Role::Unknown)
    }

    pub(super) fn new_with_role(id: NodeId, role: Role) -> Self {
        Self {
            id,
            accesskit_node: accesskit::Node::new(role),
            parent_node: None,
            child_nodes: vec![],
            opaque_node: None,
            scroll_offset: None,
            dirty_state: DirtyState::empty(),
            layout_hidden: false,   // Mercury: a11y-1 item 4
            semantic_hidden: false, // Mercury: a11y-1 item 4
            id_signature: None,     // Mercury: a11y-1 item 4
        }
    }

    /// Update this node and its subtree based on damage from the DOM.
    ///
    /// - First, if this node has damage from the DOM to be resolved, update the node from the DOM
    ///   tree, recursively populating any new children.
    /// - Next, recursively call this method for any children which are dirty, or have dirty
    ///   descendants.
    /// - Finally, update any properties on this node which are may have changed due to other
    ///   changes in the tree.
    ///
    /// At the end of this method, both `has_dirty_descendants` and `is_dirty` should be false for
    /// this node and all its descendants.
    pub(super) fn update_subtree<'update>(
        &mut self,
        ref_self: ArcRefCell<Self>,
        damage_from_parent: AccessibilityDamage,
        hidden: bool,
        context: &AccessibilityContext,
        tree: &mut AccessibilityTree,
        update: &mut AccessibilityUpdate<'update>,
    ) -> LocalAccessibilityDamage {
        let mut local_damage = LocalAccessibilityDamage::empty();

        let dom_node = update.take_dom_node(&self.id);
        let mut damage = update.take_damage(&self.id) | damage_from_parent; // Mercury: a11y-1 item 4: `mut`
        let mut children_changed = false;

        if let Some(dom_node) = dom_node {
            local_damage.insert(self.update_properties_and_children_from_dom_node(
                &ref_self, &dom_node, damage, tree, update,
            ));
            // Mercury: a11y-1 item 4: a node that is no longer hidden by its semantics has no bounds
            // yet (they are not computed in a hidden subtree), nor have its descendants: treat it as
            // having layout damage. `update_properties_and_children_from_dom_node` already walked the
            // children for the same reason.
            if local_damage.contains(LocalAccessibilityDamage::VisibilityChanged) &&
                !self.is_hidden()
            {
                damage |= AccessibilityDamage::Layout;
            }
            // Mercury: a11y-1 item 4: other nodes' names may depend on this node's children.
            if local_damage.contains(LocalAccessibilityDamage::SubtreeChanged) {
                update.semantics.note_children_changed(dom_node);
            }
            local_damage
                .insert(self.update_node_from_layout(&dom_node, damage, hidden, context, update));

            if local_damage.contains(LocalAccessibilityDamage::SubtreeChanged) {
                children_changed = true;
            }

            self.dirty_state -= DirtyState::HasDamage;
        }

        let layout_damage = damage & AccessibilityDamage::Layout;
        if self.dirty_state.descendant_has_damage() || !layout_damage.is_empty() {
            for child_node in self.children() {
                let child_node_ref = child_node.clone();
                let mut child_node = child_node.borrow_mut();
                let child_local_damage = child_node.update_subtree(
                    child_node_ref,
                    layout_damage,
                    hidden || self.is_hidden(),
                    context,
                    tree,
                    update,
                );
                if !child_local_damage.is_empty() {
                    local_damage.insert(LocalAccessibilityDamage::SubtreeChanged);
                    if child_local_damage.contains(LocalAccessibilityDamage::VisibilityChanged) {
                        children_changed = true;
                    }
                }
            }
        }
        self.dirty_state -= DirtyState::DescendantHasDamage;

        if children_changed && let Some(scroll_offset) = self.scroll_offset {
            // If this node may have new, or newly-visible, children, update their scroll offsets.
            self.set_scroll_offset(scroll_offset, update);
        }

        local_damage.insert(self.update_node_local(local_damage, update));

        if self.dirty_state.updated() {
            update.add(self);
        }

        local_damage
    }

    /// Update each of this node's ancestors based on changes which have already been applied in the
    /// tree.
    pub(super) fn update_ancestors(
        &self,
        local_damage: LocalAccessibilityDamage,
        update: &mut AccessibilityUpdate,
    ) {
        if local_damage.is_empty() {
            return;
        }
        for node in self.ancestors() {
            let mut node = node.borrow_mut();
            node.update_node_local(LocalAccessibilityDamage::SubtreeChanged, update);
            node.dirty_state -= DirtyState::DescendantHasDamage;
            if node.dirty_state.updated() {
                update.add(&mut node);
            }
        }
    }

    pub(super) fn initialize_from_dom_node(&mut self, dom_node: &ServoLayoutNode) {
        self.opaque_node = Some(dom_node.opaque());
        if let Some(dom_element) = dom_node.as_element() {
            let local_name = dom_element.local_name().to_ascii_lowercase();
            self.set_html_tag(&local_name);
        }
    }

    /// Update the given [`AccessibilityNode`] from its corresponding DOM node and
    /// [`AccessibilityDamage`].
    /// If it has new children, those will be created here, but not yet populated.
    // Any changed nodes will be added to the given [`AccessibilityUpdate`].
    fn update_properties_and_children_from_dom_node<'update>(
        &mut self,
        ref_self: &ArcRefCell<Self>,
        dom_node: &ServoLayoutNode<'update>,
        mut dom_damage: AccessibilityDamage, // Mercury: a11y-1 item 4: `mut`
        tree: &mut AccessibilityTree,
        update: &mut AccessibilityUpdate<'update>,
    ) -> LocalAccessibilityDamage {
        let mut local_damage = LocalAccessibilityDamage::empty();

        if !dom_damage.intersects(
            AccessibilityDamage::Node | AccessibilityDamage::Children | AccessibilityDamage::Layout,
        ) {
            return local_damage;
        }

        // We check for layout damage here because we need to walk the DOM children of nodes with
        // layout damage in order to be able to recompute their bounds. Text nodes have neither
        // bounds nor child nodes, so if the only damage is layout, we can early return here.
        if dom_damage == AccessibilityDamage::Layout && dom_node.is_text_node() {
            return local_damage;
        }

        update.counters.nodes_updated_from_dom += 1;

        if dom_damage.intersects(AccessibilityDamage::Node) {
            // Mercury: a11y-1 item 4: the semantics need the tree (relations) and the update (the id index).
            local_damage.insert(self.update_properties_from_dom_node(dom_node, tree, update));
        }

        // Mercury: a11y-1 item 4: a node that stopped being hidden by its semantics needs its
        // children and their bounds again (see `update_subtree`).
        if local_damage.contains(LocalAccessibilityDamage::VisibilityChanged) && !self.is_hidden() {
            dom_damage |= AccessibilityDamage::Layout;
        }

        if dom_damage.intersects(AccessibilityDamage::Children | AccessibilityDamage::Layout) {
            // If this node has damage from layout, this ensures that all of its children have
            // their corresponding DOM nodes in `update`.
            local_damage
                .insert(self.update_children_from_dom_node(ref_self, dom_node, tree, update));
        }

        local_damage
    }

    /// Update this node's [`Self::children`] from its corresponding DOM node.
    /// If it has new children, those will be created here, but not yet populated.
    fn update_children_from_dom_node<'update>(
        &mut self,
        ref_self: &ArcRefCell<AccessibilityNode>,
        dom_node: &ServoLayoutNode<'update>,
        tree: &mut AccessibilityTree,
        update: &mut AccessibilityUpdate<'update>,
    ) -> LocalAccessibilityDamage {
        // Mercury: a11y-1 item 4: an `<iframe>` has no children here: its node is a graft.
        // Mercury: A11Y-2 EE6: text runs are children of a label; stub until A11Y-2 task 10.
        super::text_runs::update_children(self, dom_node, tree, update);
        let mut remaining_dom_children = dom_adapter::flat_children(dom_node).peekable();
        let mut old_child_ids = self.child_ids().iter().peekable();

        // Iterate over existing children and DOM children while they match. No action is necessary
        // for these nodes.
        let mut unchanged_count = 0usize;
        while let Some(&old_id) = old_child_ids.peek() &&
            let Some(dom_child) = remaining_dom_children.peek()
        {
            if tree.existing_id_for_opaque(dom_child.opaque()) == Some(*old_id) {
                update.insert_dom_node(*old_id, *dom_child);
                unchanged_count += 1;
                old_child_ids.next();
                remaining_dom_children.next();
            } else {
                break;
            }
        }

        // If we iterated over all the DOM children without finding any changes, we're done.
        if old_child_ids.peek().is_none() && remaining_dom_children.peek().is_none() {
            return LocalAccessibilityDamage::empty();
        }

        // Remove all child nodes after the first `unchanged_count`.
        self.child_nodes.truncate(unchanged_count);
        let mut new_child_ids = Vec::from(self.child_ids());
        for removed_child_id in new_child_ids.split_off(unchanged_count) {
            update.set_tree_state_change(removed_child_id, TreeChange::Removed);
        }

        // Then, (re-)add all the remaining DOM children. Note that this means that some children
        // may end up being "Moved" even though they haven't changed parents, and may even be in the
        // same position as previously.
        let weak_self = ref_self.downgrade();
        for dom_child in remaining_dom_children {
            let (child_id, child_ref) = tree.get_or_create_node(&dom_child, update);
            // TODO(#47162): Since we need to update bounds for all nodes, we need to ensure every
            // AccessibilityNode has a corresponding DOM node available to be retrieved from the
            // AccessibilityUpdate. Once we no longer update bounds on all nodes, we won't need to
            // add all nodes like this.
            update.insert_dom_node(child_id, dom_child);

            // Update self.child_nodes in place.
            self.child_nodes.push(child_ref.clone());
            new_child_ids.push(child_id);

            let mut child = child_ref.borrow_mut();
            child.parent_node = Some(weak_self.clone());

            if update.is_new(&child_id) {
                self.dirty_state |= DirtyState::DescendantHasDamage;
            } else {
                update.set_tree_state_change(child_id, TreeChange::PendingMove);
            }

            self.dirty_state
                .propagate_descendant_has_damage(child.dirty_state);
        }

        // We can't update the AccessKit node's `children` in place, so we build up the full list
        // and then set it here.
        self.accesskit_node.set_children(new_child_ids);
        self.dirty_state |= DirtyState::Updated;

        LocalAccessibilityDamage::SubtreeChanged
    }

    /// Update this node's properties from its corresponding DOM node.
    // Mercury: a11y-1 item 4: takes the tree and the update, which `semantics::update_semantics` needs.
    fn update_properties_from_dom_node<'update>(
        &mut self,
        dom_node: &ServoLayoutNode<'update>,
        tree: &mut AccessibilityTree,
        update: &mut AccessibilityUpdate<'update>,
    ) -> LocalAccessibilityDamage {
        let mut local_damage = LocalAccessibilityDamage::empty();
        // Mercury: a11y-1 item 4: role, name, description, states, value, relations and actions come
        // from `mercury_a11y` (was: `self.set_role(role_from_dom_node(dom_node))`).
        local_damage.insert(semantics::update_semantics(self, dom_node, tree, update));
        // Mercury: A11Y-2 EE6: stub until A11Y-2 task 10 (`text_value` returns None now).
        if let Some(text) = super::text_runs::text_value(dom_node) {
            local_damage.insert(self.set_value(&text));
        }
        if dom_node.type_id() == Some(LayoutNodeType::Text) {
            let text_content = dom_node.text_content();
            trace!("node text content = {text_content:?}");
            // FIXME: this should take into account editing selection units (grapheme clusters?)
            local_damage.insert(self.set_value(&text_content));
        }

        local_damage
    }

    /// Update this node's bounds from the current layout geometry.
    fn update_node_from_layout(
        &mut self,
        dom_node: &ServoLayoutNode<'_>,
        layout_damage: AccessibilityDamage,
        hidden: bool,
        context: &AccessibilityContext,
        update: &mut AccessibilityUpdate,
    ) -> LocalAccessibilityDamage {
        let mut local_damage = LocalAccessibilityDamage::empty();
        // Mercury: A11Y-2 EE10: union of the node's fragment rectangles; stub until A11Y-2 task 14.
        super::geometry::bounds_for(self, dom_node);

        // Don't update bounds on nodes in hidden subtrees.
        if hidden || !layout_damage.intersects(AccessibilityDamage::Layout) {
            return local_damage;
        }

        if let Some(dom_element) = dom_node.as_element() &&
            dom_element.style_data().is_some()
        {
            let data = dom_element.element_data();
            let style = data.styles.primary();
            if style.get_display().is_none() {
                self.clear_bounds();
                local_damage.insert(self.set_hidden());
            } else {
                local_damage.insert(self.clear_hidden());
            }
        }

        if self.is_hidden() {
            return local_damage;
        }

        update.counters.nodes_updated_bounds += 1;

        // Border box without transforms. Bounds are in CSS pixels, relative to the document origin;
        // scroll containers set translations on their child nodes, and the embedder's graft node
        // carries the transform that composes them into AccessKit's coordinate space (see the
        // "Coordinates" section of
        // <https://docs.rs/accesskit/latest/accesskit/struct.Node.html>).
        // TODO(#47166): This doesn't take any CSS transforms into account.
        let bounds = process_box_area_request(
            context.layout_thread,
            context.stacking_context_tree,
            *dom_node,
            BoxAreaType::Border,
            BoxAreaInclusion::Inlines,
        )
        .map(au_rect_to_accesskit_rect);

        // For now only nodes with a box of their own get bounds; anything else, including
        // `display: none` content, gets its bounds cleared. That leaves two kinds of nodes
        // without geometry which assistive technology would like to have some:
        //
        // TODO(#47164): A text node never has bounds of its own: `LayoutBox::Text` has no
        // `LayoutBoxBase`, and `Fragment::Text` has no box area, so the query above always returns
        // `None` for one. Text nodes should get the union of the rectangles of their own
        // `Fragment::Text` fragments, once `cumulative_box_area_rect()` can handle those.
        //
        // TODO(#47163): A `display: contents` element generates no box either. Other
        // engines (Blink, WebKit, Gecko) compute its bounds as the union of the bounding boxes of
        // its rendered descendants.
        match bounds {
            Some(bounds) => self.set_bounds(bounds),
            None => self.clear_bounds(),
        }
        local_damage
    }

    /// Update this node's properties based on changes already made to the accessibility tree.
    /// For example, if there were nodes added or removed in its subtree, its computed text may have
    /// changed, so that will be recomputed here.
    /// If any changes are made, add this node to the given [`AccessibilityUpdate`].
    fn update_node_local(
        &mut self,
        local_damage: LocalAccessibilityDamage,
        update: &mut AccessibilityUpdate,
    ) -> LocalAccessibilityDamage {
        let new_damage = LocalAccessibilityDamage::empty(); // Mercury: a11y-1 item 4: no longer `mut`
        if local_damage.is_empty() {
            return new_damage;
        }
        update.counters.nodes_updated_from_tree += 1;

        // Mercury: a11y-1 item 4: upstream recomputed the name of a heading or link here from its
        // descendants' text runs when the subtree or the role changed. Names are computed from the
        // DOM now (`semantics::update_semantics`), and `semantics::apply_followups` recomputes the
        // nodes whose name depends on a node that changed (an ancestor that names itself from its
        // content, a control and its `<label>`, ...). Removed: `label_from_descendants`,
        // `NAME_FROM_CONTENTS_ROLES`, `set_label`, `clear_label` and `value`.

        new_damage
    }

    pub(super) fn print(&self, print_tree: &mut PrintTree, tree: &AccessibilityTree) {
        let focused = if tree.focused_node_id() == Some(self.id) {
            "[focused] "
        } else {
            ""
        };
        let node_string = format!("{focused}{self:?}");

        if self.child_nodes.is_empty() {
            print_tree.add_item(node_string);
            return;
        }

        print_tree.new_level(node_string);

        for child in self.children() {
            child.borrow().print(print_tree, tree);
        }
        print_tree.end_level();
    }

    pub(super) fn clone_accesskit_node(&self) -> accesskit::Node {
        self.accesskit_node.clone()
    }

    pub(super) fn id(&self) -> NodeId {
        self.id
    }

    pub(super) fn opaque_node(&self) -> Option<OpaqueNode> {
        self.opaque_node
    }

    pub(super) fn dirty_state(&self) -> DirtyState {
        self.dirty_state
    }

    pub(super) fn add_dirty_state(&mut self, state: DirtyState) {
        self.dirty_state |= state;
    }

    pub(super) fn remove_dirty_state(&mut self, state: DirtyState) {
        self.dirty_state -= state;
    }

    pub(super) fn parent(&self) -> Option<ArcRefCell<AccessibilityNode>> {
        self.parent_node.as_ref().and_then(|weak| weak.upgrade())
    }

    pub(super) fn children(
        &self,
    ) -> impl DoubleEndedIterator<Item = &ArcRefCell<AccessibilityNode>> {
        self.child_nodes.iter()
    }

    #[cfg(test)]
    pub(super) fn set_children_for_testing(
        &mut self,
        child_ids: Vec<NodeId>,
        child_nodes: Vec<ArcRefCell<AccessibilityNode>>,
    ) {
        self.accesskit_node.set_children(child_ids);
        self.child_nodes = child_nodes;
    }

    pub(super) fn ancestors(&self) -> impl Iterator<Item = ArcRefCell<AccessibilityNode>> {
        AccessibilityNodeIterator::new(self.parent(), |node| node.parent_node.clone()?.upgrade())
    }

    pub(super) fn child_ids(&self) -> &[NodeId] {
        self.accesskit_node.children()
    }

    pub(super) fn set_scroll_offset(
        &mut self,
        offset: LayoutVector2D,
        update: &mut AccessibilityUpdate,
    ) {
        self.scroll_offset = Some(offset);
        let transform = scroll_offset_to_affine(offset);
        for child in self.children() {
            let mut child = child.borrow_mut();
            if child.is_hidden() {
                continue;
            }
            child.set_transform(transform);
            if child.dirty_state.updated() {
                update.add(&mut child);
            }
        }
    }

    pub(super) fn role(&self) -> Role {
        self.accesskit_node.role()
    }

    pub(super) fn set_role(&mut self, role: Role) -> LocalAccessibilityDamage {
        if role == self.accesskit_node.role() {
            return LocalAccessibilityDamage::empty();
        }
        self.accesskit_node.set_role(role);
        self.dirty_state |= DirtyState::Updated;
        LocalAccessibilityDamage::RoleChanged
    }

    fn label(&self) -> Option<&str> {
        self.accesskit_node.label()
    }

    // Mercury: a11y-1 item 4: `set_label` and `clear_label` are removed (the name is part of the node
    // that `semantics::update_semantics` computes); `accesskit_node` and `replace_accesskit_node` below
    // are how it reads and replaces that node.

    /// Remember the `id`/`for` signature of the DOM node. Returns whether the node had one before and it
    /// is different now (the node lost an id that other elements may have referred to).
    pub(super) fn replace_id_signature(&mut self, signature: Option<String>) -> bool {
        let changed = self
            .id_signature
            .as_deref()
            .is_some_and(|old| Some(old) != signature.as_deref());
        self.id_signature = signature.map(String::into_boxed_str);
        changed
    }

    /// The accesskit node as it is now.
    pub(super) fn accesskit_node(&self) -> &accesskit::Node {
        &self.accesskit_node
    }

    /// Replace the accesskit node with a computed one (see `semantics::merge_computed`, which keeps
    /// the children, bounds, transform and hidden flag this node owns).
    pub(super) fn replace_accesskit_node(&mut self, accesskit_node: accesskit::Node) {
        self.accesskit_node = accesskit_node;
        self.dirty_state |= DirtyState::Updated;
    }

    fn html_tag(&self) -> Option<&str> {
        self.accesskit_node.html_tag()
    }

    fn set_html_tag(&mut self, html_tag: &str) {
        if Some(html_tag) == self.accesskit_node.html_tag() {
            return;
        }
        self.accesskit_node.set_html_tag(html_tag);
        self.dirty_state |= DirtyState::Updated;
    }

    fn set_value(&mut self, value: &str) -> LocalAccessibilityDamage {
        if Some(value) == self.accesskit_node.value() {
            return LocalAccessibilityDamage::empty();
        }
        self.accesskit_node.set_value(value);
        self.dirty_state |= DirtyState::Updated;
        LocalAccessibilityDamage::TextChanged
    }

    fn is_hidden(&self) -> bool {
        self.accesskit_node.is_hidden()
    }

    // Mercury: a11y-1 item 4: layout's `display: none` and the semantics' hide are two flags and the
    // node is hidden when either is set, so that `clear_hidden` (layout says visible) never clears a
    // hide that `aria-hidden` or `inert` asked for.
    fn set_hidden(&mut self) -> LocalAccessibilityDamage {
        self.layout_hidden = true;
        self.sync_hidden()
    }

    fn clear_hidden(&mut self) -> LocalAccessibilityDamage {
        self.layout_hidden = false;
        self.sync_hidden()
    }

    /// Set whether the node's semantics hide it (`aria-hidden`, `inert`, `visibility: hidden`, ...).
    pub(super) fn set_semantic_hidden(&mut self, hidden: bool) -> LocalAccessibilityDamage {
        self.semantic_hidden = hidden;
        self.sync_hidden()
    }

    /// Make the accesskit node's hidden flag the OR of the two flags.
    fn sync_hidden(&mut self) -> LocalAccessibilityDamage {
        let hidden = self.layout_hidden || self.semantic_hidden;
        if hidden == self.is_hidden() {
            return LocalAccessibilityDamage::empty();
        }
        if hidden {
            self.accesskit_node.set_hidden();
        } else {
            self.accesskit_node.clear_hidden();
        }
        self.dirty_state |= DirtyState::Updated;
        LocalAccessibilityDamage::VisibilityChanged
    }

    fn bounds(&self) -> Option<accesskit::Rect> {
        self.accesskit_node.bounds()
    }

    fn set_bounds(&mut self, bounds: accesskit::Rect) {
        if Some(bounds) == self.accesskit_node.bounds() {
            return;
        }
        self.accesskit_node.set_bounds(bounds);
        self.dirty_state |= DirtyState::Updated;
    }

    fn clear_bounds(&mut self) {
        if self.accesskit_node.bounds().is_none() {
            return;
        }
        self.accesskit_node.clear_bounds();
        self.dirty_state |= DirtyState::Updated;
    }

    fn set_transform(&mut self, transform: Affine) {
        // TODO(#47166): Right now a node will only ever have a single transform from a scroll
        // container, if any. Once we correctly support CSS transforms, a node may have multiple
        // transforms, which we'll need to be able to combine.
        if self.accesskit_node.transform() == Some(&transform) {
            return;
        }
        if transform == Affine::IDENTITY {
            self.clear_transform();
            return;
        }
        self.accesskit_node.set_transform(transform);
        self.dirty_state |= DirtyState::Updated;
    }

    fn clear_transform(&mut self) {
        if self.accesskit_node.transform().is_none() {
            return;
        }
        self.accesskit_node.clear_transform();
        self.dirty_state |= DirtyState::Updated;
    }

    pub(super) fn assert_integrity(&self, expected_parent: Option<WeakRefCell<AccessibilityNode>>) {
        debug_assert!(pref!(expensive_accessibility_test_assertions_enabled));

        if let Some(actual_parent) = &self.parent_node {
            let expected = expected_parent.expect("Actual parent but no expected parent");
            let expected = expected.upgrade().expect("Expected parent was dropped");
            let actual = actual_parent.upgrade().expect("Actual parent was dropped");
            assert!(actual.ptr_eq(&expected));
        } else {
            assert!(
                expected_parent.is_none(),
                "Expected parent but no actual parent"
            );
        }

        assert!(
            self.dirty_state.is_empty(),
            "{self:?} has dirty state {:?}",
            self.dirty_state
        );

        let children_ids: Vec<_> = self.children().map(|child| child.borrow().id).collect();
        assert_eq!(
            children_ids,
            self.child_ids(),
            "children() IDs didn't match child_ids() for {self:?}"
        );
    }
}

impl Debug for AccessibilityNode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.is_hidden() {
            write!(f, "[hidden] ")?;
        }
        write!(f, "{:?}: {:?}", self.id, self.role())?;
        if let Some(html_tag) = self.html_tag() {
            write!(f, " (html_tag: {html_tag:?})")?;
        }
        if let Some(label) = self.label() {
            write!(f, "\nlabel: {label:?}")?;
        }
        if let Some(bounds) = self.bounds() {
            write!(f, "\nbounds: {bounds:?}")?;
        }
        if !self.child_ids().is_empty() {
            write!(f, "\nchildren: {:?}", self.child_ids())?;
        }
        Ok(())
    }
}

/// <https://w3c.github.io/aria/#host_general_role>
// Mercury: a11y-1 item 4: superseded by `mercury_a11y::roles`, kept so that the upstream text stays
// diffable until servo#46241 lands.
#[allow(dead_code)]
fn role_from_role_attribute(dom_element: &ServoLayoutElement<'_>) -> Option<Role> {
    let role_attribute = dom_element.attribute(&ns!(), &local_name!("role"))?;
    role_attribute
        .as_tokens()
        .iter()
        .filter_map(|role_name_in_attribute| SUPPORTED_ARIA_ROLES.get(role_name_in_attribute))
        .next()
        .and_then(|aria_role| ARIA_ROLE_TO_ACCESSKIT_ROLE.get(aria_role))
        .cloned()
}

#[allow(dead_code)] // Mercury: a11y-1 item 4: see `role_from_role_attribute`.
fn role_from_dom_node(dom_node: &ServoLayoutNode<'_>) -> Role {
    if let Some(dom_element) = dom_node.as_element() {
        role_from_role_attribute(&dom_element).unwrap_or_else(|| {
            let local_name = dom_element.local_name().to_ascii_lowercase();
            *HTML_ELEMENT_ROLE_MAPPINGS
                .get(&local_name)
                .unwrap_or(&Role::GenericContainer)
        })
    } else if dom_node.type_id() == Some(LayoutNodeType::Text) {
        Role::TextRun
    } else {
        Role::GenericContainer
    }
}

struct AccessibilityNodeIterator<I>
where
    I: Fn(&AccessibilityNode) -> Option<ArcRefCell<AccessibilityNode>>,
{
    next_value: Option<ArcRefCell<AccessibilityNode>>,
    next_fn: I,
}

impl<I> AccessibilityNodeIterator<I>
where
    I: Fn(&AccessibilityNode) -> Option<ArcRefCell<AccessibilityNode>>,
{
    fn new(next_value: Option<ArcRefCell<AccessibilityNode>>, next_fn: I) -> Self {
        AccessibilityNodeIterator {
            next_value,
            next_fn,
        }
    }
}

impl<I> Iterator for AccessibilityNodeIterator<I>
where
    I: Fn(&AccessibilityNode) -> Option<ArcRefCell<AccessibilityNode>>,
{
    type Item = ArcRefCell<AccessibilityNode>;

    fn next(&mut self) -> Option<Self::Item> {
        let next_value = self.next_value.take();
        self.next_value = next_value
            .as_ref()
            .and_then(|node| (self.next_fn)(&node.borrow()));
        next_value
    }
}

impl DirtyState {
    pub(super) fn updated(&self) -> bool {
        self.contains(DirtyState::Updated)
    }

    pub(super) fn descendant_has_damage(&self) -> bool {
        self.contains(DirtyState::DescendantHasDamage)
    }

    pub(super) fn propagate_descendant_has_damage(&mut self, child_dirty_state: DirtyState) {
        if child_dirty_state.self_or_descendant_has_damage() {
            self.insert(DirtyState::DescendantHasDamage)
        }
    }

    pub(super) fn self_or_descendant_has_damage(&self) -> bool {
        self.intersects(DirtyState::HasDamage | DirtyState::DescendantHasDamage)
    }
}

#[allow(dead_code)] // Mercury: a11y-1 item 4: see `role_from_role_attribute`.
static HTML_ELEMENT_ROLE_MAPPINGS: LazyLock<FxHashMap<LocalName, Role>> = LazyLock::new(|| {
    [
        // FIXME: only a with href!
        (local_name!("a"), Role::Link),
        (local_name!("article"), Role::Article),
        (local_name!("aside"), Role::Complementary),
        (local_name!("body"), Role::RootWebArea),
        (local_name!("footer"), Role::ContentInfo),
        (local_name!("h1"), Role::Heading),
        (local_name!("h2"), Role::Heading),
        (local_name!("h3"), Role::Heading),
        (local_name!("h4"), Role::Heading),
        (local_name!("h5"), Role::Heading),
        (local_name!("h6"), Role::Heading),
        (local_name!("header"), Role::Banner),
        (local_name!("hr"), Role::Splitter),
        (local_name!("main"), Role::Main),
        (local_name!("nav"), Role::Navigation),
        (local_name!("p"), Role::Paragraph),
    ]
    .into_iter()
    .collect()
});

// Mercury: a11y-1 item 4: `NAME_FROM_CONTENTS_ROLES` is removed; `accname` decides which roles take
// their name from their content (see `semantics.rs`).
