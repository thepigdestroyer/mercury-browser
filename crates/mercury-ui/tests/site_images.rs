// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
#![cfg(feature = "test-support")]
//! The "Images" row of the site-information popover (images per site): absent while the active
//! tab shows no web page, otherwise the words "Shown" or "Blocked" and one button, with the
//! accessible name and role the accessibility rules ask for, and a callback the application
//! answers. Read through Slint's testing backend (`common/a11y.rs`), like `a11y_page_ui.rs`.
//!
//! The row is driven through three properties of the window that sit beside `SiteInfo`, not
//! through a field of it (two other test files build a `SiteInfo` as a literal).

mod common;

use std::cell::Cell;
use std::rc::Rc;

use common::a11y::{
    self, Node, assert_bounds, assert_inside, assert_labelled, control, dump, node_in, subtree,
    window_size,
};
use i_slint_backend_testing::AccessibleRole as Role;
use mercury_ui::{
    BrowserTab, MainWindow, SiteInfo, SitePermissionRow, SiteSecurityIcon, TabAudioState,
};
use slint::{Image, ModelRc, VecModel};

const WIDTH: f32 = 1000.0;
const HEIGHT: f32 = 700.0;

const BLOCK_BUTTON: &str = "Block images on this site";
const SHOW_BUTTON: &str = "Show images on this site";

/// A window showing a loaded page in one tab, with the site-information popover open for it.
/// The images properties are set before the popover opens, as the application does (it fills
/// them when the indicator asks for the popover, which then opens at the size they give).
fn window_with_popover(images_available: bool, images_blocked: bool) -> MainWindow {
    a11y::init();
    let window = MainWindow::new().expect("the main window can be created");
    window.set_tabs(ModelRc::new(VecModel::from(vec![BrowserTab {
        title: "Example".into(),
        icon: Image::default(),
        has_icon: false,
        active: true,
        loading: false,
        id: 1,
        pinned: false,
        url: "https://example.com/".into(),
        audio: TabAudioState::Silent,
        chip_before: 0.0,
    }])));
    window.set_is_new_tab(false);
    window.set_web_view_visible(true);
    window.set_address("https://example.com/".into());
    window.set_security_icon(SiteSecurityIcon::Padlock);
    window.set_security_label("Connection is secure".into());
    window.set_site_info(SiteInfo {
        host: "example.com".into(),
        icon: SiteSecurityIcon::Padlock,
        heading: "Connection is secure".into(),
        detail: "Information you send to this site cannot be read by others on the network.".into(),
        permissions: ModelRc::new(VecModel::from(vec![SitePermissionRow {
            name: "Store data permanently on this device".into(),
            state: "Allowed".into(),
        }])),
        site_data: "3 cookies, local storage".into(),
        can_clear_site_data: true,
        https_exception: false,
        certificate_exception: false,
    });
    window.set_site_images_available(images_available);
    window.set_site_images_blocked(images_blocked);
    a11y::open_window(&window, WIDTH, HEIGHT);
    window.invoke_show_site_info();
    window
}

/// The popover and everything in it, in reading order.
fn popover(window: &MainWindow) -> Vec<Node> {
    subtree(&control(window, Role::Groupbox, "Site information"))
}

/// The nodes that belong to the images row: the "Images" title, the state, the button.
fn images_nodes(nodes: &[Node]) -> Vec<&Node> {
    nodes
        .iter()
        .filter(|node| {
            node.label == "Images"
                || node.label == "Shown"
                || node.label == "Blocked"
                || node.label.to_lowercase().contains("images")
        })
        .collect()
}

#[test]
fn the_row_is_absent_while_images_cannot_be_set() {
    let window = window_with_popover(false, false);

    // The default: nothing about images, so every existing reading order is as it was.
    assert!(!window.get_site_images_available());
    let nodes = popover(&window);
    assert!(
        images_nodes(&nodes).is_empty(),
        "an images row in the popover:\n{}",
        dump(&nodes)
    );

    // Whatever `blocked` says, the row is not there while it is unavailable.
    window.set_site_images_blocked(true);
    let nodes = popover(&window);
    assert!(
        images_nodes(&nodes).is_empty(),
        "an images row for a blocked site that has no web page:\n{}",
        dump(&nodes)
    );
    let separate_nodes = nodes.len();

    // It appears with the property and goes with it: a title, the state and the button.
    window.set_site_images_available(true);
    let with_row = popover(&window);
    assert_eq!(with_row.len(), separate_nodes + 3, "{}", dump(&with_row));
    window.set_site_images_available(false);
    assert_eq!(popover(&window).len(), separate_nodes);
}

