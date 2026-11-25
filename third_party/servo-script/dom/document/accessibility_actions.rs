/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

// Mercury: a11y-1 item 5 (`actions-frames`), EE5. This whole file is Mercury's own: upstream
// Servo 0.8 performs one accessibility action, an untrusted `click` (it was the body of
// `Window::handle_accessibility_actions`). The one hook in `window.rs` calls
// `Window::perform_accessibility_actions` below. See `.superpowers/sdd/lane-a11y-1-brief.md`,
// item 5, and `.superpowers/sdd/a11y-1-logs/notes/item5.md`.

//! The actions an assistive technology sends to a web page.
//!
//! An [`accesskit::ActionRequest`] reaches the document's layout (`Layout::handle_accessibility_action`),
//! which keeps it until the next reflow and hands it back, with the target as an
//! [`OpaqueNode`](style::dom::OpaqueNode), in `ReflowResult::pending_accessibility_actions`. The
//! window then calls [`Window::perform_accessibility_actions`].
//!
//! # The set of actions
//!
//! The actions a node advertises are decided in `mercury_a11y::actions::SUPPORTED_ACTIONS` (a light
//! crate that `servo-script` does not depend on, so the list is copied here). This file implements
//! **exactly** that list:
//!
//! `Focus`, `Blur`, `Click`, `SetValue`, `ReplaceSelectedText`, `ScrollIntoView`, `ScrollToPoint`,
//! `ScrollUp`, `ScrollDown`, `ScrollLeft`, `ScrollRight`, `Increment`, `Decrement`, `Expand`,
//! `Collapse`, `ShowContextMenu`.
//!
//! The `match` in [`perform_on_target`] has no wildcard arm: a variant that a later AccessKit adds
//! stops this file from compiling, and has to be placed on purpose. The actions that are not
//! implemented (`CustomAction`, the two tooltip actions, `SetScrollOffset`, `SetTextSelection`
//! until EE6 and `SetSequentialFocusNavigationStartingPoint`) return with one `debug` line that
//! names the action.
//!
//! # User activation (decision D6, OD-5)
//!
//! Only the actions that **perform a click** (`Click`, and `Expand` or `Collapse` when the
//! implementation clicks) set the window's last activation timestamp, through the same
//! "activation notification steps" that a real `mousedown` runs (so `window.open`, `requestFullscreen`
//! and the like work in the click handler, as they do after a click in Chromium). `Focus`,
//! `Blur`, the scrolling actions, `SetValue` and the rest do not. The channel is embedder-only:
//! a page cannot send an `ActionRequest`, so it cannot grant itself activation.
//!
//! # What an action may touch
//!
//! Every action first checks that the document is fully active and the target is still connected.
//! The actions that operate on the control also leave a disabled target alone (native `disabled`,
//! including the one a `fieldset` hands down, and `aria-disabled="true"`) and an inert one. The
//! scrolling actions are the exception, on purpose: the person who reads a disabled button that is
//! off screen still has to be able to scroll to it (and `mercury_a11y` advertises them on one).
//!
//! # Logging (decision D9)
//!
//! A log line names the action and never the data: the `Value` of `SetValue` is what the person
//! typed.

use accesskit::{Action, ActionData, ScrollHint, ScrollUnit};
use embedder_traits::EditingAction;
use html5ever::{LocalName, local_name};
use js::context::JSContext;
use layout_api::{AccessibilityActionRequest, ScrollContainerQueryFlags};
use script_bindings::codegen::GenericBindings::ElementBinding::ScrollLogicalPosition;
use script_bindings::codegen::GenericBindings::WindowBinding::{ScrollBehavior, WindowMethods};
use script_bindings::inheritance::Castable;
use script_bindings::root::DomRoot;
use script_bindings::str::DOMString;
use style::Atom;
use stylo_dom::ElementState;
use webrender_api::units::LayoutVector2D;

