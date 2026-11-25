// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
#![cfg(feature = "test-support")]
//! The main chrome follows `ChromeMetrics`: a size changed through the generated setters moves
//! the rows, buttons and tabs that were converted. Pixel parity cannot prove that a literal was
//! replaced (the defaults are the old literals); this can. The sizes are set directly on the
//! global, with no Rust push, so a regression in either half shows up here.

mod common;

use common::a11y::{self, Rect, node_in, reading_order};
use i_slint_backend_testing::AccessibleRole as Role;
use mercury_ui::{BrowserTab, ChromeMetrics, MainWindow};
use slint::{ComponentHandle, ModelRc, VecModel};

const WIDTH: f32 = 1180.0;
const HEIGHT: f32 = 760.0;

fn window() -> MainWindow {
    a11y::init();
    let window = MainWindow::new().unwrap();
    window.set_show_home_button(true);
    a11y::open_window(&window, WIDTH, HEIGHT);
    window
}

fn sizes(window: &MainWindow) -> ChromeMetrics<'_> {
    window.global::<ChromeMetrics>()
}

fn rect(window: &MainWindow, role: Role, label: &str) -> Rect {
    node_in(&reading_order(window), role, label).rect()
}

#[test]
fn the_rows_follow_their_heights() {
    let window = window();
    let roomy_toolbar = rect(&window, Role::Navigation, "Toolbar");
    assert_eq!(roomy_toolbar, Rect::new(0.0, 48.0, WIDTH, 60.0));

    sizes(&window).set_tab_row_height(40.0);
    sizes(&window).set_toolbar_height(52.0);
    sizes(&window).set_address_height(30.0);

    let toolbar = rect(&window, Role::Navigation, "Toolbar");
    assert_eq!(
        toolbar,
        Rect::new(0.0, 40.0, WIDTH, 52.0),
        "the toolbar starts under a 40 px tab row and is 52 px high"
    );
    let address = rect(&window, Role::Search, "Address bar");
    assert_eq!(address.height, 30.0, "the address field is 30 px high");
    assert!(toolbar.contains(&address));
}

#[test]
fn the_tab_row_follows_its_padding_and_spacing() {
    let window = window();
    let roomy = rect(&window, Role::Button, "New tab");
    assert_eq!(roomy, Rect::new(WIDTH - 14.0 - 34.0, 7.0, 34.0, 34.0));

    sizes(&window).set_tab_row_padding_right(30.0);
    sizes(&window).set_tab_row_padding_top(12.0);
    sizes(&window).set_new_tab_button_size(40.0);

    let button = rect(&window, Role::Button, "New tab");
    assert_eq!(button.width, 40.0);
    assert_eq!(button.height, 40.0);
    assert_eq!(
        button.right(),
        WIDTH - 30.0,
        "30 px of padding on the right"
    );
    assert_eq!(button.y, 12.0, "12 px of padding above");
}

#[test]
fn the_toolbar_follows_its_padding_spacing_and_buttons() {
    let window = window();
    let back = rect(&window, Role::Button, "Back");
    assert_eq!(back, Rect::new(14.0, 59.0, 36.0, 34.0));

    sizes(&window).set_toolbar_padding_x(20.0);
    sizes(&window).set_toolbar_spacing(12.0);
    sizes(&window).set_button_width(40.0);
    sizes(&window).set_button_height(32.0);

    let back = rect(&window, Role::Button, "Back");
    let forward = rect(&window, Role::Button, "Forward");
    assert_eq!(back.x, 20.0, "20 px of padding on the left");
    assert_eq!((back.width, back.height), (40.0, 32.0));
    assert_eq!(forward.x, back.right() + 12.0, "12 px between the buttons");
    let menu = rect(&window, Role::Button, "Browser menu");
    assert_eq!(menu.right(), WIDTH - 20.0, "20 px of padding on the right");

    sizes(&window).set_toolbar_padding_y(8.0);
    let back = rect(&window, Role::Button, "Back");
    assert_eq!(back.y, 48.0 + 8.0, "8 px of padding above the buttons");
}

#[test]
fn the_bars_follow_their_heights() {
    let window = window();
    window.set_is_new_tab(false);
    window.set_web_view_visible(true);
    let page_y = window.get_controls_page_y();
    let page_height = window.get_web_view_height();

    sizes(&window).set_bookmarks_bar_height(50.0);
    assert_eq!(
        window.get_controls_page_y(),
        page_y + 16.0,
        "the bookmarks bar is 50 px high, not 34"
    );

    window.set_find_open(true);
    let with_roomy_find = window.get_web_view_height();
    assert_eq!(
        with_roomy_find,
        page_height - 16.0 - 42.0,
        "the find bar takes 42 px"
    );
    sizes(&window).set_find_bar_height(60.0);
    assert_eq!(window.get_web_view_height(), with_roomy_find - 18.0);
}

