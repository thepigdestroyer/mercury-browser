// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
#![cfg(feature = "test-support")]
//! What VoiceOver is given for the Orbit overview (lane brief D14, SPEC §5): at each level the
//! roles and labels of the planets, previews, chips, the card, the neighbours, the title bar and the
//! popover; decoration carries no role; the container's label names the level; each button has
//! exactly one default action and it does what a click does. The window is an `OrbitPreview`
//! filled through the real pipeline (`tests/orbit_scenes/mod.rs`).
//!
//! There is no golden reading-order file yet (a golden is a file outside this lane's list); the tests
//! assert the structure. Tests marked "ring" count what the real `RingBands` exposes (its `tab`s).

mod common;

#[allow(dead_code)]
#[path = "orbit_scenes/mod.rs"]
mod scenes;

use std::cell::RefCell;
use std::rc::Rc;

use common::a11y::{self, Node, assert_labelled, node_in, reading_order};
use i_slint_backend_testing::{AccessibleLiveness, AccessibleRole as Role};
use mercury_ui::{
    OrbitChip, OrbitNeighbour, OrbitPlanet, OrbitPopover, OrbitPreview, OrbitPreviewCard,
};
use scenes::{Frame, scene};
use slint::{ComponentHandle, Model};

type Log = Rc<RefCell<Vec<String>>>;

/// The Slint rows of the scene, as the stage shows them (with their ring arrays).
struct Rows {
    planets: Vec<OrbitPlanet>,
    previews: Vec<OrbitPreviewCard>,
    chips: Vec<OrbitChip>,
    neighbours: Vec<OrbitNeighbour>,
    popover: OrbitPopover,
}

fn window(frame: Frame, dark: bool) -> (OrbitPreview, Log, Rows) {
    a11y::init();
    let scene = scene(frame, dark);
    let window: OrbitPreview = scene.window.clone_strong();
    let rows = Rows {
        planets: scene.models.planets().iter().collect(),
        previews: scene.models.previews().iter().collect(),
        chips: scene.models.chips().iter().collect(),
        neighbours: scene.models.neighbours().iter().collect(),
        popover: scene.models.popover(),
    };
    a11y::open_window(&window, 1440.0, 900.0);
    let log: Log = Rc::default();
    window.on_planet_clicked({
        let log = Rc::clone(&log);
        move |id| log.borrow_mut().push(format!("planet {id}"))
    });
    window.on_preview_clicked({
        let log = Rc::clone(&log);
        move |id| log.borrow_mut().push(format!("preview {id}"))
    });
    window.on_chip_clicked({
        let log = Rc::clone(&log);
        move |id| log.borrow_mut().push(format!("chip {id}"))
    });
    window.on_card_clicked({
        let log = Rc::clone(&log);
        move || log.borrow_mut().push("card".into())
    });
    window.on_neighbour_clicked({
        let log = Rc::clone(&log);
        move |id| log.borrow_mut().push(format!("neighbour {id}"))
    });
    window.on_new_group_clicked({
        let log = Rc::clone(&log);
        move || log.borrow_mut().push("new-group".into())
    });
    window.on_back_clicked({
        let log = Rc::clone(&log);
        move || log.borrow_mut().push("back".into())
    });
    window.on_popover_colour_chosen({
        let log = Rc::clone(&log);
        move |i| log.borrow_mut().push(format!("colour {i}"))
    });
    (window, log, rows)
}

fn take(log: &Log) -> Vec<String> {
    std::mem::take(&mut *log.borrow_mut())
}

fn of_role(nodes: &[Node], role: Role) -> Vec<&Node> {
    nodes.iter().filter(|n| n.role == role).collect()
}

fn labels(nodes: &[&Node]) -> Vec<String> {
    nodes.iter().map(|n| n.label.clone()).collect()
}

const ALL: [Frame; 5] = [
    Frame::Overview,
    Frame::Group,
    Frame::Tab,
    Frame::NewGroup,
    Frame::MoveTab,
];

// ---- the labelling rule, every frame, both schemes

#[test]
fn every_frame_follows_the_labelling_rule() {
    for frame in ALL {
        for dark in [false, true] {
            let (window, _log, _rows) = window(frame, dark);
            let nodes = reading_order(&window);
            assert_labelled(&nodes, &format!("{frame:?} dark={dark}"));
        }
    }
}

