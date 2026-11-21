// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! The "Switch group" card's models without a window (lane brief §6 item 3): `build_card` on the
//! six groups of the mockup (`keys-overlay-light.svg`), the limits (26 tiles, 40 ring segments,
//! 9 chips) and the inputs that must not break it (no groups, 500 tabs, long names, CJK, emoji,
//! right-to-left scripts, odd numbers).

use mercury_ui::key_switcher::{
    CardInput, CardModel, HintMode, TileSize, build_card, card_layout, count_text, empty_text,
    hint_primary, hint_secondary, tile_label,
};
use mercury_ui::ring_geometry::slint_models::RingModel;
use mercury_ui::ring_geometry::{Point, RingSpec};
use mercury_ui::{GroupRow, KeyChip, KeyRingSeg, KeyTile, TabRow};
use slint::{Color, Model};

// ---- the mockup's six groups (brief §3)

struct Mock {
    id: i32,
    key: &'static str,
    name: &'static str,
    tint: u32,
    shades: &'static [u32],
    titles: &'static [&'static str],
}

const MOCKUP: [Mock; 6] = [
    Mock {
        id: 1,
        key: "A",
        name: "Research",
        tint: 0xDAE5F8,
        shades: &[0x2655A0, 0x2B62BB, 0x2F6FD6, 0x598CDE, 0x82A9E6],
        titles: &[
            "Servo book",
            "WebRender notes",
            "Slint reference",
            "AccessKit docs",
            "Forum: layout questions",
        ],
    },
    Mock {
        id: 2,
        key: "S",
        name: "Lisbon trip",
        tint: 0xF9EBD9,
        shades: &[0xA76E23, 0xCD8528, 0xE4A046, 0xECBD7F],
        titles: &[
            "Flights to Lisbon",
            "Hostel in Alfama",
            "Tram 28 timetable",
            "Pastéis de nata",
        ],
    },
    Mock {
        id: 3,
        key: "D",
        name: "Reading",
        tint: 0xE8E1F7,
        shades: &[0x60469B, 0x6C4FB0, 0x7857C5, 0x8868D4, 0x9D82DB, 0xB29DE3],
        titles: &[
            "Longreads",
            "An essay on type",
            "Paper: CRDTs",
            "Newsletter",
            "Poem of the day",
            "Review: Dune",
        ],
    },
    Mock {
        id: 4,
        key: "F",
        name: "Shopping",
        tint: 0xF8DEE5,
        shades: &[0xA23954, 0xD9486E, 0xE891A8],
        titles: &["Headphones", "Coffee grinder", "Running shoes"],
    },
    Mock {
        id: 5,
        key: "G",
        name: "Work",
        tint: 0xD9EEE3,
        shades: &[0x26774B, 0x82C5A1],
        titles: &["Sprint board", "Design review"],
    },
    Mock {
        // The mockup calls it "Loose" with H; the model has no key for the Loose pseudo-group.
        id: 6,
        key: "H",
        name: "Recipes",
        tint: 0xE7E8EA,
        shades: &[0x5E6167, 0xB0B3B8],
        titles: &["Pasta e fagioli", "Sourdough"],
    },
];

fn colour(hex: u32) -> Color {
    Color::from_rgb_u8((hex >> 16) as u8, (hex >> 8) as u8, hex as u8)
}

/// The rows of the six mockup groups. Research's first tab is the active one; no other group has
/// an active tab. Tab ids are `group * 100 + digit`.
fn mockup() -> (Vec<GroupRow>, Vec<TabRow>) {
    let mut groups = Vec::new();
    let mut tabs = Vec::new();
    for mock in &MOCKUP {
        groups.push(GroupRow {
            id: mock.id,
            name: mock.name.into(),
            tint: colour(mock.tint),
            key_label: mock.key.into(),
            count: mock.titles.len() as i32,
            ..Default::default()
        });
        for (index, (&shade, &title)) in mock.shades.iter().zip(mock.titles).enumerate() {
            tabs.push(TabRow {
                id: mock.id * 100 + index as i32 + 1,
                group: mock.id,
                index_in_group: index as i32,
                title: title.into(),
                shade: colour(shade),
                active: mock.id == 1 && index == 0,
                ..Default::default()
            });
        }
    }
    (groups, tabs)
}

fn card(
    groups: &[GroupRow],
    tabs: &[TabRow],
    selected_group: Option<i32>,
    selected_tab: Option<u8>,
) -> CardModel {
    build_card(&CardInput {
        groups,
        tabs,
        hold_ms: 400,
        mode: HintMode::OutsideTextOrHold,
        selected_group,
        selected_tab,
    })
}

fn at_rest(groups: &[GroupRow], tabs: &[TabRow]) -> CardModel {
    card(groups, tabs, None, None)
}

fn segments(tile: &KeyTile) -> Vec<KeyRingSeg> {
    tile.segments.iter().collect()
}

