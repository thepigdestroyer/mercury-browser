// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
#![cfg(feature = "test-support")]
//! What an assistive technology (VoiceOver) is given for everything a web page can cause: the
//! page's context menu, `<select>` lists, the colour picker, JavaScript dialogs, the leave
//! prompt, HTTP sign-in, permission prompts, the pop-up-blocked notice, the camera and
//! microphone bubble, and (browser chrome, but about a page) the HTTPS-only interstitial and
//! the site-information popover.
//!
//! Each state is driven on a `MainWindow` through its public properties, read through Slint's
//! testing backend (`common/a11y.rs`), and checked for: the control found by role and label,
//! its default action (what "press" does for VoiceOver) with the effect asserted, the labelling
//! rule, the rectangle VoiceOver is given, and a golden file in `tests/a11y-snapshots/`
//! (`page-ui-<state>.txt`) read as a VoiceOver user would. The rule for this area: whatever a
//! page causes names its origin the way it does on screen, so that a page cannot pass for
//! browser UI.

mod common;

use std::cell::RefCell;
use std::rc::Rc;

use common::a11y::{
    self, Node, Rect, assert_bounds, assert_inside, assert_labelled, assert_rect, assert_snapshot,
    control, dump, node_in, reading_order, subtree, window_size,
};
use i_slint_backend_testing::AccessibleRole as Role;
use mercury_ui::{
    BrowserTab, CaptureView, ContextMenuRow, MainWindow, PageRequestStyle, PageRequestView,
    SelectPopupRow, SiteInfo, SitePermissionRow, SiteSecurityIcon, TabAudioState,
};
use slint::{Color, Image, ModelRc, VecModel};

const WIDTH: f32 = 1000.0;
const HEIGHT: f32 = 700.0;

/// What a callback was called with, in order.
type Log<T> = Rc<RefCell<Vec<T>>>;

fn log<T>() -> Log<T> {
    Rc::new(RefCell::new(Vec::new()))
}

/// A window showing a loaded page in one tab, the state every overlay below appears over.
fn page_window() -> MainWindow {
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
        group: 0,
        group_name: "".into(),
        group_colour: Default::default(),
        group_tint: Default::default(),
        group_deep: Default::default(),
        group_first: false,
        group_last: false,
        group_key: "".into(),
        group_run_start: 0,
    }])));
    window.set_is_new_tab(false);
    window.set_web_view_visible(true);
    window.set_address("https://example.com/".into());
    a11y::open_window(&window, WIDTH, HEIGHT);
    window
}

/// The control with this role and label, and everything below it, in reading order.
fn group(window: &MainWindow, role: Role, label: &str) -> Vec<Node> {
    subtree(&control(window, role, label))
}

/// Every node below the first lies inside it and in the window, and has an area.
fn assert_laid_out(nodes: &[Node], window: &MainWindow) {
    assert_bounds(nodes, window_size(window));
    for node in &nodes[1..] {
        assert_inside(node, &nodes[0]);
    }
}

/// The text names `origin`: what the page's name is for.
fn assert_names_origin(reading: &str, origin: &str, what: &str) {
    assert!(
        reading.contains(origin),
        "{what} does not name its origin {origin:?}:\n{reading}"
    );
}

// ---------------------------------------------------------------------------------------------
// The page's context menu
// ---------------------------------------------------------------------------------------------

const POPUP_SELECT: i32 = 1;
const POPUP_COLOR: i32 = 2;
const POPUP_MENU: i32 = 3;

fn place(window: &MainWindow, kind: i32, rect: Rect) {
    window.set_controls_popup_x(rect.x);
    window.set_controls_popup_y(rect.y);
    window.set_controls_popup_width(rect.width);
    window.set_controls_popup_height(rect.height);
    window.set_controls_popup(kind);
}

/// Six items of 26 px and two separators of 9 px, 4 px above and below.
const MENU: Rect = Rect {
    x: 300.0,
    y: 200.0,
    width: 220.0,
    height: 6.0 * 26.0 + 2.0 * 9.0 + 8.0,
};

fn menu_item(label: &str, enabled: bool) -> ContextMenuRow {
    ContextMenuRow {
        label: label.into(),
        enabled,
        separator: false,
    }
}

fn menu_separator() -> ContextMenuRow {
    ContextMenuRow {
        label: "".into(),
        enabled: false,
        separator: true,
    }
}

fn open_context_menu(window: &MainWindow) {
    window.set_controls_menu_rows(ModelRc::new(VecModel::from(vec![
        menu_item("Back", true),
        menu_item("Forward", false),
        menu_item("Reload", true),
        menu_separator(),
        menu_item("Copy", true),
        menu_item("Select all", true),
        menu_separator(),
        menu_item("Save page as…", true),
    ])));
    window.set_controls_highlighted(0);
    place(window, POPUP_MENU, MENU);
}

#[test]
fn the_context_menu_is_a_list_of_items_without_its_separators() {
    let window = page_window();
    let clicked: Log<i32> = log();
    let record = clicked.clone();
    window.on_controls_row_clicked(move |row| record.borrow_mut().push(row));
    open_context_menu(&window);

    let nodes = group(&window, Role::List, "Page context menu");
    assert_labelled(&nodes, "the context menu");
    let labels: Vec<&str> = nodes.iter().map(|node| node.label.as_str()).collect();
    assert_eq!(
        labels,
        [
            "Page context menu",
            "Back",
            "Forward",
            "Reload",
            "Copy",
            "Select all",
            "Save page as…"
        ],
        "six items and no separator:\n{}",
        dump(&nodes)
    );
    assert!(nodes[1..].iter().all(|n| n.role == Role::ListItem));

    // The position of an item counts the separators (a loop cannot skip them): 8 rows.
    let positions: Vec<Option<(usize, usize)>> = nodes[1..].iter().map(|n| n.item).collect();
    assert_eq!(
        positions,
        [0, 1, 2, 4, 5, 7].map(|index| Some((index, 8))),
        "{}",
        dump(&nodes)
    );

    // Enabled state and the highlighted row, which is the selected one.
    assert!(!node_in(&nodes, Role::ListItem, "Forward").enabled);
    assert!(node_in(&nodes, Role::ListItem, "Back").enabled);
    assert_eq!(node_in(&nodes, Role::ListItem, "Back").selected, Some(true));
    assert_eq!(
        node_in(&nodes, Role::ListItem, "Copy").selected,
        Some(false)
    );
    window.set_controls_highlighted(2);
    assert_eq!(
        control(&window, Role::ListItem, "Reload").accessible_item_selected(),
        Some(true)
    );
    assert_eq!(
        control(&window, Role::ListItem, "Back").accessible_item_selected(),
        Some(false)
    );
    window.set_controls_highlighted(0);

    // Pressing an item does what a click on it does, by its place in the model; a disabled
    // item does nothing.
    node_in(&nodes, Role::ListItem, "Reload").press();
    node_in(&nodes, Role::ListItem, "Copy").press();
    node_in(&nodes, Role::ListItem, "Save page as…").press();
    assert_eq!(*clicked.borrow(), [2, 4, 7]);
    let forward = control(&window, Role::ListItem, "Forward");
    assert_eq!(forward.accessible_enabled(), Some(false));
    forward.invoke_accessible_default_action();
    assert_eq!(
        *clicked.borrow(),
        [2, 4, 7],
        "a disabled item is not pressed"
    );

    // Bounds: the menu is where the test put it, and each item is the row that is drawn
    // (26 px, 4 px in from each side, after the 4 px above and the 9 px separator).
    assert_rect(&nodes[0], MENU);
    assert_laid_out(&nodes, &window);
    assert_rect(
        &nodes[1],
        Rect::new(MENU.x + 4.0, MENU.y + 4.0, MENU.width - 8.0, 26.0),
    );
    assert_rect(
        &nodes[4],
        Rect::new(
            MENU.x + 4.0,
            MENU.y + 4.0 + 3.0 * 26.0 + 9.0,
            MENU.width - 8.0,
            26.0,
        ),
    );

    assert_snapshot("page-ui-context-menu", &nodes);
}

