// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! Test support: a document the pure logic can run on, built from `a11y_dom::Arena`.
//!
//! Owner: item 2 (`core-names`). Item 3 builds on it. `TestDom` is a newtype because the orphan
//! rule forbids implementing `Rendering` for `Arena` itself, and it stays the one-field tuple
//! struct `TestDom(Arena)` that the skeleton test constructs.
//!
//! Because the newtype has no room for side tables, **everything the engine would answer is
//! derived from the markup**, the way a browser derives it from the same markup:
//!
//! * `Rendering::computed_style` answers `display` and `visibility`: the user-agent default of the
//!   tag (`span` inline, `div` block, `head`, `script` and `[hidden]` none, ...), overridden by an
//!   inline `style` attribute (`display:none`, `visibility:hidden`, `display:inline`). `visibility`
//!   is inherited from the nearest ancestor that sets it, `display` is not.
//! * `HostState::native_flags` derives `disabled` (including `fieldset[disabled]`), `checked`,
//!   `required`, `read_only` (the `readonly` attribute), `invalid` (a required control with no
//!   value, or the test attribute `data-test-invalid`), `open` (`details[open]`, `dialog[open]`),
//!   `selected` (`option[selected]`, else the first option of a single-choice select), `focusable`
//!   (the HTML focusable areas, `tabindex`, `contenteditable`), `editable` (`contenteditable`
//!   hosts, and text controls that are neither readonly nor disabled), `multiline` (`textarea`,
//!   `contenteditable`) and `scroll_container` (inline `overflow` auto or scroll). `indeterminate`
//!   comes from the test attribute `data-test-indeterminate`, and from `progress` without a
//!   `value`.
//! * `control_value` is the `value` attribute (or `data-test-value`, which stands for the value the
//!   user typed) of a text control, the text of a `textarea` and an `output`, the text of the selected
//!   option of a single-choice select, and **never** the value of a password field (the contract of
//!   `HostState::control_value`).
//! * `range` is the numeric value of `input[type=range]` (the HTML defaults: 0 to 100, step 1,
//!   value in the middle), of `input[type=number]` with both `min` and `max`, of `progress` with a
//!   `value`, and of `meter`.
//! * `position_in_set` is the 1-based position of an `li` among the `li` children of its parent, of
//!   an `option` among the options of its select or datalist, and of a radio button among the radio
//!   buttons of its name in its form.
//!
//! [`TestDom::from_html`] parses a *well-formed* HTML fragment (see [`parse_html`] for what it does
//! not do), so that cases read as HTML.

use a11y_dom::{
    Arena, ArenaNode, ComputedStyle, Document, Node, NodeKind, Rect, Rendering, ancestors, closest,
    descendants, elements,
};
use accname::IdIndex;

use crate::host::{HostState, NativeFlags, RangeValue};
use crate::semantics::{NodeSemantics, compute};

/// An `Arena` as a document with no layout and no engine state, but with the answers that the
/// markup implies (see the module documentation).
pub struct TestDom(pub Arena);

impl From<Arena> for TestDom {
    fn from(arena: Arena) -> Self {
        TestDom(arena)
    }
}

impl TestDom {
    /// Parse a well-formed HTML fragment (see [`parse_html`]).
    pub fn from_html(html: &str) -> TestDom {
        TestDom(parse_html(html))
    }

    /// The first element with this `id` attribute, if any.
    pub fn find_id(&self, id: &str) -> Option<ArenaNode<'_>> {
        elements(self).find(|node| node.attr("id") == Some(id))
    }

    /// The element with this `id` attribute. **Panics** with the id in the message when there is none:
    /// a test helper.
    pub fn by_id(&self, id: &str) -> ArenaNode<'_> {
        self.find_id(id)
            .unwrap_or_else(|| panic!("the test document has no element with id {id:?}"))
    }

    /// The first element with this tag name, in document order. **Panics** when there is none.
    pub fn first(&self, tag: &str) -> ArenaNode<'_> {
        elements(self)
            .find(|node| node.local_name() == tag)
            .unwrap_or_else(|| panic!("the test document has no <{tag}> element"))
    }

    /// The id index of the whole document (what `compute` and `accname` take).
    pub fn id_index(&self) -> IdIndex<'_, ArenaNode<'_>> {
        IdIndex::build(self.0.root())
    }

    /// `compute` for the element with this `id`.
    pub fn semantics_of_id(&self, id: &str) -> NodeSemantics {
        let ids = self.id_index();
        compute(self, self.by_id(id), &ids)
    }

    /// `compute` for the first element with this tag name.
    pub fn semantics_of_tag(&self, tag: &str) -> NodeSemantics {
        let ids = self.id_index();
        compute(self, self.first(tag), &ids)
    }
}