#[test]
fn no_image_is_exposed_and_no_label_is_a_glyph() {
    for frame in ALL {
        let (window, _log, _rows) = window(frame, false);
        let nodes = reading_order(&window);
        assert!(of_role(&nodes, Role::Image).is_empty(), "{frame:?}");
        for node in &nodes {
            assert!(
                node.label.is_empty() || node.label.chars().any(char::is_alphanumeric),
                "{frame:?}: {}",
                node.label
            );
        }
    }
}

// ---- the container and the level

#[test]
fn one_list_names_the_level() {
    for (frame, label) in [
        (Frame::Overview, "Orbit, every group"),
        (Frame::Group, "Orbit, Research, 10 tabs"),
        (Frame::Tab, "Orbit, AccessKit guide, tab 4 of 10"),
    ] {
        let (window, _log, _rows) = window(frame, false);
        let nodes = reading_order(&window);
        let lists = of_role(&nodes, Role::List);
        assert!(
            lists.iter().any(|l| l.label == label),
            "{frame:?}: {:?}",
            labels(&lists)
        );
        assert_eq!(
            lists
                .iter()
                .filter(|l| l.label.starts_with("Orbit"))
                .count(),
            1,
            "{frame:?}"
        );
    }
}

#[test]
fn changing_the_label_changes_what_the_container_says() {
    let (window, _log, _rows) = window(Frame::Overview, false);
    window.set_container_label("Orbit, Lisbon trip, 8 tabs".into());
    let nodes = reading_order(&window);
    node_in(&nodes, Role::List, "Orbit, Lisbon trip, 8 tabs");
}

// ---- level 1

#[test]
fn level_one_has_a_button_per_planet_the_sun_the_new_group_ring_and_nothing_else() {
    let (window, _log, rows) = window(Frame::Overview, false);
    let nodes = reading_order(&window);
    let buttons = labels(&of_role(&nodes, Role::Button));
    for planet in &rows.planets {
        assert!(
            buttons.contains(&planet.accessible_label.to_string()),
            "{}: {buttons:?}",
            planet.name
        );
    }
    assert!(buttons.contains(&"Research, tab group, 10 tabs, key Option A".to_string()));
    assert!(buttons.contains(&"New group".to_string()));
    assert!(buttons.contains(&"Back to the page".to_string()));
    assert_eq!(buttons.len(), 7, "{buttons:?}");
    let texts = labels(&of_role(&nodes, Role::Text));
    assert_eq!(
        texts,
        vec![
            "Orbit".to_string(),
            "Personal, window 1, 40 tabs".to_string()
        ],
        "the hints, the hole texts and the glyphs are decoration; the sun is read in words"
    );
    let fields = of_role(&nodes, Role::TextInput);
    assert_eq!(fields.len(), 1);
    assert_eq!(fields[0].label, "Search tabs");
    assert_eq!(fields[0].placeholder(), "Search 40 tabs");
}

#[test]
fn the_search_field_is_500_by_40_and_centred() {
    // SPEC mockup: the field is 500 x 40 at the middle of the title bar; its text box starts 36 px
    // in and ends 16 px before the right edge.
    let (window, _log, _rows) = window(Frame::Overview, false);
    let nodes = reading_order(&window);
    let field = node_in(&nodes, Role::TextInput, "Search tabs").rect();
    assert!((field.x - (470.0 + 36.0)).abs() < 0.5, "{field:?}");
    assert!(
        (field.width - (500.0 - 36.0 - 16.0)).abs() < 0.5,
        "{field:?}"
    );
    assert!((field.height - 40.0).abs() < 0.5, "{field:?}");
    assert!((field.y - 4.0).abs() < 0.5, "{field:?}");
}

#[test]
fn the_search_placeholder_is_what_the_host_sets() {
    let (window, _log, _rows) = window(Frame::Overview, false);
    window.set_search_placeholder("Search 12 tabs".into());
    let nodes = reading_order(&window);
    assert_eq!(
        node_in(&nodes, Role::TextInput, "Search tabs").placeholder(),
        "Search 12 tabs"
    );
}

#[test]
fn the_sun_is_read_with_its_own_accessible_label_not_the_drawn_texts() {
    let (window, _log, _rows) = window(Frame::Overview, false);
    let mut sun = window.get_sun();
    sun.accessible_label = "Work, window 2, 9 tabs".into();
    window.set_sun(sun);
    let nodes = reading_order(&window);
    let texts = labels(&of_role(&nodes, Role::Text));
    assert!(
        texts.contains(&"Work, window 2, 9 tabs".to_string()),
        "{texts:?}"
    );
    assert!(
        !texts.iter().any(|t| t.contains('·')),
        "the middle dot is never read: {texts:?}"
    );
}