use crate::dom::bindings::codegen::Bindings::HTMLDetailsElementBinding::HTMLDetailsElementMethods;
use crate::dom::bindings::codegen::Bindings::HTMLInputElementBinding::HTMLInputElementMethods;
use crate::dom::bindings::codegen::Bindings::NodeBinding::NodeMethods;
use crate::dom::bindings::refcounted::Trusted;
use crate::dom::document::focus::FocusableArea;
use crate::dom::event::{EventBubbles, EventCancelable, EventComposed};
use crate::dom::eventtarget::EventTarget;
use crate::dom::html::form_controls::input_type::{InputType, ValueChangeEvents};
use crate::dom::html::form_controls::text_control::TextControlElement;
use crate::dom::node::focus::FocusTrigger;
use crate::dom::node::{Node, NodeTraits, from_untrusted_node_address};
use crate::dom::types::{
    Element, HTMLDetailsElement, HTMLInputElement, HTMLSelectElement, HTMLTextAreaElement,
};
use crate::dom::window::Window;
use crate::dom::window::scrolling_box::{ScrollAxisState, ScrollRequirement, ScrollingBox};
use crate::dom::window::useractivation::UserActivation;

/// The distance of `ScrollUnit::Item`, in CSS pixels: the line step of a mouse wheel notch
/// (Blink's `kPixelsPerLineStep`). `ScrollUnit::Page` is the size of the scrolling box.
const SCROLL_ITEM_STEP: f32 = 40.0;

/// The most scrolling boxes `ScrollToPoint` walks outwards through (nested scroll containers).
const MAX_SCROLLING_BOXES: usize = 32;

impl Window {
    /// Perform the accessibility actions that layout handed back after a reflow, in order.
    ///
    /// This is the one call of the hook in `Window::handle_accessibility_actions`.
    pub(crate) fn perform_accessibility_actions(
        &self,
        requests: Vec<AccessibilityActionRequest>,
        cx: &mut JSContext,
    ) {
        perform_all(self, requests, cx);
    }
}

/// Perform `requests` in order.
///
/// Every target is rooted **before** the first action runs: an action runs page script (a click
/// handler can remove the next target, and a collection can then free it), and a target is only
/// known to be alive immediately after the reflow that returned it.
fn perform_all(window: &Window, requests: Vec<AccessibilityActionRequest>, cx: &mut JSContext) {
    let _rooted_targets: Vec<DomRoot<Node>> = requests.iter().map(node_for_request).collect();
    for request in requests {
        perform(window, request, cx);
    }
}

/// The DOM node an action request targets.
#[expect(unsafe_code)]
fn node_for_request(request: &AccessibilityActionRequest) -> DomRoot<Node> {
    // Layout only returns a target that was still in its tree at the end of the reflow, and
    // `perform_all` converts all of them before any script runs.
    unsafe { from_untrusted_node_address(request.target.into()) }
}

/// Perform one accessibility action (the pinned entry point of this file).
pub(crate) fn perform(window: &Window, request: AccessibilityActionRequest, cx: &mut JSContext) {
    let target = node_for_request(&request);
    perform_on_target(window, &target, request.action, request.data, cx);
}

/// Perform `action` on `node`.
fn perform_on_target(
    window: &Window,
    node: &Node,
    action: Action,
    data: Option<ActionData>,
    cx: &mut JSContext,
) {
    // A document that is no longer fully active (history cache, being unloaded) and a node that
    // left the document never act.
    if !window.Document().is_fully_active() || !node.is_connected() {
        debug!("accessibility action {action:?}: the target is gone");
        return;
    }
    // An assistive technology's cursor can rest on a text run: the action goes to the element
    // that owns it.
    let Some(element) = target_element(node) else {
        debug!("accessibility action {action:?}: the target has no element");
        return;
    };

    let only_scrolls = matches!(
        action,
        Action::ScrollIntoView |
            Action::ScrollToPoint |
            Action::ScrollUp |
            Action::ScrollDown |
            Action::ScrollLeft |
            Action::ScrollRight
    );
    if !only_scrolls && !is_actionable(&element) {
        debug!("accessibility action {action:?}: the target is disabled or inert");
        return;
    }

    // No wildcard arm, on purpose: see the module documentation.
    match action {
        Action::Click => click(cx, &element),
        Action::Focus => focus(cx, &element),
        Action::Blur => blur(cx, &element),
        Action::Expand => set_expanded(cx, &element, true),
        Action::Collapse => set_expanded(cx, &element, false),
        Action::Increment => step(cx, &element, true),
        Action::Decrement => step(cx, &element, false),
        Action::SetValue => set_value(cx, &element, data),
        Action::ReplaceSelectedText => replace_selected_text(cx, &element, data),
        Action::ScrollIntoView => scroll_into_view(cx, &element, data),
        Action::ScrollToPoint => scroll_to_point(cx, &element, data),
        Action::ScrollUp | Action::ScrollDown | Action::ScrollLeft | Action::ScrollRight => {
            scroll_by_unit(cx, &element, action, data)
        },
        Action::ShowContextMenu => show_context_menu(cx, &element),
        // Not implemented: nothing advertises them (`mercury_a11y::actions::SUPPORTED_ACTIONS`).
        // `SetTextSelection` waits for the text work of the next wave (EE6).
        Action::CustomAction |
        Action::HideTooltip |
        Action::ShowTooltip |
        Action::SetScrollOffset |
        Action::SetTextSelection |
        Action::SetSequentialFocusNavigationStartingPoint => {
            debug!("accessibility action {action:?} is not supported");
        },
    }
}

