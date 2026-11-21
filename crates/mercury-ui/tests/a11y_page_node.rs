// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
#![cfg(feature = "test-support")]
//! The node the page's accessibility tree hangs from. The desktop app marks the one node of the
//! window's tree whose role is `main` and whose label is "Page content" as the graft
//! (`apps/mercury-desktop/src/accessibility.rs`, the vendored Slint backend's `mercury_graft.rs`),
//! and a graft can only be put in one place: if two nodes matched, the second would stay empty
//! (or AccessKit would refuse the tree); if none did, the page would not be read at all. So in
//! every state the main window can be in there is exactly one such node, it has no description
//! of its own, and its rectangle lies inside the window where the page is drawn.

mod common;

use common::a11y::{self, Node, Rect, node_in, reading_order, window_size};
use i_slint_backend_testing::AccessibleRole as Role;
use mercury_ui::MainWindow;
use slint::{ComponentHandle, LogicalSize, ModelRc, SharedString, VecModel};

const WIDTH: f32 = 1180.0;
const HEIGHT: f32 = 760.0;

/// What the backend looks for: these two strings are `PAGE_ROLE` and `PAGE_LABEL` of the desktop
/// app's `accessibility.rs`. Changing one of them changes both places and this test.
const PAGE_LABEL: &str = "Page content";

fn window() -> MainWindow {
    a11y::init();
    let window = MainWindow::new().unwrap();
    a11y::open_window(&window, WIDTH, HEIGHT);
    window
}

fn strings(items: &[&str]) -> ModelRc<SharedString> {
    ModelRc::new(VecModel::from(
        items
            .iter()
            .map(|item| SharedString::from(*item))
            .collect::<Vec<_>>(),
    ))
}

/// A page is shown (Servo's texture is up), with its address in the toolbar.
fn show_page(window: &MainWindow) {
    window.set_is_new_tab(false);
    window.set_web_view_visible(true);
    window.set_address("https://example.com/docs".into());
    window.set_committed_address("https://example.com/docs".into());
    window.set_page_title("Docs".into());
}

/// The one page node of `window`, after checking everything the graft relies on.
fn page_node(window: &MainWindow, state: &str) -> Node {
    let nodes = reading_order(window);
    let matching: Vec<&Node> = nodes
        .iter()
        .filter(|node| node.role == Role::Main && node.label == PAGE_LABEL)
        .collect();
    assert_eq!(
        matching.len(),
        1,
        "{state}: expected exactly one main node labelled {PAGE_LABEL:?}, found {}:\n{}",
        matching.len(),
        a11y::dump(&nodes)
    );
    // No other `main` landmark either: a screen reader's "next main region" must land on the page.
    let mains = nodes.iter().filter(|node| node.role == Role::Main).count();
    assert_eq!(mains, 1, "{state}: {mains} main nodes");
    let page = matching[0].clone();
    assert_eq!(
        page.description, "",
        "{state}: the page node's description is the page's own now"
    );

    let (width, height) = window_size(window);
    let rect = page.rect();
    assert!(
        rect.width > 0.0 && rect.height > 0.0,
        "{state}: the page node has no area ({} x {})",
        rect.width,
        rect.height
    );
    assert!(
        Rect::new(0.0, 0.0, width, height).contains(&rect),
        "{state}: the page node lies outside the {width} x {height} window: x {}, y {}, {} x {}",
        rect.x,
        rect.y,
        rect.width,
        rect.height
    );
    page
}

/// The page node is where the window says the page is.
fn assert_covers_page_area(window: &MainWindow, page: &Node, state: &str) {
    let area = Rect::new(
        window.get_controls_page_x(),
        window.get_controls_page_y(),
        window.get_web_view_width(),
        window.get_web_view_height(),
    );
    assert!(
        page.rect().same_as(&area),
        "{state}: the page node is {:?}, the page is drawn at {area:?}",
        page.rect()
    );
}

#[test]
fn there_is_one_page_node_while_a_new_tab_is_open() {
    let window = window();
    let page = page_node(&window, "new tab");
    assert_covers_page_area(&window, &page, "new tab");
}

#[test]
fn there_is_one_page_node_while_a_page_is_shown() {
    let window = window();
    show_page(&window);
    let page = page_node(&window, "page");
    assert_covers_page_area(&window, &page, "page");

    window.set_is_loading(true);
    window.set_can_go_back(true);
    window.set_zoom_percent(125);
    let page = page_node(&window, "page loading, zoomed");
    assert_covers_page_area(&window, &page, "page loading, zoomed");

    window.set_is_loading(false);
    window.set_zoom_percent(100);
    page_node(&window, "page loaded");
}