#[test]
fn a_refused_drop_notice_is_a_live_region_that_names_its_text() {
    let (window, _log, _rows) = window(Frame::Overview, false);
    assert!(
        !of_role(&reading_order(&window), Role::Text)
            .iter()
            .any(|n| n.label.contains("pinned")),
        "no notice, nothing to read"
    );
    window.set_notice("That tab is pinned, so it stays where it is".into());
    let nodes = reading_order(&window);
    let notice = node_in(
        &nodes,
        Role::Text,
        "That tab is pinned, so it stays where it is",
    );
    assert_eq!(
        notice.handle.accessible_live_region(),
        Some(AccessibleLiveness::Polite),
        "VoiceOver announces it when it appears"
    );
}

#[test]
fn level_one_buttons_do_what_a_click_does() {
    let (window, log, _rows) = window(Frame::Overview, false);
    let nodes = reading_order(&window);
    for (label, expected) in [
        ("Research, tab group, 10 tabs, key Option A", "planet 1"),
        ("Lisbon trip, tab group, 8 tabs, key Option S", "planet 2"),
        ("Reading, tab group, 9 tabs, key Option D", "planet 3"),
        ("Shopping, tab group, 7 tabs, key Option F", "planet 4"),
        ("Work, tab group, 6 tabs, key Option G", "planet 5"),
        ("New group", "new-group"),
        ("Back to the page", "back"),
    ] {
        node_in(&nodes, Role::Button, label).press();
        assert_eq!(take(&log), vec![expected], "{label}");
    }
}

#[test]
fn level_one_hides_the_other_levels() {
    let (window, _log, _rows) = window(Frame::Overview, false);
    let nodes = reading_order(&window);
    let buttons = labels(&of_role(&nodes, Role::Button));
    assert!(!buttons.iter().any(|b| b.starts_with("Open ")));
    assert!(!buttons.iter().any(|b| b.contains("tab 1 of 10")));
}

// ---- level 2

#[test]
fn level_two_has_a_button_per_preview_and_chip_and_no_planet() {
    let (window, _log, rows) = window(Frame::Group, false);
    let nodes = reading_order(&window);
    let buttons = labels(&of_role(&nodes, Role::Button));
    for card in &rows.previews {
        assert!(
            buttons.contains(&card.accessible_label.to_string()),
            "{}: {buttons:?}",
            card.title
        );
    }
    assert!(buttons.contains(&"Servo layout notes, Research, tab 1 of 10".to_string()));
    for chip in &rows.chips {
        assert!(buttons.contains(&chip.accessible_label.to_string()));
    }
    assert!(
        !buttons
            .iter()
            .any(|b| b == "Research, tab group, 10 tabs, key Option A"),
        "the planets are gone at level 2: {buttons:?}"
    );
    assert!(!buttons.contains(&"New group".to_string()));
    // 10 previews, 4 chips, the way back.
    assert_eq!(buttons.len(), 15, "{buttons:?}");
}

#[test]
fn level_two_buttons_do_what_a_click_does() {
    let (window, log, rows) = window(Frame::Group, false);
    let nodes = reading_order(&window);
    for card in &rows.previews {
        node_in(&nodes, Role::Button, &card.accessible_label).press();
        assert_eq!(take(&log), vec![format!("preview {}", card.tab)]);
    }
    for chip in &rows.chips {
        node_in(&nodes, Role::Button, &chip.accessible_label).press();
        assert_eq!(take(&log), vec![format!("chip {}", chip.group)]);
    }
}

#[test]
fn level_two_texts_are_the_breadcrumb_only() {
    let (window, _log, _rows) = window(Frame::Group, false);
    let nodes = reading_order(&window);
    assert_eq!(
        labels(&of_role(&nodes, Role::Text)),
        vec!["Orbit".to_string(), "Research".to_string()]
    );
}

// ---- level 3

#[test]
fn level_three_has_the_card_two_neighbours_and_no_planet_or_preview() {
    let (window, log, rows) = window(Frame::Tab, false);
    let nodes = reading_order(&window);
    let buttons = labels(&of_role(&nodes, Role::Button));
    assert!(
        buttons.contains(&"Open AccessKit guide".to_string()),
        "{buttons:?}"
    );
    assert_eq!(
        buttons.len(),
        4,
        "the card, two neighbours, the way back: {buttons:?}"
    );
    node_in(&nodes, Role::Button, "Open AccessKit guide").press();
    assert_eq!(take(&log), vec!["card"]);
    for near in &rows.neighbours {
        node_in(&nodes, Role::Button, &near.accessible_label).press();
        assert_eq!(take(&log), vec![format!("neighbour {}", near.tab)]);
    }
    assert_eq!(
        labels(&of_role(&nodes, Role::Text)),
        vec![
            "Orbit".to_string(),
            "Research".to_string(),
            "AccessKit guide".to_string()
        ]
    );
}

