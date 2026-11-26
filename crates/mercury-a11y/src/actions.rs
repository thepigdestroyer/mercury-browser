// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! The actions a node advertises to assistive technology.
//!
//! Owner: item 3 (`core-states`). Item 5's `accessibility_actions.rs` (script side) implements
//! exactly [`SUPPORTED_ACTIONS`]; a test pins the list, and another checks that nothing outside it
//! is ever advertised.
//!
//! # What a node advertises
//!
//! [`apply`] runs after the role and the states, and reads them. A node that is `hidden` (the
//! subtree is not exposed) advertises nothing. Every other node advertises:
//!
//! * [`Action::ScrollIntoView`] and [`Action::ScrollToPoint`]: on every element **and every text
//!   node** (the screen reader's cursor can rest on a line of text that is off screen, and asks for
//!   it to be scrolled into view), a disabled control included. Nothing else on a text node.
//! * [`Action::ScrollUp`], `ScrollDown`, `ScrollLeft`, `ScrollRight`: a scroll container
//!   (`NativeFlags::scroll_container`).
//!
//! and, **if the node is not disabled**:
//!
//! * [`Action::Focus`], [`Action::Blur`] and [`Action::ShowContextMenu`]: a focusable element
//!   (`NativeFlags::focusable`, or a valid `tabindex`).
//! * [`Action::Click`] (the "default action"): a button, link, checkbox, radio button, switch, menu
//!   item, tab, option, tree item, disclosure triangle, pop-up button and colour well; any selectable
//!   item (`aria-selected` known); a focusable generic element, group, list item, image or cell; and
//!   whatever the host says has a click listener ([`HostState::has_click_handler`]). **Not** a text
//!   field, a slider or a spin button: a person focuses those, and `Click` on a text field would do
//!   nothing a `Focus` does not.
//! * [`Action::SetValue`] and [`Action::ReplaceSelectedText`]: editable text the person can type into
//!   (`NativeFlags::editable`, not read-only). A password field is editable and advertises them: its
//!   value is never *read* (D7), but an assistive technology may type into it as a person does.
//! * [`Action::Increment`] and [`Action::Decrement`]: a slider or spin button that is not read-only
//!   (an ARIA widget needs a current value to step from; a native `input` does not).
//! * [`Action::Expand`] or [`Action::Collapse`] (the one that changes the state): a node whose
//!   expanded state is known.

use a11y_dom::{Node, NodeKind, Rendering};
use accesskit::{Action, Role};

use crate::host::HostState;
use crate::semantics::NodeSemantics;

/// Every action the engine can perform: the whole set item 5 implements, in the order of the
/// brief. A test pins this list.
pub const SUPPORTED_ACTIONS: &[Action] = &[
    Action::Focus,
    Action::Blur,
    Action::Click,
    Action::SetValue,
    Action::ReplaceSelectedText,
    Action::ScrollIntoView,
    Action::ScrollToPoint,
    Action::ScrollUp,
    Action::ScrollDown,
    Action::ScrollLeft,
    Action::ScrollRight,
    Action::Increment,
    Action::Decrement,
    Action::Expand,
    Action::Collapse,
    Action::ShowContextMenu,
];

/// Roles with a default action: pressing them does what a click does.
fn has_default_action(role: Role) -> bool {
    matches!(
        role,
        Role::Button
            | Role::DefaultButton
            | Role::Link
            | Role::CheckBox
            | Role::RadioButton
            | Role::Switch
            | Role::MenuItem
            | Role::MenuItemCheckBox
            | Role::MenuItemRadio
            | Role::Tab
            | Role::ListBoxOption
            | Role::MenuListOption
            | Role::TreeItem
            | Role::DisclosureTriangle
            | Role::ComboBox
            | Role::ColorWell
    )
}

/// Roles that are clickable when they can take focus (a `div` with a `tabindex` that does a job).
fn is_clickable_when_focusable(role: Role) -> bool {
    matches!(
        role,
        Role::GenericContainer
            | Role::Group
            | Role::ListItem
            | Role::Image
            | Role::Paragraph
            | Role::Article
            | Role::Cell
            | Role::Row
            | Role::GridCell
            | Role::ColumnHeader
            | Role::RowHeader
            | Role::Label
    )
}

/// `tabindex` with an integer value makes any element focusable (HTML).
fn has_tabindex<'n, N: Node<'n>>(node: N) -> bool {
    node.attr("tabindex")
        .is_some_and(|value| value.trim().parse::<i32>().is_ok())
}

/// Add the actions `node` supports to `semantics.node`.
pub fn apply<'n, D>(doc: &'n D, node: D::N<'n>, semantics: &mut NodeSemantics)
where
    D: Rendering + HostState,
{
    if node.kind() == NodeKind::Text {
        semantics.node.add_action(Action::ScrollIntoView);
        return;
    }
    if semantics.hidden {
        return;
    }
    let flags = doc.native_flags(node);
    let target = &mut semantics.node;
    let role = target.role();

    target.add_action(Action::ScrollIntoView);
    target.add_action(Action::ScrollToPoint);
    if flags.scroll_container {
        for action in [
            Action::ScrollUp,
            Action::ScrollDown,
            Action::ScrollLeft,
            Action::ScrollRight,
        ] {
            target.add_action(action);
        }
    }
    if target.is_disabled() {
        return;
    }

    let focusable = flags.focusable || has_tabindex(node);
    if focusable {
        target.add_action(Action::Focus);
        target.add_action(Action::Blur);
        target.add_action(Action::ShowContextMenu);
    }

    let editable = flags.editable && !target.is_read_only() && !flags.read_only;
    if editable {
        target.add_action(Action::SetValue);
        target.add_action(Action::ReplaceSelectedText);
    }

    let clickable = has_default_action(role)
        || target.is_selected().is_some()
        || (focusable && is_clickable_when_focusable(role))
        || (!editable && doc.has_click_handler(node));
    if clickable {
        target.add_action(Action::Click);
    }

    if matches!(role, Role::Slider | Role::SpinButton)
        && !target.is_read_only()
        && !flags.read_only
        && (node.is_element("input") || target.numeric_value().is_some())
    {
        target.add_action(Action::Increment);
        target.add_action(Action::Decrement);
    }

    match target.is_expanded() {
        Some(true) => target.add_action(Action::Collapse),
        Some(false) => target.add_action(Action::Expand),
        None => {}
    }
}
