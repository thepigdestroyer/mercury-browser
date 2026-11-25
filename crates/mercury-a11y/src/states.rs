// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! States, properties and values of a node (EE4).
//!
//! Owner: item 3 (`core-states`). [`apply`] runs after the role and the name are known (the role is
//! already `semantics.node.role()`) and sets, from the ARIA 1.2 value types and the HTML-AAM
//! mappings: toggled (`aria-checked`, `aria-pressed`, native `checked` and `indeterminate`),
//! expanded, selected, disabled, read-only, required, invalid, busy, modal, multiselectable,
//! has-popup, current, orientation, level, set size and position, the value of a form control, the
//! numeric value with its range and step, the placeholder, the URL of a link and `hidden`.
//!
//! # Rules worth knowing
//!
//! * **A password field has no value, no numeric value and no `aria-valuetext`** (D7). This is
//!   decided here from the element (`input type=password`) *and* from the role
//!   ([`Role::PasswordInput`]), whatever [`HostState::control_value`] says: a host that forgets the
//!   rule still leaks nothing. The state, the placeholder and the name are exposed as for any input.
//! * **Native state wins over ARIA** on the element that has it: `input type=checkbox` is checked
//!   when it is, `aria-checked` on it is ignored; `<option selected>` is selected whatever
//!   `aria-selected` says. A role that comes from `role=` has no native state, so ARIA decides.
//! * **ARIA state only on the roles that take it** (ARIA 1.2, "Used in roles"): `aria-checked` on a
//!   `div` with no role is ignored, `aria-expanded` on a paragraph too. Global attributes
//!   (`aria-busy`, `aria-current`, `aria-invalid`, `aria-haspopup`, `aria-disabled`) apply to every
//!   element.
//! * **Required-property fallbacks** (ARIA 1.2, as Blink does): a `checkbox`, `radio`, `switch`,
//!   `menuitemcheckbox` or `menuitemradio` with no `aria-checked` is unchecked, a `combobox` with no
//!   `aria-expanded` is collapsed, an `option` with no `aria-selected` is not selected, a `slider`
//!   with no `aria-valuenow` is at the midpoint of its range, a `heading` with no `aria-level` is at
//!   level 2.
//! * **`hidden`** ([`NodeSemantics::hidden`], mirrored by `semantics.node.is_hidden()`): `aria-hidden`
//!   `true`, `inert`, `input type=hidden`, the `hidden` attribute (unless the page's CSS makes the
//!   element render), computed `display: none` and `visibility: hidden|collapse`. AccessKit hides the
//!   subtree of a hidden node, so a node needs only its own state. One exception, written for the
//!   case "a focusable element inside `aria-hidden`" (an author error that is common in carousels
//!   and off-canvas menus): an element that holds keyboard focus ([`HostState::has_focus`]) is not
//!   hidden by `aria-hidden`, so that focus never lands on something a screen reader cannot see.
//!   Everything else under `aria-hidden` stays hidden.
//! * Strings that come from the page are cut at fixed sizes ([`MAX_VALUE_BYTES`],
//!   [`MAX_ATTRIBUTE_BYTES`]) at a character boundary, and numbers are clamped ([`MAX_COUNT`]):
//!   a page cannot make one node as big as the page.
//!
//! # Known gaps
//!
//! * `visibility: hidden` on a parent with `visibility: visible` on a child hides the child too
//!   (AccessKit hides subtrees).
//! * `aria-disabled` is inherited by widgets only (a `div` under a disabled toolbar stays enabled).
//! * Text runs, selection and the caret of a text control are A11Y-2 (EE6): `value` carries the
//!   text, nothing else.

use a11y_dom::{Node, NodeKind, Rendering, ancestors};
use accesskit::{
    AriaCurrent, AutoComplete, HasPopup, Invalid, Node as AccessNode, Orientation, Role, Toggled,
};

use crate::host::{HostState, NativeFlags, RangeValue};
use crate::semantics::NodeSemantics;