/// A plain keyed group `id` with `key`.
fn keyed(id: i32, key: &str, name: &str) -> GroupRow {
    GroupRow {
        id,
        name: name.into(),
        key_label: key.into(),
        ..Default::default()
    }
}

fn tab(id: i32, group: i32, index_in_group: i32) -> TabRow {
    TabRow {
        id,
        group,
        index_in_group,
        title: format!("Tab {id}").into(),
        ..Default::default()
    }
}

/// The key labels of the tiles, in order.
fn labels(model: &CardModel) -> Vec<String> {
    model
        .tiles
        .iter()
        .map(|t| t.key_label.to_string())
        .collect()
}

/// The end points of every command of a ring path (`M x y`, `L x y`, `A rx ry rot large sweep x
/// y`); panics on anything else, which also catches `NaN` and `inf`.
fn path_points(commands: &str) -> Vec<(f32, f32)> {
    let spaced: String = commands
        .chars()
        .flat_map(|c| {
            if c.is_ascii_alphabetic() {
                vec![' ', c, ' ']
            } else {
                vec![c]
            }
        })
        .collect();
    let mut tokens = spaced.split_whitespace();
    let number = |tokens: &mut std::str::SplitWhitespace| -> f32 {
        let value: f32 = tokens
            .next()
            .unwrap_or_else(|| panic!("a number is missing in {commands:?}"))
            .parse()
            .unwrap_or_else(|_| panic!("not a number in {commands:?}"));
        assert!(value.is_finite(), "{value} in {commands:?}");
        value
    };
    let mut points = Vec::new();
    while let Some(token) = tokens.next() {
        match token {
            "M" | "L" => points.push((number(&mut tokens), number(&mut tokens))),
            "A" => {
                for _ in 0..5 {
                    number(&mut tokens);
                }
                points.push((number(&mut tokens), number(&mut tokens)));
            }
            "Z" => {}
            other => panic!("unexpected {other:?} in {commands:?}"),
        }
    }
    points
}

/// Every segment of every tile parses and lies in the ring's box.
fn assert_rings_are_well_formed(model: &CardModel) {
    let side = model.layout.metrics.ring_size;
    for tile in &model.tiles {
        for seg in segments(tile) {
            let commands = seg.commands.as_str();
            assert!(commands.starts_with('M'), "{commands:?}");
            assert!(commands.ends_with('Z'), "{commands:?}");
            assert!(
                !commands.contains("NaN") && !commands.contains("inf"),
                "{commands:?}"
            );
            for (x, y) in path_points(commands) {
                assert!(
                    (-0.01..=side + 0.01).contains(&x) && (-0.01..=side + 0.01).contains(&y),
                    "({x}, {y}) outside the {side} px ring box in {commands:?}"
                );
            }
        }
        if tile.has_mark {
            assert!(tile.mark_x.is_finite() && tile.mark_y.is_finite());
        }
    }
}

// ---- the six mockup groups

#[test]
fn the_six_mockup_groups_become_six_tiles_in_order() {
    let (groups, tabs) = mockup();
    let model = at_rest(&groups, &tabs);
    assert_eq!(model.tiles.len(), 6);
    assert_eq!(labels(&model), ["A", "S", "D", "F", "G", "H"]);
    let ids: Vec<i32> = model.tiles.iter().map(|t| t.group).collect();
    assert_eq!(ids, [1, 2, 3, 4, 5, 6]);
    for (tile, mock) in model.tiles.iter().zip(&MOCKUP) {
        assert_eq!(tile.name.as_str(), mock.name);
        assert_eq!(tile.tint, colour(mock.tint), "{}", mock.name);
        assert_eq!(
            tile.count_text.as_str(),
            count_text(mock.titles.len()),
            "{}",
            mock.name
        );
        assert_eq!(
            tile.accessible_label.as_str(),
            tile_label(mock.key, mock.name, mock.titles.len())
        );
    }
    assert_eq!(model.tiles[0].count_text.as_str(), "5 tabs");
    assert_eq!(
        model.tiles[0].accessible_label.as_str(),
        "Option A, Research, 5 tabs"
    );
    assert_eq!(
        model.tiles[1].accessible_label.as_str(),
        "Option S, Lisbon trip, 4 tabs"
    );
}

#[test]
fn at_rest_nothing_is_chosen_and_the_hints_are_the_mockups() {
    let (groups, tabs) = mockup();
    let model = at_rest(&groups, &tabs);
    assert_eq!((model.selected_group, model.selected_tab), (-1, -1));
    assert!(model.chips.is_empty());
    assert_eq!(model.chosen_name.as_str(), "");
    assert_eq!(model.hint_primary, hint_primary());
    assert_eq!(
        model.hint_secondary,
        "Opens after ⌥ is held for 0.4 s on its own, so quick ⌥ + letter still types å, ß, ∂ in text fields."
    );
    assert_eq!(model.empty_text, empty_text());
}

