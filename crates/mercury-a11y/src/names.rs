// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! Accessible name and description (accname 1.2, HTML-AAM) on top of the `accname` crate.
//!
//! Owner: item 2 (`core-names`).
//!
//! # What this module does
//!
//! [`apply`] sets `label` and `description` on a [`NodeSemantics`]. The name comes from
//! `accname::name_rendered` (the host's computed `display` and `visibility` decide what is hidden
//! and what flows inline): `aria-labelledby` (IDREF list through the document's `IdIndex`; a target
//! that is hidden or `aria-hidden` still counts, its hidden descendants do not; a cycle ends),
//! `aria-label`, then the native text alternatives of HTML-AAM (`label[for]` and wrapping `label`,
//! `alt`, `legend`, `caption`, `figcaption`, the `title` of an `svg`, the `value` of a button, the
//! `placeholder` of a text field), then the content for the roles that take their name from it, then
//! `title`. The description comes from `aria-describedby`, then from `title` when the name came from
//! somewhere else.
//!
//! The decisions made around `accname` (each is tested):
//!
//! * **Prohibited names.** A role that may not have a name (`generic`, `paragraph`, `presentation`,
//!   `none`, `caption`, `code`, `emphasis`, `strong`, `insertion`, `deletion`, `subscript`,
//!   `superscript`, `term`, `time`) gets no `label`, whatever `aria-label` or `title` say (`accname`
//!   itself still answers the `title`). The `title` of such an element becomes its description.
//! * **`title` is a description when it is not the name**, also when the name came from the content
//!   (`<button title="Close">x</button>` is named "x" and described "Close"); the `accname` crate
//!   only does that when the name comes from a label. A description equal to the name is dropped.
//! * **`title` before `placeholder`** (HTML-AAM step order; the crate puts the placeholder first):
//!   a text field with neither label nor `aria-label` and with both attributes is named by its
//!   `title` and described by its `placeholder`. A `textarea` is named by its `placeholder` as a last
//!   resort (the crate does not).
//! * **A role `accname` does not know** (`image`, `doc-*`, `graphics-*`): the crate takes the element
//!   for what HTML says and may prohibit a name (`<div role=image aria-label=x>` is a generic `div`
//!   to it). When this module's role allows a name, a non-blank `aria-label` is used.
//! * **Limits.** A name or description is cut at [`MAX_TEXT_BYTES`] (at a character boundary). The
//!   `accname` crate recurses once per level of nesting, so a subtree deeper than [`MAX_NAME_DEPTH`]
//!   (from the node, from a target of `aria-labelledby`/`aria-describedby`, or from a label) is not
//!   given to it (a deep tree would overflow the stack of a layout thread, which no panic hook can
//!   catch): such a node is named by `aria-label`, `alt` or `title` only.
//!
//! # Known gaps
//!
//! * Generated content (`::before`, `::after`, CSS `content`) and `text-transform` are not part of
//!   a name: the DOM has none of it. Chromium includes both.
//! * The description does not see the computed style (`accname::description` takes none): a
//!   `display:none` child of a described-by target is part of the description. A *target* that is
//!   hidden is correctly included.
//! * Names are not localised (`input type=submit` without a value is "Submit", always).
//! * `dt` and `dfn` (role `term`) have no name; Chromium names `dt` from its content.
//!
//! See `.superpowers/sdd/a11y-1-logs/notes/item2.md` for the audit of `accname` and `a11y-dom` and
//! the list of differences from Chromium's names.

use std::collections::{BTreeSet, HashMap};

use a11y_dom::{Document, Node, NodeId, NodeKind, Rendering, ancestors, closest, elements};
use accname::IdIndex;

use crate::host::HostState;
use crate::roles;
use crate::semantics::NodeSemantics;

/// A name or a description is cut at this many bytes (at a character boundary). A page can make a
/// name as long as the page (a link around a whole article); nothing reads a name that long.
pub const MAX_TEXT_BYTES: usize = 16 * 1024;