/// The longest `value` a text control exposes, in bytes (cut at a character boundary). A text area
/// can hold a whole book; the tree is sent again on every change, and the text itself belongs to the
/// text-run work of A11Y-2.
pub const MAX_VALUE_BYTES: usize = 256 * 1024;

/// The longest string taken from an attribute (`placeholder`, `aria-valuetext`, a link's URL).
pub const MAX_ATTRIBUTE_BYTES: usize = 8 * 1024;

/// The largest level, set size or set position exposed: a page that says `aria-setsize=999999999999`
/// is not believed beyond this.
pub const MAX_COUNT: usize = 1_000_000;

/// Set the states, properties, value and `hidden` of `semantics`.
pub fn apply<'n, D>(doc: &'n D, node: D::N<'n>, semantics: &mut NodeSemantics)
where
    D: Rendering + HostState,
{
    if node.kind() != NodeKind::Element {
        return;
    }
    let cx = Cx::new(doc, node, semantics.node.role());
    if is_hidden(&cx) {
        semantics.hidden = true;
        semantics.node.set_hidden();
    }
    let target = &mut semantics.node;
    disabled(&cx, target);
    read_only(&cx, target);
    required(&cx, target);
    invalid(&cx, target);
    toggled(&cx, target);
    expanded(&cx, target);
    selected(&cx, target);
    flags_and_enums(&cx, target);
    orientation(&cx, target);
    level_and_set(&cx, target);
    value(&cx, target);
    numeric(&cx, target);
    placeholder(&cx, target);
    url(&cx, target);
}

// ---------------------------------------------------------------------------------------------
// The element being computed.
// ---------------------------------------------------------------------------------------------

/// The `type` values HTML defines for `input`; any other value is a text field.
const INPUT_TYPES: &[&str] = &[
    "hidden",
    "text",
    "search",
    "tel",
    "url",
    "email",
    "password",
    "date",
    "month",
    "week",
    "time",
    "datetime-local",
    "number",
    "range",
    "color",
    "checkbox",
    "radio",
    "file",
    "submit",
    "image",
    "reset",
    "button",
];

struct Cx<'n, D: Rendering + HostState> {
    doc: &'n D,
    node: D::N<'n>,
    tag: &'n str,
    role: Role,
    flags: NativeFlags,
    /// The lower-case `type` of an `input` (`text` when absent or not one HTML defines), `None` for
    /// other elements.
    input_type: Option<String>,
    /// An `input type=password`, or a node whose role is [`Role::PasswordInput`].
    password: bool,
}

impl<'n, D: Rendering + HostState> Cx<'n, D> {
    fn new(doc: &'n D, node: D::N<'n>, role: Role) -> Self {
        let tag = node.local_name();
        let input_type = (tag == "input").then(|| {
            let ty = node.attr("type").map(token).unwrap_or_default();
            if INPUT_TYPES.contains(&ty.as_str()) {
                ty
            } else {
                "text".to_string()
            }
        });
        let password = role == Role::PasswordInput || input_type.as_deref() == Some("password");
        Cx {
            doc,
            node,
            tag,
            role,
            flags: doc.native_flags(node),
            input_type,
            password,
        }
    }

    fn attr(&self, name: &str) -> Option<&'n str> {
        self.node.attr(name)
    }

    fn is_input(&self, ty: &str) -> bool {
        self.input_type.as_deref() == Some(ty)
    }
}

// ---------------------------------------------------------------------------------------------
// Attribute parsing.
// ---------------------------------------------------------------------------------------------

/// An ARIA token: trimmed and ASCII lower case (browsers compare token values case-insensitively).
fn token(value: &str) -> String {
    value.trim().to_ascii_lowercase()
}

/// `true` or `false`; anything else (absent, empty, `undefined`, junk) is "not specified".
fn aria_bool<'n, N: Node<'n>>(node: N, name: &str) -> Option<bool> {
    match token(node.attr(name)?).as_str() {
        "true" => Some(true),
        "false" => Some(false),
        _ => None,
    }
}

