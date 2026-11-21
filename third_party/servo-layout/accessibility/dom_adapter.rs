/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

//! Mercury: a11y-1 item 4 (`layout-glue`, EE2-EE4). New file: the adapter that lets the engine-free
//! crate `mercury-a11y` read Servo's layout DOM.
//!
//! [`AccDocument`] and [`AccNode`] implement `a11y_dom::{Document, Node, Rendering}` and
//! `mercury_a11y::HostState` over [`ServoLayoutNode`] and [`ServoLayoutElement`]. Nothing here
//! decides anything about roles, names or states: it only answers the questions `mercury-a11y` asks
//! of a DOM (tag, attributes, text, flat-tree relatives, computed `display` and `visibility`, native
//! state flags, the value of a form control).
//!
//! # What an [`AccNode`] is
//!
//! `AccNode<'dom>` is a copyable handle: the layout node plus a small [`NodeId`] that is unique per
//! node within one accessibility update. The ids are handed out by a thread-local table that
//! [`begin_update`] clears at the start of each update (the id is *not* stable across updates, and
//! nothing keeps one across updates). The table is thread-local rather than a field of a document
//! object so that the handle does not borrow it: an `accname::IdIndex<'dom, AccNode<'dom>>` can then
//! live in the update next to the layout nodes without a self-referential struct.
//!
//! # Borrowed strings
//!
//! `a11y_dom::Node::attr` and `::text` must return `&'a str` that outlives the call. The layout
//! interface (`LayoutElement::attribute_as_str`, `LayoutNode::text_content`) can only lend for the
//! duration of a `&self` borrow, although the data lives in the DOM, which is not mutated while an
//! accessibility update runs (it runs inside layout, on the script thread, and no script runs
//! meanwhile): the underlying accessors return `&'dom str`. [`lend`] widens the borrow back, in one
//! place. Nothing is copied and nothing is leaked.
//!
//! # Known gaps
//!
//! * `a11y_dom::Node::attributes()` cannot enumerate: the layout interface looks attributes up by
//!   name only. It answers from [`KNOWN_ATTRIBUTES`] (every `aria-*` attribute and the HTML ones the
//!   accessibility rules read); `attr()` itself answers any name. Nothing in `accname` or
//!   `mercury-a11y` calls `attributes()`.
//! * Every node is in tree scope 0 (the document): the layout interface does not say which shadow
//!   root contains a node, so an IDREF resolves across shadow roots, and two shadow roots that repeat
//!   an id resolve to the first.
//! * The current value of an `<input type=range>` is read back from the style attribute of the slider
//!   thumb in its user-agent shadow tree (`inset-inline-start: <percent>%`), because the layout
//!   interface exposes no accessor for it; `servo/servo` would need a `LayoutNode` method to do better.
//! * `local_name()` is the element's own local name, not lowercased: HTML elements in HTML documents
//!   are lowercase already, and the camel-case names of SVG elements correctly match nothing.

use std::cell::RefCell;

use a11y_dom::{ComputedStyle, Document, Node, NodeId, NodeKind, Rect, Rendering};
use layout_api::{AxesOverflow, LayoutElement, LayoutNode};
use mercury_a11y::{HostState, NativeFlags, RangeValue};
use rustc_hash::FxHashMap;
use script::layout_dom::{ServoLayoutElement, ServoLayoutNode};
use style::computed_values::visibility::T as Visibility;
use style::dom::{NodeInfo, OpaqueNode, TElement};
use style::selector_parser::NonTSPseudoClass;
use style::values::specified::Overflow;
use web_atoms::{LocalName, local_name, ns};

