// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
#![cfg(feature = "test-support")]
//! What VoiceOver (or any assistive technology) is given for the main window's own controls: the
//! toolbar, the address bar, the new-tab page and the node the page will hang from. Every
//! control is found by role and label, pressed through its default action (the action an
//! assistive technology's "press" triggers, which works without keyboard focus), and compared
//! with what a click does; the reading order of two states is frozen in
//! `tests/a11y-snapshots/main-window-*.txt`. The window is shown at the size it opens at.

mod common;

use std::cell::RefCell;
use std::rc::Rc;

use common::a11y::{
    self, Node, Rect, assert_bounds, assert_inside, assert_labelled, assert_rect, assert_snapshot,
    control_in, dump, node_in, reading_order,
};
use i_slint_backend_testing::AccessibleRole as Role;
use mercury_ui::MainWindow;
use slint::platform::PointerEventButton;
use slint::{ModelRc, SharedString, VecModel};

const WIDTH: f32 = 1180.0;
const HEIGHT: f32 = 760.0;

type Log = Rc<RefCell<Vec<String>>>;

/// A main window on the testing platform, with every callback the toolbar uses logging its
/// name (and the text it was given) into the returned log.
fn window() -> (MainWindow, Log) {
    a11y::init();
    let window = MainWindow::new().unwrap();
    let log = Log::default();
    let record = |name: &'static str| {
        let log = Rc::clone(&log);
        move || log.borrow_mut().push(name.to_string())
    };
    window.on_new_tab(record("new-tab"));
    window.on_go_back(record("go-back"));
    window.on_go_forward(record("go-forward"));
    window.on_reload(record("reload"));
    window.on_go_home(record("go-home"));
    let navigated = Rc::clone(&log);
    window.on_navigate(move |text| navigated.borrow_mut().push(format!("navigate {text}")));
    let edited = Rc::clone(&log);
    window
        .on_address_edited(move |text| edited.borrow_mut().push(format!("address-edited {text}")));
    a11y::open_window(&window, WIDTH, HEIGHT);
    (window, log)
}

/// A page is shown: Back is possible, Forward is not, and the page is loading.
fn page_loaded_and_loading(window: &MainWindow) {
    window.set_is_new_tab(false);
    window.set_web_view_visible(true);
    window.set_can_go_back(true);
    window.set_can_go_forward(false);
    window.set_is_loading(true);
    window.set_address("https://example.com/docs".into());
    window.set_committed_address("https://example.com/docs".into());
    window.set_page_title("Docs".into());
}

fn take(log: &Log) -> Vec<String> {
    std::mem::take(&mut *log.borrow_mut())
}

fn press(window: &MainWindow, role: Role, label: &str) {
    control_in(&reading_order(window), role, label).invoke_accessible_default_action();
}

fn node(window: &MainWindow, role: Role, label: &str) -> Node {
    node_in(&reading_order(window), role, label).clone()
}

fn index_of(nodes: &[Node], role: Role, label: &str) -> usize {
    nodes
        .iter()
        .position(|n| n.role == role && n.label == label)
        .unwrap_or_else(|| {
            panic!(
                "no {} labelled {label:?}:\n{}",
                a11y::role_name(role),
                dump(nodes)
            )
        })
}

/// The labels of the toolbar's own controls, in the order they are laid out.
const TOOLBAR: [(Role, &str); 6] = [
    (Role::Navigation, "Toolbar"),
    (Role::Button, "Back"),
    (Role::Button, "Forward"),
    (Role::Button, "Reload page"),
    (Role::Search, "Address bar"),
    (Role::Button, "Browser menu"),
];