fn aria_true<'n, N: Node<'n>>(node: N, name: &str) -> bool {
    aria_bool(node, name) == Some(true)
}

/// `true`, `false` or `mixed`.
fn aria_tristate<'n, N: Node<'n>>(node: N, name: &str) -> Option<Toggled> {
    match token(node.attr(name)?).as_str() {
        "true" => Some(Toggled::True),
        "false" => Some(Toggled::False),
        "mixed" => Some(Toggled::Mixed),
        _ => None,
    }
}

/// A finite number. Rust's parser also takes `inf` and `nan`; those are not numbers here.
fn number(text: &str) -> Option<f64> {
    let value = text.trim().parse::<f64>().ok()?;
    value.is_finite().then_some(value)
}

fn attr_number<'n, N: Node<'n>>(node: N, name: &str) -> Option<f64> {
    number(node.attr(name)?)
}

/// A whole number of at least `min`, capped at [`MAX_COUNT`].
fn count(text: &str, min: i64) -> Option<usize> {
    let value = text.trim().parse::<i64>().ok()?;
    if value < min {
        return None;
    }
    Some((value as u64).min(MAX_COUNT as u64) as usize)
}

/// `text` cut to at most `max` bytes at a character boundary.
fn capped(text: &str, max: usize) -> &str {
    if text.len() <= max {
        return text;
    }
    let mut end = max;
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end]
}

// ---------------------------------------------------------------------------------------------
// Role families (AccessKit roles, from the "Used in roles" lists of ARIA 1.2).
// ---------------------------------------------------------------------------------------------

/// Roles of a text field the user types into (a password included; its value is never read).
fn is_text_field(role: Role) -> bool {
    matches!(
        role,
        Role::TextInput
            | Role::MultilineTextInput
            | Role::SearchInput
            | Role::EmailInput
            | Role::PhoneNumberInput
            | Role::UrlInput
            | Role::PasswordInput
            | Role::NumberInput
            | Role::DateInput
            | Role::DateTimeInput
            | Role::WeekInput
            | Role::MonthInput
            | Role::TimeInput
    )
}

/// Roles that take `aria-checked` (ARIA 1.2: checkbox, menuitemcheckbox, menuitemradio, option, radio,
/// switch, treeitem).
fn supports_checked(role: Role) -> bool {
    matches!(
        role,
        Role::CheckBox
            | Role::MenuItemCheckBox
            | Role::MenuItemRadio
            | Role::ListBoxOption
            | Role::MenuListOption
            | Role::RadioButton
            | Role::Switch
            | Role::TreeItem
    )
}

/// The roles whose `aria-checked` is a required property: with none, the control is unchecked.
fn requires_checked(role: Role) -> bool {
    matches!(
        role,
        Role::CheckBox
            | Role::MenuItemCheckBox
            | Role::MenuItemRadio
            | Role::RadioButton
            | Role::Switch
    )
}

/// Roles that take `aria-expanded`.
fn supports_expanded(role: Role) -> bool {
    matches!(
        role,
        Role::Application
            | Role::Button
            | Role::DefaultButton
            | Role::CheckBox
            | Role::ComboBox
            | Role::EditableComboBox
            | Role::GridCell
            | Role::ColumnHeader
            | Role::RowHeader
            | Role::Link
            | Role::ListBox
            | Role::MenuItem
            | Role::MenuItemCheckBox
            | Role::MenuItemRadio
            | Role::Row
            | Role::Switch
            | Role::Tab
            | Role::TreeItem
            | Role::DisclosureTriangle
            | Role::Details
    )
}

/// Roles that take `aria-selected`.
fn supports_selected(role: Role) -> bool {
    matches!(
        role,
        Role::GridCell
            | Role::ColumnHeader
            | Role::RowHeader
            | Role::ListBoxOption
            | Role::MenuListOption
            | Role::Row
            | Role::Tab
            | Role::TreeItem
    )
}