/// The deepest subtree `accname` is given. The crate recurses once per level of nesting (two stack
/// frames each); browsers stop parsing at a depth of 512 and real names never nest that far.
pub const MAX_NAME_DEPTH: usize = 256;

/// Set the name (`label`) and the description of `semantics.node`.
pub fn apply<'n, D>(
    doc: &'n D,
    node: D::N<'n>,
    ids: &IdIndex<'n, D::N<'n>>,
    semantics: &mut NodeSemantics,
) where
    D: Rendering + HostState,
{
    if node.kind() != NodeKind::Element {
        return;
    }
    let prohibited = roles::name_prohibited(semantics.node.role());
    let deep = too_deep(node, ids);

    let name = if prohibited {
        None
    } else if deep {
        shallow_name(node)
    } else {
        let computed = accname::name_rendered(doc, node, ids).or_else(|| other_role_label(node));
        prefer_title_over_placeholder(node, computed)
    };
    let description = description(node, ids, name.as_deref(), deep);

    if let Some(name) = name.map(cap).filter(|n| !n.is_empty()) {
        semantics.node.set_label(name);
    }
    if let Some(description) = description.map(cap).filter(|d| !d.is_empty()) {
        semantics.node.set_description(description);
    }
}

/// The description: `aria-describedby`, else the `title` when it is not the name.
fn description<'n, N: Node<'n>>(
    node: N,
    ids: &IdIndex<'n, N>,
    name: Option<&str>,
    deep: bool,
) -> Option<String> {
    let title = node
        .attr("title")
        .map(flatten)
        .filter(|title| !title.is_empty());
    let from_attributes = if deep {
        None
    } else {
        accname::description(node, ids).map(|d| flatten(&d))
    };
    let described = match from_attributes {
        // `accname` answers the title when the name came from a label; that case is decided here.
        Some(d) if title.as_deref() != Some(d.as_str()) => Some(d).filter(|d| !d.is_empty()),
        _ => title.filter(|t| name != Some(t.as_str())),
    };
    described.or_else(|| placeholder_as_description(node, name))
}

/// A text field named by its `title` is described by its `placeholder`.
fn placeholder_as_description<'a, N: Node<'a>>(node: N, name: Option<&str>) -> Option<String> {
    if !is_text_field(node) {
        return None;
    }
    let placeholder = flatten(node.attr("placeholder")?);
    let title = flatten(node.attr("title")?);
    (!placeholder.is_empty() && name == Some(title.as_str()) && placeholder != title)
        .then_some(placeholder)
}

fn is_text_field<'a, N: Node<'a>>(node: N) -> bool {
    match node.local_name() {
        "textarea" => true,
        "input" => !matches!(
            node.attr("type")
                .map(|t| t.trim().to_ascii_lowercase())
                .as_deref(),
            Some(
                "button"
                    | "submit"
                    | "reset"
                    | "image"
                    | "checkbox"
                    | "radio"
                    | "range"
                    | "color"
                    | "file"
                    | "hidden"
            )
        ),
        _ => false,
    }
}

/// HTML-AAM puts the `title` before the `placeholder`; `accname` answers the placeholder first.
/// Also names a `textarea` by its placeholder, which the crate does not.
fn prefer_title_over_placeholder<'a, N: Node<'a>>(node: N, name: Option<String>) -> Option<String> {
    if !is_text_field(node) {
        return name;
    }
    let placeholder = node
        .attr("placeholder")
        .map(flatten)
        .filter(|p| !p.is_empty());
    let title = node.attr("title").map(flatten).filter(|t| !t.is_empty());
    match (name, placeholder, title) {
        // The name is the placeholder and nothing better exists: the title is better.
        (Some(name), Some(placeholder), Some(title)) if name == placeholder => Some(title),
        // A textarea with no other name takes its placeholder.
        (None, Some(placeholder), _) if node.local_name() == "textarea" => Some(placeholder),
        (name, _, _) => name,
    }
}