impl Document for TestDom {
    type N<'a> = ArenaNode<'a>;

    fn root(&self) -> ArenaNode<'_> {
        self.0.root()
    }

    fn node_count(&self) -> Option<usize> {
        Some(self.0.len())
    }
}

// ---------------------------------------------------------------------------------------------
// Rendering: display and visibility from the markup.
// ---------------------------------------------------------------------------------------------

impl Rendering for TestDom {
    fn computed_style<'n>(&'n self, node: ArenaNode<'n>) -> Option<ComputedStyle> {
        if node.kind() != NodeKind::Element {
            return None;
        }
        Some(ComputedStyle {
            display: Some(computed_display(node)),
            visibility: Some(computed_visibility(node)),
            ..ComputedStyle::default()
        })
    }

    fn bounds<'n>(&'n self, _node: ArenaNode<'n>) -> Option<Rect> {
        None
    }
}

/// The declarations of an inline `style` attribute: property lowercased, value lowercased and
/// stripped of `!important`; comments removed. Not a CSS parser: enough for `display:none`.
fn declarations(style: &str) -> Vec<(String, String)> {
    let mut cleaned = String::with_capacity(style.len());
    let mut rest = style;
    while let Some(start) = rest.find("/*") {
        cleaned.push_str(&rest[..start]);
        match rest[start + 2..].find("*/") {
            Some(end) => rest = &rest[start + 2 + end + 2..],
            None => rest = "",
        }
    }
    cleaned.push_str(rest);
    cleaned
        .split(';')
        .filter_map(|declaration| {
            let (property, value) = declaration.split_once(':')?;
            let value = value.trim().to_ascii_lowercase();
            let value = value.trim_end_matches("!important").trim().to_string();
            Some((property.trim().to_ascii_lowercase(), value))
        })
        .collect()
}

/// The last inline declaration of `property` on the element.
fn inline_style(node: ArenaNode<'_>, property: &str) -> Option<String> {
    let style = node.attr("style")?;
    declarations(style)
        .into_iter()
        .rev()
        .find(|(name, _)| name == property)
        .map(|(_, value)| value)
}

const DISPLAY_NONE_TAGS: &[&str] = &[
    "area", "base", "datalist", "head", "link", "meta", "noscript", "param", "script", "source",
    "style", "template", "title", "track",
];

const DISPLAY_INLINE_TAGS: &[&str] = &[
    "a", "abbr", "acronym", "b", "bdi", "bdo", "big", "br", "cite", "code", "data", "del", "dfn",
    "em", "font", "i", "ins", "kbd", "label", "mark", "q", "rp", "rt", "ruby", "s", "samp",
    "small", "span", "strike", "strong", "sub", "sup", "time", "tt", "u", "var", "wbr", "img",
    "svg", "canvas", "video", "audio", "iframe", "object", "embed", "picture",
];

const DISPLAY_INLINE_BLOCK_TAGS: &[&str] = &[
    "button", "input", "select", "textarea", "meter", "progress", "output",
];

/// The computed `display` of an element: author (inline) style over the user-agent default.
fn computed_display(node: ArenaNode<'_>) -> String {
    if let Some(display) = inline_style(node, "display") {
        return display;
    }
    let tag = node.local_name();
    if node.has_attr("hidden") || DISPLAY_NONE_TAGS.contains(&tag) {
        return "none".to_string();
    }
    if tag == "input" && input_type(node) == "hidden" {
        return "none".to_string();
    }
    if tag == "slot" {
        return "contents".to_string();
    }
    if DISPLAY_INLINE_TAGS.contains(&tag) {
        return "inline".to_string();
    }
    if DISPLAY_INLINE_BLOCK_TAGS.contains(&tag) {
        return "inline-block".to_string();
    }
    match tag {
        "li" | "summary" => "list-item",
        "table" => "table",
        "caption" => "table-caption",
        "colgroup" => "table-column-group",
        "col" => "table-column",
        "thead" => "table-header-group",
        "tbody" => "table-row-group",
        "tfoot" => "table-footer-group",
        "tr" => "table-row",
        "td" | "th" => "table-cell",
        _ => "block",
    }
    .to_string()
}

/// `visibility` is inherited: the nearest inline declaration on the element or an ancestor.
fn computed_visibility(node: ArenaNode<'_>) -> String {
    std::iter::once(node)
        .chain(ancestors(node))
        .find_map(|n| inline_style(n, "visibility"))
        .unwrap_or_else(|| "visible".to_string())
}