fn supports_read_only(role: Role) -> bool {
    is_text_field(role)
        || matches!(
            role,
            Role::CheckBox
                | Role::MenuItemCheckBox
                | Role::MenuItemRadio
                | Role::Switch
                | Role::ComboBox
                | Role::EditableComboBox
                | Role::Grid
                | Role::GridCell
                | Role::ColumnHeader
                | Role::RowHeader
                | Role::ListBox
                | Role::RadioGroup
                | Role::Slider
                | Role::SpinButton
                | Role::TreeGrid
        )
}

fn supports_required(role: Role) -> bool {
    is_text_field(role)
        || matches!(
            role,
            Role::CheckBox
                | Role::ComboBox
                | Role::EditableComboBox
                | Role::GridCell
                | Role::ListBox
                | Role::RadioGroup
                | Role::SpinButton
                | Role::Tree
                | Role::TreeGrid
                | Role::Switch
        )
}

/// A widget: the roles under a disabled `aria-disabled` ancestor inherit its state.
fn is_widget(role: Role) -> bool {
    is_text_field(role)
        || matches!(
            role,
            Role::Button
                | Role::DefaultButton
                | Role::CheckBox
                | Role::RadioButton
                | Role::Switch
                | Role::ComboBox
                | Role::EditableComboBox
                | Role::Slider
                | Role::SpinButton
                | Role::Tab
                | Role::MenuItem
                | Role::MenuItemCheckBox
                | Role::MenuItemRadio
                | Role::ListBoxOption
                | Role::MenuListOption
                | Role::TreeItem
                | Role::Link
                | Role::GridCell
                | Role::ColorWell
                | Role::DisclosureTriangle
        )
}

// ---------------------------------------------------------------------------------------------
// hidden
// ---------------------------------------------------------------------------------------------

fn is_hidden<D: Rendering + HostState>(cx: &Cx<'_, D>) -> bool {
    let node = cx.node;
    if aria_true(node, "aria-hidden") && !cx.doc.has_focus(node) {
        return true;
    }
    if node.has_attr("inert") || cx.is_input("hidden") {
        return true;
    }
    let style = cx.doc.computed_style(node);
    if let Some(style) = &style {
        if style.display.as_deref().map(token).as_deref() == Some("none") {
            return true;
        }
        if matches!(
            style.visibility.as_deref().map(token).as_deref(),
            Some("hidden" | "collapse")
        ) {
            return true;
        }
    }
    if let Some(hidden) = node.attr("hidden") {
        // `hidden=until-found` hides with `content-visibility`, which no display value undoes.
        if token(hidden) == "until-found" {
            return true;
        }
        // Otherwise the attribute is a user-agent `display: none`: the page's CSS may have made the
        // element render (`[hidden] { display: block }`), and what renders is what a screen reader
        // reads. With no computed style (no layout) the attribute decides.
        return style.is_none();
    }
    false
}

// ---------------------------------------------------------------------------------------------
// disabled, read-only, required, invalid
// ---------------------------------------------------------------------------------------------

fn disabled<D: Rendering + HostState>(cx: &Cx<'_, D>, node: &mut AccessNode) {
    let own = cx.flags.disabled || aria_true(cx.node, "aria-disabled");
    let inherited = !own
        && (is_widget(cx.role) || cx.flags.focusable)
        && ancestors(cx.node).any(|ancestor| aria_true(ancestor, "aria-disabled"));
    if own || inherited {
        node.set_disabled();
    }
}

fn read_only<D: Rendering + HostState>(cx: &Cx<'_, D>, node: &mut AccessNode) {
    if cx.flags.read_only || (supports_read_only(cx.role) && aria_true(cx.node, "aria-readonly")) {
        node.set_read_only();
    }
}

fn required<D: Rendering + HostState>(cx: &Cx<'_, D>, node: &mut AccessNode) {
    if cx.flags.required || (supports_required(cx.role) && aria_true(cx.node, "aria-required")) {
        node.set_required();
    }
}