#[test]
fn the_layout_and_the_tile_positions_are_the_mockups() {
    let (groups, tabs) = mockup();
    let model = at_rest(&groups, &tabs);
    assert_eq!(model.layout, card_layout(6));
    assert_eq!(model.layout.size, TileSize::Full);
    assert_eq!(model.layout.card_height, 330.0);
    // A tile's x and y are from the tile area's top-left (frozen note of `KeyTile`): the card's
    // `tiles-left` and `tiles-top` are added by the component.
    for (index, tile) in model.tiles.iter().enumerate() {
        assert_eq!(tile.x, index as f32 * 118.0, "tile {index}");
        assert_eq!(tile.y, 0.0, "tile {index}");
        let (px, py) = model.layout.positions[index];
        assert_eq!(tile.x + model.layout.tiles_left, px);
        assert_eq!(tile.y + model.layout.tiles_top, py);
    }
}

#[test]
fn the_metrics_for_slint_are_the_layouts_metrics() {
    for count in [6usize, 9] {
        let groups: Vec<GroupRow> = (0..count)
            .map(|i| keyed(i as i32 + 1, &format!("{}", (b'A' + i as u8) as char), "G"))
            .collect();
        let model = at_rest(&groups, &[]);
        let want = model.layout.metrics;
        let got = &model.metrics;
        assert_eq!(model.layout, card_layout(count));
        assert_eq!(got.tile_width, want.tile_width);
        assert_eq!(got.tile_height, want.tile_height);
        assert_eq!(got.ring_size, want.ring_size);
        assert_eq!((got.ring_cx, got.ring_cy), (want.ring_cx, want.ring_cy));
        assert_eq!(
            (got.halo_outer, got.halo_width),
            (want.halo_outer, want.halo_width)
        );
        assert_eq!((got.key_size, got.key_y), (want.key_size, want.key_y));
        assert_eq!(
            (got.key_radius, got.key_font),
            (want.key_radius, want.key_font)
        );
        assert_eq!((got.name_y, got.name_font), (want.name_y, want.name_font));
        assert_eq!(
            (got.count_y, got.count_font),
            (want.count_y, want.count_font)
        );
    }
    // Six groups fit full tiles, seven or more take compact ones.
    let full = at_rest(&[keyed(1, "A", "One")], &[]).metrics;
    assert_eq!(
        (full.tile_width, full.tile_height, full.key_size),
        (108.0, 168.0, 40.0)
    );
    let groups: Vec<GroupRow> = (0..7)
        .map(|i| keyed(i + 1, &format!("L{i}"), "G"))
        .collect();
    let compact = at_rest(&groups, &[]).metrics;
    assert_eq!(
        (compact.tile_width, compact.tile_height, compact.key_size),
        (72.0, 134.0, 28.0)
    );
}

// ---- the rings

#[test]
fn each_ring_has_one_segment_per_tab_in_the_tabs_shades() {
    let (groups, tabs) = mockup();
    let model = at_rest(&groups, &tabs);
    let counts: Vec<usize> = model.tiles.iter().map(|t| segments(t).len()).collect();
    assert_eq!(counts, [5, 4, 6, 3, 2, 2]);
    for (tile, mock) in model.tiles.iter().zip(&MOCKUP) {
        let shades: Vec<Color> = segments(tile).iter().map(|s| s.shade).collect();
        let want: Vec<Color> = mock.shades.iter().map(|&hex| colour(hex)).collect();
        assert_eq!(shades, want, "{}", mock.name);
    }
}

#[test]
fn the_rings_are_well_formed_paths_inside_the_ring_box() {
    let (groups, tabs) = mockup();
    assert_rings_are_well_formed(&at_rest(&groups, &tabs));
}

#[test]
fn a_tiles_ring_is_the_planet_ring_of_the_ring_geometry() {
    let (groups, tabs) = mockup();
    let model = at_rest(&groups, &tabs);
    let m = model.layout.metrics;
    let half = m.ring_size / 2.0;
    let spec = RingSpec::planet(Point::new(half, half), m.ring_radius, m.ring_band);
    assert_eq!((m.ring_radius, m.ring_band), (20.0, 7.0));
    for (index, mock) in MOCKUP.iter().enumerate() {
        let rows: Vec<TabRow> = tabs
            .iter()
            .filter(|t| t.group == mock.id)
            .cloned()
            .collect();
        let ring = RingModel::new(spec, std::slice::from_ref(&groups[index]), &rows);
        let want: Vec<String> = ring
            .segments_for_slint()
            .iter()
            .map(|s| s.commands.to_string())
            .collect();
        let got: Vec<String> = segments(&model.tiles[index])
            .iter()
            .map(|s| s.commands.to_string())
            .collect();
        assert_eq!(got, want, "{}", mock.name);
    }
}

#[test]
fn tabs_of_other_groups_do_not_reach_a_ring() {
    let groups = [keyed(1, "A", "One"), keyed(2, "S", "Two")];
    let tabs = [
        tab(1, 1, 0),
        tab(2, 2, 0),
        tab(3, 2, 1),
        tab(4, 99, 0),
        tab(5, 1, 1),
    ];
    let model = at_rest(&groups, &tabs);
    assert_eq!(segments(&model.tiles[0]).len(), 2);
    assert_eq!(segments(&model.tiles[1]).len(), 2);
}