// ---------------------------------------------------------------------------------------------
// HostState: native flags, values, ranges and positions from the markup.
// ---------------------------------------------------------------------------------------------

fn input_type(node: ArenaNode<'_>) -> String {
    node.attr("type")
        .map(|ty| ty.trim().to_ascii_lowercase())
        .unwrap_or_else(|| "text".to_string())
}

fn is_input(node: ArenaNode<'_>, ty: &str) -> bool {
    node.is_element("input") && input_type(node) == ty
}

/// An `input` whose value is text the user edits (a password too: see `control_value`).
fn is_text_like_input(node: ArenaNode<'_>) -> bool {
    node.is_element("input")
        && matches!(
            input_type(node).as_str(),
            "text"
                | "search"
                | "tel"
                | "url"
                | "email"
                | "password"
                | "number"
                | "date"
                | "datetime-local"
                | "month"
                | "time"
                | "week"
        )
}

fn is_form_control(node: ArenaNode<'_>) -> bool {
    matches!(
        node.local_name(),
        "button" | "fieldset" | "input" | "optgroup" | "option" | "select" | "textarea"
    )
}

/// A `disabled` attribute on the control, or a disabled `fieldset` ancestor (the controls inside the
/// first `legend` of that fieldset stay enabled), or a disabled `optgroup` for an option.
fn is_disabled(node: ArenaNode<'_>) -> bool {
    if !is_form_control(node) {
        return false;
    }
    if node.has_attr("disabled") {
        return true;
    }
    if node.is_element("option")
        && node
            .parent()
            .is_some_and(|p| p.is_element("optgroup") && p.has_attr("disabled"))
    {
        return true;
    }
    ancestors(node).any(|fieldset| {
        if !(fieldset.is_element("fieldset") && fieldset.has_attr("disabled")) {
            return false;
        }
        let first_legend = fieldset.children().find(|c| c.is_element("legend"));
        let inside_legend = first_legend.is_some_and(|legend| ancestors(node).any(|a| a == legend));
        !inside_legend
    })
}

fn contenteditable(node: ArenaNode<'_>) -> bool {
    node.attr("contenteditable").is_some_and(|value| {
        let value = value.trim().to_ascii_lowercase();
        value.is_empty() || value == "true" || value == "plaintext-only"
    })
}