/// A role the `accname` crate does not know can make it prohibit a name that this module's role
/// allows: the only author name that can be used then is a non-blank `aria-label`.
fn other_role_label<'a, N: Node<'a>>(node: N) -> Option<String> {
    if !accname::role(node).is_some_and(accname::name_is_prohibited) {
        return None;
    }
    let label = flatten(node.attr("aria-label")?);
    (!label.is_empty()).then_some(label)
}

/// The name of a node whose subtree is too deep for `accname`: `aria-label`, then `alt`, then `title`.
fn shallow_name<'a, N: Node<'a>>(node: N) -> Option<String> {
    ["aria-label", "alt", "title"]
        .iter()
        .filter_map(|attribute| node.attr(attribute))
        .map(flatten)
        .find(|value| !value.is_empty())
}

/// Collapse whitespace the way the accname specification flattens a string.
fn flatten(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Cut a string at [`MAX_TEXT_BYTES`] bytes, at a character boundary.
fn cap(mut text: String) -> String {
    if text.len() > MAX_TEXT_BYTES {
        let mut end = MAX_TEXT_BYTES;
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        text.truncate(end);
    }
    text
}

/// Whether a subtree is deeper than `limit` levels (counting the root as level 0). Iterative.
fn deeper_than<'a, N: Node<'a>>(root: N, limit: usize) -> bool {
    let mut stack = vec![(root, 0usize)];
    while let Some((node, depth)) = stack.pop() {
        if depth > limit {
            return true;
        }
        stack.extend(node.children().map(|child| (child, depth + 1)));
    }
    false
}

/// Whether any subtree `accname` would walk for this node is deeper than [`MAX_NAME_DEPTH`].
fn too_deep<'n, N: Node<'n>>(node: N, ids: &IdIndex<'n, N>) -> bool {
    let tag = node.local_name();
    let walks_own_content = matches!(tag, "fieldset" | "figure" | "table" | "svg")
        || accname::role(node).is_some_and(accname::allows_name_from_content);
    if walks_own_content && deeper_than(node, MAX_NAME_DEPTH) {
        return true;
    }
    for attribute in ["aria-labelledby", "aria-describedby"] {
        let Some(value) = node.attr(attribute) else {
            continue;
        };
        if value
            .split_whitespace()
            .filter_map(|id| ids.get(node, id))
            .any(|target| deeper_than(target, MAX_NAME_DEPTH))
        {
            return true;
        }
    }
    let labelable = matches!(
        tag,
        "input" | "select" | "textarea" | "meter" | "progress" | "output" | "button"
    );
    labelable
        && (ids
            .labels_for(node)
            .iter()
            .any(|label| deeper_than(*label, MAX_NAME_DEPTH))
            || closest(node, "label").is_some_and(|label| deeper_than(label, MAX_NAME_DEPTH)))
}

// ---------------------------------------------------------------------------------------------
// The reverse map: whose name depends on this node.
// ---------------------------------------------------------------------------------------------

/// The reverse of `aria-labelledby` and `aria-describedby`, built once per document: for each node,
/// the elements that name or describe themselves by it. Build it when many nodes need their
/// [`dependants`]; [`dependants`] itself builds one per call.
#[derive(Debug, Default)]
pub struct DependantsIndex {
    referrers: HashMap<NodeId, Vec<NodeId>>,
}

impl DependantsIndex {
    /// One pass over the document: every element with `aria-labelledby` or `aria-describedby`.
    pub fn build<'n, D>(doc: &'n D, ids: &IdIndex<'n, D::N<'n>>) -> Self
    where
        D: Document,
    {
        let mut referrers: HashMap<NodeId, Vec<NodeId>> = HashMap::new();
        for element in elements(doc) {
            for attribute in ["aria-labelledby", "aria-describedby"] {
                let Some(value) = element.attr(attribute) else {
                    continue;
                };
                for target in value
                    .split_whitespace()
                    .filter_map(|id| ids.get(element, id))
                {
                    let list = referrers.entry(target.id()).or_default();
                    if list.last() != Some(&element.id()) {
                        list.push(element.id());
                    }
                }
            }
        }
        DependantsIndex { referrers }
    }

    /// The nodes whose name or description depends on `node`; see [`dependants`].
    pub fn dependants<'n, N: Node<'n>>(&self, node: N, ids: &IdIndex<'n, N>) -> Vec<NodeId> {
        let mut out: BTreeSet<NodeId> = BTreeSet::new();
        let chain: Vec<N> = std::iter::once(node).chain(ancestors(node)).collect();
        for member in &chain {
            // `aria-labelledby` / `aria-describedby` pointing at the node or at an ancestor of it:
            // the target's text includes the node's.
            if let Some(list) = self.referrers.get(&member.id()) {
                out.extend(list.iter().copied());
            }
            match member.local_name() {
                // `label[for]` or a wrapping label names its control.
                "label" => out.extend(labeled_control(*member, ids).map(|c| c.id())),
                // The native text alternatives that are a child of the element they name.
                "legend" => out.extend(
                    member
                        .parent()
                        .filter(|p| p.is_element("fieldset"))
                        .map(|p| p.id()),
                ),
                "figcaption" => out.extend(closest(*member, "figure").map(|f| f.id())),
                "caption" => out.extend(
                    member
                        .parent()
                        .filter(|p| p.is_element("table"))
                        .map(|p| p.id()),
                ),
                "title" => out.extend(
                    member
                        .parent()
                        .filter(|p| p.is_element("svg"))
                        .map(|p| p.id()),
                ),
                _ => {}
            }
        }
        // A role that names itself from its content (link, heading, button, ...) depends on every
        // descendant: an ancestor of the node is one.
        out.extend(
            ancestors(node)
                .filter(|a| accname::role(*a).is_some_and(accname::allows_name_from_content))
                .map(|a| a.id()),
        );
        out.remove(&node.id());
        out.into_iter().collect()
    }
}