/// `aria-invalid` when it is present and not empty (`false` included: the page says the control is
/// fine, whatever constraint validation thinks), else the native `:invalid`. A value ARIA does not
/// list counts as `true`, as Blink treats it.
fn invalid<D: Rendering + HostState>(cx: &Cx<'_, D>, node: &mut AccessNode) {
    let explicit = cx
        .attr("aria-invalid")
        .map(token)
        .filter(|value| !value.is_empty());
    match explicit.as_deref() {
        Some("false") => {}
        Some("grammar") => node.set_invalid(Invalid::Grammar),
        Some("spelling") => node.set_invalid(Invalid::Spelling),
        Some(_) => node.set_invalid(Invalid::True),
        None => {
            if cx.flags.invalid {
                node.set_invalid(Invalid::True);
            }
        }
    }
}

// ---------------------------------------------------------------------------------------------
// toggled, expanded, selected
// ---------------------------------------------------------------------------------------------

fn toggled<D: Rendering + HostState>(cx: &Cx<'_, D>, node: &mut AccessNode) {
    let role = cx.role;
    let native_checkable = cx.is_input("checkbox") || cx.is_input("radio");
    if native_checkable && supports_checked(role) {
        let state = if cx.is_input("checkbox") && cx.flags.indeterminate {
            Toggled::Mixed
        } else {
            Toggled::from(cx.flags.checked)
        };
        node.set_toggled(state);
        return;
    }
    if supports_checked(role) {
        let state = match aria_tristate(cx.node, "aria-checked") {
            // `mixed` exists for checkboxes only; elsewhere it is not a valid value.
            Some(Toggled::Mixed) if !matches!(role, Role::CheckBox | Role::MenuItemCheckBox) => {
                Some(Toggled::False)
            }
            Some(state) => Some(state),
            None if requires_checked(role) => Some(Toggled::False),
            None => None,
        };
        if let Some(state) = state {
            node.set_toggled(state);
        }
        return;
    }
    if matches!(role, Role::Button | Role::DefaultButton)
        && let Some(state) = aria_tristate(cx.node, "aria-pressed")
    {
        node.set_toggled(state);
    }
}

fn expanded<D: Rendering + HostState>(cx: &Cx<'_, D>, node: &mut AccessNode) {
    let role = cx.role;
    if role == Role::Details {
        node.set_expanded(cx.flags.open);
        return;
    }
    if role == Role::DisclosureTriangle {
        let open = cx
            .node
            .parent()
            .filter(|parent| parent.is_element("details"))
            .map(|details| cx.doc.native_flags(details).open);
        if let Some(open) = open {
            node.set_expanded(open);
            return;
        }
    }
    if !supports_expanded(role) {
        return;
    }
    let state = match (role, aria_bool(cx.node, "aria-expanded")) {
        (_, Some(state)) => Some(state),
        // A `select` is a combobox whose popup state the host knows; the required `aria-expanded`
        // of any other combobox is "collapsed".
        (Role::ComboBox | Role::EditableComboBox, None) => {
            Some(cx.tag == "select" && cx.flags.open)
        }
        _ => None,
    };
    if let Some(state) = state {
        node.set_expanded(state);
    }
}

fn selected<D: Rendering + HostState>(cx: &Cx<'_, D>, node: &mut AccessNode) {
    if cx.tag == "option" {
        node.set_selected(cx.flags.selected);
        return;
    }
    if !supports_selected(cx.role) {
        return;
    }
    match aria_bool(cx.node, "aria-selected") {
        Some(state) => node.set_selected(state),
        // `aria-selected` is a required property of an option: with none it is not selected.
        None if matches!(cx.role, Role::ListBoxOption | Role::MenuListOption) => {
            node.set_selected(false);
        }
        None => {}
    }
}

// ---------------------------------------------------------------------------------------------
// busy, modal, multiselectable, has-popup, current, auto-complete
// ---------------------------------------------------------------------------------------------