/// The attributes `AccNode::attributes()` reports (see the module documentation).
const KNOWN_ATTRIBUTES: &[&str] = &[
    "role",
    "id",
    "for",
    "title",
    "alt",
    "type",
    "value",
    "placeholder",
    "href",
    "name",
    "lang",
    "tabindex",
    "hidden",
    "inert",
    "disabled",
    "checked",
    "selected",
    "open",
    "required",
    "readonly",
    "multiple",
    "min",
    "max",
    "step",
    "contenteditable",
    "aria-activedescendant",
    "aria-atomic",
    "aria-autocomplete",
    "aria-braillelabel",
    "aria-brailleroledescription",
    "aria-busy",
    "aria-checked",
    "aria-colcount",
    "aria-colindex",
    "aria-colspan",
    "aria-controls",
    "aria-current",
    "aria-describedby",
    "aria-description",
    "aria-details",
    "aria-disabled",
    "aria-errormessage",
    "aria-expanded",
    "aria-flowto",
    "aria-haspopup",
    "aria-hidden",
    "aria-invalid",
    "aria-keyshortcuts",
    "aria-label",
    "aria-labelledby",
    "aria-level",
    "aria-live",
    "aria-modal",
    "aria-multiline",
    "aria-multiselectable",
    "aria-orientation",
    "aria-owns",
    "aria-placeholder",
    "aria-posinset",
    "aria-pressed",
    "aria-readonly",
    "aria-relevant",
    "aria-required",
    "aria-roledescription",
    "aria-rowcount",
    "aria-rowindex",
    "aria-rowspan",
    "aria-selected",
    "aria-setsize",
    "aria-sort",
    "aria-valuemax",
    "aria-valuemin",
    "aria-valuenow",
    "aria-valuetext",
];

/// The attributes whose value is a list of ids of other elements (IDREF lists).
pub(super) const IDREF_ATTRIBUTES: [&str; 8] = [
    "aria-labelledby",
    "aria-describedby",
    "aria-controls",
    "aria-owns",
    "aria-flowto",
    "aria-details",
    "aria-errormessage",
    "aria-activedescendant",
];

/// How many attribute names the per-thread atom cache keeps. The names asked for come from a fixed
/// set in `accname` and `mercury-a11y`, so this is a ceiling that is never reached; past it a name is
/// looked up without being cached.
const MAX_CACHED_ATTRIBUTE_NAMES: usize = 512;

/// The most nodes of a subtree the check "does this node need the document's ids" looks at before it
/// answers yes to be safe.
const IDREF_SCAN_LIMIT: usize = 64;

/// The deepest `UA widget` the value readers climb through (text container, inner container, host).
const WIDGET_DEPTH: usize = 6;

/// The most nodes the radio-group search looks at; a group in a bigger form has no position.
const RADIO_SCAN_LIMIT: usize = 5000;

thread_local! {
    /// Opaque layout node address to the id handed out by [`AccNode::new`] in this update.
    static NODE_IDS: RefCell<FxHashMap<OpaqueNode, u32>> = RefCell::new(FxHashMap::default());
    /// Interned attribute names, so a lookup does not intern an atom (and, for an attribute the page
    /// does not use, allocate and free one) on every call.
    static ATTRIBUTE_ATOMS: RefCell<FxHashMap<Box<str>, LocalName>> =
        RefCell::new(FxHashMap::default());
}

/// Start a new accessibility update: ids handed out before this are forgotten.
pub(super) fn begin_update() {
    NODE_IDS.with_borrow_mut(|ids| ids.clear());
}

fn intern(opaque: OpaqueNode) -> NodeId {
    NODE_IDS.with_borrow_mut(|ids| {
        let next = ids.len() as u32;
        NodeId(*ids.entry(opaque).or_insert(next))
    })
}

fn with_attribute_atom<R>(name: &str, f: impl FnOnce(&LocalName) -> R) -> R {
    ATTRIBUTE_ATOMS.with_borrow_mut(|cache| {
        if let Some(atom) = cache.get(name) {
            return f(atom);
        }
        let atom = LocalName::from(name);
        if cache.len() < MAX_CACHED_ATTRIBUTE_NAMES {
            f(cache.entry(name.into()).or_insert(atom))
        } else {
            f(&atom)
        }
    })
}