#[test]
fn the_new_tab_state_names_every_control_and_landmark() {
    let (window, _) = window();
    let nodes = reading_order(&window);
    assert_labelled(&nodes, "a new tab");

    for (role, label) in TOOLBAR {
        node_in(&nodes, role, label);
    }
    node_in(&nodes, Role::Button, "New tab");
    node_in(&nodes, Role::TextInput, "Address and search");
    node_in(&nodes, Role::Region, "New tab page");
    node_in(&nodes, Role::TextInput, "Search or enter address");
    node_in(&nodes, Role::Button, "Search");
    node_in(&nodes, Role::Main, "Page content");

    // The home button only exists when the appearance setting asks for it.
    assert!(!nodes.iter().any(|n| n.label == "Home page"));

    // Back and Forward say they cannot be used; the others can.
    assert!(!node_in(&nodes, Role::Button, "Back").enabled);
    assert!(!node_in(&nodes, Role::Button, "Forward").enabled);
    for label in ["Reload page", "New tab", "Browser menu", "Search"] {
        assert!(
            node_in(&nodes, Role::Button, label).enabled,
            "{label} should be enabled"
        );
    }

    // The search field says what it is for and shows the same words as the Text drawn over it.
    let search = node_in(&nodes, Role::TextInput, "Search or enter address");
    assert_eq!(
        search.placeholder(),
        "Search with DuckDuckGo or enter an address"
    );

    // The page node carries no description of its own any more: the engine's tree hangs from
    // it (`tests/a11y_page_node.rs` pins that the node exists exactly once).
    let page = node_in(&nodes, Role::Main, "Page content");
    assert_eq!(page.description, "");

    // The new-tab page keeps its captions as plain text, and nothing else reads twice: the
    // logo, the placeholder and the arrow in the search button are hidden.
    let texts: Vec<&str> = nodes
        .iter()
        .filter(|n| n.role == Role::Text)
        .map(|n| n.label.as_str())
        .collect();
    assert!(texts.contains(&"Mercury Browser"));
    assert!(texts.contains(&"Fast, lightweight and private by default"));
    assert!(
        !texts.iter().any(|label| label.starts_with("Search with ")),
        "{texts:?}"
    );
    assert!(!texts.contains(&"→"));
    assert!(
        nodes.iter().all(|n| n.role != Role::Image),
        "an image is exposed:\n{}",
        dump(&nodes)
    );

    assert_snapshot("main-window-new-tab", &nodes);

    // The placeholder is the Text drawn over the field: it follows the search engine's name.
    window.set_search_engine_name("Example Search".into());
    assert_eq!(
        node(&window, Role::TextInput, "Search or enter address").placeholder(),
        "Search with Example Search or enter an address"
    );
}

#[test]
fn the_page_state_follows_what_the_window_shows() {
    let (window, _) = window();
    page_loaded_and_loading(&window);
    window.set_zoom_percent(125);
    let nodes = reading_order(&window);
    assert_labelled(&nodes, "a loaded page");

    assert!(node_in(&nodes, Role::Button, "Back").enabled);
    assert!(!node_in(&nodes, Role::Button, "Forward").enabled);
    assert_eq!(
        node_in(&nodes, Role::Button, "Reload page").description,
        "The page is loading"
    );
    assert_eq!(
        node_in(&nodes, Role::Text, "Page zoom 125 percent").role,
        Role::Text
    );
    assert_eq!(
        node_in(&nodes, Role::TextInput, "Address and search").value,
        "https://example.com/docs"
    );

    // The new-tab page is gone, and the page's texture is a decoration: no image is exposed.
    assert!(
        nodes
            .iter()
            .all(|n| n.label != "New tab page" && n.label != "Search")
    );
    assert!(
        nodes.iter().all(|n| n.role != Role::Image),
        "{}",
        dump(&nodes)
    );
    node_in(&nodes, Role::Main, "Page content");

    assert_snapshot("main-window-page", &nodes);

    // The page finished loading, and the zoom is back to the default: the description and the
    // zoom badge go.
    window.set_is_loading(false);
    window.set_zoom_percent(100);
    let nodes = reading_order(&window);
    assert_labelled(&nodes, "a loaded page, finished");
    assert_eq!(node_in(&nodes, Role::Button, "Reload page").description, "");
    assert!(
        nodes.iter().all(|n| !n.label.starts_with("Page zoom")),
        "{}",
        dump(&nodes)
    );
}

#[test]
fn the_zoom_badge_says_the_zoom_in_words() {
    let (window, _) = window();
    page_loaded_and_loading(&window);
    for percent in [50, 125, 300] {
        window.set_zoom_percent(percent);
        let nodes = reading_order(&window);
        node_in(&nodes, Role::Text, &format!("Page zoom {percent} percent"));
        assert!(nodes.iter().all(|n| n.label != format!("{percent}%")));
    }
    // The badge shows only when the page's zoom is not the default one.
    window.set_default_zoom_percent(150);
    window.set_zoom_percent(150);
    assert!(
        reading_order(&window)
            .iter()
            .all(|n| !n.label.starts_with("Page zoom"))
    );
    window.set_zoom_percent(100);
    node_in(&reading_order(&window), Role::Text, "Page zoom 100 percent");
}