#[test]
fn a_group_without_tabs_has_an_empty_ring_and_the_count_of_its_row() {
    let mut group = keyed(1, "A", "Empty");
    group.count = 3;
    let model = at_rest(&[group], &[]);
    assert!(segments(&model.tiles[0]).is_empty());
    assert!(!model.tiles[0].has_mark);
    assert_eq!(model.tiles[0].count_text.as_str(), "3 tabs");
}

#[test]
fn a_ring_follows_index_in_group_not_the_order_of_the_rows() {
    let groups = [keyed(1, "A", "One")];
    let mut tabs = [tab(1, 1, 2), tab(2, 1, 0), tab(3, 1, 1)];
    tabs[0].shade = colour(0x0000FF);
    tabs[1].shade = colour(0xFF0000);
    tabs[2].shade = colour(0x00FF00);
    let model = at_rest(&groups, &tabs);
    let shades: Vec<Color> = segments(&model.tiles[0]).iter().map(|s| s.shade).collect();
    assert_eq!(
        shades,
        [colour(0xFF0000), colour(0x00FF00), colour(0x0000FF)]
    );
}

// ---- the active mark

#[test]
fn only_the_group_with_an_active_tab_has_a_mark() {
    let (groups, tabs) = mockup();
    let model = at_rest(&groups, &tabs);
    let marks: Vec<bool> = model.tiles.iter().map(|t| t.has_mark).collect();
    assert_eq!(marks, [true, false, false, false, false, false]);
}

#[test]
fn the_mark_is_10_and_a_half_px_from_the_ring_centre_on_the_active_segments_side() {
    let (groups, tabs) = mockup();
    let model = at_rest(&groups, &tabs);
    let tile = &model.tiles[0];
    let m = model.layout.metrics;
    // Five tabs: the first one's middle is at 36 degrees (the mockup's dot is at +6.2, -8.5).
    let (dx, dy) = (tile.mark_x - m.ring_cx, tile.mark_y - m.ring_cy);
    assert!(
        (dx - 6.2).abs() < 0.1 && (dy + 8.5).abs() < 0.1,
        "({dx}, {dy})"
    );
    assert!((dx.hypot(dy) - 10.5).abs() < 0.05);
    // In tile coordinates: the ring centre is (54, 46), so the dot is at about (60.2, 37.5).
    assert!((tile.mark_x - 60.2).abs() < 0.1 && (tile.mark_y - 37.5).abs() < 0.1);
}

#[test]
fn the_mark_follows_the_active_tab() {
    let (groups, mut tabs) = mockup();
    for tab in &mut tabs {
        tab.active = tab.group == 3 && tab.index_in_group == 2;
    }
    let model = at_rest(&groups, &tabs);
    let marks: Vec<bool> = model.tiles.iter().map(|t| t.has_mark).collect();
    assert_eq!(marks, [false, false, true, false, false, false]);
    // Reading has six tabs of 60 degrees: the third one's middle is at 150 degrees.
    let m = model.layout.metrics;
    let tile = &model.tiles[2];
    let rad = 150.0_f32.to_radians();
    let want = (m.ring_cx + 10.5 * rad.sin(), m.ring_cy - 10.5 * rad.cos());
    assert!((tile.mark_x - want.0).abs() < 0.1 && (tile.mark_y - want.1).abs() < 0.1);
}

#[test]
fn the_mark_stays_inside_the_ring_for_compact_tiles_too() {
    let groups: Vec<GroupRow> = (0..9)
        .map(|i| keyed(i + 1, &format!("{}", (b'A' + i as u8) as char), "G"))
        .collect();
    let mut tabs: Vec<TabRow> = (0..9).map(|i| tab(i + 1, i + 1, 0)).collect();
    for tab in &mut tabs {
        tab.active = true;
    }
    let model = at_rest(&groups, &tabs);
    let m = model.layout.metrics;
    for tile in &model.tiles {
        assert!(tile.has_mark);
        let distance = (tile.mark_x - m.ring_cx).hypot(tile.mark_y - m.ring_cy);
        assert!((distance - 14.0 * 0.525).abs() < 0.05, "{distance}");
    }
}

// ---- the chosen group, the chips and the chosen tab

#[test]
fn choosing_lisbon_shows_its_four_numbered_chips() {
    let (groups, tabs) = mockup();
    let model = card(&groups, &tabs, Some(2), None);
    assert_eq!(model.selected_group, 2);
    assert_eq!(model.selected_tab, -1);
    assert_eq!(model.chosen_name.as_str(), "Lisbon trip");
    let chips: Vec<KeyChip> = model.chips.clone();
    assert_eq!(chips.len(), 4);
    let mock = &MOCKUP[1];
    for (index, chip) in chips.iter().enumerate() {
        assert_eq!(chip.index, index as i32 + 1);
        assert_eq!(chip.title.as_str(), mock.titles[index]);
        assert_eq!(chip.shade, colour(mock.shades[index]));
        assert_eq!(
            chip.accessible_label.as_str(),
            format!("Tab {}, {}", index + 1, mock.titles[index])
        );
    }
    // The tiles stay what they were.
    assert_eq!(model.tiles.len(), 6);
    assert_eq!(model.hint_primary, hint_primary());
}