/// Widen a borrow of DOM data back to the lifetime of the DOM itself.
///
/// The strings this is used on (attribute values, text node data, local names) are stored in the DOM
/// nodes that the `'dom` in [`AccNode<'dom>`] already keeps alive; the layout interface only narrows
/// their lifetime to a `&self` borrow. They are not modified while the accessibility update runs
/// (see the module documentation), so a reference to one stays valid for the whole update, which is
/// the longest any [`AccNode`] or anything derived from one lives.
#[expect(unsafe_code)]
fn lend<'a>(string: &str) -> &'a str {
    // SAFETY: see above; the pointer comes from a live `&str`, only the lifetime changes.
    unsafe { &*(string as *const str) }
}

/// The flat-tree parent of a node.
#[expect(unsafe_code)]
fn flat_tree_parent(node: ServoLayoutNode<'_>) -> Option<ServoLayoutNode<'_>> {
    // SAFETY: `dangerous_flat_tree_parent` must only be called from the script thread while no
    // layout worker threads run. The accessibility update runs on the script thread after layout has
    // finished (`AccessibilityTree::update_tree`), and already walks parents the same way
    // (`collect_dom_node_ancestors`).
    unsafe { node.dangerous_flat_tree_parent() }
}

/// Whether this element is an `<iframe>` or a `<frame>`: its DOM children are fallback content that
/// is never rendered, and its accessibility node is a graft (a node with a `tree_id` cannot have
/// children), so it has none in the accessibility tree either.
pub(super) fn is_frame_element(node: &ServoLayoutNode<'_>) -> bool {
    node.as_html_element().is_some_and(|element| {
        *element.local_name() == local_name!("iframe") ||
            *element.local_name() == local_name!("frame")
    })
}

/// The flat-tree children of a node as the accessibility tree sees them: none for a frame element.
pub(super) fn flat_children<'a, 'dom>(
    node: &'a ServoLayoutNode<'dom>,
) -> impl Iterator<Item = ServoLayoutNode<'dom>> + 'a {
    let is_frame = is_frame_element(node);
    node.flat_tree_children().filter(move |_| !is_frame)
}

/// A node of the layout DOM as `mercury-a11y` sees it: an element or a text node.
#[derive(Clone, Copy)]
pub(super) struct AccNode<'dom> {
    node: ServoLayoutNode<'dom>,
    id: NodeId,
}

impl PartialEq for AccNode<'_> {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
    }
}

impl Eq for AccNode<'_> {}

impl<'dom> AccNode<'dom> {
    /// The handle of a layout node. The node must be an element or a text node.
    pub(super) fn new(node: ServoLayoutNode<'dom>) -> Self {
        Self {
            node,
            id: intern(node.opaque()),
        }
    }

    /// The layout node behind the handle.
    pub(super) fn dom_node(self) -> ServoLayoutNode<'dom> {
        self.node
    }
}

impl<'dom> Node<'dom> for AccNode<'dom> {
    fn id(self) -> NodeId {
        self.id
    }

    fn kind(self) -> NodeKind {
        if self.node.is_text_node() {
            NodeKind::Text
        } else {
            NodeKind::Element
        }
    }

    fn parent(self) -> Option<Self> {
        let parent = flat_tree_parent(self.node)?;
        parent.is_element().then(|| AccNode::new(parent))
    }

    fn children(self) -> impl Iterator<Item = Self> + 'dom {
        // The accessibility tree has a node for every flat-tree child, comments included; the rules
        // only know elements and text.
        flat_children(&self.node)
            .filter(|child| child.is_element() || child.is_text_node())
            .map(AccNode::new)
            .collect::<Vec<_>>()
            .into_iter()
    }

    fn local_name(self) -> &'dom str {
        match self.node.as_element() {
            Some(element) => lend(element.local_name()),
            None => "",
        }
    }

    fn attributes(self) -> impl Iterator<Item = (&'dom str, &'dom str)> + 'dom {
        KNOWN_ATTRIBUTES
            .iter()
            .filter_map(move |name| self.attr(name).map(|value| (*name, value)))
    }

    fn text(self) -> &'dom str {
        if !self.node.is_text_node() {
            return "";
        }
        let content = self.node.text_content();
        lend(&content)
    }

    fn attr(self, name: &str) -> Option<&'dom str> {
        let element = self.node.as_element()?;
        with_attribute_atom(name, |atom| {
            element.attribute_as_str(&ns!(), atom).map(lend)
        })
    }
}