fn options_of(owner: ArenaNode<'_>) -> Vec<ArenaNode<'_>> {
    descendants(owner)
        .filter(|node| node.is_element("option"))
        .collect()
}

/// Whether this `option` is selected: the `selected` attribute, or the first option of a select
/// that is single-choice with no option selected.
fn option_selected(option: ArenaNode<'_>) -> bool {
    if option.has_attr("selected") {
        return true;
    }
    let Some(select) = closest(option, "select") else {
        return false;
    };
    if select.has_attr("multiple") {
        return false;
    }
    let options = options_of(select);
    !options.iter().any(|o| o.has_attr("selected")) && options.first() == Some(&option)
}

fn flag_attr(node: ArenaNode<'_>, name: &str) -> bool {
    node.attr(name)
        .is_some_and(|value| !value.trim().eq_ignore_ascii_case("false"))
}

fn is_focusable(node: ArenaNode<'_>) -> bool {
    if node.kind() != NodeKind::Element {
        return false;
    }
    if node
        .attr("tabindex")
        .is_some_and(|t| t.trim().parse::<i32>().is_ok())
        || contenteditable(node)
    {
        return true;
    }
    match node.local_name() {
        "a" | "area" => node.has_attr("href"),
        "button" | "select" | "textarea" => true,
        "input" => input_type(node) != "hidden",
        "iframe" => true,
        "audio" | "video" => node.has_attr("controls"),
        "summary" => node.parent().is_some_and(|details| {
            details.is_element("details")
                && details.children().find(|c| c.is_element("summary")) == Some(node)
        }),
        _ => false,
    }
}

fn text_of(node: ArenaNode<'_>) -> String {
    a11y_dom::subtree_text(node)
}

fn number_attr(node: ArenaNode<'_>, name: &str) -> Option<f64> {
    let value = node.attr(name)?.trim().parse::<f64>().ok()?;
    value.is_finite().then_some(value)
}

/// The value the control has for the host: `data-test-value` (the value typed), else the `value`
/// attribute.
fn raw_value(node: ArenaNode<'_>) -> Option<String> {
    node.attr("data-test-value")
        .or_else(|| node.attr("value"))
        .map(str::to_string)
}

fn is_empty_value(node: ArenaNode<'_>) -> bool {
    match node.local_name() {
        "textarea" => {
            text_of(node).is_empty() && node.attr("data-test-value").is_none_or(str::is_empty)
        }
        "select" => options_of(node)
            .into_iter()
            .find(|o| option_selected(*o))
            .is_none_or(|o| {
                o.attr("value")
                    .map_or_else(|| text_of(o), str::to_string)
                    .is_empty()
            }),
        _ => raw_value(node).is_none_or(|v| v.is_empty()),
    }
}

impl HostState for TestDom {
    fn native_flags<'n>(&'n self, node: ArenaNode<'n>) -> NativeFlags {
        if node.kind() != NodeKind::Element {
            return NativeFlags::default();
        }
        let tag = node.local_name();
        let disabled = is_disabled(node);
        let editing_host = contenteditable(node);
        let text_control = is_text_like_input(node) || tag == "textarea";
        let required = matches!(tag, "input" | "select" | "textarea") && node.has_attr("required");
        let scrolls = ["overflow", "overflow-x", "overflow-y"]
            .iter()
            .filter_map(|property| inline_style(node, property))
            .any(|value| value == "auto" || value == "scroll");
        NativeFlags {
            checked: tag == "input"
                && matches!(input_type(node).as_str(), "checkbox" | "radio")
                && node.has_attr("checked"),
            indeterminate: flag_attr(node, "data-test-indeterminate")
                || (tag == "progress" && !node.has_attr("value")),
            disabled,
            required,
            read_only: matches!(tag, "input" | "textarea") && node.has_attr("readonly"),
            invalid: flag_attr(node, "data-test-invalid")
                || (required && !disabled && is_empty_value(node)),
            open: matches!(tag, "details" | "dialog") && node.has_attr("open"),
            selected: tag == "option" && option_selected(node),
            focusable: is_focusable(node) && !disabled,
            editable: editing_host || (text_control && !disabled && !node.has_attr("readonly")),
            multiline: tag == "textarea" || editing_host,
            scroll_container: scrolls,
        }
    }

    fn control_value<'n>(&'n self, node: ArenaNode<'n>) -> Option<String> {
        match node.local_name() {
            "input" => {
                let ty = input_type(node);
                // A password field never answers (the contract of the method).
                let valued = matches!(
                    ty.as_str(),
                    "text"
                        | "search"
                        | "tel"
                        | "url"
                        | "email"
                        | "number"
                        | "range"
                        | "date"
                        | "datetime-local"
                        | "month"
                        | "time"
                        | "week"
                        | "color"
                );
                if valued { raw_value(node) } else { None }
            }
            "textarea" => Some(
                node.attr("data-test-value")
                    .map(str::to_string)
                    .unwrap_or_else(|| text_of(node)),
            ),
            "output" => Some(text_of(node)),
            "select" => {
                if node.has_attr("multiple") {
                    return None;
                }
                options_of(node)
                    .into_iter()
                    .find(|o| option_selected(*o))
                    .map(|o| text_of(o).split_whitespace().collect::<Vec<_>>().join(" "))
            }
            _ => None,
        }
    }

    fn range<'n>(&'n self, node: ArenaNode<'n>) -> Option<RangeValue> {
        let typed_value = |fallback: Option<f64>| {
            number_attr(node, "data-test-value")
                .or_else(|| number_attr(node, "value"))
                .or(fallback)
        };
        match node.local_name() {
            "input" => match input_type(node).as_str() {
                "range" => {
                    let min = number_attr(node, "min").unwrap_or(0.0);
                    let max = number_attr(node, "max").unwrap_or(100.0).max(min);
                    let value = typed_value(Some(min + (max - min) / 2.0))?.clamp(min, max);
                    let step = match node.attr("step").map(|s| s.trim().to_ascii_lowercase()) {
                        Some(step) if step == "any" => None,
                        Some(step) => step.parse::<f64>().ok().filter(|v| *v > 0.0).or(Some(1.0)),
                        None => Some(1.0),
                    };
                    Some(RangeValue {
                        value,
                        min,
                        max,
                        step,
                    })
                }
                "number" => {
                    let min = number_attr(node, "min")?;
                    let max = number_attr(node, "max")?;
                    let step = number_attr(node, "step").filter(|v| *v > 0.0).or(Some(1.0));
                    Some(RangeValue {
                        value: typed_value(None)?,
                        min,
                        max,
                        step,
                    })
                }
                _ => None,
            },
            "progress" => {
                let value = number_attr(node, "value")?;
                let max = number_attr(node, "max").filter(|v| *v > 0.0).unwrap_or(1.0);
                Some(RangeValue {
                    value: value.clamp(0.0, max),
                    min: 0.0,
                    max,
                    step: None,
                })
            }
            "meter" => {
                let min = number_attr(node, "min").unwrap_or(0.0);
                let max = number_attr(node, "max").unwrap_or(1.0).max(min);
                let value = number_attr(node, "value").unwrap_or(0.0).clamp(min, max);
                Some(RangeValue {
                    value,
                    min,
                    max,
                    step: None,
                })
            }
            _ => None,
        }
    }

    fn position_in_set<'n>(&'n self, node: ArenaNode<'n>) -> Option<(usize, usize)> {
        let position_among = |siblings: Vec<ArenaNode<'n>>| {
            let index = siblings.iter().position(|s| *s == node)?;
            Some((index + 1, siblings.len()))
        };
        match node.local_name() {
            "li" => {
                let parent = node.parent()?;
                position_among(parent.children().filter(|c| c.is_element("li")).collect())
            }
            "option" => {
                let owner = closest(node, "select").or_else(|| closest(node, "datalist"))?;
                position_among(options_of(owner))
            }
            "input" if input_type(node) == "radio" => {
                let name = node.attr("name").filter(|n| !n.is_empty())?;
                let scope = closest(node, "form").unwrap_or_else(|| self.0.root());
                position_among(
                    descendants(scope)
                        .filter(|c| is_input(*c, "radio") && c.attr("name") == Some(name))
                        .collect(),
                )
            }
            _ => None,
        }
    }
}