#[test]
fn there_is_one_page_node_while_the_toolbar_is_hidden_or_has_a_home_button() {
    let window = window();
    show_page(&window);
    window.set_show_home_button(true);
    page_node(&window, "home button");
    window.set_chrome_hidden(true);
    let page = page_node(&window, "toolbar hidden");
    // Fullscreen: the page node covers the whole window.
    assert!(page.rect().same_as(&Rect::new(0.0, 0.0, WIDTH, HEIGHT)));
    window.set_chrome_hidden(false);
    page_node(&window, "toolbar back");
}

#[test]
fn there_is_one_page_node_under_every_bar_and_notice() {
    let window = window();
    show_page(&window);

    window.set_find_open(true);
    window.set_find_query("slint".into());
    page_node(&window, "find bar");
    window.set_find_open(false);

    window.set_download_offer_text("example.com wants to download a file.".into());
    window.set_download_offer_visible(true);
    let page = page_node(&window, "download offer");
    assert_covers_page_area(&window, &page, "download offer");
    window.set_download_offer_visible(false);

    window.set_password_save_problem("Couldn't save the login for example.com.".into());
    let page = page_node(&window, "password notice");
    assert_covers_page_area(&window, &page, "password notice");
    window.set_password_save_problem("".into());

    window.set_settings_notice("The settings file could not be read.".into());
    page_node(&window, "settings notice");
    window.set_settings_notice("".into());

    window.set_session_notice("Your tabs were restored.".into());
    page_node(&window, "session notice");
    window.set_session_notice("".into());

    window.set_recovery_sessions(strings(&[
        "Today, 10:42 (5 tabs)",
        "Yesterday, 18:03 (2 tabs)",
    ]));
    let page = page_node(&window, "session chooser");
    assert_covers_page_area(&window, &page, "session chooser");
    window.set_recovery_sessions(ModelRc::default());

    window.set_downloads_button_visible(true);
    window.set_downloads_panel_open(true);
    page_node(&window, "downloads panel");
    window.set_downloads_panel_open(false);

    // Back to the plain page: nothing left behind.
    let page = page_node(&window, "plain page again");
    assert_covers_page_area(&window, &page, "plain page again");
}

#[test]
fn there_is_one_page_node_under_dialogs_and_open_menus() {
    let window = window();
    show_page(&window);

    window.set_clear_dialog_open(true);
    page_node(&window, "clear browsing data dialog");
    window.set_clear_dialog_open(false);

    window.set_quit_prompt_title("Cancel 2 downloads and quit?".into());
    window.set_quit_prompt_body("report.pdf and 1 more are still downloading.".into());
    window.set_quit_prompt_confirm_label("Cancel downloads and quit".into());
    window.set_quit_prompt_keep_label("Keep downloading".into());
    window.set_quit_prompt_open(true);
    page_node(&window, "quit prompt");
    window.set_quit_prompt_open(false);

    // The browser menu is a popup; its content is part of the window's tree too.
    common::a11y::control_in(&reading_order(&window), Role::Button, "Browser menu")
        .invoke_accessible_default_action();
    node_in(&reading_order(&window), Role::List, "Browser menu");
    page_node(&window, "browser menu open");
}

#[test]
fn there_is_one_page_node_at_other_window_sizes() {
    let window = window();
    show_page(&window);
    for (width, height) in [(900.0, 600.0), (1600.0, 900.0), (640.0, 480.0)] {
        window.window().set_size(LogicalSize::new(width, height));
        let state = format!("window {width} x {height}");
        let page = page_node(&window, &state);
        assert_covers_page_area(&window, &page, &state);
    }
}

#[test]
fn the_page_node_keeps_its_place_when_the_window_changes_state() {
    // A graft is a node of the window's tree: tab switches and navigation re-fill the window's
    // properties constantly, and the node must stay the same one (one match, same rectangle).
    let window = window();
    show_page(&window);
    let before = page_node(&window, "before").rect();
    for round in 0..5 {
        window.set_is_new_tab(round % 2 == 0);
        window.set_web_view_visible(round % 2 == 1);
        window.set_address(format!("https://example.com/{round}").into());
        let state = format!("round {round}");
        page_node(&window, &state);
    }
    show_page(&window);
    assert!(page_node(&window, "after").rect().same_as(&before));
}