/// Whether the element has any IDREF attribute, or is a `<label for>`: the document then uses ids.
pub(super) fn has_idref_attribute(node: AccNode<'_>) -> bool {
    IDREF_ATTRIBUTES
        .iter()
        .any(|name| node.attr(name).is_some()) ||
        (node.is_element("label") && node.has_attr("for"))
}

/// Whether computing this element's semantics may look other elements up by id (`aria-labelledby`,
/// `<label for>`, ...), so that the document's id index must be built. It is a bounded check: a link
/// or a heading names itself from its content, which may contain a `aria-labelledby` of its own, so
/// its small subtree is looked at; a subtree that is not small is assumed to need the index.
pub(super) fn needs_document_index(node: AccNode<'_>) -> bool {
    if has_idref_attribute(node) ||
        matches!(
            node.local_name(),
            "input" | "button" | "select" | "textarea" | "meter" | "output" | "progress"
        )
    {
        return true;
    }
    if !may_name_from_content(node) {
        return false;
    }
    let mut seen = 0usize;
    for descendant in a11y_dom::descendants(node) {
        seen += 1;
        if seen > IDREF_SCAN_LIMIT {
            return true;
        }
        if descendant.attr("aria-labelledby").is_some() ||
            descendant.attr("aria-describedby").is_some()
        {
            return true;
        }
    }
    false
}

/// A cheap, over-approximating answer to "may this element take its accessible name from its
/// content, or from a child such as a `legend`, `caption` or `figcaption`": the tags HTML-AAM gives
/// such a name to, or any element with an explicit `role`.
pub(super) fn may_name_from_content(node: AccNode<'_>) -> bool {
    node.kind() == NodeKind::Element &&
        (matches!(
            node.local_name(),
            "a" | "button" |
                "h1" |
                "h2" |
                "h3" |
                "h4" |
                "h5" |
                "h6" |
                "summary" |
                "li" |
                "td" |
                "th" |
                "tr" |
                "option" |
                "label" |
                "legend" |
                "figcaption" |
                "figure" |
                "fieldset" |
                "caption" |
                "table" |
                "svg" |
                "title" |
                "dt" |
                "dfn" |
                "output"
        ) || node.has_attr("role"))
}

