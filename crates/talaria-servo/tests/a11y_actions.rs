// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! The actions an assistive technology sends to a web page (a11y-1, item 5, EE5).
//!
//! Each test loads `tests/pages/a11y/actions-controls.html` in a headless Servo through the
//! harness of item 3 (`tests/common/a11y_harness.rs`), sends one accessibility action to a control
//! as VoiceOver would (`A11yPage::act`) and checks the effect on the page's DOM through
//! `A11yPage::eval`. The engine side is `third_party/servo-script/dom/document/accessibility_actions.rs`.
//!
//! **Every test that starts Servo is named `headless_servo_<what>` and is `#[ignore]`d**: it needs
//! a GL context, so it runs where Servo can render (`make test-a11y-pages`, the macOS CI job) and
//! never on a machine whose screen is locked. The one test without Servo checks that these tests
//! cover every action `mercury-a11y` advertises.
//!
//! Which action each test covers: `Click` (`click_*`, `a_disabled_*`), `Focus`, `Blur`,
//! `SetValue`, `ReplaceSelectedText`, `Increment`, `Decrement`, `Expand`, `Collapse`,
//! `ScrollIntoView`, `ScrollToPoint`, `ScrollUp`, `ScrollDown`, `ScrollLeft`, `ScrollRight` and
//! `ShowContextMenu`. A negative test (a disabled control, a read-only one, a node that has been
//! removed) ends with a barrier: an action whose effect is seen, sent after the one that must do
//! nothing. Actions are handled in order, so once the barrier has run the other has been handled.

mod common;

use accesskit::{Action, ActionData, NodeId, Point, Role, ScrollUnit};
use common::a11y_harness::A11yPage;

/// The page, as a string: the harness loads it as a `data:` URL.
const CONTROLS: &str = include_str!("../../../tests/pages/a11y/actions-controls.html");