fn flags_and_enums<D: Rendering + HostState>(cx: &Cx<'_, D>, node: &mut AccessNode) {
    let element = cx.node;
    if aria_true(element, "aria-busy") {
        node.set_busy();
    }
    if matches!(cx.role, Role::Dialog | Role::AlertDialog) && aria_true(element, "aria-modal") {
        node.set_modal();
    }
    let multiselectable = match cx.role {
        Role::ListBox if cx.tag == "select" => element.has_attr("multiple"),
        Role::ListBox | Role::Grid | Role::TabList | Role::Tree | Role::TreeGrid => {
            aria_true(element, "aria-multiselectable")
        }
        _ => false,
    };
    if multiselectable {
        node.set_multiselectable();
    }
    if let Some(popup) = element.attr("aria-haspopup").map(token) {
        match popup.as_str() {
            "" | "false" | "undefined" => {}
            "listbox" => node.set_has_popup(HasPopup::Listbox),
            "tree" => node.set_has_popup(HasPopup::Tree),
            "grid" => node.set_has_popup(HasPopup::Grid),
            "dialog" => node.set_has_popup(HasPopup::Dialog),
            // `true`, `menu` and every other value: a menu.
            _ => node.set_has_popup(HasPopup::Menu),
        }
    }
    if let Some(current) = element.attr("aria-current").map(token) {
        match current.as_str() {
            "" | "false" | "undefined" => {}
            "page" => node.set_aria_current(AriaCurrent::Page),
            "step" => node.set_aria_current(AriaCurrent::Step),
            "location" => node.set_aria_current(AriaCurrent::Location),
            "date" => node.set_aria_current(AriaCurrent::Date),
            "time" => node.set_aria_current(AriaCurrent::Time),
            // `true` and every other value.
            _ => node.set_aria_current(AriaCurrent::True),
        }
    }
    if matches!(
        cx.role,
        Role::ComboBox | Role::EditableComboBox | Role::SearchInput | Role::TextInput
    ) && let Some(complete) = element.attr("aria-autocomplete").map(token)
    {
        match complete.as_str() {
            "inline" => node.set_auto_complete(AutoComplete::Inline),
            "list" => node.set_auto_complete(AutoComplete::List),
            "both" => node.set_auto_complete(AutoComplete::Both),
            _ => {}
        }
    }
}

// ---------------------------------------------------------------------------------------------
// orientation
// ---------------------------------------------------------------------------------------------

/// `aria-orientation` where the role takes it, else the role's implicit orientation.
fn orientation<D: Rendering + HostState>(cx: &Cx<'_, D>, node: &mut AccessNode) {
    let implicit = match cx.role {
        Role::ScrollBar | Role::ListBox | Role::Menu | Role::Tree | Role::TreeGrid => {
            Orientation::Vertical
        }
        Role::Slider | Role::TabList | Role::Toolbar | Role::MenuBar | Role::Splitter => {
            Orientation::Horizontal
        }
        _ => return,
    };
    let explicit = match cx.attr("aria-orientation").map(token).as_deref() {
        Some("horizontal") => Some(Orientation::Horizontal),
        Some("vertical") => Some(Orientation::Vertical),
        _ => None,
    };
    node.set_orientation(explicit.unwrap_or(implicit));
}

// ---------------------------------------------------------------------------------------------
// level, set size and position
// ---------------------------------------------------------------------------------------------