// ---------------------------------------------------------------------------------------------
// A small HTML fragment parser, so that cases read as HTML.
// ---------------------------------------------------------------------------------------------

const VOID_ELEMENTS: &[&str] = &[
    "area", "base", "br", "col", "embed", "hr", "img", "input", "link", "meta", "param", "source",
    "track", "wbr",
];

/// Elements whose content is raw text up to their end tag.
const RAW_TEXT_ELEMENTS: &[&str] = &["script", "style", "textarea", "title"];

enum Event {
    Open {
        name: String,
        attributes: Vec<(String, String)>,
        self_closing: bool,
    },
    Close(String),
    Text(String),
}

/// Parse a **well-formed** HTML fragment into an `Arena`.
///
/// What it does: elements; attributes (double-quoted, single-quoted, unquoted, boolean); void
/// elements (`img`, `input`, `br`, ...); `/>` on any element; comments and doctype; raw text in
/// `script`, `style`, `textarea` and `title`; the character references `&amp; &lt; &gt; &quot;
/// &apos; &nbsp;` and `&#N;` / `&#xN;`; lowercase tag and attribute names; the first of a repeated
/// attribute wins. Text nodes keep their whitespace, as in a browser's DOM.
///
/// What it does not do: the error recovery of the HTML specification (an `li` closing the previous
/// `li`, an implied `tbody`, foster parenting, a `p` closed by a block). Write the end tags. A
/// fragment is wrapped in a `body` element unless it starts with `html` or `body`, so that the arena
/// has one root. `svg` and `math` children are plain elements. It never panics, on any input.
pub fn parse_html(html: &str) -> Arena {
    let events = tokenize(html);
    let wrap = !matches!(
        events.first(),
        Some(Event::Open { name, .. }) if name == "html" || name == "body"
    );
    let mut builder = Arena::builder();
    let mut open: Vec<String> = Vec::new();
    if wrap {
        builder = builder.open("body");
        open.push("body".to_string());
    }
    for event in events {
        match event {
            Event::Open {
                name,
                attributes,
                self_closing,
            } => {
                builder = builder.open(&name);
                for (attribute, value) in &attributes {
                    builder = builder.attr(attribute, value);
                }
                if self_closing || VOID_ELEMENTS.contains(&name.as_str()) {
                    builder = builder.close();
                } else {
                    open.push(name);
                }
            }
            Event::Close(name) => {
                if let Some(position) = open.iter().rposition(|o| *o == name) {
                    while open.len() > position {
                        open.pop();
                        builder = builder.close();
                    }
                    if open.is_empty() {
                        break;
                    }
                }
            }
            Event::Text(text) => {
                if !open.is_empty() {
                    builder = builder.text(&text);
                }
            }
        }
    }
    builder.build()
}