#[test]
fn the_home_button_is_read_between_reload_and_the_address_bar() {
    let (window, log) = window();
    window.set_show_home_button(true);
    let nodes = reading_order(&window);
    assert_labelled(&nodes, "a window with the home button");
    let reload = index_of(&nodes, Role::Button, "Reload page");
    let home = index_of(&nodes, Role::Button, "Home page");
    let address_bar = index_of(&nodes, Role::Search, "Address bar");
    assert!(reload < home && home < address_bar, "{}", dump(&nodes));
    assert!(node_in(&nodes, Role::Button, "Home page").enabled);
    press(&window, Role::Button, "Home page");
    assert_eq!(take(&log), ["go-home"]);

    window.set_show_home_button(false);
    assert!(
        reading_order(&window)
            .iter()
            .all(|n| n.label != "Home page")
    );
}

#[test]
fn chrome_hidden_leaves_only_the_page() {
    let (window, _) = window();
    page_loaded_and_loading(&window);
    window.set_chrome_hidden(true);
    let nodes = reading_order(&window);
    assert_labelled(&nodes, "a fullscreen page");
    for (role, label) in TOOLBAR {
        assert!(
            !nodes.iter().any(|n| n.role == role && n.label == label),
            "{label} is read although the toolbar is hidden:\n{}",
            dump(&nodes)
        );
    }
    assert!(
        nodes
            .iter()
            .all(|n| n.label != "New tab" && n.label != "Address and search")
    );
    assert!(nodes.iter().all(|n| n.role != Role::TabList));
    node_in(&nodes, Role::Main, "Page content");

    window.set_chrome_hidden(false);
    node_in(&reading_order(&window), Role::Navigation, "Toolbar");
}

#[test]
fn every_default_action_does_what_a_click_does() {
    let (window, log) = window();
    page_loaded_and_loading(&window);
    window.set_can_go_forward(true);
    window.set_show_home_button(true);

    press(&window, Role::Button, "New tab");
    assert_eq!(take(&log), ["new-tab"]);
    press(&window, Role::Button, "Back");
    assert_eq!(take(&log), ["go-back"]);
    press(&window, Role::Button, "Forward");
    assert_eq!(take(&log), ["go-forward"]);
    press(&window, Role::Button, "Reload page");
    assert_eq!(take(&log), ["reload"]);
    press(&window, Role::Button, "Home page");
    assert_eq!(take(&log), ["go-home"]);

    // The browser menu opens: its list appears among the window's popups.
    let list = |window: &MainWindow| {
        reading_order(window)
            .iter()
            .any(|n| n.role == Role::List && n.label == "Browser menu")
    };
    assert!(!list(&window), "the menu is open before it was asked for");
    press(&window, Role::Button, "Browser menu");
    assert!(
        list(&window),
        "the browser menu did not open:\n{}",
        dump(&reading_order(&window))
    );
    assert_eq!(take(&log), Vec::<String>::new());
}

#[test]
fn a_click_in_the_reported_rectangle_does_what_the_default_action_does() {
    // VoiceOver draws its cursor in the rectangle the window reports for a control and clicks at
    // its centre. A click there, which the testing platform dispatches as a pointer press and
    // release, must do exactly what the default action does: so the rectangle is where the
    // control really is (no doubled or shifted offset from the layers Slint adds for `opacity`).
    let (window, log) = window();
    page_loaded_and_loading(&window);
    window.set_can_go_forward(true);
    window.set_show_home_button(true);
    let click = |label: &str| {
        control_in(&reading_order(&window), Role::Button, label)
            .mock_single_click(PointerEventButton::Left);
    };
    for (label, event) in [
        ("New tab", "new-tab"),
        ("Back", "go-back"),
        ("Forward", "go-forward"),
        ("Reload page", "reload"),
        ("Home page", "go-home"),
    ] {
        click(label);
        assert_eq!(take(&log), [event], "a click on {label}");
    }
    // A disabled button ignores the click, as it ignores the default action.
    window.set_can_go_back(false);
    click("Back");
    assert_eq!(take(&log), Vec::<String>::new());

    // The page is gone again: the new-tab page's Search button, with a typed text.
    window.set_is_new_tab(true);
    window.set_web_view_visible(false);
    control_in(
        &reading_order(&window),
        Role::TextInput,
        "Search or enter address",
    )
    .set_accessible_value("clicked");
    click("Search");
    assert_eq!(take(&log), ["navigate clicked"]);
}