// ---------------------------------------------------------------------------------------------
// <select> lists
// ---------------------------------------------------------------------------------------------

/// The 20 px caption with the origin, then six rows of 24 px; a multi-select adds 40 px.
const SELECT: Rect = Rect {
    x: 200.0,
    y: 150.0,
    width: 240.0,
    height: 20.0 + 6.0 * 24.0,
};
const SELECT_MULTIPLE: Rect = Rect {
    height: SELECT.height + 40.0,
    ..SELECT
};

fn select_row(label: &str, heading: bool, disabled: bool, selected: bool) -> SelectPopupRow {
    SelectPopupRow {
        label: label.into(),
        heading,
        disabled,
        selected,
        in_group: !heading && label != "Anything else",
    }
}

fn open_select(window: &MainWindow, origin: &str, multiple: bool) {
    window.set_controls_origin(origin.into());
    window.set_controls_select_rows(ModelRc::new(VecModel::from(vec![
        select_row("Fruit", true, false, false),
        select_row("Apple", false, false, true),
        select_row("Banana", false, true, false),
        select_row("Vegetables", true, false, false),
        select_row("Carrot", false, false, multiple),
        select_row("Anything else", false, false, false),
    ])));
    window.set_controls_select_multiple(multiple);
    window.set_controls_highlighted(1);
    place(
        window,
        POPUP_SELECT,
        if multiple { SELECT_MULTIPLE } else { SELECT },
    );
}

#[test]
fn a_select_list_names_its_page_and_reads_as_options() {
    let window = page_window();
    let clicked: Log<i32> = log();
    let record = clicked.clone();
    window.on_controls_row_clicked(move |row| record.borrow_mut().push(row));
    open_select(&window, "shop.example", false);

    let nodes = group(&window, Role::List, "Options from shop.example");
    assert_labelled(&nodes, "a single select list");
    let labels: Vec<(&str, Role)> = nodes[1..]
        .iter()
        .map(|node| (node.label.as_str(), node.role))
        .collect();
    assert_eq!(
        labels,
        [
            ("Fruit", Role::Text),
            ("Apple", Role::ListItem),
            ("Banana", Role::ListItem),
            ("Vegetables", Role::Text),
            ("Carrot", Role::ListItem),
            ("Anything else", Role::ListItem),
        ],
        "option groups are headings, options are items:\n{}",
        dump(&nodes)
    );
    // No Done button in a single select, and no hint about choosing several.
    assert!(nodes.iter().all(|node| node.label != "Done"));
    assert_eq!(nodes[0].description, "");

    // Selected state, position among all rows (headings count) and the disabled option.
    let apple = node_in(&nodes, Role::ListItem, "Apple");
    assert_eq!(apple.selected, Some(true));
    assert_eq!(apple.item, Some((1, 6)));
    assert_eq!(
        node_in(&nodes, Role::ListItem, "Carrot").selected,
        Some(false)
    );
    assert_eq!(
        node_in(&nodes, Role::ListItem, "Anything else").item,
        Some((5, 6))
    );
    assert!(!node_in(&nodes, Role::ListItem, "Banana").enabled);

    // Pressing an option is a click on its row; a disabled option does nothing.
    node_in(&nodes, Role::ListItem, "Carrot").press();
    node_in(&nodes, Role::ListItem, "Apple").press();
    node_in(&nodes, Role::ListItem, "Banana").press();
    assert_eq!(*clicked.borrow(), [4, 1], "Banana is disabled");

    // Bounds: where the page's list was placed; each row is 24 px under the 20 px caption.
    assert_rect(&nodes[0], SELECT);
    assert_laid_out(&nodes, &window);
    let apple_rect = apple.rect();
    assert!((apple_rect.x - SELECT.x).abs() < 0.01, "{apple_rect:?}");
    assert!(
        (apple_rect.y - (SELECT.y + 20.0 + 24.0)).abs() < 0.01,
        "{apple_rect:?}"
    );
    assert!((apple_rect.height - 24.0).abs() < 0.01, "{apple_rect:?}");

    assert_snapshot("page-ui-select-single", &nodes);
}

#[test]
fn a_multi_select_list_has_a_done_button_and_says_several_can_be_chosen() {
    let window = page_window();
    let done: Log<()> = log();
    let record = done.clone();
    window.on_controls_select_done(move || record.borrow_mut().push(()));
    open_select(&window, "shop.example", true);

    let nodes = group(&window, Role::List, "Options from shop.example");
    assert_labelled(&nodes, "a multiple select list");
    assert_eq!(
        nodes[0].description,
        "Several options can be chosen. Choose Done when finished."
    );
    // Two options are chosen (Apple and Carrot).
    assert_eq!(
        node_in(&nodes, Role::ListItem, "Carrot").selected,
        Some(true)
    );
    assert_eq!(
        node_in(&nodes, Role::ListItem, "Apple").selected,
        Some(true)
    );

    let button = node_in(&nodes, Role::Button, "Done");
    button.press();
    assert_eq!(done.borrow().len(), 1);
    // The button is inside the list's footer, the 40 px under the rows.
    assert_rect(&nodes[0], SELECT_MULTIPLE);
    assert_laid_out(&nodes, &window);
    assert!(
        button.rect().y >= SELECT.bottom() - 0.01,
        "{:?}",
        button.rect()
    );
    // Done is 72 x 28 px, 8 px in from the right, 6 px under the footer's top edge.
    assert_rect(
        button,
        Rect::new(
            SELECT.x + 240.0 - 8.0 - 72.0,
            SELECT.y + 164.0 + 6.0,
            72.0,
            28.0,
        ),
    );

    assert_snapshot("page-ui-select-multiple", &nodes);
}

#[test]
fn a_select_without_an_origin_is_still_a_labelled_list() {
    let window = page_window();
    open_select(&window, "", false);
    let nodes = group(&window, Role::List, "Options");
    assert_labelled(&nodes, "a select list with no origin");
}

// ---------------------------------------------------------------------------------------------
// The colour picker
// ---------------------------------------------------------------------------------------------