/// The deepest first descendant of a node: a node without children, whose id index is empty (it
/// holds at most that one node).
pub(super) fn first_leaf(node: AccNode<'_>) -> AccNode<'_> {
    let mut leaf = node;
    while let Some(child) = leaf.children().next() {
        leaf = child;
    }
    leaf
}

/// The document root above a node: the topmost element of the flat tree.
pub(super) fn document_root(node: AccNode<'_>) -> AccNode<'_> {
    let mut top = node;
    while let Some(parent) = top.parent() {
        top = parent;
    }
    top
}

/// The elements a native control's value can be hosted by: a node inside a user-agent widget (the
/// text of a text field, the thumb of a slider) belongs to the element that owns the widget. Returns
/// that element, if the node is inside one.
pub(super) fn widget_host(node: ServoLayoutNode<'_>) -> Option<ServoLayoutNode<'_>> {
    let mut current = node;
    for _ in 0..WIDGET_DEPTH {
        current = flat_tree_parent(current)?;
        if current.is_root_of_user_agent_widget() {
            return Some(current);
        }
    }
    None
}

/// Node-by-node state of an element, as the selectors read it.
#[derive(Clone, Copy, Default)]
struct StateBits {
    checked: bool,
    indeterminate: bool,
    disabled: bool,
    required: bool,
    invalid: bool,
    open: bool,
}

/// The native state flags of an element (what `:checked`, `:disabled`, ... match).
#[expect(unsafe_code)]
fn read_state(element: ServoLayoutElement<'_>) -> StateBits {
    // SAFETY: `dangerous_style_element` must only be called from the script thread while no layout
    // worker threads run; see `flat_tree_parent`. Only the state bits are read.
    let element = unsafe { element.dangerous_style_element() };
    let state = element.state();
    StateBits {
        checked: state.contains(NonTSPseudoClass::Checked.state_flag()),
        indeterminate: state.contains(NonTSPseudoClass::Indeterminate.state_flag()),
        disabled: state.contains(NonTSPseudoClass::Disabled.state_flag()),
        required: state.contains(NonTSPseudoClass::Required.state_flag()),
        invalid: state.contains(NonTSPseudoClass::Invalid.state_flag()),
        open: state.contains(NonTSPseudoClass::Open.state_flag()),
    }
}

/// The value of an attribute that is not looked up through [`Node::attr`] (static names).
fn attribute<'e>(element: &'e ServoLayoutElement<'_>, name: &LocalName) -> Option<&'e str> {
    element.attribute_as_str(&ns!(), name)
}

/// The `type` of an `<input>`, lowercased, `text` when absent. (An invalid value is not mapped to
/// `text`: no code here treats it as something else.)
fn input_type_of(element: &ServoLayoutElement<'_>) -> String {
    attribute(element, &local_name!("type"))
        .map(|value| value.to_ascii_lowercase())
        .unwrap_or_else(|| "text".to_owned())
}

/// Whether the element is a text-entry control: `<textarea>`, or an `<input>` whose type is text-like.
fn is_text_entry(name: &str, type_: &str) -> bool {
    name == "textarea" ||
        (name == "input" &&
            matches!(
                type_,
                "text" | "search" | "url" | "tel" | "email" | "password" | "number"
            ))
}

/// The number in an attribute value, if it is a finite one.
fn parse_number(value: &str) -> Option<f64> {
    value.trim().parse::<f64>().ok().filter(|v| v.is_finite())
}

/// `step` of a range-like input: `None` for `any`, the value when it is a positive number, else 1.
fn step_of(element: &ServoLayoutElement<'_>) -> Option<f64> {
    match attribute(element, &local_name!("step")) {
        Some(step) if step.trim().eq_ignore_ascii_case("any") => None,
        Some(step) => parse_number(step).filter(|v| *v > 0.0).or(Some(1.0)),
        None => Some(1.0),
    }
}

/// The text of a node's direct text-node children, concatenated.
fn own_text(node: &ServoLayoutNode<'_>) -> String {
    let mut text = String::new();
    for child in node.flat_tree_children() {
        if child.is_text_node() {
            text.push_str(&child.text_content());
        }
    }
    text
}

/// The text a user-agent widget shows for a text control, **never for a password**.
///
/// `<input>` and `<textarea>`: the user-agent shadow tree is `inner container > editor > text` (the
/// placeholder is a later sibling of the editor); number inputs and the like show their value as a
/// lone text node in the shadow root. Anything else yields `None`.
fn widget_text(host: ServoLayoutNode<'_>) -> Option<String> {
    let element = host.as_element()?;
    let name: &str = element.local_name();
    if name != "input" && name != "textarea" {
        return None;
    }
    let type_ = if name == "input" {
        input_type_of(&element)
    } else {
        String::new()
    };
    // A password field has no value to give (D7), whatever its shadow text is.
    if type_ == "password" || (name == "input" && !is_text_entry(name, &type_)) {
        return None;
    }
    let first = host.flat_tree_children().next()?;
    if first.is_text_node() {
        return Some(first.text_content().to_string());
    }
    let editor = first.flat_tree_children().next()?;
    editor.is_element().then(|| own_text(&editor))
}

/// The label of an `<option>`: its `label` attribute, else its text with whitespace collapsed.
fn option_label(option: &ServoLayoutNode<'_>) -> String {
    if let Some(element) = option.as_element() &&
        let Some(label) = attribute(&element, &local_name!("label")) &&
        !label.is_empty()
    {
        return label.to_owned();
    }
    let mut text = String::new();
    for child in option.dom_children() {
        if child.is_text_node() {
            text.push_str(&child.text_content());
        }
    }
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The `<option>` elements of a `<select>`, in order, including those in `<optgroup>`s.
fn select_options<'dom>(select: &ServoLayoutNode<'dom>) -> Vec<ServoLayoutNode<'dom>> {
    let mut options = Vec::new();
    for child in select.dom_children() {
        let Some(element) = child.as_element() else {
            continue;
        };
        let name: &str = element.local_name();
        if name == "option" {
            options.push(child);
        } else if name == "optgroup" {
            options.extend(child.dom_children().filter(|grandchild| {
                grandchild
                    .as_element()
                    .is_some_and(|e| *e.local_name() == local_name!("option"))
            }));
        }
    }
    options
}

/// The current percentage along the track of an `<input type=range>`, read back from the slider
/// thumb's style attribute in its user-agent shadow tree (see the module documentation).
fn slider_percent(input: ServoLayoutNode<'_>) -> Option<f64> {
    fn visit(node: ServoLayoutNode<'_>, depth: usize) -> Option<f64> {
        for child in node.flat_tree_children() {
            if let Some(element) = child.as_element() &&
                let Some(style) = attribute(&element, &local_name!("style")) &&
                let Some(rest) = style.trim().strip_prefix("inset-inline-start:") &&
                let Some(number) = rest.split('%').next() &&
                let Some(percent) = parse_number(number)
            {
                return Some(percent);
            }
            if depth < 4 &&
                let Some(percent) = visit(child, depth + 1)
            {
                return Some(percent);
            }
        }
        None
    }
    visit(input, 0)
}

/// The accessibility document: a thin handle on the root of the flat tree.
pub(super) struct AccDocument<'dom> {
    root: AccNode<'dom>,
}

impl<'dom> AccDocument<'dom> {
    pub(super) fn new(root: AccNode<'dom>) -> Self {
        Self { root }
    }
}

impl<'dom> Document for AccDocument<'dom> {
    type N<'a>
        = AccNode<'a>
    where
        Self: 'a;

    fn root(&self) -> AccNode<'_> {
        self.root
    }
}