#[test]
fn a_chip_reads_tab_number_comma_title() {
    let (groups, tabs) = mockup();
    let model = card(&groups, &tabs, Some(1), None);
    assert_eq!(model.chips.len(), 5);
    assert_eq!(
        model.chips[1].accessible_label.as_str(),
        "Tab 2, WebRender notes"
    );
    assert_eq!(model.chosen_name.as_str(), "Research");
}

#[test]
fn a_chosen_tab_is_kept_while_the_digit_exists_and_dropped_when_it_does_not() {
    let (groups, tabs) = mockup();
    for (digit, want) in [
        (1, 1),
        (3, 3),
        (4, 4),
        (5, -1),
        (6, -1),
        (9, -1),
        (0, -1),
        (200, -1),
    ] {
        let model = card(&groups, &tabs, Some(2), Some(digit));
        assert_eq!(model.selected_group, 2);
        assert_eq!(model.selected_tab, want, "digit {digit} of 4 tabs");
        assert_eq!(model.chips.len(), 4, "the chips do not depend on the digit");
    }
}

#[test]
fn a_chosen_tab_without_a_chosen_group_is_dropped() {
    let (groups, tabs) = mockup();
    let model = card(&groups, &tabs, None, Some(2));
    assert_eq!((model.selected_group, model.selected_tab), (-1, -1));
    assert!(model.chips.is_empty());
}

#[test]
fn choosing_an_unknown_group_gives_minus_one_and_no_chips() {
    let (groups, tabs) = mockup();
    for unknown in [0, 7, 99, -1, i32::MAX, i32::MIN] {
        let model = card(&groups, &tabs, Some(unknown), Some(1));
        assert_eq!(model.selected_group, -1, "group {unknown}");
        assert_eq!(model.selected_tab, -1, "group {unknown}");
        assert!(model.chips.is_empty(), "group {unknown}");
        assert_eq!(model.chosen_name.as_str(), "", "group {unknown}");
        assert_eq!(model.tiles.len(), 6);
    }
}

#[test]
fn choosing_a_group_that_has_no_key_gives_minus_one() {
    let (mut groups, tabs) = mockup();
    groups[1].key_label = "".into();
    let model = card(&groups, &tabs, Some(2), Some(1));
    assert_eq!(model.tiles.len(), 5);
    assert_eq!((model.selected_group, model.selected_tab), (-1, -1));
    assert!(model.chips.is_empty());
    assert_eq!(model.chosen_name.as_str(), "");
}

#[test]
fn twelve_tabs_give_nine_chips_numbered_one_to_nine() {
    let groups = [keyed(1, "A", "Big")];
    let tabs: Vec<TabRow> = (0..12).map(|i| tab(i + 1, 1, i)).collect();
    let model = card(&groups, &tabs, Some(1), Some(9));
    assert_eq!(model.chips.len(), 9);
    let digits: Vec<i32> = model.chips.iter().map(|c| c.index).collect();
    assert_eq!(digits, (1..=9).collect::<Vec<_>>());
    let titles: Vec<String> = model.chips.iter().map(|c| c.title.to_string()).collect();
    let want: Vec<String> = (1..=9).map(|i| format!("Tab {i}")).collect();
    assert_eq!(titles, want);
    assert_eq!(model.selected_tab, 9);
    // The ring still draws all twelve.
    assert_eq!(segments(&model.tiles[0]).len(), 12);
}

#[test]
fn chips_follow_index_in_group_not_the_order_of_the_rows() {
    let groups = [keyed(1, "A", "One")];
    let tabs = [tab(10, 1, 2), tab(20, 1, 0), tab(30, 1, 1)];
    let model = card(&groups, &tabs, Some(1), None);
    let titles: Vec<String> = model.chips.iter().map(|c| c.title.to_string()).collect();
    assert_eq!(titles, ["Tab 20", "Tab 30", "Tab 10"]);
    let digits: Vec<i32> = model.chips.iter().map(|c| c.index).collect();
    assert_eq!(digits, [1, 2, 3]);
}

#[test]
fn a_group_with_no_tabs_has_no_chips_even_when_chosen() {
    let model = card(&[keyed(1, "A", "Empty")], &[], Some(1), Some(1));
    assert_eq!(model.selected_group, 1);
    assert_eq!(model.chosen_name.as_str(), "Empty");
    assert!(model.chips.is_empty());
    assert_eq!(model.selected_tab, -1);
}

// ---- which groups are tiles