const PICKER: Rect = Rect {
    x: 250.0,
    y: 120.0,
    width: 232.0,
    height: 282.0,
};

/// The picker's sixteen swatches, as the desktop application lists them.
const SWATCHES: [(u8, u8, u8); 16] = [
    (0x00, 0x00, 0x00),
    (0x55, 0x55, 0x55),
    (0xaa, 0xaa, 0xaa),
    (0xff, 0xff, 0xff),
    (0xe5, 0x39, 0x35),
    (0xfb, 0x8c, 0x00),
    (0xfd, 0xd8, 0x35),
    (0x43, 0xa0, 0x47),
    (0x00, 0x89, 0x7b),
    (0x00, 0xac, 0xc1),
    (0x1e, 0x88, 0xe5),
    (0x39, 0x49, 0xab),
    (0x8e, 0x24, 0xaa),
    (0xd8, 0x1b, 0x60),
    (0x6d, 0x4c, 0x41),
    (0x54, 0x6e, 0x7a),
];

fn hex_of((red, green, blue): (u8, u8, u8)) -> String {
    format!("#{red:02x}{green:02x}{blue:02x}")
}

fn open_color_picker(window: &MainWindow, origin: &str) {
    window.set_controls_origin(origin.into());
    window.set_controls_color_swatches(ModelRc::new(VecModel::from(
        SWATCHES
            .iter()
            .map(|(r, g, b)| Color::from_rgb_u8(*r, *g, *b))
            .collect::<Vec<_>>(),
    )));
    window.set_controls_color(Color::from_rgb_u8(0x1e, 0x88, 0xe5));
    window.set_controls_color_hex("#1e88e5".into());
    window.set_controls_color_hex_valid(true);
    window.set_controls_color_hue(0.5);
    window.set_controls_color_lightness(0.5);
    place(window, POPUP_COLOR, PICKER);
}

fn near(a: f32, b: f32) -> bool {
    (a - b).abs() < 0.001
}

#[test]
fn the_colour_picker_has_named_swatches_and_buttons() {
    let window = page_window();
    let swatches: Log<i32> = log();
    let record = swatches.clone();
    window.on_controls_color_swatch_picked(move |index| record.borrow_mut().push(index));
    let done: Log<()> = log();
    let record = done.clone();
    window.on_controls_color_done(move || record.borrow_mut().push(()));
    let dismissed: Log<()> = log();
    let record = dismissed.clone();
    window.on_controls_dismissed(move || record.borrow_mut().push(()));
    open_color_picker(&window, "paint.example");

    let nodes = group(&window, Role::Groupbox, "Colour picker from paint.example");
    assert_labelled(&nodes, "the colour picker");

    // Sixteen swatches, each named by its value in the form the hex field uses, in order.
    let swatch_nodes: Vec<&Node> = nodes
        .iter()
        .filter(|node| node.role == Role::Button && node.label.starts_with('#'))
        .collect();
    assert_eq!(swatch_nodes.len(), 16, "{}", dump(&nodes));
    for (index, (node, colour)) in swatch_nodes.iter().zip(SWATCHES).enumerate() {
        assert_eq!(node.label, hex_of(colour), "swatch {index}");
        assert_eq!(node.item, Some((index, 16)), "swatch {index}");
    }

    // Pressing a swatch picks it; Cancel is what a click on it does (the picker is dismissed);
    // Done accepts.
    node_in(&nodes, Role::Button, "#43a047").press();
    assert_eq!(*swatches.borrow(), [7]);
    node_in(&nodes, Role::Button, "Cancel").press();
    assert_eq!(dismissed.borrow().len(), 1);
    node_in(&nodes, Role::Button, "Done").press();
    assert_eq!(done.borrow().len(), 1);

    // The colour area is exposed as two sliders (hue, lightness): a drag cannot be made
    // without a pointer. They report their value and pick through the same callback as a drag.
    let picked: Log<(f32, f32)> = log();
    let record = picked.clone();
    window.on_controls_color_area_picked(move |x, y| record.borrow_mut().push((x, y)));
    let hue = node_in(&nodes, Role::Slider, "Hue");
    let lightness = node_in(&nodes, Role::Slider, "Lightness");
    assert_eq!(hue.handle.accessible_value().unwrap(), "180");
    assert_eq!(lightness.handle.accessible_value().unwrap(), "50");
    assert_eq!(hue.handle.accessible_value_maximum(), Some(360.0));
    assert_eq!(lightness.handle.accessible_value_maximum(), Some(100.0));
    hue.handle.invoke_accessible_increment_action();
    hue.handle.invoke_accessible_decrement_action();
    lightness.handle.invoke_accessible_increment_action();
    lightness.handle.invoke_accessible_decrement_action();
    let picked = picked.borrow();
    assert_eq!(picked.len(), 4, "{picked:?}");
    // The area's second coordinate is the fraction from the top: white at 0.
    assert!(
        near(picked[0].0, 0.5 + 15.0 / 360.0) && near(picked[0].1, 0.5),
        "{picked:?}"
    );
    assert!(
        near(picked[1].0, 0.5 - 15.0 / 360.0) && near(picked[1].1, 0.5),
        "{picked:?}"
    );
    assert!(
        near(picked[2].0, 0.5) && near(picked[2].1, 0.45),
        "{picked:?}"
    );
    assert!(
        near(picked[3].0, 0.5) && near(picked[3].1, 0.55),
        "{picked:?}"
    );

    // The hex field is read-only text with its value.
    let hex = node_in(&nodes, Role::Text, "Colour value #1e88e5");
    assert_eq!(hex.description, "");

    // Bounds: the picker is where the page's picker was placed, the swatches are on the
    // 27 px grid 32 px under its top, in two rows of eight.
    assert_rect(&nodes[0], PICKER);
    assert_laid_out(&nodes, &window);
    assert_rect(
        swatch_nodes[0],
        Rect::new(PICKER.x + 10.0, PICKER.y + 32.0, 23.0, 23.0),
    );
    assert_rect(
        swatch_nodes[9],
        Rect::new(PICKER.x + 10.0 + 27.0, PICKER.y + 32.0 + 27.0, 23.0, 23.0),
    );
    // Cancel and Done are 72 x 28 px at the right, 10 px in, 244 px under the top (the buttons
    // set `opacity`, which Slint moves to a parent of its own: the offset must not be counted twice).
    let done_x = PICKER.x + 232.0 - 10.0 - 72.0;
    assert_rect(
        node_in(&nodes, Role::Button, "Done"),
        Rect::new(done_x, PICKER.y + 244.0, 72.0, 28.0),
    );
    assert_rect(
        node_in(&nodes, Role::Button, "Cancel"),
        Rect::new(done_x - 8.0 - 72.0, PICKER.y + 244.0, 72.0, 28.0),
    );

    assert_snapshot("page-ui-color-picker", &nodes);
}