impl Rendering for AccDocument<'_> {
    /// `display` and `visibility` from the element's primary style, exactly as
    /// `AccessibilityNode::update_node_from_layout` reads `display`. An element that has no style
    /// (it is inside a `display: none` subtree, or not styled yet) answers `None`.
    fn computed_style<'n>(&'n self, node: AccNode<'n>) -> Option<ComputedStyle> {
        let element = node.node.as_element()?;
        element.style_data()?;
        let data = element.element_data();
        let style = data.styles.get_primary()?;
        let display = style.get_box().display;
        let display = if display.is_none() {
            "none"
        } else if display.is_contents() {
            "contents"
        } else if display.is_inline_flow() {
            "inline"
        } else {
            "block"
        };
        let visibility = match style.get_inherited_box().visibility {
            Visibility::Visible => "visible",
            Visibility::Hidden => "hidden",
            Visibility::Collapse => "collapse",
        };
        Some(ComputedStyle {
            display: Some(display.to_owned()),
            visibility: Some(visibility.to_owned()),
            ..ComputedStyle::default()
        })
    }

    /// Bounds are the accessibility node's own business (`update_node_from_layout`); the rules do not
    /// read them.
    fn bounds<'n>(&'n self, _node: AccNode<'n>) -> Option<Rect> {
        None
    }

    fn is_rendered<'n>(&'n self, node: AccNode<'n>) -> bool {
        self.computed_style(node)
            .is_some_and(|style| style.display.as_deref() != Some("none"))
    }
}