fn level_and_set<D: Rendering + HostState>(cx: &Cx<'_, D>, node: &mut AccessNode) {
    let role = cx.role;
    let element = cx.node;
    match role {
        Role::Heading => {
            let from_tag = || match cx.tag {
                "h1" => Some(1),
                "h2" => Some(2),
                "h3" => Some(3),
                "h4" => Some(4),
                "h5" => Some(5),
                "h6" => Some(6),
                _ => None,
            };
            let level = element
                .attr("aria-level")
                .and_then(|text| count(text, 1))
                .or_else(from_tag)
                .unwrap_or(2);
            node.set_level(level);
        }
        Role::ListItem | Role::Row | Role::TreeItem | Role::Tab => {
            if let Some(level) = element.attr("aria-level").and_then(|text| count(text, 1)) {
                node.set_level(level);
            }
        }
        _ => {}
    }
    let takes_position = matches!(
        role,
        Role::ListItem
            | Role::Article
            | Role::MenuItem
            | Role::MenuItemCheckBox
            | Role::MenuItemRadio
            | Role::ListBoxOption
            | Role::MenuListOption
            | Role::RadioButton
            | Role::Row
            | Role::Tab
            | Role::TreeItem
    );
    if !takes_position {
        return;
    }
    // `aria-setsize` is at least 0; -1 says "unknown" and exposes nothing.
    let size = element.attr("aria-setsize").and_then(|text| count(text, 0));
    let position = element
        .attr("aria-posinset")
        .and_then(|text| count(text, 1));
    if size.is_some() || position.is_some() {
        if let Some(size) = size {
            node.set_size_of_set(size);
        }
        if let Some(position) = position {
            node.set_position_in_set(position);
        }
    } else if let Some((position, size)) = cx.doc.position_in_set(element) {
        node.set_size_of_set(size.min(MAX_COUNT));
        node.set_position_in_set(position.min(MAX_COUNT));
    }
}

// ---------------------------------------------------------------------------------------------
// value
// ---------------------------------------------------------------------------------------------

/// The `input` types whose value is text a person typed or picked, and so is the node's value.
/// (`range` is a number; `checkbox`, `radio`, the buttons, `file`, `image` and `hidden` have no text
/// value; `password` has none by rule.)
fn input_has_text_value(ty: &str) -> bool {
    !matches!(
        ty,
        "password"
            | "checkbox"
            | "radio"
            | "button"
            | "submit"
            | "reset"
            | "image"
            | "file"
            | "hidden"
            | "range"
    )
}

fn value<D: Rendering + HostState>(cx: &Cx<'_, D>, node: &mut AccessNode) {
    // D7: a password field exposes no value of any kind.
    if cx.password {
        return;
    }
    let text_value = match cx.tag {
        "input" => cx.input_type.as_deref().is_some_and(input_has_text_value),
        "textarea" | "select" | "output" => true,
        _ => false,
    };
    if text_value && let Some(text) = cx.doc.control_value(cx.node) {
        node.set_value(capped(&text, MAX_VALUE_BYTES));
    }
    // `aria-valuetext` is the value a person hears for a range.
    if has_numeric_value(cx)
        && let Some(text) = cx.attr("aria-valuetext")
    {
        let text = text.trim();
        if !text.is_empty() {
            node.set_value(capped(text, MAX_ATTRIBUTE_BYTES));
        }
    }
}

// ---------------------------------------------------------------------------------------------
// numeric value, minimum, maximum, step
// ---------------------------------------------------------------------------------------------

/// Whether the node is a range-like widget (a `separator` only when it is focusable, a splitter).
fn has_numeric_value<D: Rendering + HostState>(cx: &Cx<'_, D>) -> bool {
    match cx.role {
        Role::Slider
        | Role::SpinButton
        | Role::ProgressIndicator
        | Role::Meter
        | Role::ScrollBar => true,
        Role::Splitter => cx.flags.focusable || cx.node.has_attr("tabindex"),
        _ => false,
    }
}

fn set_range(node: &mut AccessNode, range: RangeValue) {
    if !range.value.is_finite() || !range.min.is_finite() || !range.max.is_finite() {
        return;
    }
    node.set_numeric_value(range.value);
    node.set_min_numeric_value(range.min);
    node.set_max_numeric_value(range.max.max(range.min));
    if let Some(step) = range.step.filter(|step| step.is_finite() && *step > 0.0) {
        node.set_numeric_value_step(step);
    }
}