#[test]
fn an_incomplete_hex_value_says_so() {
    let window = page_window();
    open_color_picker(&window, "paint.example");
    window.set_controls_color_hex("#1e8".into());
    window.set_controls_color_hex_valid(false);
    let nodes = group(&window, Role::Groupbox, "Colour picker from paint.example");
    assert_labelled(&nodes, "the colour picker with an incomplete value");
    let hex = node_in(&nodes, Role::Text, "Colour value #1e8");
    assert_eq!(hex.description, "Not a complete colour yet");
}

#[test]
fn a_colour_picker_without_an_origin_is_still_a_labelled_group() {
    let window = page_window();
    open_color_picker(&window, "");
    let nodes = group(&window, Role::Groupbox, "Colour picker");
    assert_labelled(&nodes, "a colour picker with no origin");
}

// ---------------------------------------------------------------------------------------------
// Dialogs, the leave prompt, sign-in and permission prompts
// ---------------------------------------------------------------------------------------------

const BLOCK_DIALOGS: &str = "Don't allow this page to show more dialogs";

type Answer = (i32, bool, String, String, bool);

fn answer(id: i32, accepted: bool, text: &str, password: &str, checked: bool) -> Answer {
    (
        id,
        accepted,
        text.to_string(),
        password.to_string(),
        checked,
    )
}

fn answers(window: &MainWindow) -> Log<Answer> {
    let answers = log();
    let record = answers.clone();
    window.on_page_request_answered(move |id, accepted, text, password, checked| {
        record.borrow_mut().push((
            id,
            accepted,
            text.to_string(),
            password.to_string(),
            checked,
        ));
    });
    answers
}

fn alert(id: i32, origin: &str, message: &str) -> PageRequestView {
    PageRequestView {
        id,
        style: PageRequestStyle::Alert,
        heading: format!("{origin} says").into(),
        message: message.into(),
        accept_label: "OK".into(),
        choice_label: BLOCK_DIALOGS.into(),
        ..PageRequestView::default()
    }
}

fn confirm(id: i32, origin: &str, message: &str) -> PageRequestView {
    PageRequestView {
        style: PageRequestStyle::Confirm,
        decline_label: "Cancel".into(),
        ..alert(id, origin, message)
    }
}

fn prompt(id: i32, origin: &str, message: &str, default: &str) -> PageRequestView {
    PageRequestView {
        style: PageRequestStyle::Prompt,
        value: default.into(),
        decline_label: "Cancel".into(),
        ..alert(id, origin, message)
    }
}

fn leave_page(id: i32, origin: &str) -> PageRequestView {
    PageRequestView {
        id,
        style: PageRequestStyle::LeavePage,
        heading: "Leave this page?".into(),
        message: "Changes you made may not be saved.".into(),
        detail: format!("Asked by {origin}").into(),
        accept_label: "Leave".into(),
        decline_label: "Stay".into(),
        choice_label: "Don't allow this page to ask again".into(),
        ..PageRequestView::default()
    }
}

fn sign_in(id: i32, origin: &str) -> PageRequestView {
    PageRequestView {
        id,
        style: PageRequestStyle::SignIn,
        heading: format!("Sign in to {origin}").into(),
        message: format!("{origin} needs a user name and password.").into(),
        warning: "This connection is not secure (not HTTPS): your user name and password will \
                  be sent unencrypted."
            .into(),
        value: "ann".into(),
        secret: "hunter2-saved".into(),
        accept_label: "Sign in".into(),
        decline_label: "Cancel".into(),
        ..PageRequestView::default()
    }
}

fn permission(id: i32, origin: &str) -> PageRequestView {
    PageRequestView {
        id,
        style: PageRequestStyle::Permission,
        heading: format!("{origin} wants to know your location").into(),
        accept_label: "Allow".into(),
        decline_label: "Block".into(),
        ..PageRequestView::default()
    }
}

/// The camera and microphone prompt, with the choice that lasts until the user quits.
fn capture_permission(id: i32, origin: &str) -> PageRequestView {
    PageRequestView {
        heading: format!("{origin} wants to use your camera and microphone").into(),
        choice_label: "Remember for this site until you quit".into(),
        ..permission(id, origin)
    }
}

/// The same prompt when macOS refuses: why, and a button that opens System Settings.
fn capture_permission_refused(id: i32, origin: &str) -> PageRequestView {
    PageRequestView {
        message: "macOS does not allow Mercury Browser to use the camera. You can allow it in \
                  System Settings, Privacy & Security, Camera."
            .into(),
        choice_label: "".into(),
        accept_label: "Open Privacy Settings".into(),
        decline_label: "Close".into(),
        ..capture_permission(id, origin)
    }
}

/// A sheet's card is centred in the page area, 4 px above its top edge (it overlaps the
/// toolbar), 460 px wide at most; the rectangle VoiceOver gets says so.
fn assert_sheet_card(window: &MainWindow, card: &Node) {
    let width = (window.get_web_view_width() - 32.0).min(460.0);
    let expected = Rect::new(
        window.get_controls_page_x() + (window.get_web_view_width() - width) / 2.0,
        window.get_controls_page_y() - 4.0,
        width,
        card.rect().height,
    );
    assert_rect(card, expected);
    assert!(card.rect().height > 0.0);
}

#[test]
fn an_alert_is_a_group_named_by_who_says_it() {
    let window = page_window();
    let answered = answers(&window);
    window.set_page_request(alert(7, "example.com", "Your session has expired."));

    let nodes = group(&window, Role::Groupbox, "example.com says");
    assert_labelled(&nodes, "an alert");
    assert_eq!(nodes[0].description, "Your session has expired.");
    // The heading and the message are the group's label and description, not read again.
    let roles: Vec<(Role, &str)> = nodes[1..]
        .iter()
        .map(|node| (node.role, node.label.as_str()))
        .collect();
    assert_eq!(
        roles,
        [(Role::Checkbox, BLOCK_DIALOGS), (Role::Button, "OK"),],
        "{}",
        dump(&nodes)
    );

    // The choice is a check box: it starts unchecked, and pressing it toggles it like a click.
    let choice = control(&window, Role::Checkbox, BLOCK_DIALOGS);
    assert_eq!(choice.accessible_checkable(), Some(true));
    assert_eq!(choice.accessible_checked(), Some(false));
    choice.invoke_accessible_default_action();
    assert_eq!(choice.accessible_checked(), Some(true));
    choice.invoke_accessible_default_action();
    assert_eq!(choice.accessible_checked(), Some(false));
    choice.invoke_accessible_default_action();

    // OK answers with the choice.
    node_in(&nodes, Role::Button, "OK").press();
    assert_eq!(*answered.borrow(), [answer(7, true, "", "", true)]);

    assert_laid_out(&nodes, &window);
    assert_sheet_card(&window, &nodes[0]);
    assert_snapshot("page-ui-alert", &nodes);
}

