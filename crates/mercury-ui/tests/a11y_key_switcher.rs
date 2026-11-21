// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
#![cfg(feature = "test-support")]
//! The "Switch group" card (`ui/groups/key-switcher.slint`, SPEC §4.7 and the §5 table row "Key
//! switcher") read the way an assistive technology reads it. For each state (at rest, a group
//! chosen, a tab chosen, no keyed groups, 26 groups, hidden) the reading order is checked
//! against the labelling rule and the bounds VoiceOver is given, and frozen in
//! `tests/a11y-snapshots/key-switcher-*.txt`; the controls are found by role and label and
//! pressed through their default action, which must do what a click does and nothing while the
//! card is hidden.
//!
//! The card is a list of tiles ("Switch group"), each one "Option A, Research, 5 tabs" (the word
//! VoiceOver users know for the Option key); a "Change keys" button; the first hint and, in the
//! place of the second one while a group is chosen, a list of that group's numbered tabs. The
//! title, the "⌥ held" chip, the halos, the rings, the marks and the keycap letters are not read:
//! a tile's label already says them.
//!
//! The fixtures build the tiles with `RingModel` as the card's Rust side does; they are kept
//! apart from `key_switcher_render.rs` because a test binary can have only one platform.
//!
//! `cargo test --release -p mercury-ui --features test-support --test a11y_key_switcher`; bless
//! the golden files with `MERCURY_A11Y_BLESS=1` in front of it.

mod common;

use std::cell::RefCell;
use std::rc::Rc;

use common::a11y::{self, Node, Rect};
use i_slint_backend_testing::AccessibleRole;
use mercury_ui::ring_geometry::slint_models::RingModel;
use mercury_ui::ring_geometry::{Point, RingSpec};
use mercury_ui::{GroupRow, KeyChip, KeyMetrics, KeyRingSeg, KeySwitcherPreview, KeyTile, TabRow};
use slint::{Color, ModelRc, VecModel};

use AccessibleRole as Role;

const WINDOW: (f32, f32) = (1440.0, 900.0);
const HINT_PRIMARY: &str =
    "Tap a letter for a group, then 1–9 for one of its tabs. Release ⌥ to go there.";
const HINT_SECONDARY: &str = "Opens after ⌥ is held for 0.4 s on its own, so quick ⌥ + letter still types å, ß, ∂ in text fields.";
const EMPTY_TEXT: &str = "No group has a key yet. Choose Change keys… to give each group a letter.";

// ---------------------------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------------------------

struct FixtureGroup {
    id: i32,
    key: String,
    name: String,
    tabs: Vec<(String, u32)>,
    active: Option<usize>,
}