/// Two pinned tabs and three others, so the strip has both kinds of slot.
fn with_tabs(window: &MainWindow) {
    let tab = |id: i32, title: &str, pinned: bool| BrowserTab {
        title: title.into(),
        id,
        pinned,
        active: id == 3,
        ..Default::default()
    };
    let tabs = vec![
        tab(1, "Mail", true),
        tab(2, "Calendar", true),
        tab(3, "Docs", false),
        tab(4, "News", false),
        tab(5, "Chat", false),
    ];
    window.set_tabs(ModelRc::new(VecModel::from(tabs)));
    window.set_pinned_tab_count(2);
    window.set_active_tab_index(2);
    window.set_is_new_tab(false);
}

#[test]
fn the_tabs_follow_their_widths_and_spacing() {
    let window = window();
    with_tabs(&window);
    let tab = |title: &str| rect(&window, Role::Tab, title);

    // The strip starts 10 px in; a pinned tab is 40 px wide, the others at most 184, 6 px apart.
    assert_eq!(tab("Mail").x, 10.0);
    assert_eq!(tab("Mail").width, 40.0);
    assert_eq!(tab("Calendar").x, 10.0 + 40.0 + 6.0);
    assert_eq!(tab("Docs").x, 10.0 + 2.0 * 46.0);
    assert_eq!(tab("Docs").width, 184.0);
    assert_eq!(tab("News").x, tab("Docs").right() + 6.0);

    sizes(&window).set_tab_pinned_width(41.0);
    sizes(&window).set_tab_spacing(7.0);
    sizes(&window).set_tab_max_width(150.0);

    assert_eq!(tab("Mail").width, 41.0, "a pinned tab is 41 px wide");
    assert_eq!(tab("Calendar").x, 10.0 + 41.0 + 7.0, "tabs are 7 px apart");
    assert_eq!(tab("Docs").width, 150.0, "no tab is wider than 150 px");
    assert_eq!(tab("News").x, tab("Docs").right() + 7.0);
}

#[test]
fn a_crowded_strip_follows_the_minimum_width_and_the_list_button() {
    let window = window();
    with_tabs(&window);
    a11y::open_window(&window, 400.0, HEIGHT);
    let tab = |title: &str| rect(&window, Role::Tab, title);
    let list = || rect(&window, Role::Button, "Show all tabs");

    assert_eq!(tab("Docs").width, 112.0, "the narrowest tab is 112 px");
    assert_eq!(list().width, 34.0);

    sizes(&window).set_tab_min_width(120.0);
    sizes(&window).set_tab_list_button_width(36.0);

    assert_eq!(tab("Docs").width, 120.0, "the narrowest tab is 120 px");
    assert_eq!(list().width, 36.0, "the all-tabs button is 36 px wide");
}

#[test]
fn the_tab_row_follows_its_left_and_bottom_padding() {
    let window = window();
    with_tabs(&window);
    let mail = || rect(&window, Role::Tab, "Mail");
    assert_eq!((mail().x, mail().y, mail().height), (10.0, 7.0, 38.0));

    sizes(&window).set_tab_row_padding_left(25.0);
    assert_eq!(mail().x, 25.0, "25 px of padding left of the strip");

    sizes(&window).set_tab_row_padding_bottom(10.0);
    assert_eq!(
        (mail().y, mail().height),
        (7.0, 31.0),
        "10 px of padding under the tabs leaves them 31 px high"
    );
}

#[test]
fn the_tab_row_follows_the_space_between_the_strip_and_the_new_tab_button() {
    let window = window();
    with_tabs(&window);
    // A crowded strip fills its width, so its "all tabs" button is the strip's right end.
    a11y::open_window(&window, 400.0, HEIGHT);
    let gap = || {
        let new_tab = rect(&window, Role::Button, "New tab");
        let list = rect(&window, Role::Button, "Show all tabs");
        new_tab.x - list.right()
    };
    assert_eq!(gap(), 8.0);

    sizes(&window).set_tab_row_spacing(20.0);
    assert_eq!(gap(), 20.0, "20 px between the strip and the button");
}