/// The element an action on `node` applies to: the node itself, or the parent element of a text
/// node.
fn target_element(node: &Node) -> Option<DomRoot<Element>> {
    match node.downcast::<Element>() {
        Some(element) => Some(DomRoot::from_ref(element)),
        None => node.GetParentElement(),
    }
}

/// Whether the action may operate on the control: it is neither disabled nor inert.
fn is_actionable(element: &Element) -> bool {
    !element.disabled_state() &&
        !attribute_is_true(element, &local_name!("aria-disabled")) &&
        !element.upcast::<Node>().is_inert()
}

/// Whether the attribute is present and its value is `true` (ASCII case-insensitive).
fn attribute_is_true(element: &Element, name: &LocalName) -> bool {
    element
        .get_nullable_string_attribute(name)
        .is_some_and(|value| value.str().eq_ignore_ascii_case("true"))
}

/// The value of an attribute that holds a number, if it does.
fn number_attribute(element: &Element, name: &LocalName) -> Option<f64> {
    element
        .get_nullable_string_attribute(name)
        .and_then(|value| value.str().trim().parse::<f64>().ok())
        .filter(|number| number.is_finite())
}

// ---------------------------------------------------------------------------------------------
// Click, focus, blur, context menu
// ---------------------------------------------------------------------------------------------

/// Dispatch a click as a person's click would be handled: the activation notification steps of
/// the HTML Standard first (D6), then an untrusted `click` event at `element`.
///
/// The event is the one the engine already sent for this action (it keeps the default action of a
/// link, a button or a checkbox). It is not trusted, so the places that accept only trusted
/// clicks need their own call: see [`click`] for `<select>`.
fn click_with_user_activation(cx: &mut JSContext, element: &Element) {
    let document = element.owner_document();
    if document.is_fully_active() {
        UserActivation::handle_user_activation_notification(&document);
    }
    element
        .upcast::<Node>()
        .fire_synthetic_pointer_event_not_trusted(cx, atom!("click"));
}

/// `Click`: the default action of the element.
fn click(cx: &mut JSContext, element: &Element) {
    click_with_user_activation(cx, element);
    // The activation behavior of `<select>` opens the picker for a trusted click only, and a
    // person who presses a pop-up button expects the list to open.
    if let Some(select) = element.downcast::<HTMLSelectElement>() &&
        element.is_connected() &&
        !element.state().contains(ElementState::OPEN)
    {
        select.show_menu(cx);
    }
}

/// `Focus`: give the element keyboard focus, as `element.focus()` does (events included), and
/// bring it into view when it is not.
fn focus(cx: &mut JSContext, element: &Element) {
    if !element
        .upcast::<Node>()
        .run_the_focusing_steps(cx, None, FocusTrigger::Script)
    {
        debug!("accessibility action Focus: the target is not focusable");
        return;
    }
    let scroll_axis = ScrollAxisState {
        position: ScrollLogicalPosition::Center,
        requirement: ScrollRequirement::IfNotVisible,
    };
    element.scroll_into_view_with_options(
        cx,
        ScrollBehavior::Auto,
        scroll_axis,
        scroll_axis,
        None,
        None,
    );
}