fn tokenize(html: &str) -> Vec<Event> {
    let bytes = html.as_bytes();
    let mut events = Vec::new();
    let mut position = 0;
    let mut text_start = 0;
    let flush = |events: &mut Vec<Event>, from: usize, to: usize| {
        if to > from {
            let decoded = decode_entities(&html[from..to]);
            if !decoded.is_empty() {
                events.push(Event::Text(decoded));
            }
        }
    };
    while position < bytes.len() {
        if bytes[position] != b'<' {
            position += 1;
            continue;
        }
        let rest = &html[position..];
        if rest.starts_with("<!--") {
            flush(&mut events, text_start, position);
            position = rest
                .find("-->")
                .map_or(bytes.len(), |end| position + end + 3);
            text_start = position;
            continue;
        }
        let next = bytes.get(position + 1).copied().unwrap_or(0);
        if next == b'!' || next == b'?' {
            flush(&mut events, text_start, position);
            position = rest.find('>').map_or(bytes.len(), |end| position + end + 1);
            text_start = position;
            continue;
        }
        let closing = next == b'/';
        let name_start = position + if closing { 2 } else { 1 };
        if !bytes
            .get(name_start)
            .is_some_and(|b| b.is_ascii_alphabetic())
        {
            position += 1;
            continue;
        }
        flush(&mut events, text_start, position);
        let (tag_end, name, attributes, self_closing) = scan_tag(html, name_start);
        position = tag_end;
        text_start = position;
        if closing {
            events.push(Event::Close(name));
            continue;
        }
        let raw = RAW_TEXT_ELEMENTS.contains(&name.as_str()) && !self_closing;
        events.push(Event::Open {
            name: name.clone(),
            attributes,
            self_closing,
        });
        if raw {
            let close_tag = format!("</{name}");
            let end = html[position..]
                .to_ascii_lowercase()
                .find(&close_tag)
                .map_or(bytes.len(), |e| position + e);
            if end > position {
                let content = &html[position..end];
                // Character references are decoded in textarea and title, not in script and style.
                let text = if name == "script" || name == "style" {
                    content.to_string()
                } else {
                    decode_entities(content)
                };
                events.push(Event::Text(text));
            }
            position = end;
            text_start = position;
        }
    }
    flush(&mut events, text_start, bytes.len());
    events
}

/// Scan from the first character of a tag name to just after the closing `>`.
fn scan_tag(html: &str, name_start: usize) -> (usize, String, Vec<(String, String)>, bool) {
    let bytes = html.as_bytes();
    let is_space = |b: u8| b.is_ascii_whitespace();
    let mut position = name_start;
    while position < bytes.len()
        && !is_space(bytes[position])
        && bytes[position] != b'>'
        && bytes[position] != b'/'
    {
        position += 1;
    }
    let name = html[name_start..position].to_ascii_lowercase();
    let mut attributes: Vec<(String, String)> = Vec::new();
    let mut self_closing = false;
    loop {
        while position < bytes.len() && is_space(bytes[position]) {
            position += 1;
        }
        match bytes.get(position) {
            None => break,
            Some(b'>') => {
                position += 1;
                break;
            }
            Some(b'/') => {
                self_closing = bytes.get(position + 1) == Some(&b'>');
                position += 1;
                continue;
            }
            Some(_) => {}
        }
        let attribute_start = position;
        while position < bytes.len()
            && !is_space(bytes[position])
            && !matches!(bytes[position], b'=' | b'>' | b'/')
        {
            position += 1;
        }
        let attribute = html[attribute_start..position].to_ascii_lowercase();
        while position < bytes.len() && is_space(bytes[position]) {
            position += 1;
        }
        let mut value = String::new();
        if bytes.get(position) == Some(&b'=') {
            position += 1;
            while position < bytes.len() && is_space(bytes[position]) {
                position += 1;
            }
            match bytes.get(position) {
                Some(&quote) if quote == b'"' || quote == b'\'' => {
                    let value_start = position + 1;
                    let end = html[value_start..]
                        .find(quote as char)
                        .map_or(bytes.len(), |e| value_start + e);
                    value = decode_entities(&html[value_start..end]);
                    position = (end + 1).min(bytes.len());
                }
                _ => {
                    let value_start = position;
                    while position < bytes.len()
                        && !is_space(bytes[position])
                        && bytes[position] != b'>'
                    {
                        position += 1;
                    }
                    value = decode_entities(&html[value_start..position]);
                }
            }
        } else if attribute.is_empty() {
            // A stray character that starts no attribute name: skip it so that the loop advances.
            position += 1;
        }
        if !attribute.is_empty() && !attributes.iter().any(|(known, _)| *known == attribute) {
            attributes.push((attribute, value));
        }
    }
    (position, name, attributes, self_closing)
}