/// Every action the tests of this file press. `advertised_actions_are_the_ones_these_tests_cover`
/// compares it with `mercury_a11y::actions::SUPPORTED_ACTIONS`.
const COVERED_ACTIONS: &[Action] = &[
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

fn load() -> A11yPage {
    A11yPage::load(CONTROLS)
}

fn js_string(page: &A11yPage, script: &str) -> String {
    match page.eval(script) {
        serde_json::Value::String(text) => text,
        other => other.to_string(),
    }
}

fn js_bool(page: &A11yPage, script: &str) -> bool {
    page.eval(script).as_bool().unwrap_or(false)
}

fn js_number(page: &A11yPage, script: &str) -> f64 {
    page.eval(script).as_f64().unwrap_or(f64::NAN)
}

/// The events the page recorded, as `type:id` in order, joined by spaces.
fn log(page: &A11yPage) -> String {
    js_string(page, "window.log.join(' ')")
}

fn log_count(page: &A11yPage, entry: &str) -> usize {
    log(page)
        .split(' ')
        .filter(|recorded| *recorded == entry)
        .count()
}

/// The position of `entry` in the event log, to compare the order of two events.
fn log_position(page: &A11yPage, entry: &str) -> Option<usize> {
    log(page).split(' ').position(|recorded| recorded == entry)
}

/// Every node whose accessible name is `name`, with its role.
fn nodes_named(page: &A11yPage, name: &str) -> Vec<(NodeId, Role)> {
    fn walk(node: accesskit_consumer::Node<'_>, name: &str, found: &mut Vec<(NodeId, Role)>) {
        if node.label().as_deref() == Some(name) {
            found.push((node.locate().0, node.role()));
        }
        for child in node.children() {
            walk(child, name, found);
        }
    }
    let mut found = Vec::new();
    walk(page.tree().state().root(), name, &mut found);
    found
}

/// The one node named `name`, whatever its role (the roles of form controls are item 2's to
/// decide; the tests are about what the action does).
fn node_named(page: &A11yPage, name: &str) -> NodeId {
    let found = nodes_named(page, name);
    match found.as_slice() {
        [(id, _)] => *id,
        _ => panic!("expected exactly one node named {name:?}, found {found:?}"),
    }
}

fn value(text: &str) -> Option<ActionData> {
    Some(ActionData::Value(text.into()))
}

/// Send `ShowContextMenu` to the context target and wait until the page has seen it. It has no
/// side effect on the page and does not count as user activation, so it is the barrier of the
/// tests that check what did *not* happen.
fn barrier(page: &mut A11yPage) {
    let before = log_count(page, "contextmenu:context");
    let target = node_named(page, "Context target");
    page.act(target, Action::ShowContextMenu, None);
    page.spin_until(|page| log_count(page, "contextmenu:context") > before);
}

#[test]
fn advertised_actions_are_the_ones_these_tests_cover() {
    for advertised in mercury_a11y::actions::SUPPORTED_ACTIONS {
        assert!(
            COVERED_ACTIONS.contains(advertised),
            "{advertised:?} is advertised by mercury-a11y but no test of a11y_actions.rs presses it"
        );
    }
    for covered in COVERED_ACTIONS {
        assert!(
            mercury_a11y::actions::SUPPORTED_ACTIONS.contains(covered),
            "{covered:?} is pressed by a11y_actions.rs but mercury-a11y no longer advertises it"
        );
    }
}

// ---------------------------------------------------------------------------------------------
// Click: the default action, and user activation (D6)
// ---------------------------------------------------------------------------------------------

#[test]
#[ignore = "starts a headless Servo: needs a GL context"]
fn headless_servo_click_presses_a_button_and_counts_as_user_activation() {
    let mut page = load();
    assert!(
        !js_bool(&page, "navigator.userActivation.hasBeenActive"),
        "a freshly loaded page has had no user activation"
    );

    let button = page.find(Role::Button, "Press me");
    page.act(button, Action::Click, None);
    page.spin_until(|page| js_number(page, "window.pressed") == 1.0);

    assert!(
        js_bool(&page, "window.activeDuringClick"),
        "the click handler ran with transient activation, so that a popup or fullscreen request in it works (OD-5)"
    );
    assert!(js_bool(&page, "navigator.userActivation.hasBeenActive"));
}

#[test]
#[ignore = "starts a headless Servo: needs a GL context"]
fn headless_servo_click_follows_a_link_and_toggles_a_checkbox() {
    let mut page = load();

    let link = page.find(Role::Link, "Go to target");
    page.act(link, Action::Click, None);
    page.spin_until(|page| js_string(page, "location.hash") == "#target");

    let checkbox = node_named(&page, "Subscribe");
    assert!(!js_bool(&page, "document.getElementById('check').checked"));
    page.act(checkbox, Action::Click, None);
    page.spin_until(|page| js_bool(page, "document.getElementById('check').checked"));
}

#[test]
#[ignore = "starts a headless Servo: needs a GL context"]
fn headless_servo_a_disabled_control_is_left_alone() {
    let mut page = load();

    let disabled_button = page.find(Role::Button, "Disabled button");
    page.act(disabled_button, Action::Click, None);
    let disabled_field = node_named(&page, "Off");
    page.act(disabled_field, Action::SetValue, value("changed"));
    page.act(disabled_field, Action::Focus, None);
    barrier(&mut page);

    assert_eq!(log_count(&page, "click:disabled-button"), 0);
    assert_eq!(
        js_string(&page, "document.getElementById('off').value"),
        "unchanged"
    );
    assert_ne!(js_string(&page, "document.activeElement.id"), "off");
    assert!(
        !js_bool(&page, "navigator.userActivation.hasBeenActive"),
        "a click that did nothing is no user activation"
    );
}

#[test]
#[ignore = "starts a headless Servo: needs a GL context"]
fn headless_servo_a_detached_node_is_left_alone() {
    let mut page = load();

    let doomed = page.find(Role::Button, "Soon gone");
    // The node leaves the DOM before the tree has been told: the action reaches layout while the
    // tree still has it, and the script side must see that it is no longer connected.
    page.eval("document.getElementById('gone').remove(); 1");
    page.act(doomed, Action::Click, None);
    barrier(&mut page);

    assert_eq!(log_count(&page, "click:gone"), 0);
    assert!(!js_bool(&page, "navigator.userActivation.hasBeenActive"));
}

// ---------------------------------------------------------------------------------------------
// Focus and blur
// ---------------------------------------------------------------------------------------------

#[test]
#[ignore = "starts a headless Servo: needs a GL context"]
fn headless_servo_focus_focuses_a_field_without_user_activation() {
    let mut page = load();

    let name = node_named(&page, "Name");
    page.act(name, Action::Focus, None);
    page.spin_until(|page| js_string(page, "document.activeElement.id") == "name");

    assert!(
        log(&page).contains("focus:name"),
        "focus event: {}",
        log(&page)
    );
    assert!(
        !js_bool(&page, "navigator.userActivation.hasBeenActive"),
        "Focus is not a click: it must not grant user activation (D6)"
    );
}

#[test]
#[ignore = "starts a headless Servo: needs a GL context"]
fn headless_servo_blur_takes_the_focus_away() {
    let mut page = load();
    let name = node_named(&page, "Name");
    page.act(name, Action::Focus, None);
    page.spin_until(|page| js_string(page, "document.activeElement.id") == "name");

    page.act(name, Action::Blur, None);
    page.spin_until(|page| js_string(page, "document.activeElement.id") != "name");

    assert!(
        log(&page).contains("blur:name"),
        "blur event: {}",
        log(&page)
    );
}

// ---------------------------------------------------------------------------------------------
// Values
// ---------------------------------------------------------------------------------------------

#[test]
#[ignore = "starts a headless Servo: needs a GL context"]
fn headless_servo_set_value_types_into_a_field_and_fires_input_then_change() {
    let mut page = load();

    let name = node_named(&page, "Name");
    page.act(name, Action::SetValue, value("Grace Hopper"));
    page.spin_until(|page| log_position(page, "change:name").is_some());

    assert_eq!(
        js_string(&page, "document.getElementById('name').value"),
        "Grace Hopper"
    );
    let input = log_position(&page, "input:name").expect("an input event");
    let change = log_position(&page, "change:name").expect("a change event");
    assert!(input < change, "input before change: {}", log(&page));
    assert_eq!(
        js_number(&page, "document.getElementById('name').selectionStart"),
        "Grace Hopper".len() as f64,
        "the caret is at the end of the new value"
    );
    assert!(
        !js_bool(&page, "navigator.userActivation.hasBeenActive"),
        "SetValue is not a click"
    );
}

#[test]
#[ignore = "starts a headless Servo: needs a GL context"]
fn headless_servo_set_value_on_a_single_line_field_turns_line_breaks_into_spaces() {
    let mut page = load();

    let name = node_named(&page, "Name");
    page.act(name, Action::SetValue, value("first\nsecond"));
    page.spin_until(|page| log_position(page, "change:name").is_some());

    assert_eq!(
        js_string(&page, "document.getElementById('name').value"),
        "first second"
    );
}

#[test]
#[ignore = "starts a headless Servo: needs a GL context"]
fn headless_servo_set_value_honours_maxlength() {
    let mut page = load();

    let short = node_named(&page, "Short");
    page.act(short, Action::SetValue, value("abcdefghij"));
    page.spin_until(|page| log_position(page, "change:short").is_some());

    assert_eq!(
        js_string(&page, "document.getElementById('short').value"),
        "abcde"
    );
}

#[test]
#[ignore = "starts a headless Servo: needs a GL context"]
fn headless_servo_set_value_leaves_a_read_only_field_alone() {
    let mut page = load();

    let locked = node_named(&page, "Locked");
    page.act(locked, Action::SetValue, value("changed"));
    page.act(locked, Action::ReplaceSelectedText, value("changed"));
    barrier(&mut page);

    assert_eq!(
        js_string(&page, "document.getElementById('locked').value"),
        "fixed"
    );
    assert_eq!(log_count(&page, "input:locked"), 0);
    assert_eq!(log_count(&page, "change:locked"), 0);
}

#[test]
#[ignore = "starts a headless Servo: needs a GL context"]
fn headless_servo_replace_selected_text_replaces_only_the_selection() {
    let mut page = load();
    page.eval("document.getElementById('name').setSelectionRange(0, 3); 1");

    let name = node_named(&page, "Name");
    page.act(name, Action::ReplaceSelectedText, value("Dr"));
    page.spin_until(|page| log_position(page, "change:name").is_some());

    assert_eq!(
        js_string(&page, "document.getElementById('name').value"),
        "Dr Lovelace"
    );
    assert_eq!(
        js_number(&page, "document.getElementById('name').selectionStart"),
        2.0,
        "the caret is after the inserted text"
    );
}

#[test]
#[ignore = "starts a headless Servo: needs a GL context"]
fn headless_servo_set_value_fills_a_text_area_and_a_slider() {
    let mut page = load();

    let notes = node_named(&page, "Notes");
    page.act(notes, Action::SetValue, value("line one\nline two"));
    page.spin_until(|page| log_position(page, "change:notes").is_some());
    assert_eq!(
        js_string(&page, "document.getElementById('notes').value"),
        "line one\nline two"
    );

    let volume = node_named(&page, "Volume");
    page.act(
        volume,
        Action::SetValue,
        Some(ActionData::NumericValue(8.0)),
    );
    page.spin_until(|page| log_position(page, "change:volume").is_some());
    assert_eq!(
        js_string(&page, "document.getElementById('volume').value"),
        "8"
    );
    let input = log_position(&page, "input:volume").expect("an input event");
    let change = log_position(&page, "change:volume").expect("a change event");
    assert!(input < change, "input before change: {}", log(&page));
}

#[test]
#[ignore = "starts a headless Servo: needs a GL context"]
fn headless_servo_increment_and_decrement_step_a_slider_a_spin_button_and_an_aria_slider() {
    let mut page = load();

    let volume = node_named(&page, "Volume");
    page.act(volume, Action::Increment, None);
    page.spin_until(|page| js_string(page, "document.getElementById('volume').value") == "6");
    page.act(volume, Action::Decrement, None);
    page.act(volume, Action::Decrement, None);
    page.spin_until(|page| js_string(page, "document.getElementById('volume').value") == "4");
    assert!(
        log(&page).contains("input:volume"),
        "events: {}",
        log(&page)
    );
    assert!(
        log(&page).contains("change:volume"),
        "events: {}",
        log(&page)
    );

    let count = node_named(&page, "Count");
    page.act(count, Action::Increment, None);
    page.spin_until(|page| js_string(page, "document.getElementById('count').value") == "2");

    let brightness = node_named(&page, "Brightness");
    page.act(brightness, Action::Increment, None);
    page.spin_until(|page| {
        js_string(
            page,
            "document.getElementById('brightness').getAttribute('aria-valuenow')",
        ) == "4"
    });
    page.act(brightness, Action::Decrement, None);
    page.spin_until(|page| {
        js_string(
            page,
            "document.getElementById('brightness').getAttribute('aria-valuenow')",
        ) == "3"
    });
    assert!(
        !js_bool(&page, "navigator.userActivation.hasBeenActive"),
        "stepping a value is not a click"
    );
}

// ---------------------------------------------------------------------------------------------
// Expand and collapse
// ---------------------------------------------------------------------------------------------

#[test]
#[ignore = "starts a headless Servo: needs a GL context"]
fn headless_servo_expand_and_collapse_open_and_close_a_details_element() {
    let mut page = load();

    let summary = node_named(&page, "More");
    page.act(summary, Action::Expand, None);
    page.spin_until(|page| js_bool(page, "document.getElementById('more').open"));
    page.spin_until(|page| log_position(page, "toggle:more").is_some());

    page.act(summary, Action::Collapse, None);
    page.spin_until(|page| !js_bool(page, "document.getElementById('more').open"));
    assert!(
        !js_bool(&page, "navigator.userActivation.hasBeenActive"),
        "opening a details element is not a click"
    );
}

#[test]
#[ignore = "starts a headless Servo: needs a GL context"]
fn headless_servo_expand_and_collapse_click_a_button_that_has_an_expanded_state() {
    let mut page = load();
    let menu = page.find(Role::Button, "Menu");

    page.act(menu, Action::Expand, None);
    page.spin_until(|page| {
        js_string(
            page,
            "document.getElementById('menu').getAttribute('aria-expanded')",
        ) == "true"
    });
    assert!(
        js_bool(&page, "navigator.userActivation.hasBeenActive"),
        "Expand that clicks counts as user activation (D6)"
    );

    // Already expanded: nothing to do, and no second click.
    let clicks = log_count(&page, "click:menu");
    page.act(menu, Action::Expand, None);
    barrier(&mut page);
    assert_eq!(log_count(&page, "click:menu"), clicks);

    page.act(menu, Action::Collapse, None);
    page.spin_until(|page| {
        js_string(
            page,
            "document.getElementById('menu').getAttribute('aria-expanded')",
        ) == "false"
    });
}

// ---------------------------------------------------------------------------------------------
// Scrolling
// ---------------------------------------------------------------------------------------------

#[test]
#[ignore = "starts a headless Servo: needs a GL context"]
fn headless_servo_scroll_into_view_scrolls_the_page_and_a_scroll_container() {
    let mut page = load();
    assert_eq!(js_number(&page, "window.scrollY"), 0.0);

    let far = page.find(Role::Heading, "Far heading");
    page.act(far, Action::ScrollIntoView, None);
    page.spin_until(|page| js_number(page, "window.scrollY") > 0.0);

    let deep = page.find(Role::Heading, "Deep heading");
    page.act(deep, Action::ScrollIntoView, None);
    page.spin_until(|page| js_number(page, "document.getElementById('scroller').scrollTop") > 0.0);
}

#[test]
#[ignore = "starts a headless Servo: needs a GL context"]
fn headless_servo_scroll_actions_move_a_scroll_container_by_an_item_and_a_page() {
    let mut page = load();
    let scroller = node_named(&page, "Scroller");
    let top = |page: &A11yPage| js_number(page, "document.getElementById('scroller').scrollTop");
    let left = |page: &A11yPage| js_number(page, "document.getElementById('scroller').scrollLeft");

    page.act(
        scroller,
        Action::ScrollDown,
        Some(ActionData::ScrollUnit(ScrollUnit::Item)),
    );
    page.spin_until(|page| top(page) == 40.0);

    let before = top(&page);
    page.act(
        scroller,
        Action::ScrollDown,
        Some(ActionData::ScrollUnit(ScrollUnit::Page)),
    );
    page.spin_until(|page| top(page) > before + 40.0);
    let page_height = js_number(&page, "document.getElementById('scroller').clientHeight");
    assert!(
        (top(&page) - before - page_height).abs() <= 1.0,
        "a page is the height of the scroll container: moved {}, height {page_height}",
        top(&page) - before
    );

    page.act(
        scroller,
        Action::ScrollUp,
        Some(ActionData::ScrollUnit(ScrollUnit::Page)),
    );
    page.spin_until(|page| top(page) == 0.0);

    page.act(
        scroller,
        Action::ScrollRight,
        Some(ActionData::ScrollUnit(ScrollUnit::Item)),
    );
    page.spin_until(|page| left(page) == 40.0);
    page.act(
        scroller,
        Action::ScrollLeft,
        Some(ActionData::ScrollUnit(ScrollUnit::Item)),
    );
    page.spin_until(|page| left(page) == 0.0);
}

#[test]
#[ignore = "starts a headless Servo: needs a GL context"]
fn headless_servo_scroll_to_point_puts_the_element_at_the_point() {
    let mut page = load();

    let far = page.find(Role::Heading, "Far heading");
    page.act(
        far,
        Action::ScrollToPoint,
        Some(ActionData::ScrollToPoint(Point { x: 0.0, y: 50.0 })),
    );
    page.spin_until(|page| {
        let top = js_number(
            page,
            "document.getElementById('far').getBoundingClientRect().top",
        );
        (top - 50.0).abs() <= 1.0
    });
}

// ---------------------------------------------------------------------------------------------
// Context menu
// ---------------------------------------------------------------------------------------------

#[test]
#[ignore = "starts a headless Servo: needs a GL context"]
fn headless_servo_show_context_menu_fires_a_contextmenu_event() {
    let mut page = load();

    let target = node_named(&page, "Context target");
    page.act(target, Action::ShowContextMenu, None);
    page.spin_until(|page| log_position(page, "contextmenu:context").is_some());

    assert!(
        !js_bool(&page, "navigator.userActivation.hasBeenActive"),
        "a context menu request is not a click"
    );
}
