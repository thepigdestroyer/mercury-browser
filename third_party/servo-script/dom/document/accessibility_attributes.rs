/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

// Mercury: a11y-1 item 4 (`layout-glue`), EE3 and EE4. This whole file is Mercury's own: upstream
// Servo 0.8 damages the accessibility tree for a change of `role` and of the data of a text node
// only. See `.superpowers/sdd/lane-a11y-1-brief.md`, item 4.

//! Which DOM changes make a node's accessibility properties stale.
//!
//! `mercury-a11y` computes a node's role, name, description, states, value and relations from its
//! attributes, its native state and its value, so every change to one of those has to damage the
//! node (`AccessibilityDamage::Node`), or the tree keeps the old answer until the node is damaged
//! for another reason. The damage is lazy: [`Node::add_pending_accessibility_damage`] records
//! nothing unless the document's accessibility is active, so a page pays for what is here (a name
//! comparison) only while an assistive technology is attached.
//!
//! * [`damage_for_attribute_change`]: called once, from `Element::attribute_mutated`, for every
//!   attribute that is set, changed or removed. It damages the node for the attributes in
//!   [`affects_accessibility`]. Other elements that name themselves by an `id` or `for` that changed
//!   are found in layout, not here: layout remembers the `id`/`for` each node had and recomputes the
//!   elements that look others up by id when one changed, once per update (`semantics.rs`), where a
//!   script-side search would cost a walk of the document for every changed id.
//! * [`damage_for_state_change`]: called from `Element::set_state`, the one place the native state
//!   flags (`:checked`, `:disabled`, `:invalid`, `:indeterminate`, `:required`, `:read-write`,
//!   `:open`, `:modal`, ...) change, with the flag that changed; only the flags that become
//!   accessibility state damage the node (`hover`, `focus` and `active` do not).
//! * [`damage_for_value_change`]: called from the places where the *value* of a form control changes
//!   without an attribute changing (the text a user typed, a range input's number, a select's
//!   selection). Every site is listed in `.superpowers/sdd/a11y-1-logs/notes/item4.md`.

use html5ever::LocalName;
use js::context::{JSContext, NoGC};
use layout_api::AccessibilityDamage;
use stylo_dom::ElementState;

use crate::dom::node::Node;

/// Whether a change to the attribute with this name can change what `mercury-a11y` computes for the
/// node: every `aria-*` attribute, the attributes that name, describe or give a state to a native
/// element, and those that say which elements refer to which.
fn affects_accessibility(name: &LocalName) -> bool {
    let name: &str = name;
    if name.starts_with("aria-") {
        return true;
    }
    matches!(
        name,
        "role" |
            "alt" |
            "title" |
            "for" |
            "id" |
            "value" |
            "placeholder" |
            "type" |
            "hidden" |
            "disabled" |
            "checked" |
            "selected" |
            "open" |
            "href" |
            "required" |
            "readonly" |
            "min" |
            "max" |
            "step" |
            "inert" |
            "multiple" |
            "contenteditable" |
            "tabindex" |
            "name" |
            "label" |
            "summary" |
            "scope" |
            "span" |
            "colspan" |
            "rowspan"
    )
}

/// Damage a node's accessibility properties after one of its attributes changed.
///
/// Called for every mutation of every attribute; a name that does not matter costs one comparison.
pub(crate) fn damage_for_attribute_change(node: &Node, name: &LocalName, cx: &mut JSContext) {
    if affects_accessibility(name) {
        node.add_pending_accessibility_damage(cx.no_gc(), AccessibilityDamage::Node);
    }
}

/// The native state flags that `mercury-a11y` reads (see `HostState::native_flags`).
fn accessibility_state() -> ElementState {
    ElementState::CHECKED |
        ElementState::INDETERMINATE |
        ElementState::DISABLED |
        ElementState::INVALID |
        ElementState::REQUIRED |
        ElementState::READWRITE |
        ElementState::OPEN |
        ElementState::MODAL |
        ElementState::INERT
}

/// Damage a node's accessibility properties after one of its native state flags changed (`which` is
/// the flag or flags that did).
pub(crate) fn damage_for_state_change(node: &Node, which: ElementState, no_gc: &NoGC) {
    if which.intersects(accessibility_state()) {
        node.add_pending_accessibility_damage(no_gc, AccessibilityDamage::Node);
    }
}

/// Damage a node's accessibility properties after the value of the control changed without an
/// attribute changing: the text of a text field, the number of a range input, the selected option of
/// a `<select>`.
pub(crate) fn damage_for_value_change(node: &Node, no_gc: &NoGC) {
    node.add_pending_accessibility_damage(no_gc, AccessibilityDamage::Node);
}