fn hex(rgb: u32) -> Color {
    Color::from_rgb_u8((rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8)
}

fn group(
    id: i32,
    key: &str,
    name: &str,
    titles: &[&str],
    shades: &[u32],
    active: Option<usize>,
) -> FixtureGroup {
    FixtureGroup {
        id,
        key: key.to_owned(),
        name: name.to_owned(),
        tabs: titles
            .iter()
            .zip(shades)
            .map(|(title, shade)| ((*title).to_owned(), *shade))
            .collect(),
        active,
    }
}

/// The six groups of the mockup (the sixth, "Loose" there, is "Recipes": the model has no key
/// for the Loose pseudo-group).
fn mockup_groups() -> Vec<FixtureGroup> {
    vec![
        group(
            1,
            "A",
            "Research",
            &[
                "Servo layout notes",
                "WebRender notes",
                "Slint rendering",
                "AccessKit",
                "Forum thread",
            ],
            &[0x2655A0, 0x2B62BB, 0x2F6FD6, 0x598CDE, 0x82A9E6],
            Some(0),
        ),
        group(
            2,
            "S",
            "Lisbon trip",
            &["Flights", "Hotel", "Tram 28", "Pasteis"],
            &[0xA76E23, 0xCD8528, 0xE4A046, 0xECBD7F],
            None,
        ),
        group(
            3,
            "D",
            "Reading",
            &["Essay", "Paper", "Long read", "Blog", "Thread", "Notes"],
            &[0x60469B, 0x6C4FB0, 0x7857C5, 0x8868D4, 0x9D82DB, 0xB29DE3],
            None,
        ),
        group(
            4,
            "F",
            "Shopping",
            &["Shoes", "Lamp", "Cart"],
            &[0xA23954, 0xD9486E, 0xE891A8],
            None,
        ),
        group(
            5,
            "G",
            "Work",
            &["Tracker", "Mail"],
            &[0x26774B, 0x82C5A1],
            None,
        ),
        group(
            6,
            "H",
            "Recipes",
            &["Soup", "Bread"],
            &[0x5E6167, 0xB0B3B8],
            None,
        ),
    ]
}

fn many_groups(count: usize) -> Vec<FixtureGroup> {
    (0..count)
        .map(|index| {
            let letter = char::from(b'A' + index as u8).to_string();
            let tab_count = 1 + index % 4;
            let titles: Vec<String> = (1..=tab_count).map(|n| format!("Page {n}")).collect();
            let titles: Vec<&str> = titles.iter().map(String::as_str).collect();
            let shades: Vec<u32> = (0..tab_count)
                .map(|n| 0x335599 + 0x101010 * n as u32)
                .collect();
            group(
                index as i32 + 1,
                &letter,
                &format!("Group {letter}"),
                &titles,
                &shades,
                None,
            )
        })
        .collect()
}

struct Measures {
    tile_width: f32,
    tile_height: f32,
    ring_size: f32,
    ring_cx: f32,
    ring_cy: f32,
    ring_radius: f32,
    ring_band: f32,
    halo_outer: f32,
    halo_width: f32,
    key_size: f32,
    key_y: f32,
    name_y: f32,
    count_y: f32,
    gap_x: f32,
    gap_y: f32,
    columns: usize,
}

const FULL: Measures = Measures {
    tile_width: 108.0,
    tile_height: 168.0,
    ring_size: 48.0,
    ring_cx: 54.0,
    ring_cy: 46.0,
    ring_radius: 20.0,
    ring_band: 7.0,
    halo_outer: 47.5,
    halo_width: 22.0,
    key_size: 40.0,
    key_y: 88.0,
    name_y: 136.0,
    count_y: 152.0,
    gap_x: 10.0,
    gap_y: 10.0,
    columns: 6,
};

const COMPACT: Measures = Measures {
    tile_width: 72.0,
    tile_height: 134.0,
    ring_size: 34.0,
    ring_cx: 36.0,
    ring_cy: 33.0,
    ring_radius: 14.0,
    ring_band: 5.0,
    halo_outer: 33.0,
    halo_width: 15.0,
    key_size: 28.0,
    key_y: 70.0,
    name_y: 104.0,
    count_y: 118.0,
    gap_x: 6.0,
    gap_y: 8.0,
    columns: 9,
};

fn count_text(tabs: usize) -> String {
    if tabs == 1 {
        "1 tab".to_owned()
    } else {
        format!("{tabs} tabs")
    }
}

fn key_tile(group: &FixtureGroup, position: (f32, f32), m: &Measures) -> KeyTile {
    let group_row = GroupRow {
        id: group.id,
        name: group.name.as_str().into(),
        key_label: group.key.as_str().into(),
        count: group.tabs.len() as i32,
        ..Default::default()
    };
    let tabs: Vec<TabRow> = group
        .tabs
        .iter()
        .enumerate()
        .map(|(index, (title, shade))| TabRow {
            id: group.id * 1000 + index as i32,
            group: group.id,
            index_in_group: index as i32,
            title: title.as_str().into(),
            shade: hex(*shade),
            active: group.active == Some(index),
            ..Default::default()
        })
        .collect();
    let centre = m.ring_size / 2.0;
    let model = RingModel::new(
        RingSpec::planet(Point::new(centre, centre), m.ring_radius, m.ring_band),
        &[group_row],
        &tabs,
    );
    let arcs = model.segments_for_slint();
    let segments: Vec<KeyRingSeg> = arcs
        .iter()
        .enumerate()
        .map(|(flat, arc)| KeyRingSeg {
            commands: arc.commands.clone(),
            shade: tabs[model.tab_rows[flat]].shade,
        })
        .collect();
    let mark = group
        .active
        .and_then(|active| model.tab_rows.iter().position(|row| *row == active))
        .map(|flat| {
            (
                centre + (arcs[flat].mid_x - centre) * 0.525,
                centre + (arcs[flat].mid_y - centre) * 0.525,
            )
        });
    KeyTile {
        group: group.id,
        x: position.0,
        y: position.1,
        key_label: group.key.as_str().into(),
        name: group.name.as_str().into(),
        count_text: count_text(group.tabs.len()).into(),
        accessible_label: format!(
            "Option {}, {}, {}",
            group.key,
            group.name,
            count_text(group.tabs.len())
        )
        .into(),
        tint: hex(0xDAE5F8),
        segments: ModelRc::new(VecModel::from(segments)),
        has_mark: mark.is_some(),
        mark_x: mark.map_or(0.0, |m| m.0),
        mark_y: mark.map_or(0.0, |m| m.1),
    }
}

/// A scene: which groups, which are chosen, whether the card is shown.
struct Scene {
    groups: Vec<FixtureGroup>,
    selected_group: Option<i32>,
    selected_tab: Option<i32>,
    shown: bool,
}

impl Scene {
    fn new(groups: Vec<FixtureGroup>) -> Self {
        Self {
            groups,
            selected_group: None,
            selected_tab: None,
            shown: true,
        }
    }

    fn chosen(mut self, group: i32, tab: Option<i32>) -> Self {
        self.selected_group = Some(group);
        self.selected_tab = tab;
        self
    }

    fn hidden(mut self) -> Self {
        self.shown = false;
        self
    }
}

struct Rig {
    window: KeySwitcherPreview,
    log: Rc<RefCell<Vec<String>>>,
    measures: Measures,
    positions: Vec<(f32, f32)>,
    tiles_bottom: f32,
    card_height: f32,
}

fn open(scene: &Scene) -> Rig {
    a11y::init();
    let measures = if scene.groups.len() <= 6 {
        FULL
    } else {
        COMPACT
    };
    let positions: Vec<(f32, f32)> = (0..scene.groups.len())
        .map(|index| {
            (
                (index % measures.columns) as f32 * (measures.tile_width + measures.gap_x),
                (index / measures.columns) as f32 * (measures.tile_height + measures.gap_y),
            )
        })
        .collect();
    let rows = scene.groups.len().div_ceil(measures.columns);
    let tiles_bottom = 66.0
        + if rows == 0 {
            0.0
        } else {
            rows as f32 * measures.tile_height + (rows - 1) as f32 * measures.gap_y
        };
    let card_height = if scene.groups.is_empty() {
        200.0
    } else {
        tiles_bottom + 96.0
    };

    let window = KeySwitcherPreview::new().expect("a window");
    let tiles: Vec<KeyTile> = scene
        .groups
        .iter()
        .zip(&positions)
        .map(|(group, position)| key_tile(group, *position, &measures))
        .collect();
    let chosen = scene
        .selected_group
        .and_then(|id| scene.groups.iter().find(|group| group.id == id));
    let chips: Vec<KeyChip> = chosen
        .map(|group| {
            group
                .tabs
                .iter()
                .take(9)
                .enumerate()
                .map(|(index, (title, shade))| KeyChip {
                    index: index as i32 + 1,
                    title: title.as_str().into(),
                    shade: hex(*shade),
                    accessible_label: format!("Tab {}, {title}", index + 1).into(),
                })
                .collect()
        })
        .unwrap_or_default();
    window.set_reduce_motion(true);
    window.set_tiles(ModelRc::new(VecModel::from(tiles)));
    window.set_chips(ModelRc::new(VecModel::from(chips)));
    window.set_metrics(KeyMetrics {
        tile_width: measures.tile_width,
        tile_height: measures.tile_height,
        ring_size: measures.ring_size,
        ring_cx: measures.ring_cx,
        ring_cy: measures.ring_cy,
        halo_outer: measures.halo_outer,
        halo_width: measures.halo_width,
        key_size: measures.key_size,
        key_y: measures.key_y,
        key_radius: 9.0,
        key_font: 20.0,
        name_y: measures.name_y,
        name_font: 12.5,
        count_y: measures.count_y,
        count_font: 10.5,
    });
    window.set_selected_group(scene.selected_group.unwrap_or(-1));
    window.set_selected_tab(scene.selected_tab.unwrap_or(-1));
    window.set_chosen_name(chosen.map_or("", |group| group.name.as_str()).into());
    window.set_hint_primary(HINT_PRIMARY.into());
    window.set_hint_secondary(HINT_SECONDARY.into());
    window.set_empty_text(EMPTY_TEXT.into());
    window.set_card_width(760.0);
    window.set_card_height(card_height);
    window.set_tiles_left(30.0);
    window.set_tiles_top(66.0);
    window.set_shown(scene.shown);

    let log = Rc::new(RefCell::new(Vec::new()));
    window.on_tile_clicked({
        let log = Rc::clone(&log);
        move |group| log.borrow_mut().push(format!("tile:{group}"))
    });
    window.on_chip_clicked({
        let log = Rc::clone(&log);
        move |digit| log.borrow_mut().push(format!("chip:{digit}"))
    });
    window.on_change_keys({
        let log = Rc::clone(&log);
        move || log.borrow_mut().push("change-keys".to_owned())
    });
    window.on_dismissed({
        let log = Rc::clone(&log);
        move || log.borrow_mut().push("dismissed".to_owned())
    });
    a11y::open_window(&window, WINDOW.0, WINDOW.1);
    Rig {
        window,
        log,
        measures,
        positions,
        tiles_bottom,
        card_height,
    }
}

impl Rig {
    fn nodes(&self) -> Vec<Node> {
        a11y::reading_order(&self.window)
    }

    fn card(&self) -> (f32, f32) {
        (
            ((WINDOW.0 - 760.0) / 2.0).round(),
            ((WINDOW.1 - self.card_height) / 2.0).round(),
        )
    }

    fn take_log(&self) -> Vec<String> {
        std::mem::take(&mut *self.log.borrow_mut())
    }

    /// The rectangle of tile `index` in the window.
    fn tile_rect(&self, index: usize) -> Rect {
        let (card_x, card_y) = self.card();
        let (x, y) = self.positions[index];
        Rect::new(
            card_x + 30.0 + x,
            card_y + 66.0 + y,
            self.measures.tile_width,
            self.measures.tile_height,
        )
    }

    /// The rectangle of chip `index` in the window.
    fn chip_rect(&self, index: usize) -> Rect {
        let (card_x, card_y) = self.card();
        Rect::new(
            card_x + 30.0 + index as f32 * 78.0,
            card_y + self.tiles_bottom + 42.0,
            72.0,
            24.0,
        )
    }
}

/// Reads the window the way VoiceOver does, checks the labelling rule and the bounds, and
/// compares the reading order with `tests/a11y-snapshots/<snapshot>.txt`.
fn freeze(rig: &Rig, snapshot: &str) -> Vec<Node> {
    let nodes = rig.nodes();
    a11y::assert_labelled(&nodes, snapshot);
    a11y::assert_bounds(&nodes, a11y::window_size(&rig.window));
    a11y::assert_snapshot(snapshot, &nodes);
    nodes
}

fn labels(nodes: &[Node], role: Role) -> Vec<String> {
    nodes
        .iter()
        .filter(|node| node.role == role)
        .map(|node| node.label.clone())
        .collect()
}

/// The words of a tile as a screen reader says them.
const TILE_LABELS: [&str; 6] = [
    "Option A, Research, 5 tabs",
    "Option S, Lisbon trip, 4 tabs",
    "Option D, Reading, 6 tabs",
    "Option F, Shopping, 3 tabs",
    "Option G, Work, 2 tabs",
    "Option H, Recipes, 2 tabs",
];

// ---------------------------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------------------------

#[test]
fn at_rest_the_card_reads_the_link_the_tiles_and_the_first_hint_then_the_second() {
    let rig = open(&Scene::new(mockup_groups()));

    let nodes = freeze(&rig, "key-switcher-rest");

    // One list of six tiles, labelled "Switch group"; each tile says its key, group and tabs.
    let list = a11y::node_in(&nodes, Role::List, "Switch group");
    assert_eq!(labels(&nodes, Role::ListItem), TILE_LABELS);
    for (index, label) in TILE_LABELS.iter().enumerate() {
        let tile = a11y::node_in(&nodes, Role::ListItem, label);
        assert_eq!(tile.item, Some((index, 6)), "{label}");
        assert_eq!(tile.selected, Some(false), "{label}: nothing is chosen yet");
        assert!(tile.enabled);
        a11y::assert_rect(tile, rig.tile_rect(index));
        a11y::assert_inside(tile, list);
    }
    // The link is a button named without its ellipsis.
    let link = a11y::node_in(&nodes, Role::Button, "Change keys");
    assert_eq!(labels(&nodes, Role::Button), ["Change keys"]);
    assert!(link.rect().width > 40.0 && link.rect().height > 16.0);
    // The two hints are readable text, in this order, and nothing else is: the title, the held
    // chip and everything inside a tile stay silent.
    assert_eq!(labels(&nodes, Role::Text), [HINT_PRIMARY, HINT_SECONDARY]);
    assert!(
        nodes
            .iter()
            .all(|node| node.role != Role::Image && node.label != "⌥ held"),
        "no image, no held chip"
    );
    assert!(
        a11y::reading_order(&rig.window)
            .iter()
            .all(|node| node.label != "Switch group" || node.role == Role::List),
        "the title is not read as well as the list"
    );
    // No chips while no group is chosen.
    assert!(labels(&nodes, Role::List) == ["Switch group"]);
}

#[test]
fn a_chosen_group_reads_selected_and_its_numbered_tabs_replace_the_second_hint() {
    let rig = open(&Scene::new(mockup_groups()).chosen(2, None));

    let nodes = freeze(&rig, "key-switcher-group-chosen");

    let selected: Vec<(String, Option<bool>)> = nodes
        .iter()
        .filter(|node| node.label.starts_with("Option "))
        .map(|node| (node.label.clone(), node.selected))
        .collect();
    let chosen: Vec<&str> = selected
        .iter()
        .filter(|(_, selected)| *selected == Some(true))
        .map(|(label, _)| label.as_str())
        .collect();
    assert_eq!(chosen, ["Option S, Lisbon trip, 4 tabs"], "{selected:?}");
    // The chosen group's tabs, as a list named after it, one numbered item per tab.
    let tabs = a11y::node_in(&nodes, Role::List, "Tabs of Lisbon trip");
    let chip_labels = [
        "Tab 1, Flights",
        "Tab 2, Hotel",
        "Tab 3, Tram 28",
        "Tab 4, Pasteis",
    ];
    assert_eq!(
        nodes
            .iter()
            .filter(|node| node.label.starts_with("Tab "))
            .map(|node| node.label.as_str())
            .collect::<Vec<_>>(),
        chip_labels
    );
    for (index, label) in chip_labels.iter().enumerate() {
        let chip = a11y::node_in(&nodes, Role::ListItem, label);
        assert_eq!(chip.item, Some((index, 4)), "{label}");
        assert_eq!(chip.selected, Some(false), "{label}: no tab is chosen yet");
        a11y::assert_rect(chip, rig.chip_rect(index));
        a11y::assert_inside(chip, tabs);
    }
    // Only the first hint remains: the second is replaced by the chips.
    assert_eq!(labels(&nodes, Role::Text), [HINT_PRIMARY]);
}

#[test]
fn a_chosen_tab_reads_selected_among_the_tabs() {
    let rig = open(&Scene::new(mockup_groups()).chosen(1, Some(3)));

    let nodes = freeze(&rig, "key-switcher-tab-chosen");

    let selected: Vec<&str> = nodes
        .iter()
        .filter(|node| node.label.starts_with("Tab ") && node.selected == Some(true))
        .map(|node| node.label.as_str())
        .collect();
    assert_eq!(selected, ["Tab 3, Slint rendering"]);
    let tabs = a11y::node_in(&nodes, Role::List, "Tabs of Research");
    assert_eq!(
        nodes
            .iter()
            .filter(|node| node.label.starts_with("Tab "))
            .count(),
        5
    );
    a11y::assert_inside(
        a11y::node_in(&nodes, Role::ListItem, "Tab 5, Forum thread"),
        tabs,
    );
}

#[test]
fn with_no_keyed_group_the_card_reads_the_link_and_the_empty_text() {
    let rig = open(&Scene::new(Vec::new()));

    let nodes = freeze(&rig, "key-switcher-empty");

    assert_eq!(labels(&nodes, Role::Button), ["Change keys"]);
    assert_eq!(labels(&nodes, Role::Text), [EMPTY_TEXT]);
    assert!(
        labels(&nodes, Role::List).is_empty(),
        "no empty list of tiles"
    );
    assert!(labels(&nodes, Role::ListItem).is_empty());
}

#[test]
fn twenty_six_groups_read_as_one_list_of_compact_tiles_inside_the_window() {
    let rig = open(&Scene::new(many_groups(26)));

    let nodes = freeze(&rig, "key-switcher-26-tiles");

    let items: Vec<&Node> = nodes
        .iter()
        .filter(|node| node.role == Role::ListItem)
        .collect();
    assert_eq!(items.len(), 26);
    let list = a11y::node_in(&nodes, Role::List, "Switch group");
    for (index, item) in items.iter().enumerate() {
        assert_eq!(item.item, Some((index, 26)));
        a11y::assert_rect(item, rig.tile_rect(index));
        a11y::assert_inside(item, list);
    }
    assert_eq!(items[0].label, "Option A, Group A, 1 tab");
    assert_eq!(items[25].label, "Option Z, Group Z, 2 tabs");
    // The card (580 px) lies inside the 900 px window, and so does everything read in it.
    assert_eq!(rig.card(), (340.0, 160.0));
    assert!(
        Rect::new(0.0, 0.0, WINDOW.0, WINDOW.1).contains(&Rect::new(340.0, 160.0, 760.0, 580.0))
    );
}

#[test]
fn a_hidden_card_is_not_read_at_all() {
    let rig = open(&Scene::new(mockup_groups()).chosen(1, Some(2)).hidden());

    let nodes = freeze(&rig, "key-switcher-hidden");

    assert!(
        nodes.is_empty(),
        "nothing is read while the card is hidden: {}",
        a11y::dump(&nodes)
    );
    // Shown, it is read; hidden again, it is gone at once.
    rig.window.set_shown(true);
    assert!(!rig.nodes().is_empty());
    rig.window.set_shown(false);
    assert!(rig.nodes().is_empty());
    assert!(rig.take_log().is_empty());
}

#[test]
fn pressing_a_tile_does_what_a_click_on_it_does() {
    let rig = open(&Scene::new(mockup_groups()));
    for (index, label) in TILE_LABELS.iter().enumerate() {
        a11y::control(&rig.window, Role::ListItem, label).invoke_accessible_default_action();
        assert_eq!(rig.take_log(), [format!("tile:{}", index + 1)], "{label}");
    }
    // Twenty-six compact tiles: the last one too.
    let rig = open(&Scene::new(many_groups(26)));
    a11y::control(&rig.window, Role::ListItem, "Option Z, Group Z, 2 tabs")
        .invoke_accessible_default_action();
    assert_eq!(rig.take_log(), ["tile:26"]);
}

#[test]
fn pressing_a_tab_or_change_keys_does_what_a_click_on_it_does() {
    let rig = open(&Scene::new(mockup_groups()).chosen(1, None));
    let chips = [
        "Tab 1, Servo layout notes",
        "Tab 2, WebRender notes",
        "Tab 3, Slint rendering",
        "Tab 4, AccessKit",
        "Tab 5, Forum thread",
    ];
    for (index, label) in chips.iter().enumerate() {
        a11y::control(&rig.window, Role::ListItem, label).invoke_accessible_default_action();
        assert_eq!(rig.take_log(), [format!("chip:{}", index + 1)], "{label}");
    }
    a11y::control(&rig.window, Role::Button, "Change keys").invoke_accessible_default_action();
    assert_eq!(rig.take_log(), ["change-keys"]);
    // Pressing does not close the card (the state machine does that when it hears the event).
    assert!(!rig.nodes().is_empty());
    assert!(rig.take_log().is_empty());
}

#[test]
fn the_controls_are_found_by_role_and_label_in_every_shown_state() {
    let states = [
        ("at rest", Scene::new(mockup_groups())),
        (
            "a group chosen",
            Scene::new(mockup_groups()).chosen(3, None),
        ),
        (
            "a tab chosen",
            Scene::new(mockup_groups()).chosen(3, Some(6)),
        ),
        ("no keyed group", Scene::new(Vec::new())),
        ("26 groups", Scene::new(many_groups(26))),
    ];
    for (what, scene) in states {
        let rig = open(&scene);
        let nodes = rig.nodes();
        a11y::assert_labelled(&nodes, what);
        a11y::assert_bounds(&nodes, a11y::window_size(&rig.window));
        // The link is always there, found with the helpers the other tests use.
        let link = a11y::control(&rig.window, Role::Button, "Change keys");
        assert!(
            link.accessible_label()
                .is_some_and(|label| label == "Change keys"),
            "{what}"
        );
        if scene.groups.is_empty() {
            continue;
        }
        // Every tile is a selectable list item, and exactly the chosen group's is selected.
        let tiles: Vec<&Node> = nodes
            .iter()
            .filter(|node| node.role == Role::ListItem && node.label.starts_with("Option "))
            .collect();
        assert_eq!(tiles.len(), scene.groups.len(), "{what}");
        for (tile, group) in tiles.iter().zip(&scene.groups) {
            assert_eq!(
                tile.selected,
                Some(scene.selected_group == Some(group.id)),
                "{what}: {}",
                tile.label
            );
        }
        // The chosen tab, if any, is the only selected chip.
        for chip in nodes.iter().filter(|node| node.label.starts_with("Tab ")) {
            let digit: i32 = chip.label[4..5].parse().expect("a digit");
            assert_eq!(
                chip.selected,
                Some(scene.selected_tab == Some(digit)),
                "{what}: {}",
                chip.label
            );
        }
    }
}