#[test]
fn a_confirm_dialog_has_two_buttons() {
    let window = page_window();
    let answered = answers(&window);
    window.set_page_request(confirm(8, "example.com", "Delete the draft?"));

    let nodes = group(&window, Role::Groupbox, "example.com says");
    assert_labelled(&nodes, "a confirm dialog");
    assert_eq!(nodes[0].description, "Delete the draft?");
    node_in(&nodes, Role::Button, "Cancel").press();
    node_in(&nodes, Role::Button, "OK").press();
    assert_eq!(
        *answered.borrow(),
        [
            answer(8, false, "", "", false),
            answer(8, true, "", "", false),
        ]
    );
    assert_laid_out(&nodes, &window);
    assert_snapshot("page-ui-confirm", &nodes);
}

#[test]
fn a_prompt_has_a_field_that_can_be_read_and_set() {
    let window = page_window();
    let answered = answers(&window);
    window.set_page_request(prompt(9, "example.com", "What is your name?", "Ann"));

    let nodes = group(&window, Role::Groupbox, "example.com says");
    assert_labelled(&nodes, "a prompt");
    assert_eq!(nodes[0].description, "What is your name?");
    // The field has no visible label, so it has a spoken one, and the default value.
    let field = node_in(&nodes, Role::TextInput, "Your answer");
    assert_eq!(field.value, "Ann");
    assert_eq!(field.placeholder(), "");

    // Setting the value from assistive technology types into the field: OK answers with it.
    field.handle.set_accessible_value("Bob");
    assert_eq!(field.handle.accessible_value().unwrap(), "Bob");
    node_in(&nodes, Role::Button, "OK").press();
    assert_eq!(*answered.borrow(), [answer(9, true, "Bob", "", false)]);
    assert_laid_out(&nodes, &window);
    assert_snapshot("page-ui-prompt", &nodes);
}

#[test]
fn the_leave_prompt_names_who_asked() {
    let window = page_window();
    let answered = answers(&window);
    window.set_page_request(leave_page(10, "example.com"));

    let nodes = group(&window, Role::Groupbox, "Leave this page?");
    assert_labelled(&nodes, "the leave prompt");
    // Who asked comes first, then what the prompt says.
    assert_eq!(
        nodes[0].description,
        "Asked by example.com\nChanges you made may not be saved."
    );
    node_in(&nodes, Role::Button, "Stay").press();
    node_in(&nodes, Role::Button, "Leave").press();
    assert_eq!(
        *answered.borrow(),
        [
            answer(10, false, "", "", false),
            answer(10, true, "", "", false),
        ]
    );
    assert_laid_out(&nodes, &window);
    assert_snapshot("page-ui-leave-page", &nodes);
}

#[test]
fn sign_in_has_two_fields_and_never_hands_out_the_password() {
    let window = page_window();
    let answered = answers(&window);
    window.set_page_request(sign_in(11, "example.com"));

    let nodes = group(&window, Role::Groupbox, "Sign in to example.com");
    assert_labelled(&nodes, "the sign-in sheet");
    // The message, then the warning about the connection, in the description.
    assert_eq!(
        nodes[0].description,
        "example.com needs a user name and password.\nThis connection is not secure (not HTTPS): \
         your user name and password will be sent unencrypted."
    );
    let user = node_in(&nodes, Role::TextInput, "User name");
    assert_eq!(user.value, "ann");
    assert_eq!(user.placeholder(), "User name");
    // The password field is filled with the saved password, which assistive technology must
    // never be given.
    let password = node_in(&nodes, Role::TextInput, "Password");
    assert_eq!(password.value, "", "the saved password is not exposed");
    assert_eq!(password.placeholder(), "Password");
    assert!(
        !dump(&nodes).contains("hunter2"),
        "the password is in the reading:\n{}",
        dump(&nodes)
    );

    // Sign in sends the saved values, as a click would.
    node_in(&nodes, Role::Button, "Sign in").press();
    node_in(&nodes, Role::Button, "Cancel").press();
    assert_eq!(
        *answered.borrow(),
        [
            answer(11, true, "ann", "hunter2-saved", false),
            answer(11, false, "ann", "hunter2-saved", false),
        ]
    );
    assert_laid_out(&nodes, &window);
    assert_snapshot("page-ui-sign-in", &nodes);
}

#[test]
fn a_permission_prompt_is_a_group_named_by_the_site_with_block_and_allow() {
    let window = page_window();
    let answered = answers(&window);
    window.set_page_request(permission(12, "example.com"));

    let nodes = group(
        &window,
        Role::Groupbox,
        "example.com wants to know your location",
    );
    assert_labelled(&nodes, "a permission prompt");
    assert_eq!(nodes[0].description, "");
    let labels: Vec<&str> = nodes[1..].iter().map(|n| n.label.as_str()).collect();
    assert_eq!(labels, ["Block", "Allow"], "{}", dump(&nodes));
    node_in(&nodes, Role::Button, "Block").press();
    node_in(&nodes, Role::Button, "Allow").press();
    // The prompt answers through the same callback as a sheet, with no text and no password.
    assert_eq!(
        *answered.borrow(),
        [
            answer(12, false, "", "", false),
            answer(12, true, "", "", false),
        ]
    );
    assert_laid_out(&nodes, &window);
    // It hangs 4 px above the page area's top edge, inside the window.
    assert!((nodes[0].rect().y - (window.get_controls_page_y() - 4.0)).abs() < 0.01);
    assert_snapshot("page-ui-permission", &nodes);
}

#[test]
fn a_camera_prompt_has_a_remember_choice_and_a_refusal_says_why() {
    let window = page_window();
    let answered = answers(&window);
    window.set_page_request(capture_permission(13, "meet.example"));

    let nodes = group(
        &window,
        Role::Groupbox,
        "meet.example wants to use your camera and microphone",
    );
    assert_labelled(&nodes, "the camera and microphone prompt");
    let choice = control(
        &window,
        Role::Checkbox,
        "Remember for this site until you quit",
    );
    assert_eq!(choice.accessible_checked(), Some(false));
    choice.invoke_accessible_default_action();
    assert_eq!(choice.accessible_checked(), Some(true));
    node_in(&nodes, Role::Button, "Allow").press();
    assert_eq!(*answered.borrow(), [answer(13, true, "", "", true)]);
    assert_laid_out(&nodes, &window);
    assert_snapshot("page-ui-permission-camera", &nodes);

    // When macOS refuses, the prompt says why in its description and offers System Settings.
    window.set_page_request(capture_permission_refused(14, "meet.example"));
    let nodes = group(
        &window,
        Role::Groupbox,
        "meet.example wants to use your camera and microphone",
    );
    assert_labelled(&nodes, "the refused camera prompt");
    assert!(
        nodes[0]
            .description
            .starts_with("macOS does not allow Mercury Browser"),
        "{}",
        dump(&nodes)
    );
    node_in(&nodes, Role::Button, "Open Privacy Settings");
    node_in(&nodes, Role::Button, "Close");
    assert!(nodes.iter().all(|node| node.role != Role::Checkbox));
    assert_laid_out(&nodes, &window);
    assert_snapshot("page-ui-permission-camera-refused", &nodes);
}