#[test]
fn a_group_without_a_key_is_not_a_tile() {
    let (mut groups, tabs) = mockup();
    groups[0].key_label = "".into();
    groups[3].key_label = "".into();
    let model = at_rest(&groups, &tabs);
    assert_eq!(labels(&model), ["S", "D", "G", "H"]);
    let ids: Vec<i32> = model.tiles.iter().map(|t| t.group).collect();
    assert_eq!(ids, [2, 3, 5, 6]);
    assert_eq!(model.layout, card_layout(4));
    // The tiles are packed: the first is at the start of the tile area.
    assert_eq!((model.tiles[0].x, model.tiles[1].x), (0.0, 118.0));
}

#[test]
fn a_duplicate_key_label_keeps_the_first_group() {
    let groups = [
        keyed(1, "A", "First"),
        keyed(2, "B", "Second"),
        keyed(3, "A", "Third"),
        keyed(4, "a", "Fourth"),
        keyed(5, "C", "Fifth"),
    ];
    let model = at_rest(&groups, &[]);
    assert_eq!(labels(&model), ["A", "B", "C"]);
    let names: Vec<String> = model.tiles.iter().map(|t| t.name.to_string()).collect();
    assert_eq!(names, ["First", "Second", "Fifth"]);
}

#[test]
fn no_keyed_group_gives_an_empty_card_with_its_text() {
    let (mut groups, tabs) = mockup();
    for group in &mut groups {
        group.key_label = "".into();
    }
    let model = card(&groups, &tabs, Some(1), Some(1));
    assert!(model.tiles.is_empty());
    assert_eq!(model.layout, card_layout(0));
    assert_eq!(model.layout.rows, 0);
    assert_eq!(model.empty_text, empty_text());
    assert_eq!((model.selected_group, model.selected_tab), (-1, -1));
    assert!(model.chips.is_empty());
}

#[test]
fn twenty_seven_labelled_groups_give_twenty_six_compact_tiles() {
    let groups: Vec<GroupRow> = (0..27)
        .map(|i| keyed(i + 1, &format!("K{i}"), &format!("Group {i}")))
        .collect();
    let model = at_rest(&groups, &[]);
    assert_eq!(model.tiles.len(), 26);
    assert_eq!(model.layout.size, TileSize::Compact);
    assert_eq!((model.layout.columns, model.layout.rows), (9, 3));
    assert_eq!(model.layout.card_height, 580.0);
    // The first 26 stay, the 27th is dropped.
    let ids: Vec<i32> = model.tiles.iter().map(|t| t.group).collect();
    assert_eq!(ids, (1..=26).collect::<Vec<_>>());
    // Positions are from the tile area: nine per row, 78 px apart, rows 142 px apart.
    assert_eq!((model.tiles[1].x, model.tiles[1].y), (78.0, 0.0));
    assert_eq!((model.tiles[9].x, model.tiles[9].y), (0.0, 142.0));
    assert_eq!(
        (model.tiles[25].x, model.tiles[25].y),
        (7.0 * 78.0, 2.0 * 142.0)
    );
}

#[test]
fn seven_groups_switch_to_compact_tiles_and_six_do_not() {
    for (count, size, height) in [
        (6, TileSize::Full, 330.0),
        (7, TileSize::Compact, 296.0),
        (9, TileSize::Compact, 296.0),
        (10, TileSize::Compact, 438.0),
    ] {
        let groups: Vec<GroupRow> = (0..count)
            .map(|i| keyed(i + 1, &format!("K{i}"), "G"))
            .collect();
        let model = at_rest(&groups, &[]);
        assert_eq!(model.layout.size, size, "{count} groups");
        assert_eq!(model.layout.card_height, height, "{count} groups");
    }
}

#[test]
fn hint_modes_reach_the_second_hint() {
    let (groups, tabs) = mockup();
    for (mode, hold) in [
        (HintMode::OutsideTextOrHold, 250),
        (HintMode::OutsideTextOrHold, 1000),
        (HintMode::Always, 400),
        (HintMode::OutsideText, 400),
    ] {
        let model = build_card(&CardInput {
            groups: &groups,
            tabs: &tabs,
            hold_ms: hold,
            mode,
            selected_group: None,
            selected_tab: None,
        });
        assert_eq!(model.hint_secondary, hint_secondary(mode, hold));
        assert_eq!(model.hint_primary, hint_primary());
    }
}

// ---- the ring limit

#[test]
fn sixty_tabs_give_forty_segments_with_the_active_tab_present() {
    let groups = [keyed(1, "A", "Many")];
    let mut tabs: Vec<TabRow> = (0..60).map(|i| tab(i + 1, 1, i)).collect();
    // Every tab has its own shade: red channel = index; the active one is number 50 (index 49).
    for (i, tab) in tabs.iter_mut().enumerate() {
        tab.shade = Color::from_rgb_u8(i as u8, 0, 0);
        tab.active = i == 49;
    }
    let model = at_rest(&groups, &tabs);
    let segs = segments(&model.tiles[0]);
    assert_eq!(segs.len(), 40);
    let reds: Vec<u8> = segs.iter().map(|s| s.shade.red()).collect();
    let mut want: Vec<u8> = (0..39).collect();
    want.push(49);
    assert_eq!(
        reds, want,
        "the first 39 tabs and the active one as the 40th"
    );
    assert!(model.tiles[0].has_mark, "the active tab has its mark");
    assert_rings_are_well_formed(&model);
    // The chips are not limited by the ring.
    assert_eq!(card(&groups, &tabs, Some(1), None).chips.len(), 9);
}