impl HostState for AccDocument<'_> {
    fn native_flags<'n>(&'n self, node: AccNode<'n>) -> NativeFlags {
        let Some(element) = node.node.as_element() else {
            return NativeFlags::default();
        };
        let state = read_state(element);
        let name: &str = element.local_name();
        let input_type = if name == "input" {
            input_type_of(&element)
        } else {
            String::new()
        };
        let is_input = name == "input";
        let text_entry = is_text_entry(name, &input_type);
        let has = |attribute_name: LocalName| attribute(&element, &attribute_name).is_some();

        let contenteditable =
            attribute(&element, &local_name!("contenteditable")).is_some_and(|value| {
                value.is_empty() ||
                    value.eq_ignore_ascii_case("true") ||
                    value.eq_ignore_ascii_case("plaintext-only")
            });
        let has_tabindex = attribute(&element, &local_name!("tabindex"))
            .is_some_and(|value| value.trim().parse::<i32>().is_ok());
        let native_focusable = match name {
            "a" | "area" => has(local_name!("href")),
            "button" | "select" | "textarea" => !state.disabled,
            "input" => input_type != "hidden" && !state.disabled,
            "summary" => true,
            "audio" | "video" => has(local_name!("controls")),
            _ => false,
        };

        let scroll_container = element.is_root() ||
            element
                .style_data()
                .and_then(|_| {
                    let data = element.element_data();
                    let style = data.styles.get_primary()?;
                    let overflow = AxesOverflow::from(&**style);
                    Some(
                        matches!(overflow.x, Overflow::Auto | Overflow::Scroll) ||
                            matches!(overflow.y, Overflow::Auto | Overflow::Scroll),
                    )
                })
                .unwrap_or(false);

        NativeFlags {
            checked: state.checked && name != "option",
            indeterminate: state.indeterminate &&
                (name == "progress" || (is_input && input_type == "checkbox")),
            disabled: state.disabled,
            required: state.required,
            read_only: text_entry && has(local_name!("readonly")),
            invalid: state.invalid,
            open: state.open,
            selected: state.checked && name == "option",
            focusable: native_focusable || has_tabindex || contenteditable,
            editable: contenteditable ||
                (text_entry && !state.disabled && !has(local_name!("readonly"))),
            multiline: name == "textarea" || contenteditable,
            scroll_container,
            ..NativeFlags::default()
        }
    }

    fn control_value<'n>(&'n self, node: AccNode<'n>) -> Option<String> {
        let element = node.node.as_element()?;
        let name: &str = element.local_name();
        match name {
            "input" | "textarea" => widget_text(node.node),
            "select" => {
                if attribute(&element, &local_name!("multiple")).is_some() {
                    return None;
                }
                let options = select_options(&node.node);
                let selected = options
                    .iter()
                    .find(|option| option.as_element().is_some_and(|e| read_state(e).checked))
                    .or_else(|| options.first())?;
                Some(option_label(selected))
            },
            "output" => {
                let mut text = String::new();
                for descendant in a11y_dom::descendants(node).take(IDREF_SCAN_LIMIT * 16) {
                    if descendant.kind() == NodeKind::Text {
                        text.push_str(descendant.text());
                    }
                }
                Some(text)
            },
            _ => None,
        }
    }

    fn range<'n>(&'n self, node: AccNode<'n>) -> Option<RangeValue> {
        let element = node.node.as_element()?;
        let name: &str = element.local_name();
        let number =
            |attribute_name: LocalName| attribute(&element, &attribute_name).and_then(parse_number);
        match name {
            "input" => match input_type_of(&element).as_str() {
                "range" => {
                    let min = number(local_name!("min")).unwrap_or(0.0);
                    let max = number(local_name!("max")).unwrap_or(100.0).max(min);
                    let value = slider_percent(node.node)
                        .map(|percent| min + (max - min) * (percent / 100.0).clamp(0.0, 1.0))
                        .or_else(|| number(local_name!("value")))
                        .unwrap_or(min + (max - min) / 2.0)
                        .clamp(min, max);
                    Some(RangeValue {
                        value,
                        min,
                        max,
                        step: step_of(&element),
                    })
                },
                "number" => {
                    let min = number(local_name!("min"))?;
                    let max = number(local_name!("max"))?;
                    if max < min {
                        return None;
                    }
                    let value = widget_text(node.node)
                        .and_then(|text| parse_number(&text))
                        .or_else(|| number(local_name!("value")))?;
                    Some(RangeValue {
                        value: value.clamp(min, max),
                        min,
                        max,
                        step: step_of(&element),
                    })
                },
                _ => None,
            },
            "progress" => {
                let value = number(local_name!("value"))?;
                let max = number(local_name!("max"))
                    .filter(|max| *max > 0.0)
                    .unwrap_or(1.0);
                Some(RangeValue {
                    value: value.clamp(0.0, max),
                    min: 0.0,
                    max,
                    step: None,
                })
            },
            "meter" => {
                let min = number(local_name!("min")).unwrap_or(0.0);
                let max = number(local_name!("max")).unwrap_or(1.0).max(min);
                let value = number(local_name!("value")).unwrap_or(0.0).clamp(min, max);
                Some(RangeValue {
                    value,
                    min,
                    max,
                    step: None,
                })
            },
            _ => None,
        }
    }

    fn position_in_set<'n>(&'n self, node: AccNode<'n>) -> Option<(usize, usize)> {
        let element = node.node.as_element()?;
        let name: &str = element.local_name();
        match name {
            "li" => {
                let parent = node.parent()?;
                if !matches!(parent.local_name(), "ul" | "ol" | "menu") {
                    return None;
                }
                let items: Vec<_> = parent
                    .children()
                    .filter(|child| child.is_element("li"))
                    .collect();
                let position = items.iter().position(|item| *item == node)?;
                Some((position + 1, items.len()))
            },
            "option" => {
                let mut select = node.parent()?;
                for _ in 0..WIDGET_DEPTH {
                    if select.is_element("select") {
                        break;
                    }
                    select = select.parent()?;
                }
                if !select.is_element("select") {
                    return None;
                }
                let options = select_options(&select.node);
                let position = options
                    .iter()
                    .position(|option| option.opaque() == node.node.opaque())?;
                Some((position + 1, options.len()))
            },
            "input" if input_type_of(&element) == "radio" => {
                let group =
                    attribute(&element, &local_name!("name")).filter(|group| !group.is_empty())?;
                let scope = {
                    let mut scope = node.parent()?;
                    while !scope.is_element("form") {
                        match scope.parent() {
                            Some(parent) => scope = parent,
                            None => break,
                        }
                    }
                    scope
                };
                let mut radios = Vec::new();
                for candidate in a11y_dom::descendants(scope).take(RADIO_SCAN_LIMIT) {
                    if candidate.is_element("input") &&
                        candidate
                            .attr("type")
                            .is_some_and(|t| t.eq_ignore_ascii_case("radio")) &&
                        candidate.attr("name") == Some(group)
                    {
                        radios.push(candidate);
                    }
                }
                let position = radios.iter().position(|radio| *radio == node)?;
                Some((position + 1, radios.len()))
            },
            _ => None,
        }
    }
}