#[test]
fn the_search_button_navigates_with_what_was_typed() {
    let (window, log) = window();
    let nodes = reading_order(&window);
    let field = control_in(&nodes, Role::TextInput, "Search or enter address");
    assert_eq!(field.accessible_value().unwrap(), "");

    // Setting the field's value is what an assistive technology does to type into it.
    field.set_accessible_value("slint accessibility");
    assert_eq!(
        node(&window, Role::TextInput, "Search or enter address").value,
        "slint accessibility"
    );
    press(&window, Role::Button, "Search");
    assert_eq!(take(&log), ["navigate slint accessibility"]);

    // Typing in the address field reaches the application and the window's address.
    let address = control_in(&nodes, Role::TextInput, "Address and search");
    address.set_accessible_value("https://example.org/");
    assert_eq!(window.get_address(), "https://example.org/");
    assert_eq!(take(&log), ["address-edited https://example.org/"]);
}

#[test]
fn a_disabled_button_says_so_and_does_nothing() {
    let (window, log) = window();
    for (label, event) in [("Back", "go-back"), ("Forward", "go-forward")] {
        let handle = control_in(&reading_order(&window), Role::Button, label);
        assert_eq!(
            handle.accessible_enabled(),
            Some(false),
            "{label} is enabled"
        );
        handle.invoke_accessible_default_action();
        assert_eq!(
            take(&log),
            Vec::<String>::new(),
            "{label} acted while disabled"
        );

        // The same button works as soon as the window says it can.
        if label == "Back" {
            window.set_can_go_back(true);
        } else {
            window.set_can_go_forward(true);
        }
        let handle = control_in(&reading_order(&window), Role::Button, label);
        assert_eq!(handle.accessible_enabled(), Some(true));
        handle.invoke_accessible_default_action();
        assert_eq!(take(&log), [event]);
    }
}

#[test]
fn retry_and_dismiss_still_occur_once() {
    // `apps/mercury-desktop/src/passwords/sign_in.rs` finds the save notice's buttons by label
    // among the window's controls that are not text, and fails on a second match.
    let (window, _) = window();
    let count = |window: &MainWindow, label: &str| {
        reading_order(window)
            .iter()
            .filter(|n| n.role != Role::Text && n.label == label)
            .count()
    };
    assert_eq!(count(&window, "Retry"), 0);
    assert_eq!(count(&window, "Dismiss"), 0);

    window.set_password_save_problem("Couldn't save the login for example.com.".into());
    assert_eq!((count(&window, "Retry"), count(&window, "Dismiss")), (1, 1));
    assert_labelled(&reading_order(&window), "the password save notice");
    window.set_password_save_problem("".into());
    assert_eq!((count(&window, "Retry"), count(&window, "Dismiss")), (0, 0));

    // With the session bar under the toolbar as well, in either of its two forms: its buttons
    // are named otherwise, so the notice's are still the only ones with these labels.
    window.set_password_save_problem("Couldn't save the login for example.com.".into());
    window.set_recovery_sessions(ModelRc::new(VecModel::from(vec![SharedString::from(
        "Session A (2 tabs)",
    )])));
    assert_eq!((count(&window, "Retry"), count(&window, "Dismiss")), (1, 1));
    assert_labelled(
        &reading_order(&window),
        "the save notice and the restore offer",
    );
    window.set_recovery_sessions(ModelRc::default());
    window.set_session_notice("Your tabs were restored.".into());
    assert_eq!((count(&window, "Retry"), count(&window, "Dismiss")), (1, 1));
    assert_labelled(
        &reading_order(&window),
        "the save notice and the session message",
    );
    window.set_password_save_problem("".into());
    assert_eq!((count(&window, "Retry"), count(&window, "Dismiss")), (0, 0));
    assert_labelled(&reading_order(&window), "the session message");
    window.set_session_notice("".into());

    // The settings notice has the other "Dismiss", once.
    window.set_settings_notice("The settings file could not be read.".into());
    assert_eq!((count(&window, "Retry"), count(&window, "Dismiss")), (0, 1));
    assert_labelled(&reading_order(&window), "the settings notice");
}