fn decode_entities(text: &str) -> String {
    if !text.contains('&') {
        return text.to_string();
    }
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(amp) = rest.find('&') {
        out.push_str(&rest[..amp]);
        let after = &rest[amp + 1..];
        let decoded = after.find(';').filter(|end| *end <= 10).and_then(|end| {
            let name = &after[..end];
            let character = match name {
                "amp" => Some('&'),
                "lt" => Some('<'),
                "gt" => Some('>'),
                "quot" => Some('"'),
                "apos" => Some('\''),
                "nbsp" => Some('\u{a0}'),
                _ => name.strip_prefix('#').and_then(|number| {
                    let code = match number.strip_prefix(['x', 'X']) {
                        Some(hex) => u32::from_str_radix(hex, 16).ok()?,
                        None => number.parse::<u32>().ok()?,
                    };
                    char::from_u32(code)
                }),
            };
            character.map(|c| (c, end))
        });
        match decoded {
            Some((character, end)) => {
                out.push(character);
                rest = &after[end + 1..];
            }
            None => {
                out.push('&');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tags(dom: &TestDom) -> Vec<String> {
        elements(dom).map(|n| n.local_name().to_string()).collect()
    }

    #[test]
    fn the_parser_builds_elements_attributes_and_text() {
        let dom = TestDom::from_html(
            r#"<div id=a class="x y" hidden data-k='v'>one <b>two</b> &amp; &#65;&#x42;</div>"#,
        );
        assert_eq!(tags(&dom), ["body", "div", "b"]);
        let div = dom.by_id("a");
        assert_eq!(div.attr("class"), Some("x y"));
        assert_eq!(div.attr("hidden"), Some(""));
        assert_eq!(div.attr("data-k"), Some("v"));
        assert_eq!(a11y_dom::subtree_text(div), "one two & AB");
    }

    #[test]
    fn void_elements_comments_and_raw_text() {
        let dom = TestDom::from_html(
            "<p>a<br>b<img alt=x><!-- <b>no</b> --><input type=text></p>\
             <textarea id=t>1 &lt; 2 <b>not a tag</b></textarea><script>if (a<b) {}</script>",
        );
        assert_eq!(
            tags(&dom),
            ["body", "p", "br", "img", "input", "textarea", "script"]
        );
        assert_eq!(
            a11y_dom::subtree_text(dom.by_id("t")),
            "1 < 2 <b>not a tag</b>"
        );
        assert_eq!(a11y_dom::subtree_text(dom.first("script")), "if (a<b) {}");
    }

    #[test]
    fn a_fragment_gets_one_root_and_an_html_document_keeps_its_own() {
        let dom = TestDom::from_html("<p>a</p><p>b</p>");
        assert_eq!(dom.0.root().local_name(), "body");
        assert_eq!(
            dom.first("p").parent().map(|p| p.local_name()),
            Some("body")
        );
        let dom = TestDom::from_html("<html><body><p>a</p></body></html>");
        assert_eq!(dom.0.root().local_name(), "html");
        assert_eq!(tags(&dom), ["html", "body", "p"]);
    }

    #[test]
    fn mismatched_and_unclosed_tags_do_not_panic() {
        let dom = TestDom::from_html("<div><span>a</div></span><b>unclosed");
        assert_eq!(tags(&dom), ["body", "div", "span", "b"]);
        let dom = TestDom::from_html("</p><<>& <a href=x");
        assert!(!dom.0.is_empty());
        let dom = TestDom::from_html("<a =x / b=\"unterminated");
        assert!(!dom.0.is_empty());
    }

    #[test]
    fn display_is_the_user_agent_default_overridden_by_inline_style() {
        let dom = TestDom::from_html(
            "<div id=a><span id=b>x</span><p id=c hidden>y</p>\
             <i id=d style='Display: block !important'>z</i><input id=e type=hidden></div>",
        );
        let display = |id: &str| {
            dom.computed_style(dom.by_id(id))
                .and_then(|style| style.display)
                .unwrap()
        };
        assert_eq!(display("a"), "block");
        assert_eq!(display("b"), "inline");
        assert_eq!(display("c"), "none");
        assert_eq!(display("d"), "block");
        assert_eq!(display("e"), "none");
    }

    #[test]
    fn visibility_is_inherited_and_can_be_overridden() {
        let dom = TestDom::from_html(
            "<div id=a style='visibility:hidden'><span id=b>x</span>\
             <span id=c style='visibility:visible'>y</span></div><p id=d>z</p>",
        );
        let visibility = |id: &str| {
            dom.computed_style(dom.by_id(id))
                .and_then(|style| style.visibility)
                .unwrap()
        };
        assert_eq!(visibility("a"), "hidden");
        assert_eq!(visibility("b"), "hidden");
        assert_eq!(visibility("c"), "visible");
        assert_eq!(visibility("d"), "visible");
    }

    #[test]
    fn flags_follow_the_markup() {
        let dom = TestDom::from_html(
            "<form><fieldset id=f disabled><legend><input id=in-legend></legend>\
             <input id=off></fieldset>\
             <input id=on type=checkbox checked required>\
             <input id=ro readonly value=x><textarea id=ta></textarea>\
             <div id=ce contenteditable>x</div><details id=d open><summary id=s>s</summary></details>\
             <div id=sc style='overflow: auto'></div></form>",
        );
        let flags = |id: &str| dom.native_flags(dom.by_id(id));
        assert!(!flags("in-legend").disabled);
        assert!(flags("off").disabled && !flags("off").focusable);
        assert!(flags("on").checked && flags("on").required && flags("on").focusable);
        assert!(flags("ro").read_only && !flags("ro").editable);
        assert!(flags("ta").editable && flags("ta").multiline);
        assert!(flags("ce").editable && flags("ce").multiline && flags("ce").focusable);
        assert!(flags("d").open);
        assert!(flags("s").focusable);
        assert!(flags("sc").scroll_container);
    }

    #[test]
    fn a_required_empty_control_is_invalid_and_a_filled_one_is_not() {
        let dom = TestDom::from_html(
            "<input id=a required><input id=b required value=x><input id=c data-test-invalid>",
        );
        let flags = |id: &str| dom.native_flags(dom.by_id(id));
        assert!(flags("a").invalid);
        assert!(!flags("b").invalid);
        assert!(flags("c").invalid);
    }

    #[test]
    fn values_never_include_a_password() {
        let dom = TestDom::from_html(
            "<input id=t value=hello><input id=p type=password value=secret data-test-value=typed>\
             <textarea id=a>area</textarea><output id=o>out</output>\
             <select id=s><option>One</option><option selected>Two</option></select>\
             <input id=cb type=checkbox value=on>",
        );
        let value = |id: &str| dom.control_value(dom.by_id(id));
        assert_eq!(value("t").as_deref(), Some("hello"));
        assert_eq!(value("p"), None);
        assert_eq!(value("a").as_deref(), Some("area"));
        assert_eq!(value("o").as_deref(), Some("out"));
        assert_eq!(value("s").as_deref(), Some("Two"));
        assert_eq!(value("cb"), None);
    }

    #[test]
    fn ranges_use_the_html_defaults() {
        let dom = TestDom::from_html(
            "<input id=r type=range><input id=r2 type=range min=10 max=20 value=99 step=any>\
             <input id=n type=number min=1 max=5 value=3><input id=n2 type=number value=3>\
             <progress id=p value=5 max=10></progress><progress id=p2></progress>\
             <meter id=m value=0.4></meter>",
        );
        let range = |id: &str| dom.range(dom.by_id(id));
        let r = range("r").unwrap();
        assert_eq!(
            (r.value, r.min, r.max, r.step),
            (50.0, 0.0, 100.0, Some(1.0))
        );
        let r2 = range("r2").unwrap();
        assert_eq!((r2.value, r2.step), (20.0, None));
        assert_eq!(range("n").map(|r| r.value), Some(3.0));
        assert_eq!(range("n2"), None);
        assert_eq!(range("p").map(|r| (r.value, r.max)), Some((5.0, 10.0)));
        assert_eq!(range("p2"), None);
        assert!(dom.native_flags(dom.by_id("p2")).indeterminate);
        assert_eq!(range("m").map(|r| (r.value, r.max)), Some((0.4, 1.0)));
    }

    #[test]
    fn positions_in_sets() {
        let dom = TestDom::from_html(
            "<ul><li id=a>1</li><li id=b>2</li></ul>\
             <select><option id=o1>1</option><option id=o2>2</option></select>\
             <form><input id=r1 type=radio name=g><input id=r2 type=radio name=g>\
             <input id=r3 type=radio name=h></form>",
        );
        let position = |id: &str| dom.position_in_set(dom.by_id(id));
        assert_eq!(position("a"), Some((1, 2)));
        assert_eq!(position("b"), Some((2, 2)));
        assert_eq!(position("o2"), Some((2, 2)));
        assert_eq!(position("r2"), Some((2, 2)));
        assert_eq!(position("r3"), Some((1, 1)));
    }

    #[test]
    fn the_arena_constructor_still_works_for_the_skeleton() {
        let dom = TestDom(Arena::builder().open("p").text("hello").close().build());
        assert_eq!(dom.first("p").local_name(), "p");
        let dom: TestDom = Arena::builder().open("p").close().build().into();
        assert_eq!(dom.0.len(), 1);
    }
}