#[test]
fn a_new_request_replaces_the_group_and_no_sheet_leaves_no_group() {
    let window = page_window();
    assert!(
        reading_order(&window)
            .iter()
            .all(|node| node.label != "example.com says"),
        "no request, no group"
    );
    window.set_page_request(alert(1, "example.com", "First"));
    let first = group(&window, Role::Groupbox, "example.com says");
    assert_eq!(first[0].description, "First");
    window.set_page_request(confirm(2, "example.com", "Second"));
    let second = group(&window, Role::Groupbox, "example.com says");
    assert_eq!(second[0].description, "Second");
    window.set_page_request(PageRequestView::default());
    assert!(
        reading_order(&window)
            .iter()
            .all(|node| node.label != "example.com says"),
        "the group is gone with the request"
    );
}

// ---------------------------------------------------------------------------------------------
// A page cannot pass for the browser
// ---------------------------------------------------------------------------------------------

#[test]
fn the_same_ask_from_two_origins_reads_differently() {
    let window = page_window();
    let message = "Your computer is infected. Choose OK to clean it.";
    let mut readings = Vec::new();
    for origin in ["example.com", "evil.example"] {
        window.set_page_request(alert(1, origin, message));
        let nodes = group(&window, Role::Groupbox, &format!("{origin} says"));
        let reading = dump(&nodes);
        assert_names_origin(&reading, origin, "an alert");
        readings.push(reading);

        window.set_page_request(confirm(2, origin, message));
        readings.push(dump(&group(
            &window,
            Role::Groupbox,
            &format!("{origin} says"),
        )));
        window.set_page_request(prompt(3, origin, message, ""));
        readings.push(dump(&group(
            &window,
            Role::Groupbox,
            &format!("{origin} says"),
        )));
        window.set_page_request(leave_page(4, origin));
        let reading = dump(&group(&window, Role::Groupbox, "Leave this page?"));
        assert_names_origin(&reading, origin, "the leave prompt");
        readings.push(reading);
        window.set_page_request(sign_in(5, origin));
        let reading = dump(&group(
            &window,
            Role::Groupbox,
            &format!("Sign in to {origin}"),
        ));
        assert_names_origin(&reading, origin, "sign-in");
        readings.push(reading);
        window.set_page_request(permission(6, origin));
        let reading = dump(&group(
            &window,
            Role::Groupbox,
            &format!("{origin} wants to know your location"),
        ));
        assert_names_origin(&reading, origin, "a permission prompt");
        readings.push(reading);
    }
    // Six states, two origins: the readings of one state differ between the origins.
    let per_origin = readings.len() / 2;
    for state in 0..per_origin {
        assert_ne!(
            readings[state],
            readings[per_origin + state],
            "state {state} reads the same for two origins:\n{}",
            readings[state]
        );
    }

    // Popups placed over the page name theirs too.
    let mut popups = Vec::new();
    for origin in ["shop.example", "other.example"] {
        window.set_page_request(PageRequestView::default());
        open_select(&window, origin, false);
        let reading = dump(&group(
            &window,
            Role::List,
            &format!("Options from {origin}"),
        ));
        assert_names_origin(&reading, origin, "a select list");
        popups.push(reading);
        open_color_picker(&window, origin);
        let reading = dump(&group(
            &window,
            Role::Groupbox,
            &format!("Colour picker from {origin}"),
        ));
        assert_names_origin(&reading, origin, "the colour picker");
        popups.push(reading);
    }
    assert_ne!(popups[0], popups[2]);
    assert_ne!(popups[1], popups[3]);
}

#[test]
fn page_text_that_imitates_the_browser_stays_inside_the_description() {
    let window = page_window();
    // A page chooses the message of a dialog and the labels of the options in a select. None
    // of it may become the name of a control: the browser's own words label the group.
    let message = "Mercury Browser: your passwords were stolen. Choose OK.";
    window.set_page_request(alert(1, "evil.example", message));
    let nodes = group(&window, Role::Groupbox, "evil.example says");
    assert_eq!(nodes[0].description, message);
    assert!(
        nodes[1..]
            .iter()
            .all(|node| !node.label.contains("Mercury Browser")),
        "page text became a label:\n{}",
        dump(&nodes)
    );
    // Nothing outside the sheet reads it either.
    let window_reading = reading_order(&window);
    assert!(
        window_reading
            .iter()
            .filter(|node| node.description != message)
            .all(|node| !node.label.contains("your passwords were stolen")),
        "the message is read outside the sheet"
    );

    window.set_page_request(PageRequestView::default());
    window.set_controls_origin("evil.example".into());
    window.set_controls_select_rows(ModelRc::new(VecModel::from(vec![select_row(
        "Mercury Browser settings",
        false,
        false,
        false,
    )])));
    window.set_controls_select_multiple(false);
    place(&window, POPUP_SELECT, SELECT);
    let list = group(&window, Role::List, "Options from evil.example");
    // The page's option is an item of a list named for the page, never a control of its own.
    assert_eq!(list[1].role, Role::ListItem);
    assert_eq!(list[1].label, "Mercury Browser settings");
    assert!(list[1].item.is_some());
}

// ---------------------------------------------------------------------------------------------
// The pop-up-blocked notice
// ---------------------------------------------------------------------------------------------

#[test]
fn the_pop_up_blocked_notice_names_the_origin_and_has_two_buttons() {
    let window = page_window();
    let allowed: Log<()> = log();
    let record = allowed.clone();
    window.on_allow_pop_ups(move || record.borrow_mut().push(()));
    let dismissed: Log<()> = log();
    let record = dismissed.clone();
    window.on_dismiss_pop_up_blocked(move || record.borrow_mut().push(()));
    window.set_pop_up_blocked_origin("ads.example".into());

    let nodes = group(&window, Role::Groupbox, "Pop-up blocked");
    assert_labelled(&nodes, "the pop-up-blocked notice");
    let labels: Vec<(Role, &str)> = nodes[1..]
        .iter()
        .map(|node| (node.role, node.label.as_str()))
        .collect();
    assert_eq!(
        labels,
        [
            (Role::Button, "Allow pop-ups from ads.example"),
            (Role::Button, "Dismiss the pop-up notice"),
        ],
        "the cross is a button with a name, not a glyph:\n{}",
        dump(&nodes)
    );

    node_in(&nodes, Role::Button, "Allow pop-ups from ads.example").press();
    assert_eq!((allowed.borrow().len(), dismissed.borrow().len()), (1, 0));
    node_in(&nodes, Role::Button, "Dismiss the pop-up notice").press();
    assert_eq!((allowed.borrow().len(), dismissed.borrow().len()), (1, 1));

    // It is part of the toolbar, and its buttons are inside it.
    assert_laid_out(&nodes, &window);
    let toolbar = reading_order(&window)
        .into_iter()
        .find(|node| node.role == Role::Navigation && node.label == "Toolbar")
        .expect("the toolbar is a landmark");
    assert_inside(&nodes[0], &toolbar);
    assert!((nodes[0].rect().height - 34.0).abs() < 0.01);
    assert!((nodes[2].rect().width - 24.0).abs() < 0.01);

    // Another origin reads differently.
    window.set_pop_up_blocked_origin("tracker.example".into());
    let other = group(&window, Role::Groupbox, "Pop-up blocked");
    assert_ne!(dump(&nodes), dump(&other));
    assert_names_origin(&dump(&other), "tracker.example", "the notice");

    window.set_pop_up_blocked_origin("ads.example".into());
    assert_snapshot("page-ui-pop-up-blocked", &nodes);
}