/// `Blur`: take the focus away from the element when it has it, as `element.blur()` does.
fn blur(cx: &mut JSContext, element: &Element) {
    if !element.focus_state() {
        return;
    }
    element
        .owner_document()
        .focus_handler()
        .focus(cx, &FocusableArea::Viewport, FocusTrigger::Script);
}

/// `ShowContextMenu`: an untrusted `contextmenu` event at the element. The page's handler sees it
/// as it sees the one from a secondary click; the browser's own menu opens only for a trusted
/// event.
fn show_context_menu(cx: &mut JSContext, element: &Element) {
    element
        .upcast::<Node>()
        .fire_synthetic_pointer_event_not_trusted(cx, Atom::from("contextmenu"));
}

// ---------------------------------------------------------------------------------------------
// Expand and collapse
// ---------------------------------------------------------------------------------------------

/// The `<details>` element whose open state `element` shows: the element itself, or the
/// `<summary>` that is its first summary child.
fn details_for(element: &Element) -> Option<DomRoot<HTMLDetailsElement>> {
    if let Some(details) = element.downcast::<HTMLDetailsElement>() {
        return Some(DomRoot::from_ref(details));
    }
    if element.local_name() != &local_name!("summary") {
        return None;
    }
    let parent = element.upcast::<Node>().GetParentElement()?;
    let details = DomRoot::downcast::<HTMLDetailsElement>(parent)?;
    let summary = details.find_corresponding_summary_element()?;
    (*summary.upcast::<Element>() == *element).then_some(details)
}

/// `Expand` and `Collapse`: open or close what the element controls.
///
/// * `<details>` (or its `<summary>`): the `open` attribute, which fires `toggle`;
/// * `<select>`: the picker, which a click would open;
/// * anything else with `aria-expanded`: a click, when the state differs from the one asked for
///   (a disclosure button, a menu button and a combo box handle the click themselves).
///
/// Nothing happens when the element has none of these, or is in the state asked for already.
fn set_expanded(cx: &mut JSContext, element: &Element, expand: bool) {
    if let Some(details) = details_for(element) {
        if details.Open() != expand {
            details.SetOpen(cx, expand);
        }
        return;
    }

    if let Some(select) = element.downcast::<HTMLSelectElement>() {
        let is_open = element.state().contains(ElementState::OPEN);
        if expand && !is_open {
            let document = element.owner_document();
            if document.is_fully_active() {
                UserActivation::handle_user_activation_notification(&document);
            }
            select.show_menu(cx);
        } else if !expand && is_open {
            element
                .owner_document()
                .embedder_controls()
                .hide_embedder_control(element);
            element.set_open_state(cx.no_gc(), false);
        }
        return;
    }

    let current = element
        .get_nullable_string_attribute(&local_name!("aria-expanded"))
        .and_then(|value| match &*value.str().to_ascii_lowercase() {
            "true" => Some(true),
            "false" => Some(false),
            _ => None,
        });
    match current {
        Some(current) if current != expand => click_with_user_activation(cx, element),
        Some(_) => {},
        None => debug!("accessibility action Expand or Collapse: the target has no expanded state"),
    }
}

// ---------------------------------------------------------------------------------------------
// Values
// ---------------------------------------------------------------------------------------------

/// The text of a number as a person would type it: `5`, not `5.0`.
fn number_to_text(number: f64) -> String {
    format!("{number}")
}