#[test]
fn an_active_tab_among_the_first_39_is_not_repeated_and_the_40th_tab_fills_the_ring() {
    let groups = [keyed(1, "A", "Many")];
    let mut tabs: Vec<TabRow> = (0..60).map(|i| tab(i + 1, 1, i)).collect();
    for (i, tab) in tabs.iter_mut().enumerate() {
        tab.shade = Color::from_rgb_u8(i as u8, 0, 0);
        tab.active = i == 3;
    }
    let model = at_rest(&groups, &tabs);
    let reds: Vec<u8> = segments(&model.tiles[0])
        .iter()
        .map(|s| s.shade.red())
        .collect();
    assert_eq!(reds, (0..40).collect::<Vec<u8>>());
    assert!(model.tiles[0].has_mark);
}

#[test]
fn forty_tabs_draw_forty_segments_and_forty_one_draw_forty() {
    let groups = [keyed(1, "A", "Many")];
    for (count, want) in [(39, 39), (40, 40), (41, 40)] {
        let tabs: Vec<TabRow> = (0..count).map(|i| tab(i + 1, 1, i)).collect();
        let model = at_rest(&groups, &tabs);
        assert_eq!(segments(&model.tiles[0]).len(), want, "{count} tabs");
    }
}

#[test]
fn without_an_active_tab_a_big_ring_has_no_mark() {
    let groups = [keyed(1, "A", "Many")];
    let tabs: Vec<TabRow> = (0..60).map(|i| tab(i + 1, 1, i)).collect();
    let model = at_rest(&groups, &tabs);
    assert_eq!(segments(&model.tiles[0]).len(), 40);
    assert!(!model.tiles[0].has_mark);
}

// ---- totality

#[test]
fn no_groups_and_no_tabs_build_an_empty_card() {
    let model = at_rest(&[], &[]);
    assert!(model.tiles.is_empty() && model.chips.is_empty());
    assert_eq!(model.layout.rows, 0);
    let model = card(&[], &[], Some(0), Some(0));
    assert_eq!((model.selected_group, model.selected_tab), (-1, -1));
    // Tabs with no groups are left out.
    let tabs: Vec<TabRow> = (0..5).map(|i| tab(i + 1, 1, i)).collect();
    assert!(at_rest(&[], &tabs).tiles.is_empty());
}

#[test]
fn groups_without_tabs_build_tiles_with_empty_rings() {
    let groups: Vec<GroupRow> = (0..26)
        .map(|i| keyed(i + 1, &format!("K{i}"), &format!("Group {i}")))
        .collect();
    let model = card(&groups, &[], Some(5), Some(3));
    assert_eq!(model.tiles.len(), 26);
    assert!(
        model
            .tiles
            .iter()
            .all(|t| segments(t).is_empty() && !t.has_mark)
    );
    assert_eq!(model.selected_group, 5);
    assert!(model.chips.is_empty());
    assert_eq!(model.selected_tab, -1);
}

#[test]
fn twenty_six_groups_of_five_hundred_tabs_each_stay_bounded() {
    let groups: Vec<GroupRow> = (0..26)
        .map(|i| {
            let mut group = keyed(i + 1, &format!("K{i}"), &format!("Group {i}"));
            group.count = 500;
            group
        })
        .collect();
    let mut tabs = Vec::new();
    for g in 0..26 {
        for i in 0..500 {
            let mut t = tab(g * 1000 + i + 1, g + 1, i);
            t.active = i == 250;
            tabs.push(t);
        }
    }
    let model = card(&groups, &tabs, Some(26), Some(9));
    assert_eq!(model.tiles.len(), 26);
    for tile in &model.tiles {
        assert_eq!(segments(tile).len(), 40);
        assert!(tile.has_mark, "the active tab 250 is beyond the first 39");
        assert_eq!(tile.count_text.as_str(), "500 tabs");
    }
    assert_eq!(model.chips.len(), 9);
    assert_eq!(model.selected_tab, 9);
    assert_rings_are_well_formed(&model);
}

#[test]
fn five_hundred_tabs_in_one_group_give_forty_segments_and_nine_chips() {
    let groups = [keyed(1, "A", "Big")];
    let tabs: Vec<TabRow> = (0..500).map(|i| tab(i + 1, 1, i)).collect();
    let model = card(&groups, &tabs, Some(1), Some(1));
    assert_eq!(segments(&model.tiles[0]).len(), 40);
    assert_eq!(model.chips.len(), 9);
}