#[test]
fn the_row_says_shown_or_blocked_and_offers_the_other() {
    let window = window_with_popover(true, false);

    // Images on: "Shown", and the button blocks them.
    window.set_site_images_blocked(false);
    let nodes = popover(&window);
    node_in(&nodes, Role::Text, "Images");
    node_in(&nodes, Role::Text, "Shown");
    let button = node_in(&nodes, Role::Button, BLOCK_BUTTON);
    assert!(button.enabled);
    assert!(
        !nodes
            .iter()
            .any(|node| node.label == "Blocked" || node.label == SHOW_BUTTON),
        "{}",
        dump(&nodes)
    );

    // Images off: "Blocked", and the button shows them again.
    window.set_site_images_blocked(true);
    let nodes = popover(&window);
    node_in(&nodes, Role::Text, "Images");
    node_in(&nodes, Role::Text, "Blocked");
    node_in(&nodes, Role::Button, SHOW_BUTTON);
    assert!(
        !nodes
            .iter()
            .any(|node| node.label == "Shown" || node.label == BLOCK_BUTTON),
        "{}",
        dump(&nodes)
    );
}

#[test]
fn the_button_calls_back_and_leaves_the_state_to_the_application() {
    let window = window_with_popover(true, false);
    let toggled = Rc::new(Cell::new(0));
    let counter = Rc::clone(&toggled);
    window.on_site_images_toggle(move || counter.set(counter.get() + 1));

    node_in(&popover(&window), Role::Button, BLOCK_BUTTON).press();
    assert_eq!(toggled.get(), 1);
    // The row does not flip itself: the application decides, then sets the property.
    assert!(!window.get_site_images_blocked());
    node_in(&popover(&window), Role::Text, "Shown");

    window.set_site_images_blocked(true);
    node_in(&popover(&window), Role::Button, SHOW_BUTTON).press();
    assert_eq!(toggled.get(), 2);
    assert!(window.get_site_images_blocked());
}

#[test]
fn the_row_follows_the_labelling_rule_and_lies_inside_the_popover() {
    for (blocked, button) in [(false, BLOCK_BUTTON), (true, SHOW_BUTTON)] {
        let window = window_with_popover(true, blocked);
        let nodes = popover(&window);
        assert_labelled(&nodes, "the popover with its images row");

        // The button has the role and a name that says what it does and to what; the caption
        // inside it ("Block", "Show") is not read a second time.
        let node = node_in(&nodes, Role::Button, button);
        assert_eq!(node.role_name(), "button");
        assert!(
            !nodes.iter().any(|other| other.role == Role::Text
                && (other.label == "Block" || other.label == "Show")),
            "the caption is read as well:\n{}",
            dump(&nodes)
        );

        // Everything is laid out: inside the popover, inside the window, with an area.
        assert_bounds(&nodes, window_size(&window));
        for node in &nodes[1..] {
            assert_inside(node, &nodes[0]);
        }
        // The row comes after the cookies row, as the last block of the popover.
        let clear = nodes
            .iter()
            .position(|node| node.label == "Clear cookies and site data")
            .expect("the clear button");
        let title = nodes
            .iter()
            .position(|node| node.label == "Images")
            .expect("the title");
        assert!(title > clear, "{}", dump(&nodes));
    }
}

#[test]
fn the_reading_order_of_the_row() {
    let window = window_with_popover(true, true);
    let nodes = popover(&window);

    let row: Vec<(&str, &str)> = nodes
        .iter()
        .skip_while(|node| node.label != "Images")
        .map(|node| (node.role_name(), node.label.as_str()))
        .collect();

    assert_eq!(
        row,
        [
            ("text", "Images"),
            ("text", "Blocked"),
            ("button", SHOW_BUTTON)
        ]
    );
}