// ---------------------------------------------------------------------------------------------
// Camera and microphone
// ---------------------------------------------------------------------------------------------

fn capture(camera: bool, microphone: bool, origin: &str, note: &str) -> CaptureView {
    let devices = match (camera, microphone) {
        (true, true) => "camera and microphone",
        (true, false) => "camera",
        _ => "microphone",
    };
    CaptureView {
        camera,
        microphone,
        heading: format!("{origin} can use your {devices}").into(),
        note: note.into(),
    }
}

#[test]
fn the_capture_button_opens_a_bubble_with_close_and_stop() {
    let window = page_window();
    let stops: Log<()> = log();
    let record = stops.clone();
    window.on_capture_stop(move || record.borrow_mut().push(()));
    window.set_capture(capture(true, true, "meet.example", ""));

    // The button is named for the site and what it may use, and says what it does.
    let heading = "meet.example can use your camera and microphone";
    let button = control(&window, Role::Button, heading);
    assert_eq!(
        button.accessible_description().unwrap(),
        "Opens a bubble with Stop"
    );
    // Its glyphs are drawn, not read: nothing below the button has a role.
    let below = subtree(&button);
    assert_eq!(below.len(), 1, "{}", dump(&below));
    let toolbar = reading_order(&window)
        .into_iter()
        .find(|node| node.role == Role::Navigation && node.label == "Toolbar")
        .expect("the toolbar is a landmark");
    assert_inside(&below[0], &toolbar);
    assert_bounds(&below, window_size(&window));
    // Both devices: 52 x 28 px, one device 34 px.
    let size = button.size();
    assert!((size.width - 52.0).abs() < 0.01 && (size.height - 28.0).abs() < 0.01);

    // Pressing it opens the bubble, like a click.
    assert!(!window.get_capture_bubble_open());
    button.invoke_accessible_default_action();
    assert!(window.get_capture_bubble_open());

    let nodes = group(&window, Role::Groupbox, heading);
    assert_labelled(&nodes, "the capture bubble");
    assert_eq!(
        nodes[0].description,
        "Stop takes the access away and reloads the page. It will have to ask again."
    );
    let labels: Vec<&str> = nodes[1..].iter().map(|node| node.label.as_str()).collect();
    assert_eq!(labels, ["Close", "Stop"], "{}", dump(&nodes));
    assert_laid_out(&nodes, &window);
    assert_snapshot("page-ui-capture-bubble", &nodes);

    // Close closes it; Stop stops and closes.
    node_in(&nodes, Role::Button, "Close").press();
    assert!(!window.get_capture_bubble_open());
    assert!(stops.borrow().is_empty());
    control(&window, Role::Button, heading).invoke_accessible_default_action();
    let nodes = group(&window, Role::Groupbox, heading);
    node_in(&nodes, Role::Button, "Stop").press();
    assert_eq!(stops.borrow().len(), 1);
    assert!(!window.get_capture_bubble_open());
}

#[test]
fn the_bubble_describes_why_stop_did_not_work_and_one_device_reads_as_one() {
    let window = page_window();
    window.set_capture(capture(
        false,
        true,
        "talk.example",
        "The page did not reload.",
    ));
    window.set_capture_bubble_open(true);
    let heading = "talk.example can use your microphone";
    let button = control(&window, Role::Button, heading);
    assert!((button.size().width - 34.0).abs() < 0.01);
    let nodes = group(&window, Role::Groupbox, heading);
    assert_labelled(&nodes, "the capture bubble with a note");
    assert_eq!(nodes[0].description, "The page did not reload.");
    assert_names_origin(&dump(&nodes), "talk.example", "the capture bubble");
    assert_snapshot("page-ui-capture-bubble-note", &nodes);
}

#[test]
fn a_tab_whose_page_may_capture_says_so_in_text_not_an_image() {
    let window = page_window();
    window.set_capture_tabs(ModelRc::new(VecModel::from(vec![capture(
        true,
        false,
        "meet.example",
        "",
    )])));
    let nodes = reading_order(&window);
    assert!(
        nodes.iter().all(|node| node.role != Role::Image),
        "an image is exposed:\n{}",
        dump(&nodes)
    );
    let mark = node_in(&nodes, Role::Text, "Can use your camera");
    let tab = node_in(&nodes, Role::Tab, "Example");
    assert_inside(mark, tab);
    assert!(mark.rect().width > 0.0 && mark.rect().height > 0.0);
}

// ---------------------------------------------------------------------------------------------
// Site security
// ---------------------------------------------------------------------------------------------

fn site_info(icon: SiteSecurityIcon, host: &str, heading: &str) -> SiteInfo {
    SiteInfo {
        host: host.into(),
        icon,
        heading: heading.into(),
        detail: "Information you send to this site cannot be read by others on the network.".into(),
        permissions: ModelRc::new(VecModel::from(vec![SitePermissionRow {
            name: "Store data permanently on this device".into(),
            state: "Allowed".into(),
        }])),
        site_data: "3 cookies, local storage".into(),
        can_clear_site_data: true,
        https_exception: false,
        certificate_exception: false,
    }
}

#[test]
fn the_https_only_interstitial_offers_go_back_and_a_named_exception() {
    let window = page_window();
    let events: Log<&'static str> = log();
    let record = events.clone();
    window.on_https_only_go_back(move || record.borrow_mut().push("back"));
    let record = events.clone();
    window.on_https_only_continue(move || record.borrow_mut().push("continue"));
    window.set_https_only_interstitial_shown(true);
    window.set_https_only_interstitial_host("plain.example".into());

    for (tried_https, state) in [(false, "https-only"), (true, "https-only-failed")] {
        window.set_https_only_interstitial_tried_https(tried_https);
        let nodes = group(&window, Role::Groupbox, "HTTPS-only mode");
        assert_labelled(&nodes, state);
        // The heading and the explanation are text (they name the host), then two buttons
        // whose captions are not read twice.
        let reading: Vec<(Role, &str)> = nodes[1..]
            .iter()
            .map(|node| (node.role, node.label.as_str()))
            .collect();
        assert_eq!(reading.len(), 4, "{}", dump(&nodes));
        assert_eq!(reading[0].0, Role::Text);
        assert_eq!(reading[1].0, Role::Text);
        assert!(reading[1].1.contains("plain.example"), "{}", dump(&nodes));
        assert_eq!(
            reading[2..],
            [
                (Role::Button, "Go back"),
                (Role::Button, "Continue to plain.example (not secure)"),
            ]
        );

        // It covers the page area.
        assert_rect(
            &nodes[0],
            Rect::new(
                window.get_controls_page_x(),
                window.get_controls_page_y(),
                window.get_web_view_width(),
                window.get_web_view_height(),
            ),
        );
        assert_laid_out(&nodes, &window);
        assert_snapshot(&format!("page-ui-{state}"), &nodes);
    }

    node_in(
        &group(&window, Role::Groupbox, "HTTPS-only mode"),
        Role::Button,
        "Go back",
    )
    .press();
    node_in(
        &group(&window, Role::Groupbox, "HTTPS-only mode"),
        Role::Button,
        "Continue to plain.example (not secure)",
    )
    .press();
    assert_eq!(*events.borrow(), ["back", "continue"]);
}

