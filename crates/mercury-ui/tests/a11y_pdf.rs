// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
#![cfg(feature = "test-support")]
//! What VoiceOver (or any assistive technology) is given while the active tab shows a PDF: the
//! viewer's toolbar, its page area, and the three panels that stand in for them (opening,
//! password, error), inside the real `MainWindow`, with `pdf-viewer-shown` on. Every button is
//! pressed through its default action and compared with what a click does (the
//! `PdfViewerModel` callback it must raise), and the reading order of each state is frozen in
//! `tests/a11y-snapshots/main-window-pdf-*.txt`.
//!
//! The window must offer one `main` landmark: the "Page content" node of the web view stays in
//! the tree under the viewer, so it says nothing about assistive technology, and the PDF's
//! page area is a region.

mod common;

use std::cell::RefCell;
use std::rc::Rc;

use common::a11y::{
    self, Node, assert_bounds, assert_labelled, assert_snapshot, dump, node_in, reading_order,
};
use i_slint_backend_testing::AccessibleRole as Role;
use mercury_ui::{MainWindow, PdfFitMode, PdfViewState, PdfViewerModel};
use slint::ComponentHandle;

const WIDTH: f32 = 1180.0;
const HEIGHT: f32 = 760.0;

type Log = Rc<RefCell<Vec<String>>>;

/// A main window with a PDF open in the page area, in `state`, with every callback of the
/// viewer logging its name (and the text it was given) into the returned log.
fn window(state: PdfViewState) -> (MainWindow, Log) {
    a11y::init();
    let window = MainWindow::new().unwrap();
    window.set_is_new_tab(false);
    window.set_web_view_visible(false);
    window.set_pdf_viewer_shown(true);
    window.set_address("file:///Users/me/three-pages.pdf".into());
    window.set_committed_address("file:///Users/me/three-pages.pdf".into());
    window.set_page_title("three-pages.pdf".into());

    let log = Log::default();
    let model = window.global::<PdfViewerModel>();
    model.set_state(state);
    model.set_title("three-pages.pdf".into());
    model.set_page(1);
    model.set_page_count(3);
    let record = |name: &'static str| {
        let log = Rc::clone(&log);
        move || log.borrow_mut().push(name.to_string())
    };
    model.on_zoom_in(record("zoom-in"));
    model.on_zoom_out(record("zoom-out"));
    model.on_fit_width(record("fit-width"));
    model.on_fit_page(record("fit-page"));
    model.on_rotate(record("rotate"));
    model.on_find(record("find"));
    model.on_save_copy(record("save-copy"));
    model.on_open_externally(record("open-externally"));
    model.on_retry(record("retry"));
    let submitted = Rc::clone(&log);
    model.on_submit_password(move |text| {
        submitted
            .borrow_mut()
            .push(format!("submit-password {text}"))
    });
    a11y::open_window(&window, WIDTH, HEIGHT);
    (window, log)
}

fn take(log: &Log) -> Vec<String> {
    std::mem::take(&mut *log.borrow_mut())
}

fn press(window: &MainWindow, role: Role, label: &str) {
    node_in(&reading_order(window), role, label).press();
}

fn mains(nodes: &[Node]) -> Vec<&Node> {
    nodes.iter().filter(|n| n.role == Role::Main).collect()
}

/// The window has one `main`, and while a PDF is shown it is not the one that says Mercury
/// passes no page content on.
fn assert_one_landmark(nodes: &[Node]) {
    let found = mains(nodes);
    assert_eq!(
        found.len(),
        1,
        "expected one main landmark:\n{}",
        dump(nodes)
    );
    // The status line of an empty page area is not read under the viewer that covers it.
    assert!(
        nodes
            .iter()
            .all(|n| n.label != "Type an address or search with DuckDuckGo."),
        "the page area's message is read under the viewer:\n{}",
        dump(nodes)
    );
    assert!(
        found[0].description.is_empty(),
        "the main landmark still says the page is not passed on:\n{}",
        dump(nodes)
    );
}