fn numeric<D: Rendering + HostState>(cx: &Cx<'_, D>, node: &mut AccessNode) {
    // D7 again: nothing numeric on a password, whatever the host would answer.
    if cx.password || !has_numeric_value(cx) {
        return;
    }
    let element = cx.node;
    // A native control: the host knows the value, the bounds and the step.
    if let Some(range) = cx.doc.range(element) {
        set_range(node, range);
        return;
    }
    // A native `input type=number` the host has no bounds for: the number typed, and any of
    // `min`, `max` and `step` that parse.
    if cx.is_input("number") {
        if let Some(value) = cx.doc.control_value(element).as_deref().and_then(number) {
            node.set_numeric_value(value);
        }
        if let Some(min) = attr_number(element, "min") {
            node.set_min_numeric_value(min);
        }
        if let Some(max) = attr_number(element, "max") {
            node.set_max_numeric_value(max);
        }
        if let Some(step) = attr_number(element, "step").filter(|step| *step > 0.0) {
            node.set_numeric_value_step(step);
        }
        return;
    }
    if matches!(cx.tag, "progress" | "meter") || cx.is_input("range") {
        // A native control the host gave no range for (`progress` with no `value`) is indeterminate.
        return;
    }
    // ARIA widgets. `aria-valuemin` and `aria-valuemax` default to 0 and 100 (a spin button has no
    // default bounds).
    let (default_min, default_max) = if cx.role == Role::SpinButton {
        (None, None)
    } else {
        (Some(0.0), Some(100.0))
    };
    let min = attr_number(element, "aria-valuemin").or(default_min);
    let max = attr_number(element, "aria-valuemax").or(default_max);
    let max = match (min, max) {
        (Some(min), Some(max)) => Some(max.max(min)),
        (_, max) => max,
    };
    let now = attr_number(element, "aria-valuenow").or_else(|| match (cx.role, min, max) {
        // A slider with no value is at the midpoint of its range.
        (Role::Slider, Some(min), Some(max)) => Some(min + (max - min) / 2.0),
        _ => None,
    });
    // No value: indeterminate (a progress bar) or not yet known; the bounds alone mean nothing.
    let Some(now) = now else {
        return;
    };
    node.set_numeric_value(now);
    if let Some(min) = min {
        node.set_min_numeric_value(min);
    }
    if let Some(max) = max {
        node.set_max_numeric_value(max);
    }
}

// ---------------------------------------------------------------------------------------------
// placeholder, URL
// ---------------------------------------------------------------------------------------------

fn placeholder<D: Rendering + HostState>(cx: &Cx<'_, D>, node: &mut AccessNode) {
    // The placeholder of a password field is fine to expose ("Password"); it is not the value.
    let native = match cx.tag {
        "textarea" => true,
        "input" => cx.input_type.as_deref().is_some_and(|ty| {
            matches!(
                ty,
                "text" | "search" | "url" | "tel" | "email" | "password" | "number"
            )
        }),
        _ => false,
    };
    let text = if native {
        cx.attr("placeholder")
    } else if is_text_field(cx.role) || matches!(cx.role, Role::ComboBox | Role::EditableComboBox) {
        cx.attr("aria-placeholder")
    } else {
        None
    };
    if let Some(text) = text.map(str::trim).filter(|text| !text.is_empty()) {
        node.set_placeholder(capped(text, MAX_ATTRIBUTE_BYTES));
    }
}

fn url<D: Rendering + HostState>(cx: &Cx<'_, D>, node: &mut AccessNode) {
    if cx.role != Role::Link || !matches!(cx.tag, "a" | "area") {
        return;
    }
    let Some(href) = cx.attr("href") else {
        return;
    };
    let resolved = cx.doc.link_url(cx.node);
    let text = resolved.as_deref().unwrap_or_else(|| href.trim());
    // A `data:` or `javascript:` URL can be as long as the page; a link that long exposes none.
    if !text.is_empty() && text.len() <= MAX_ATTRIBUTE_BYTES {
        node.set_url(text);
    }
}