/// `Increment` and `Decrement`: one step.
///
/// * `<input>` with a step (`number`, `range`, the date types): `stepUp()` or `stepDown()`, then
///   the `input` and `change` events a spin button's arrow fires.
/// * An ARIA `slider` or `spinbutton` on another element: `aria-valuenow` moves by one step (see
///   [`aria_next_value`]), then the same two events. No key event is synthesised.
fn step(cx: &mut JSContext, element: &Element, increment: bool) {
    if let Some(input) = element.downcast::<HTMLInputElement>() {
        if !input.is_mutable() {
            debug!("accessibility action Increment or Decrement: the input is read-only");
            return;
        }
        let before = input.Value().str().to_string();
        let result = if increment {
            input.StepUp(cx, 1)
        } else {
            input.StepDown(cx, 1)
        };
        if result.is_ok() && input.Value().str().to_string() != before {
            input
                .fire_value_change_events(cx, ValueChangeEvents::Input | ValueChangeEvents::Change);
        }
        return;
    }

    let is_slider = element
        .get_nullable_string_attribute(&local_name!("role"))
        .is_some_and(|value| {
            matches!(
                value
                    .str()
                    .to_ascii_lowercase()
                    .split_ascii_whitespace()
                    .next(),
                Some("slider" | "spinbutton")
            )
        });
    if !is_slider || attribute_is_true(element, &local_name!("aria-readonly")) {
        debug!("accessibility action Increment or Decrement: the target has no value to step");
        return;
    }
    let minimum = number_attribute(element, &local_name!("aria-valuemin")).unwrap_or(0.0);
    let maximum = number_attribute(element, &local_name!("aria-valuemax")).unwrap_or(100.0);
    // A slider without a value is at the midpoint, as the browsers report it.
    let now = number_attribute(element, &local_name!("aria-valuenow"))
        .unwrap_or_else(|| (minimum + maximum) / 2.0);
    let next = aria_next_value(now, minimum, maximum, increment);
    if next == now {
        return;
    }
    element.set_string_attribute(
        cx,
        &local_name!("aria-valuenow"),
        DOMString::from(number_to_text(next)),
    );
    let target = element.upcast::<EventTarget>();
    target.fire_event_with_params(
        cx,
        atom!("input"),
        EventBubbles::Bubbles,
        EventCancelable::NotCancelable,
        EventComposed::Composed,
    );
    target.fire_bubbling_event(cx, atom!("change"));
}

/// The value one step above or below `now` for an ARIA slider, within `[minimum, maximum]`.
///
/// ARIA has no step: the step is 1 (the default of `<input type=range>`), or a tenth of the range
/// when the range is 1 or smaller (a 0 to 1 slider moves in tenths, not from one end to the other). A range that is empty or inverted does not move.
fn aria_next_value(now: f64, minimum: f64, maximum: f64, increment: bool) -> f64 {
    let range = maximum - minimum;
    if range.is_nan() || range <= 0.0 || now.is_nan() {
        return now;
    }
    let step = if range > 1.0 { 1.0 } else { range / 10.0 };
    let next = if increment { now + step } else { now - step };
    // Round away the noise of binary fractions (0.1 + 0.2).
    let next = (next * 1e9).round() / 1e9;
    next.clamp(minimum, maximum)
}

/// `SetValue`: replace the value of an editable control.
///
/// * A text field or text area: the typing path (see [`edit_text_control`]), with the whole text
///   selected first; a `Value` is typed, a `NumericValue` is typed as its text.
/// * `<input type=range>`: `value`, sanitised by the input type, then `input` and `change`.
///
/// Anything else (a checkbox, a pop-up button, `contenteditable`) has nothing to set.
fn set_value(cx: &mut JSContext, element: &Element, data: Option<ActionData>) {
    let text = match data {
        Some(ActionData::Value(value)) => value.to_string(),
        Some(ActionData::NumericValue(number)) => number_to_text(number),
        _ => {
            debug!("accessibility action SetValue: no value was sent");
            return;
        },
    };

    if let Some(input) = element.downcast::<HTMLInputElement>() &&
        matches!(*input.input_type(), InputType::Range(_))
    {
        if !input.is_mutable() {
            debug!("accessibility action SetValue: the input is read-only");
            return;
        }
        let before = input.Value().str().to_string();
        if input.SetValue(cx, DOMString::from(text)).is_ok() &&
            input.Value().str().to_string() != before
        {
            input
                .fire_value_change_events(cx, ValueChangeEvents::Input | ValueChangeEvents::Change);
        }
        return;
    }

    edit_text_control(cx, element, &text, true);
}