#[test]
fn the_site_information_popover_is_a_group_with_named_buttons() {
    let window = page_window();
    let requested: Log<()> = log();
    let record = requested.clone();
    window.on_site_info_requested(move || record.borrow_mut().push(()));
    let events: Log<String> = log();
    let record = events.clone();
    window
        .on_site_permission_reset(move |index| record.borrow_mut().push(format!("reset {index}")));
    let record = events.clone();
    window.on_site_data_clear(move || record.borrow_mut().push("clear".into()));
    let record = events.clone();
    window.on_site_https_exception_remove(move || record.borrow_mut().push("remove".into()));

    window.set_security_icon(SiteSecurityIcon::Padlock);
    window.set_security_label("Connection is secure".into());
    let mut info = site_info(
        SiteSecurityIcon::Padlock,
        "münchen.example",
        "Connection is secure",
    );
    info.https_exception = true;
    window.set_site_info(info);

    // The indicator is in the address bar, and opens the popover from its default action as
    // from a click.
    let order = reading_order(&window);
    let indicator = node_in(
        &order,
        Role::Button,
        "Site information: Connection is secure",
    );
    assert_inside(indicator, node_in(&order, Role::Search, "Address bar"));
    indicator.press();
    assert_eq!(requested.borrow().len(), 1);

    let nodes = group(&window, Role::Groupbox, "Site information");
    assert_labelled(&nodes, "the site-information popover");
    // The host as the address bar shows it comes first; the buttons name what they act on.
    assert_eq!(nodes[1].role, Role::Text);
    assert_eq!(nodes[1].label, "münchen.example");
    for label in [
        "Reset permission: Store data permanently on this device",
        "Clear cookies and site data",
        "Remove the HTTPS-only exception",
    ] {
        node_in(&nodes, Role::Button, label);
    }
    node_in(
        &nodes,
        Role::Button,
        "Reset permission: Store data permanently on this device",
    )
    .press();
    node_in(&nodes, Role::Button, "Clear cookies and site data").press();
    node_in(&nodes, Role::Button, "Remove the HTTPS-only exception").press();
    assert_eq!(*events.borrow(), ["reset 0", "clear", "remove"]);
    assert_laid_out(&nodes, &window);
    assert_snapshot("page-ui-site-info-secure", &nodes);
}

#[test]
fn the_warning_popover_and_a_disabled_clear_button() {
    let window = page_window();
    let cleared: Log<()> = log();
    let record = cleared.clone();
    window.on_site_data_clear(move || record.borrow_mut().push(()));
    window.set_security_icon(SiteSecurityIcon::Warning);
    window.set_security_label("Not secure: you allowed an invalid certificate".into());
    let mut info = site_info(
        SiteSecurityIcon::Warning,
        "self-signed.test",
        "Not secure: you allowed an invalid certificate",
    );
    info.detail = "".into();
    info.permissions = ModelRc::new(VecModel::from(Vec::new()));
    info.site_data = "None".into();
    info.can_clear_site_data = false;
    info.certificate_exception = true;
    info.https_exception = true;
    window.set_site_info(info);
    window.invoke_show_site_info();

    let nodes = group(&window, Role::Groupbox, "Site information");
    assert_labelled(&nodes, "the warning popover");
    assert_names_origin(&dump(&nodes), "self-signed.test", "the popover");
    // Clear is disabled: it says so and does nothing.
    let clear = control(&window, Role::Button, "Clear cookies and site data");
    assert_eq!(clear.accessible_enabled(), Some(false));
    clear.invoke_accessible_default_action();
    assert!(cleared.borrow().is_empty());
    assert!(!node_in(&nodes, Role::Button, "Clear cookies and site data").enabled);
    assert_laid_out(&nodes, &window);
    assert_snapshot("page-ui-site-info-warning", &nodes);
}

// ---------------------------------------------------------------------------------------------
// The whole window
// ---------------------------------------------------------------------------------------------

/// A state of the window with something a page caused over it: its name, and how to show it.
type Overlay = (&'static str, fn(&MainWindow));

fn clear_overlays(window: &MainWindow) {
    window.set_page_request(PageRequestView::default());
    window.set_controls_popup(0);
    window.set_pop_up_blocked_origin("".into());
    window.set_capture(CaptureView::default());
    window.set_capture_bubble_open(false);
    window.set_https_only_interstitial_shown(false);
}

#[test]
fn every_overlay_leaves_the_whole_window_labelled_and_unique() {
    let window = page_window();
    assert_labelled(&reading_order(&window), "the page with no overlay");

    let states: [Overlay; 14] = [
        ("an alert", |w| {
            w.set_page_request(alert(1, "example.com", "Hello"))
        }),
        ("a confirm dialog", |w| {
            w.set_page_request(confirm(2, "example.com", "Sure?"))
        }),
        ("a prompt", |w| {
            w.set_page_request(prompt(3, "example.com", "Name?", "Ann"))
        }),
        ("the leave prompt", |w| {
            w.set_page_request(leave_page(4, "example.com"))
        }),
        ("sign-in", |w| w.set_page_request(sign_in(5, "example.com"))),
        ("a permission prompt", |w| {
            w.set_page_request(permission(6, "example.com"))
        }),
        ("a camera prompt", |w| {
            w.set_page_request(capture_permission(7, "example.com"))
        }),
        ("a select list", |w| open_select(w, "example.com", true)),
        ("the colour picker", |w| open_color_picker(w, "example.com")),
        ("the context menu", open_context_menu),
        ("the pop-up notice", |w| {
            w.set_pop_up_blocked_origin("ads.example".into())
        }),
        ("the capture bubble", |w| {
            w.set_capture(capture(true, true, "example.com", ""));
            w.set_capture_bubble_open(true);
        }),
        ("the HTTPS-only interstitial", |w| {
            w.set_https_only_interstitial_host("plain.example".into());
            w.set_https_only_interstitial_shown(true);
        }),
        ("the site information popover", |w| {
            w.set_security_icon(SiteSecurityIcon::Padlock);
            w.set_security_label("Connection is secure".into());
            w.set_site_info(site_info(
                SiteSecurityIcon::Padlock,
                "example.com",
                "Connection is secure",
            ));
            w.invoke_show_site_info();
        }),
    ];
    for (name, show) in states {
        clear_overlays(&window);
        show(&window);
        let nodes = reading_order(&window);
        assert_labelled(&nodes, name);
        assert_bounds(&nodes, window_size(&window));
    }
}