#[test]
fn a_ready_pdf_names_the_toolbar_and_the_pages() {
    let (window, _) = window(PdfViewState::Ready);
    let nodes = reading_order(&window);
    assert_labelled(&nodes, "a PDF that is shown");
    assert_bounds(
        &nodes
            .iter()
            .filter(|n| n.label.starts_with("PDF "))
            .cloned()
            .collect::<Vec<_>>(),
        (WIDTH, HEIGHT),
    );

    assert_one_landmark(&nodes);
    node_in(&nodes, Role::Navigation, "PDF toolbar");
    node_in(&nodes, Role::Region, "PDF document, page 1 of 3");
    node_in(&nodes, Role::Text, "Document title: three-pages.pdf");
    node_in(&nodes, Role::TextInput, "Page number");
    node_in(&nodes, Role::Text, "of 3 pages");
    node_in(&nodes, Role::Text, "Zoom 100 percent");
    for label in [
        "Zoom out",
        "Zoom in",
        "Fit width",
        "Fit page",
        "Rotate clockwise",
        "Find in document",
        "Save a copy",
        "Open in Preview",
    ] {
        assert!(
            node_in(&nodes, Role::Button, label).enabled,
            "{label} should be enabled"
        );
    }

    // The toolbar is read before the pages, and the pages after the web view's node.
    let index = |role: Role, label: &str| {
        nodes
            .iter()
            .position(|n| n.role == role && n.label == label)
            .unwrap()
    };
    assert!(
        index(Role::Navigation, "PDF toolbar") < index(Role::Region, "PDF document, page 1 of 3")
    );

    // The panels of the other states are not read.
    for label in [
        "Opening the PDF",
        "This PDF needs a password",
        "This PDF could not be shown",
    ] {
        assert!(nodes.iter().all(|n| n.label != label), "{}", dump(&nodes));
    }

    assert_snapshot("main-window-pdf-ready", &nodes);
}

#[test]
fn the_pages_follow_the_page_the_window_is_on() {
    let (window, _) = window(PdfViewState::Ready);
    window.global::<PdfViewerModel>().set_page(3);
    node_in(
        &reading_order(&window),
        Role::Region,
        "PDF document, page 3 of 3",
    );
}

#[test]
fn the_toolbar_buttons_do_what_a_click_does() {
    let (window, log) = window(PdfViewState::Ready);
    for (label, callback) in [
        ("Zoom out", "zoom-out"),
        ("Zoom in", "zoom-in"),
        ("Fit width", "fit-width"),
        ("Fit page", "fit-page"),
        ("Rotate clockwise", "rotate"),
        ("Find in document", "find"),
        ("Save a copy", "save-copy"),
        ("Open in Preview", "open-externally"),
    ] {
        press(&window, Role::Button, label);
        assert_eq!(take(&log), [callback], "pressing {label}");
    }
}

#[test]
fn the_toolbar_says_which_toggles_are_on_and_which_buttons_cannot_be_used() {
    let (window, log) = window(PdfViewState::Ready);
    let model = window.global::<PdfViewerModel>();
    model.set_fit(PdfFitMode::Width);
    model.set_find_open(false);
    let nodes = reading_order(&window);
    assert_eq!(
        node_in(&nodes, Role::Button, "Fit width").checked,
        Some(true)
    );
    assert_eq!(
        node_in(&nodes, Role::Button, "Fit page").checked,
        Some(false)
    );
    assert_eq!(
        node_in(&nodes, Role::Button, "Find in document").checked,
        Some(false)
    );

    model.set_fit(PdfFitMode::Page);
    model.set_find_open(true);
    model.set_can_zoom_in(false);
    model.set_can_zoom_out(false);
    model.set_can_save_copy(false);
    model.set_can_open_externally(false);
    model.set_zoom_percent(400);
    let nodes = reading_order(&window);
    assert_labelled(&nodes, "a PDF at its limits");
    assert_eq!(
        node_in(&nodes, Role::Button, "Fit width").checked,
        Some(false)
    );
    assert_eq!(
        node_in(&nodes, Role::Button, "Fit page").checked,
        Some(true)
    );
    assert_eq!(
        node_in(&nodes, Role::Button, "Find in document").checked,
        Some(true)
    );
    node_in(&nodes, Role::Text, "Zoom 400 percent");
    for label in ["Zoom in", "Zoom out", "Save a copy", "Open in Preview"] {
        assert!(
            !node_in(&nodes, Role::Button, label).enabled,
            "{label} should be disabled"
        );
    }

    // A disabled button does nothing when an assistive technology presses it.
    for label in ["Zoom in", "Zoom out", "Save a copy", "Open in Preview"] {
        press(&window, Role::Button, label);
    }
    assert_eq!(take(&log), Vec::<String>::new());
}