/// `ReplaceSelectedText`: type over the selection of a text field or text area (at the caret when
/// nothing is selected).
fn replace_selected_text(cx: &mut JSContext, element: &Element, data: Option<ActionData>) {
    let Some(ActionData::Value(value)) = data else {
        debug!("accessibility action ReplaceSelectedText: no text was sent");
        return;
    };
    edit_text_control(cx, element, &value, false);
}

/// Type `text` into a text field or text area, over the selection (all of the text when
/// `replace_all`), through the code that real typing runs: `maxlength` cuts it, a single-line
/// field turns line breaks into spaces, the caret moves to the end of what was inserted, and
/// `beforeinput` (which the page may cancel) and `input` fire. A `change` event follows when the
/// value changed, as a browser fires one when an assistive technology sets a value.
///
/// A read-only or disabled control, and an input that is not a text field, are left alone.
fn edit_text_control(cx: &mut JSContext, element: &Element, text: &str, replace_all: bool) {
    let control: &dyn TextControlElement = if let Some(input) =
        element.downcast::<HTMLInputElement>()
    {
        if !input.is_textual_or_password() || matches!(*input.input_type(), InputType::Hidden(_)) {
            debug!("accessibility action SetValue or ReplaceSelectedText: not a text field");
            return;
        }
        input
    } else if let Some(text_area) = element.downcast::<HTMLTextAreaElement>() {
        text_area
    } else {
        debug!("accessibility action SetValue or ReplaceSelectedText: not a text field");
        return;
    };

    if control.read_only_or_disabled() {
        debug!("accessibility action SetValue or ReplaceSelectedText: the control is read-only");
        return;
    }

    let before = control.value_text().str().to_string();
    if replace_all {
        control.select_all();
    }
    let action = if text.is_empty() {
        // Nothing to insert: delete the selection, and only the selection.
        if !control.has_uncollapsed_selection() {
            return;
        }
        EditingAction::Delete
    } else {
        EditingAction::InsertText(text.to_owned())
    };
    if !control.perform_editing_action(cx, action) {
        // The page cancelled `beforeinput`, or the control left the page.
        return;
    }
    if control.value_text().str().to_string() != before {
        queue_change_event(element);
    }
}

/// Queue a `change` event at `element`, on the task source that carries the `input` event the
/// typing path queued just before, so that `input` comes first.
fn queue_change_event(element: &Element) {
    let target = Trusted::new(element.upcast::<EventTarget>());
    element
        .owner_global()
        .task_manager()
        .user_interaction_task_source()
        .queue(task!(accessibility_change_event: move |cx| {
            let target = target.root();
            target.fire_bubbling_event(cx, atom!("change"));
        }));
}

// ---------------------------------------------------------------------------------------------
// Scrolling
// ---------------------------------------------------------------------------------------------

/// The block and inline alignment `ScrollIntoView` asks for: `nearest` on both axes, unless the
/// assistive technology sent a hint.
fn alignment_for_hint(hint: Option<ScrollHint>) -> (ScrollLogicalPosition, ScrollLogicalPosition) {
    use ScrollLogicalPosition::{End, Nearest, Start};
    match hint {
        None => (Nearest, Nearest),
        Some(ScrollHint::TopLeft) => (Start, Start),
        Some(ScrollHint::BottomRight) => (End, End),
        Some(ScrollHint::TopEdge) => (Start, Nearest),
        Some(ScrollHint::BottomEdge) => (End, Nearest),
        Some(ScrollHint::LeftEdge) => (Nearest, Start),
        Some(ScrollHint::RightEdge) => (Nearest, End),
    }
}

/// `ScrollIntoView`: scroll every container between the element and the viewport so that the
/// element is visible, with `{block: "nearest", inline: "nearest"}` (a hint moves the edges).
fn scroll_into_view(cx: &mut JSContext, element: &Element, data: Option<ActionData>) {
    if !element.has_css_layout_box() {
        return;
    }
    let hint = match data {
        Some(ActionData::ScrollHint(hint)) => Some(hint),
        _ => None,
    };
    let (block, inline) = alignment_for_hint(hint);
    element.scroll_into_view_with_options(
        cx,
        ScrollBehavior::Auto,
        ScrollAxisState::new_always_scroll_position(block),
        ScrollAxisState::new_always_scroll_position(inline),
        None,
        None,
    );
}