#[test]
fn no_control_is_read_twice() {
    // A button's glyph or caption is hidden: nothing else is listed inside a labelled button.
    let (window, _) = window();
    window.set_show_home_button(true);
    for state in 0..2 {
        if state == 1 {
            page_loaded_and_loading(&window);
        }
        let nodes = reading_order(&window);
        for button in nodes.iter().filter(|n| n.role == Role::Button) {
            let inside: Vec<&Node> = nodes
                .iter()
                .filter(|other| {
                    other.label != button.label
                        && other.role != Role::Navigation
                        && other.role != Role::Search
                        && button.rect().contains(&other.rect())
                })
                .collect();
            assert!(
                inside.is_empty(),
                "{} also lists {} inside its rectangle:\n{}",
                button.label,
                inside.len(),
                dump(&nodes)
            );
        }
    }
}

#[test]
fn the_bounds_given_to_voiceover_are_where_the_controls_are_drawn() {
    let (window, _) = window();
    window.set_show_home_button(true);
    let size = a11y::window_size(&window);
    assert_eq!(size, (WIDTH, HEIGHT));

    let nodes = reading_order(&window);
    assert_bounds(&nodes, size);

    // The toolbar is the 60 px under the 48 px tab row, and the address bar sits in it.
    let toolbar = node_in(&nodes, Role::Navigation, "Toolbar");
    assert_rect(toolbar, Rect::new(0.0, 48.0, WIDTH, 60.0));
    let address_bar = node_in(&nodes, Role::Search, "Address bar");
    assert_inside(address_bar, toolbar);
    assert_inside(
        node_in(&nodes, Role::TextInput, "Address and search"),
        address_bar,
    );
    assert_inside(node_in(&nodes, Role::Button, "Browser menu"), toolbar);

    // The toolbar buttons are 36 x 34 px, 8 px apart, starting 14 px from the window's edge.
    let mut x = 14.0;
    for label in ["Back", "Forward", "Reload page", "Home page"] {
        assert_rect(
            node_in(&nodes, Role::Button, label),
            Rect::new(x, 59.0, 36.0, 34.0),
        );
        x += 36.0 + 8.0;
    }
    assert_eq!(
        address_bar.rect().x,
        x,
        "the address bar follows the last button after 8 px"
    );
    // The menu button is the last one, 14 px from the right edge; "+" ends the tab row.
    assert_rect(
        node_in(&nodes, Role::Button, "Browser menu"),
        Rect::new(WIDTH - 14.0 - 36.0, 59.0, 36.0, 34.0),
    );
    assert_rect(
        node_in(&nodes, Role::Button, "New tab"),
        Rect::new(WIDTH - 14.0 - 34.0, 7.0, 34.0, 34.0),
    );

    // The new-tab page and what is on it lie inside the page area, which is where the window
    // says the page is; the page node covers the same area.
    let page_area = Rect::new(
        window.get_controls_page_x(),
        window.get_controls_page_y(),
        window.get_web_view_width(),
        window.get_web_view_height(),
    );
    let region = node_in(&nodes, Role::Region, "New tab page");
    assert!(page_area.contains(&region.rect()));
    for (role, label) in [
        (Role::TextInput, "Search or enter address"),
        (Role::Button, "Search"),
    ] {
        assert_inside(node_in(&nodes, role, label), region);
    }
    assert_rect(node_in(&nodes, Role::Main, "Page content"), page_area);

    // A page is shown, the toolbar is hidden: the page node still covers the page area.
    page_loaded_and_loading(&window);
    window.set_chrome_hidden(true);
    let nodes = reading_order(&window);
    assert_bounds(&nodes, size);
    let page_area = Rect::new(
        window.get_controls_page_x(),
        window.get_controls_page_y(),
        window.get_web_view_width(),
        window.get_web_view_height(),
    );
    assert_rect(node_in(&nodes, Role::Main, "Page content"), page_area);
    assert_eq!(
        (page_area.x, page_area.y, page_area.width, page_area.height),
        (0.0, 0.0, WIDTH, HEIGHT)
    );
}