#[test]
fn a_pdf_that_is_opening_is_named() {
    let (window, _) = window(PdfViewState::Loading);
    let nodes = reading_order(&window);
    assert_labelled(&nodes, "a PDF that is opening");
    assert_one_landmark(&nodes);
    node_in(&nodes, Role::Text, "Opening the PDF");
    assert!(
        nodes
            .iter()
            .all(|n| n.label != "PDF toolbar" && n.role != Role::Region),
        "{}",
        dump(&nodes)
    );
    assert_snapshot("main-window-pdf-loading", &nodes);
}

#[test]
fn the_password_panel_asks_for_the_password_and_never_hands_it_on() {
    let (window, log) = window(PdfViewState::Password);
    let nodes = reading_order(&window);
    assert_labelled(&nodes, "a PDF that needs a password");
    assert_one_landmark(&nodes);
    node_in(&nodes, Role::Groupbox, "This PDF needs a password");
    let field = node_in(&nodes, Role::TextInput, "Password");
    assert_eq!(field.value, "");
    assert!(!node_in(&nodes, Role::Button, "Open").enabled);
    assert_snapshot("main-window-pdf-password", &nodes);

    // Nothing is sent while the field is empty.
    press(&window, Role::Button, "Open");
    assert_eq!(take(&log), Vec::<String>::new());

    // Typed text enables the button, which submits it, and the field is emptied after.
    field.handle.set_accessible_value("hunter2");
    let nodes = reading_order(&window);
    assert!(node_in(&nodes, Role::Button, "Open").enabled);
    assert_eq!(
        node_in(&nodes, Role::TextInput, "Password").value,
        "",
        "the password field hands its text to assistive technology:\n{}",
        dump(&nodes)
    );
    press(&window, Role::Button, "Open");
    assert_eq!(take(&log), ["submit-password hunter2"]);

    // A wrong password is said in text after the field.
    window
        .global::<PdfViewerModel>()
        .set_message("That password is not right.".into());
    let nodes = reading_order(&window);
    assert_labelled(&nodes, "a PDF whose password was refused");
    node_in(&nodes, Role::Text, "That password is not right.");
    assert_snapshot("main-window-pdf-password-refused", &nodes);
}

#[test]
fn the_error_panel_says_why_and_offers_three_ways_on() {
    let (window, log) = window(PdfViewState::Error);
    window
        .global::<PdfViewerModel>()
        .set_message("The file is damaged.".into());
    let nodes = reading_order(&window);
    assert_labelled(&nodes, "a PDF that could not be shown");
    assert_one_landmark(&nodes);
    node_in(&nodes, Role::Groupbox, "This PDF could not be shown");
    node_in(&nodes, Role::Text, "The file is damaged.");
    assert!(
        nodes.iter().all(|n| n.label != "PDF toolbar"),
        "{}",
        dump(&nodes)
    );
    for (label, callback) in [
        ("Save a copy", "save-copy"),
        ("Open in Preview", "open-externally"),
        ("Try again", "retry"),
    ] {
        assert!(node_in(&nodes, Role::Button, label).enabled);
        press(&window, Role::Button, label);
        assert_eq!(take(&log), [callback], "pressing {label}");
    }
    assert_snapshot("main-window-pdf-error", &nodes);

    // A document that cannot be saved or opened elsewhere disables those two.
    let model = window.global::<PdfViewerModel>();
    model.set_can_save_copy(false);
    model.set_can_open_externally(false);
    let nodes = reading_order(&window);
    assert!(!node_in(&nodes, Role::Button, "Save a copy").enabled);
    assert!(!node_in(&nodes, Role::Button, "Open in Preview").enabled);
    press(&window, Role::Button, "Save a copy");
    press(&window, Role::Button, "Open in Preview");
    assert_eq!(take(&log), Vec::<String>::new());
}

#[test]
fn the_web_page_node_only_speaks_when_no_pdf_is_shown() {
    let (window, _) = window(PdfViewState::Ready);
    window.set_pdf_viewer_shown(false);
    window.set_web_view_visible(true);
    let nodes = reading_order(&window);
    let page = node_in(&nodes, Role::Main, "Page content");
    assert_eq!(
        page.description,
        "Mercury doesn't pass web page content to assistive technology yet."
    );
    assert!(
        nodes
            .iter()
            .all(|n| n.label != "PDF toolbar" && n.role != Role::Region),
        "{}",
        dump(&nodes)
    );
}