/// A scroll position kept inside `[0, content - size]` on both axes. `content` and `size` are
/// `(width, height)`.
fn clamp_scroll_position(
    position: LayoutVector2D,
    content: (f32, f32),
    size: (f32, f32),
) -> LayoutVector2D {
    let maximum_x = (content.0 - size.0).max(0.0);
    let maximum_y = (content.1 - size.1).max(0.0);
    LayoutVector2D::new(
        position.x.clamp(0.0, maximum_x),
        position.y.clamp(0.0, maximum_y),
    )
}

/// Scroll `scrolling_box` to `position` (kept inside what it can reach), when that moves it.
fn scroll_box_to(cx: &mut JSContext, scrolling_box: &ScrollingBox, position: LayoutVector2D) {
    let content = scrolling_box.content_size();
    let size = scrolling_box.size(cx.no_gc());
    let position = clamp_scroll_position(
        position,
        (content.width, content.height),
        (size.width, size.height),
    );
    if position != scrolling_box.scroll_position() {
        scrolling_box.scroll_to(cx, position, ScrollBehavior::Auto);
    }
}

/// `ScrollToPoint`: scroll the containers around the element until its top-left corner is at the
/// point, which is in CSS pixels from the top-left corner of the viewport (the origin of the
/// document's tree). The innermost container goes first and what it cannot reach is left to the
/// next one, so a point that is out of reach leaves the element as near as it can get.
fn scroll_to_point(cx: &mut JSContext, element: &Element, data: Option<ActionData>) {
    let Some(ActionData::ScrollToPoint(point)) = data else {
        debug!("accessibility action ScrollToPoint: no point was sent");
        return;
    };
    if !element.has_css_layout_box() || !point.x.is_finite() || !point.y.is_finite() {
        return;
    }
    let (wanted_x, wanted_y) = (point.x as f32, point.y as f32);

    let mut scrolling_box = element.scrolling_box(ScrollContainerQueryFlags::empty());
    for _ in 0..MAX_SCROLLING_BOXES {
        let Some(current_box) = scrolling_box else {
            break;
        };
        let Some(rect) = element.upcast::<Node>().border_box(cx) else {
            break;
        };
        let delta = LayoutVector2D::new(
            rect.origin.x.to_f32_px() - wanted_x,
            rect.origin.y.to_f32_px() - wanted_y,
        );
        if delta.x.abs() < 0.5 && delta.y.abs() < 0.5 {
            break;
        }
        let position = current_box.scroll_position() + delta;
        scroll_box_to(cx, &current_box, position);
        scrolling_box = current_box.parent();
    }
}

/// How far a scrolling action moves: a page is the size of the scrolling box, an item is a line.
/// A missing unit is a page. `size` is `(width, height)`.
fn scroll_delta(action: Action, unit: Option<ScrollUnit>, size: (f32, f32)) -> (f32, f32) {
    let (horizontal, vertical) = match unit {
        Some(ScrollUnit::Item) => (SCROLL_ITEM_STEP, SCROLL_ITEM_STEP),
        Some(ScrollUnit::Page) | None => size,
    };
    match action {
        Action::ScrollUp => (0.0, -vertical),
        Action::ScrollDown => (0.0, vertical),
        Action::ScrollLeft => (-horizontal, 0.0),
        Action::ScrollRight => (horizontal, 0.0),
        _ => (0.0, 0.0),
    }
}