/// The nodes whose accessible name or description depends on `node`, in id order, without `node`
/// itself: the elements whose `aria-labelledby` or `aria-describedby` names `node` or one of its
/// ancestors (a target's text includes its descendants'); the control of a `label` that is `node` or
/// contains it (`for`, or the first labelable descendant of a wrapping label); the `fieldset`,
/// `figure`, `table` or `svg` that a `legend`, `figcaption`, `caption` or `title` names; and the
/// ancestors that take their name from their content. It over-approximates (an ancestor with its own
/// `aria-label` is listed too): the caller damages these nodes, and an unneeded damage is cheap.
///
/// Builds the reverse index of the whole document on each call; use [`DependantsIndex`] to ask many
/// times.
pub fn dependants<'n, D>(doc: &'n D, node: D::N<'n>, ids: &IdIndex<'n, D::N<'n>>) -> Vec<NodeId>
where
    D: Document,
{
    DependantsIndex::build(doc, ids).dependants(node, ids)
}

/// The control a `label` names: the element its `for` attribute points at, else the first
/// labelable element inside it.
fn labeled_control<'n, N: Node<'n>>(label: N, ids: &IdIndex<'n, N>) -> Option<N> {
    let labelable = |candidate: &N| {
        candidate.kind() == NodeKind::Element
            && matches!(
                candidate.local_name(),
                "button" | "input" | "meter" | "output" | "progress" | "select" | "textarea"
            )
            && !(candidate.local_name() == "input"
                && candidate
                    .attr("type")
                    .is_some_and(|t| t.trim().eq_ignore_ascii_case("hidden")))
    };
    match label.attr("for") {
        Some(target) => ids.get(label, target).filter(labelable),
        None => a11y_dom::descendants(label).find(labelable),
    }
}