// ---- the new-group popover

#[test]
fn the_popover_is_a_labelled_group_with_a_field_and_six_colour_choices() {
    let (window, log, rows) = window(Frame::NewGroup, false);
    let nodes = reading_order(&window);
    let sheet = node_in(&nodes, Role::Groupbox, "New group");
    assert_eq!(sheet.rect().width, 230.0);
    let field = node_in(&nodes, Role::TextInput, "Group name");
    assert!(sheet.rect().contains(&field.rect()));
    let dots = of_role(&nodes, Role::RadioButton);
    assert_eq!(
        labels(&dots),
        ["blue", "amber", "violet", "rose", "green", "teal"]
    );
    assert_eq!(
        dots.iter().filter(|d| d.checked == Some(true)).count(),
        1,
        "exactly one colour is chosen"
    );
    let selected = usize::try_from(rows.popover.selected).expect("a colour is chosen");
    assert_eq!(dots[selected].checked, Some(true));
    node_in(&nodes, Role::RadioButton, "rose").press();
    assert_eq!(take(&log), vec!["colour 3"]);
    assert_eq!(rows.popover.colour_names.row_count(), 6);
}

#[test]
fn the_popover_field_is_empty_and_announces_its_placeholder() {
    let (window, _log, rows) = window(Frame::NewGroup, false);
    assert_eq!(
        rows.popover.name, "",
        "the scene no longer pre-fills a name"
    );
    let mut popover = window.get_popover();
    popover.placeholder = "Untitled group".into();
    popover.selected = 2;
    window.set_popover(popover);
    let nodes = reading_order(&window);
    let field = node_in(&nodes, Role::TextInput, "Group name");
    assert_eq!(field.value, "");
    assert_eq!(field.placeholder(), "Untitled group");
    let dots = of_role(&nodes, Role::RadioButton);
    assert_eq!(
        dots[2].checked,
        Some(true),
        "the dot at `selected` is chosen"
    );
    assert_eq!(dots.iter().filter(|d| d.checked == Some(true)).count(), 1);
}

#[test]
fn the_scene_popover_carries_the_placeholder_and_a_chosen_dot() {
    let (_window, _log, rows) = window(Frame::NewGroup, false);
    assert_eq!(rows.popover.placeholder, "Untitled group");
    assert!(
        rows.popover.selected >= 0,
        "an empty colour shows the one the controller will use"
    );
}

#[test]
fn without_a_popover_there_is_no_group_box() {
    let (window, _log, _rows) = window(Frame::Overview, false);
    let nodes = reading_order(&window);
    assert!(of_role(&nodes, Role::Groupbox).is_empty());
}

// ---- the drag frames

#[test]
fn the_drag_ghost_and_the_target_ring_are_decoration() {
    let (window, _log, _rows) = window(Frame::MoveTab, false);
    let nodes = reading_order(&window);
    assert!(
        !nodes.iter().any(|n| n.label.contains("Pasteis de nata")),
        "the ghost is not read"
    );
    assert_eq!(of_role(&nodes, Role::Button).len(), 7);
}

// ---- ring (needs the finished ring-bands)

#[test]
fn ring_tabs_exist_only_inside_the_rings() {
    let (window, _log, rows) = window(Frame::Overview, false);
    let nodes = reading_order(&window);
    let tabs = of_role(&nodes, Role::Tab);
    let expected: usize = rows.planets.iter().map(|p| p.tabs.row_count()).sum();
    assert_eq!(
        tabs.len(),
        expected,
        "every tab of every ring, nothing else"
    );
    assert_eq!(expected, 40);
    let planets: Vec<_> = of_role(&nodes, Role::Button)
        .into_iter()
        .filter(|b| b.label.contains("tab group"))
        .collect();
    for tab in tabs {
        assert!(
            planets.iter().any(|p| p.rect().contains(&tab.rect())),
            "the tab {:?} is outside every planet",
            tab.label
        );
    }
}

#[test]
fn ring_the_level_two_centre_ring_lists_the_tabs_of_the_group() {
    let (window, _log, _rows) = window(Frame::Group, false);
    let nodes = reading_order(&window);
    assert_eq!(of_role(&nodes, Role::Tab).len(), 10);
    node_in(&nodes, Role::List, "Research");
}