/// `ScrollUp`, `ScrollDown`, `ScrollLeft`, `ScrollRight`: scroll the element's own scrolling box
/// (or the nearest one around it, the viewport at the end) by an item or a page.
fn scroll_by_unit(cx: &mut JSContext, element: &Element, action: Action, data: Option<ActionData>) {
    let unit = match data {
        Some(ActionData::ScrollUnit(unit)) => Some(unit),
        _ => None,
    };
    let Some(scrolling_box) = element.scrolling_box(ScrollContainerQueryFlags::Inclusive) else {
        debug!("accessibility action {action:?}: nothing scrolls around the target");
        return;
    };
    let size = scrolling_box.size(cx.no_gc());
    let (dx, dy) = scroll_delta(action, unit, (size.width, size.height));
    let position = scrolling_box.scroll_position() + LayoutVector2D::new(dx, dy);
    scroll_box_to(cx, &scrolling_box, position);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_hint_moves_the_edges_and_no_hint_is_nearest() {
        assert!(matches!(
            alignment_for_hint(None),
            (
                ScrollLogicalPosition::Nearest,
                ScrollLogicalPosition::Nearest
            )
        ));
        assert!(matches!(
            alignment_for_hint(Some(ScrollHint::TopLeft)),
            (ScrollLogicalPosition::Start, ScrollLogicalPosition::Start)
        ));
        assert!(matches!(
            alignment_for_hint(Some(ScrollHint::BottomRight)),
            (ScrollLogicalPosition::End, ScrollLogicalPosition::End)
        ));
        assert!(matches!(
            alignment_for_hint(Some(ScrollHint::TopEdge)),
            (ScrollLogicalPosition::Start, ScrollLogicalPosition::Nearest)
        ));
        assert!(matches!(
            alignment_for_hint(Some(ScrollHint::RightEdge)),
            (ScrollLogicalPosition::Nearest, ScrollLogicalPosition::End)
        ));
    }

    #[test]
    fn an_aria_slider_steps_by_one_inside_its_range() {
        assert_eq!(aria_next_value(5.0, 0.0, 10.0, true), 6.0);
        assert_eq!(aria_next_value(5.0, 0.0, 10.0, false), 4.0);
        assert_eq!(aria_next_value(10.0, 0.0, 10.0, true), 10.0);
        assert_eq!(aria_next_value(0.0, 0.0, 10.0, false), 0.0);
    }

    #[test]
    fn an_aria_slider_with_a_small_range_steps_by_a_tenth_of_it() {
        let next = aria_next_value(0.5, 0.0, 1.0, true);
        assert!((next - 0.6).abs() < 1e-9, "{next}");
        let next = aria_next_value(0.3, 0.0, 0.5, false);
        assert!((next - 0.25).abs() < 1e-9, "{next}");
    }

    #[test]
    fn an_empty_or_inverted_range_does_not_move() {
        assert_eq!(aria_next_value(3.0, 5.0, 5.0, true), 3.0);
        assert_eq!(aria_next_value(3.0, 5.0, 1.0, true), 3.0);
        assert!(aria_next_value(f64::NAN, 0.0, 10.0, true).is_nan());
    }

    #[test]
    fn a_number_is_typed_without_a_fraction_when_it_has_none() {
        assert_eq!(number_to_text(5.0), "5");
        assert_eq!(number_to_text(-2.5), "-2.5");
        assert_eq!(number_to_text(0.0), "0");
    }

    #[test]
    fn scrolling_moves_by_a_page_or_an_item_and_a_missing_unit_is_a_page() {
        let size = (300.0, 200.0);
        assert_eq!(
            scroll_delta(Action::ScrollDown, Some(ScrollUnit::Page), size),
            (0.0, 200.0)
        );
        assert_eq!(scroll_delta(Action::ScrollUp, None, size), (0.0, -200.0));
        assert_eq!(
            scroll_delta(Action::ScrollRight, Some(ScrollUnit::Item), size),
            (SCROLL_ITEM_STEP, 0.0)
        );
        assert_eq!(scroll_delta(Action::ScrollLeft, None, size), (-300.0, 0.0));
        assert_eq!(scroll_delta(Action::Click, None, size), (0.0, 0.0));
    }

    #[test]
    fn a_scroll_position_stays_inside_the_scrollable_area() {
        let size = (300.0, 200.0);
        let clamped = clamp_scroll_position(LayoutVector2D::new(900.0, -5.0), (1000.0, 800.0), size);
        assert_eq!((clamped.x, clamped.y), (700.0, 0.0));
        // Content smaller than the box: nothing to scroll.
        let clamped = clamp_scroll_position(LayoutVector2D::new(10.0, 10.0), (100.0, 100.0), size);
        assert_eq!((clamped.x, clamped.y), (0.0, 0.0));
    }
}