#[test]
fn names_and_titles_of_any_script_come_through_whole() {
    let names = [
        "x".repeat(64),
        "é".repeat(64),
        "研究ノート：京都の旅行計画とレシピ集".to_owned(),
        "Trip ✈️🌍 with 👨‍👩‍👧‍👦 family".to_owned(),
        "מחקר ופיתוח של הדפדפן".to_owned(),
        "بحث وتطوير المتصفح".to_owned(),
        "e\u{301}\u{301}\u{301} combining".to_owned(),
        String::new(),
    ];
    let groups: Vec<GroupRow> = names
        .iter()
        .enumerate()
        .map(|(i, name)| keyed(i as i32 + 1, &format!("{}", (b'A' + i as u8) as char), name))
        .collect();
    let tabs: Vec<TabRow> = names
        .iter()
        .enumerate()
        .map(|(i, title)| {
            let mut t = tab(i as i32 + 1, i as i32 + 1, 0);
            t.title = title.as_str().into();
            t
        })
        .collect();
    let model = at_rest(&groups, &tabs);
    for (tile, name) in model.tiles.iter().zip(&names) {
        assert_eq!(tile.name.as_str(), name.as_str());
        assert!(tile.accessible_label.as_str().contains(name.as_str()));
    }
    for (index, name) in names.iter().enumerate() {
        let chosen = card(&groups, &tabs, Some(index as i32 + 1), Some(1));
        assert_eq!(chosen.chosen_name.as_str(), name.as_str());
        assert_eq!(chosen.chips[0].title.as_str(), name.as_str());
        assert!(
            chosen.chips[0]
                .accessible_label
                .as_str()
                .ends_with(name.as_str())
        );
    }
}

#[test]
fn a_name_of_ten_thousand_characters_does_not_break_anything() {
    let long = "界".repeat(10_000);
    let groups = [keyed(1, "A", &long)];
    let mut tabs = [tab(1, 1, 0)];
    tabs[0].title = long.as_str().into();
    let model = card(&groups, &tabs, Some(1), Some(1));
    assert_eq!(model.chosen_name.as_str(), long.as_str());
    assert_eq!(model.chips[0].title.as_str(), long.as_str());
}

#[test]
fn odd_numbers_in_the_rows_do_not_panic() {
    let mut a = keyed(i32::MIN, "A", "Min");
    a.count = i32::MIN;
    let mut b = keyed(i32::MAX, "B", "Max");
    b.count = i32::MAX;
    let mut c = keyed(0, "C", "Zero");
    c.count = -7;
    let groups = [a, b, c];
    let tabs = [
        tab(i32::MIN, i32::MIN, i32::MIN),
        tab(i32::MAX, i32::MAX, i32::MAX),
        tab(0, 0, -1),
        tab(0, 0, -1),
        tab(5, i32::MAX, 0),
    ];
    for selected in [None, Some(i32::MIN), Some(0), Some(i32::MAX)] {
        let model = card(&groups, &tabs, selected, Some(255));
        assert_eq!(model.tiles.len(), 3);
        assert_rings_are_well_formed(&model);
    }
    let model = at_rest(&groups, &tabs);
    assert_eq!(
        model.tiles[0].count_text.as_str(),
        "0 tabs",
        "a negative count is none"
    );
    assert_eq!(model.tiles[2].count_text.as_str(), "0 tabs");
    assert_eq!(
        model.tiles[1].count_text.as_str(),
        format!("{} tabs", i32::MAX)
    );
}

#[test]
fn building_twice_gives_the_same_model() {
    let (groups, tabs) = mockup();
    let first = card(&groups, &tabs, Some(3), Some(2));
    let second = card(&groups, &tabs, Some(3), Some(2));
    assert_eq!(first.layout, second.layout);
    assert_eq!(first.tiles.len(), second.tiles.len());
    for (a, b) in first.tiles.iter().zip(&second.tiles) {
        let flat = |t: &KeyTile| {
            let segs: Vec<(String, Color)> = segments(t)
                .iter()
                .map(|s| (s.commands.to_string(), s.shade))
                .collect();
            (
                (t.group, t.x, t.y, t.has_mark, t.mark_x, t.mark_y, t.tint),
                (
                    t.key_label.to_string(),
                    t.name.to_string(),
                    t.count_text.to_string(),
                ),
                (t.accessible_label.to_string(), segs),
            )
        };
        assert_eq!(flat(a), flat(b));
    }
    assert_eq!(first.chips, second.chips);
    assert_eq!(
        (first.selected_group, first.selected_tab),
        (second.selected_group, second.selected_tab)
    );
}

#[test]
fn every_ring_of_every_state_is_well_formed() {
    let (groups, tabs) = mockup();
    for selected in [None, Some(1), Some(2), Some(6)] {
        assert_rings_are_well_formed(&card(&groups, &tabs, selected, Some(1)));
    }
    // One tab: a single arc nearly a full turn.
    let one = [keyed(1, "A", "One")];
    let model = at_rest(&one, &[tab(1, 1, 0)]);
    assert_eq!(segments(&model.tiles[0]).len(), 1);
    assert_rings_are_well_formed(&model);
}
