// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! The Orbit zoom state machine (SPEC §4.6 and §6, brief D2, D5, D8, D9, D10): the level table, click
//! on every kind of target, ⏎, the Esc layers, ⌘↑, the arrows, the pinch table, drag and drop,
//! reconciling with changed rows, the new-group popover, a table of every (level, event) pair and
//! 1,000 random event sequences. Pure Rust: no Slint window.

use mercury_ui::orbit_view::drag::{self, DropTarget, Source};
use mercury_ui::orbit_view::fixtures::{orbit_groups, orbit_input, orbit_tabs};
use mercury_ui::orbit_view::machine::preview_order;
use mercury_ui::orbit_view::{
    Decision, Effect, GroupOp, Key, KeyContext, LOOSE_GROUP_ID, Level, Mods, OrbitInput,
    OrbitMachine, Target, View,
};
use mercury_ui::{GroupRow, TabRow};

// ---------------------------------------------------------------------------------------------
// Helpers

fn opened() -> OrbitMachine {
    opened_with(&orbit_input(false))
}

fn opened_with(input: &OrbitInput) -> OrbitMachine {
    let mut machine = OrbitMachine::new(7);
    machine.open(input);
    machine
}

/// Level 2 of group `group`.
fn at_group(group: i32) -> OrbitMachine {
    let mut machine = opened();
    machine.click(Target::Planet(group));
    assert_eq!(machine.view().level, Level::Group);
    machine
}

/// Level 3 of tab `tab` of Research (1).
fn at_tab(tab: i32) -> OrbitMachine {
    let mut machine = at_group(1);
    machine.click(Target::Preview(tab));
    assert_eq!(machine.view().level, Level::Tab);
    machine
}

fn press(machine: &mut OrbitMachine, key: Key) -> Vec<Effect> {
    machine.key(key, Mods::NONE, KeyContext::default())
}

fn command(machine: &mut OrbitMachine, key: Key) -> Vec<Effect> {
    machine.key(key, Mods::COMMAND, KeyContext::default())
}

fn typing(machine: &mut OrbitMachine, key: Key) -> Vec<Effect> {
    machine.key(
        key,
        Mods::NONE,
        KeyContext {
            text_input_focused: true,
        },
    )
}

fn zoomed(machine: &OrbitMachine) -> Vec<Effect> {
    vec![Effect::ZoomedTo(machine.view())]
}

fn big_group(n: usize) -> OrbitInput {
    let groups = vec![GroupRow {
        id: 1,
        name: "Big".into(),
        ..Default::default()
    }];
    let tabs: Vec<TabRow> = (0..n)
        .map(|i| TabRow {
            id: 500 + i as i32,
            group: 1,
            index_in_group: i as i32,
            title: format!("Tab {i}").into(),
            ..Default::default()
        })
        .collect();
    OrbitInput::new(&groups, &tabs, "P", "W", 500, false)
}

// ---------------------------------------------------------------------------------------------
// open and the shared state

#[test]
fn open_starts_at_level_one_focused_on_the_active_tabs_group_and_asks_for_a_picture() {
    let mut machine = OrbitMachine::new(7);
    let effects = machine.open(&OrbitInput {
        active_tab: 120,
        ..orbit_input(false)
    });
    assert_eq!(effects, vec![Effect::CapturePicture]);
    let view = machine.view();
    assert_eq!(view.level, Level::Groups);
    assert_eq!(view.group, Some(3), "tab 120 is in Reading");
    assert_eq!(view.tab, None);
    assert_eq!(view.focus, Some(Target::Planet(3)));
    assert_eq!(view.hover, None);
    assert!(view.drag.is_none() && view.popover.is_none() && view.pinch.is_none());
    assert!(view.query.is_empty());
}

#[test]
fn open_with_an_unknown_active_tab_focuses_the_first_planet() {
    let mut machine = OrbitMachine::new(1);
    machine.open(&OrbitInput {
        active_tab: 9999,
        ..orbit_input(false)
    });
    assert_eq!(machine.view().focus, Some(Target::Planet(1)));
}

#[test]
fn open_with_no_groups_has_no_focus() {
    let mut machine = OrbitMachine::new(1);
    assert_eq!(
        machine.open(&OrbitInput::default()),
        vec![Effect::CapturePicture]
    );
    let view = machine.view();
    assert_eq!(
        (view.level, view.group, view.focus),
        (Level::Groups, None, None)
    );
}

#[test]
fn the_session_nonce_is_only_stored() {
    assert_eq!(OrbitMachine::new(0xDEAD_BEEF).session(), 0xDEAD_BEEF);
    let mut machine = OrbitMachine::new(42);
    machine.open(&orbit_input(false));
    assert_eq!(machine.session(), 42);
}

#[test]
fn opening_again_resets_every_part_of_the_state() {
    let mut machine = at_tab(104);
    machine.search_changed("servo");
    machine.hover(Target::Card);
    machine.open(&orbit_input(false));
    let view = machine.view();
    assert_eq!(view.level, Level::Groups);
    assert!(view.query.is_empty() && view.hover.is_none() && view.tab.is_none());
    assert_eq!(view.focus, Some(Target::Planet(1)));
}

#[test]
fn hover_records_the_target_and_backdrop_is_nothing() {
    let mut machine = opened();
    machine.hover(Target::Planet(2));
    assert_eq!(machine.view().hover, Some(Target::Planet(2)));
    machine.hover(Target::Backdrop);
    assert_eq!(machine.view().hover, None);
}

#[test]
fn search_changed_stores_the_query() {
    let mut machine = opened();
    machine.search_changed("servo notes");
    assert_eq!(machine.view().query, "servo notes");
    machine.search_changed("");
    assert_eq!(machine.view().query, "");
}

#[test]
fn level_two_order_is_the_active_tab_first_then_strip_order() {
    let mut input = orbit_input(false);
    assert_eq!(
        preview_order(&input, 1),
        vec![101, 102, 103, 104, 105, 106, 107, 108, 109, 110]
    );
    input.active_tab = 104;
    assert_eq!(
        preview_order(&input, 1),
        vec![104, 101, 102, 103, 105, 106, 107, 108, 109, 110]
    );
    assert_eq!(
        preview_order(&input, 2)[0],
        111,
        "the active tab is not in Lisbon"
    );
    assert!(preview_order(&input, 99).is_empty());
}

// ---------------------------------------------------------------------------------------------
// Click, level 1 / 2 / 3 (D9)

#[test]
fn click_on_a_planet_zooms_to_level_two_of_that_group_focused_on_its_first_preview() {
    let mut machine = opened();
    let effects = machine.click(Target::Planet(2));
    let view = machine.view();
    assert_eq!(view.level, Level::Group);
    assert_eq!(view.group, Some(2));
    assert_eq!(view.tab, Some(111));
    assert_eq!(view.focus, Some(Target::Preview(111)));
    assert_eq!(effects, zoomed(&machine));
}

#[test]
fn zooming_into_the_active_tabs_group_focuses_the_active_tab() {
    let mut machine = OrbitMachine::new(1);
    machine.open(&OrbitInput {
        active_tab: 105,
        ..orbit_input(false)
    });
    machine.click(Target::Planet(1));
    assert_eq!(machine.view().tab, Some(105));
}

#[test]
fn level_one_ignores_every_click_that_is_not_a_known_planet() {
    for target in [
        Target::Planet(99),
        Target::Sun,
        Target::Preview(103),
        Target::Chip(2),
        Target::NewGroup,
        Target::Card,
        Target::Neighbour(105),
        Target::Backdrop,
    ] {
        let mut machine = opened();
        let before = machine.view();
        assert!(machine.click(target).is_empty(), "{target:?}");
        assert_eq!(machine.view(), before, "{target:?}");
    }
}

#[test]
fn click_on_a_preview_zooms_to_level_three_of_that_tab() {
    let mut machine = at_group(1);
    let effects = machine.click(Target::Preview(104));
    let view = machine.view();
    assert_eq!(view.level, Level::Tab);
    assert_eq!((view.group, view.tab), (Some(1), Some(104)));
    assert_eq!(view.focus, Some(Target::Card));
    assert_eq!(effects, zoomed(&machine));
}

#[test]
fn click_on_a_chip_zooms_to_level_two_of_the_other_group() {
    let mut machine = at_group(1);
    let effects = machine.click(Target::Chip(3));
    let view = machine.view();
    assert_eq!(view.level, Level::Group);
    assert_eq!((view.group, view.tab), (Some(3), Some(119)));
    assert_eq!(effects, zoomed(&machine));
}

#[test]
fn click_on_the_chip_of_the_shown_group_changes_nothing() {
    let mut machine = at_group(1);
    machine.key(Key::Right, Mods::NONE, KeyContext::default());
    let before = machine.view();
    assert!(machine.click(Target::Chip(1)).is_empty());
    assert_eq!(machine.view(), before);
}

#[test]
fn level_two_ignores_the_hole_empty_space_and_every_other_kind_of_target() {
    for target in [
        Target::Planet(2),
        Target::Sun,
        Target::NewGroup,
        Target::Card,
        Target::Neighbour(102),
        Target::Backdrop,
        Target::Preview(9999),
        Target::Chip(9999),
        // A tab of another group is not on this level's stage.
        Target::Preview(120),
    ] {
        let mut machine = at_group(1);
        let before = machine.view();
        assert!(machine.click(target).is_empty(), "{target:?}");
        assert_eq!(machine.view(), before, "{target:?}");
    }
}

#[test]
fn click_on_the_card_leaves_orbit_and_activates_the_tab() {
    let mut machine = at_tab(104);
    assert_eq!(
        machine.click(Target::Card),
        vec![Effect::Close {
            activate: Some(104)
        }]
    );
}

#[test]
fn click_on_a_neighbour_card_shows_that_tab_at_level_three() {
    let mut machine = at_tab(104);
    let effects = machine.click(Target::Neighbour(105));
    let view = machine.view();
    assert_eq!((view.level, view.tab), (Level::Tab, Some(105)));
    assert_eq!(effects, zoomed(&machine));
}

#[test]
fn click_outside_the_card_and_the_neighbours_goes_back_to_level_two() {
    let mut machine = at_tab(104);
    let effects = machine.click(Target::Backdrop);
    let view = machine.view();
    assert_eq!(view.level, Level::Group);
    assert_eq!((view.group, view.tab), (Some(1), Some(104)));
    assert_eq!(view.focus, Some(Target::Preview(104)));
    assert_eq!(effects, zoomed(&machine));
}

#[test]
fn level_three_ignores_clicks_on_targets_that_cannot_be_there() {
    for target in [
        Target::Planet(2),
        Target::Sun,
        Target::Chip(2),
        Target::NewGroup,
        Target::Preview(103),
        Target::Neighbour(9999),
    ] {
        let mut machine = at_tab(104);
        let before = machine.view();
        assert!(machine.click(target).is_empty(), "{target:?}");
        assert_eq!(machine.view(), before, "{target:?}");
    }
}

// ---------------------------------------------------------------------------------------------
// ⏎ (D9, D6)

#[test]
fn enter_at_level_one_zooms_into_the_focused_planet() {
    let mut machine = opened();
    press(&mut machine, Key::Right);
    let effects = press(&mut machine, Key::Enter);
    assert_eq!(machine.view().level, Level::Group);
    assert_eq!(machine.view().group, Some(2));
    assert_eq!(effects, zoomed(&machine));
}

#[test]
fn enter_without_a_focus_does_nothing() {
    let mut machine = OrbitMachine::new(1);
    machine.open(&OrbitInput::default());
    assert!(press(&mut machine, Key::Enter).is_empty());
    assert_eq!(machine.view().level, Level::Groups);

    // Level 2 of a group that has no tab has no focus either.
    let mut input = orbit_input(false);
    input.tabs.retain(|t| t.group != 2);
    let mut machine = opened_with(&input);
    machine.click(Target::Planet(2));
    assert_eq!(machine.view().level, Level::Group);
    assert_eq!(machine.view().focus, None);
    assert!(press(&mut machine, Key::Enter).is_empty());
    assert_eq!(machine.view().level, Level::Group);
}

#[test]
fn enter_at_level_two_zooms_into_the_focused_preview() {
    let mut machine = at_group(1);
    press(&mut machine, Key::Right);
    press(&mut machine, Key::Right);
    let effects = press(&mut machine, Key::Enter);
    assert_eq!(
        (machine.view().level, machine.view().tab),
        (Level::Tab, Some(103))
    );
    assert_eq!(effects, zoomed(&machine));
}

#[test]
fn enter_at_level_three_opens_the_tab() {
    let mut machine = at_tab(104);
    assert_eq!(
        press(&mut machine, Key::Enter),
        vec![Effect::Close {
            activate: Some(104)
        }]
    );
}

#[test]
fn enter_with_a_query_zooms_to_the_top_result_at_level_three() {
    let mut machine = opened();
    machine.search_changed("interview");
    let effects = press(&mut machine, Key::Enter);
    let view = machine.view();
    assert_eq!(
        (view.level, view.group, view.tab),
        (Level::Tab, Some(3), Some(127))
    );
    assert_eq!(effects, zoomed(&machine));
}

#[test]
fn enter_with_a_query_works_from_level_two_and_from_the_search_field() {
    let mut machine = at_group(1);
    machine.search_changed("tram");
    let effects = machine.key(
        Key::Enter,
        Mods::NONE,
        KeyContext {
            text_input_focused: true,
        },
    );
    assert_eq!(
        (machine.view().level, machine.view().tab),
        (Level::Tab, Some(113))
    );
    assert_eq!(machine.view().group, Some(2));
    assert_eq!(effects, zoomed(&machine));
}

#[test]
fn enter_with_a_query_that_matches_nothing_does_nothing() {
    let mut machine = opened();
    machine.search_changed("no such tab");
    let before = machine.view();
    assert!(press(&mut machine, Key::Enter).is_empty());
    assert_eq!(machine.view(), before);
}

#[test]
fn enter_at_level_three_with_a_query_still_opens_the_shown_tab() {
    let mut machine = opened();
    machine.search_changed("interview");
    press(&mut machine, Key::Enter);
    assert_eq!(
        press(&mut machine, Key::Enter),
        vec![Effect::Close {
            activate: Some(127)
        }]
    );
}

// ---------------------------------------------------------------------------------------------
// Esc (D8): drag, popover, search, close; ⌘↑

#[test]
fn escape_closes_orbit_to_the_page_from_every_level() {
    for mut machine in [opened(), at_group(1), at_tab(104)] {
        assert_eq!(
            press(&mut machine, Key::Escape),
            vec![Effect::Close { activate: None }]
        );
    }
}

#[test]
fn escape_layer_one_cancels_a_drag_in_flight_first() {
    let mut machine = at_group(1);
    machine.search_changed("servo");
    machine.drag_started(Source::Tab(102));
    assert!(machine.view().drag.is_some());
    assert!(press(&mut machine, Key::Escape).is_empty());
    assert!(machine.view().drag.is_none());
    assert_eq!(
        machine.view().query,
        "servo",
        "the drag was the first layer"
    );
    // Then the search, then the page.
    assert_eq!(press(&mut machine, Key::Escape), vec![Effect::ClearSearch]);
    assert_eq!(machine.view().query, "");
    assert_eq!(
        press(&mut machine, Key::Escape),
        vec![Effect::Close { activate: None }]
    );
}

#[test]
fn escape_layer_two_closes_the_popover_before_the_search() {
    let mut machine = at_group(1);
    machine.search_changed("servo");
    machine.key(Key::N, Mods::NONE, KeyContext::default());
    assert!(machine.view().popover.is_some());
    assert!(press(&mut machine, Key::Escape).is_empty());
    assert!(machine.view().popover.is_none());
    assert_eq!(machine.view().query, "servo");
    assert_eq!(press(&mut machine, Key::Escape), vec![Effect::ClearSearch]);
    assert_eq!(
        press(&mut machine, Key::Escape),
        vec![Effect::Close { activate: None }]
    );
}

#[test]
fn escape_with_a_query_clears_it_at_every_level_before_leaving() {
    for mut machine in [opened(), at_group(1), at_tab(104)] {
        machine.search_changed("x");
        let level = machine.view().level;
        assert_eq!(press(&mut machine, Key::Escape), vec![Effect::ClearSearch]);
        assert_eq!(machine.view().level, level);
        assert_eq!(
            press(&mut machine, Key::Escape),
            vec![Effect::Close { activate: None }]
        );
    }
}

#[test]
fn command_up_climbs_one_level_and_leaves_orbit_at_level_one() {
    let mut machine = at_tab(104);
    let effects = command(&mut machine, Key::Up);
    assert_eq!(
        (machine.view().level, machine.view().tab),
        (Level::Group, Some(104))
    );
    assert_eq!(machine.view().focus, Some(Target::Preview(104)));
    assert_eq!(effects, zoomed(&machine));

    let effects = command(&mut machine, Key::Up);
    assert_eq!(machine.view().level, Level::Groups);
    assert_eq!(machine.view().group, Some(1));
    assert_eq!(machine.view().tab, None);
    assert_eq!(machine.view().focus, Some(Target::Planet(1)));
    assert_eq!(effects, zoomed(&machine));

    assert_eq!(
        command(&mut machine, Key::Up),
        vec![Effect::Close { activate: None }]
    );
}

#[test]
fn command_up_does_not_clear_the_search() {
    let mut machine = at_group(1);
    machine.search_changed("servo");
    command(&mut machine, Key::Up);
    assert_eq!(machine.view().query, "servo");
}

// ---------------------------------------------------------------------------------------------
// Arrows (D5, D9)

#[test]
fn level_one_left_and_right_walk_the_planets_in_group_order_and_clamp() {
    let mut machine = opened();
    assert!(
        press(&mut machine, Key::Left).is_empty(),
        "clamped at the first"
    );
    assert_eq!(machine.view().focus, Some(Target::Planet(1)));
    for group in 2..=5 {
        assert!(press(&mut machine, Key::Right).is_empty());
        assert_eq!(machine.view().focus, Some(Target::Planet(group)));
        assert_eq!(machine.view().group, Some(group));
    }
    press(&mut machine, Key::Right);
    assert_eq!(
        machine.view().focus,
        Some(Target::Planet(5)),
        "clamped, not wrapped"
    );
    press(&mut machine, Key::Left);
    assert_eq!(machine.view().focus, Some(Target::Planet(4)));
}

#[test]
fn level_one_up_and_down_move_to_the_nearest_planet_above_and_below() {
    // Five planets 72 degrees apart from the top: group 1 at the top, 2 right and above the sun's
    // centre line, 3 and 4 at the bottom, 5 left.
    let mut machine = opened();
    press(&mut machine, Key::Up);
    assert_eq!(
        machine.view().focus,
        Some(Target::Planet(1)),
        "nothing above the top planet"
    );
    press(&mut machine, Key::Down);
    assert_eq!(machine.view().focus, Some(Target::Planet(2)));
    press(&mut machine, Key::Down);
    assert_eq!(machine.view().focus, Some(Target::Planet(3)));
    press(&mut machine, Key::Down);
    assert_eq!(
        machine.view().focus,
        Some(Target::Planet(3)),
        "nothing below the bottom"
    );
    press(&mut machine, Key::Up);
    assert_eq!(machine.view().focus, Some(Target::Planet(2)));
    press(&mut machine, Key::Up);
    assert_eq!(machine.view().focus, Some(Target::Planet(1)));
}

#[test]
fn level_one_arrows_with_one_group_stay_put() {
    let mut input = orbit_input(false);
    input.groups.truncate(1);
    input.tabs.retain(|t| t.group == 1);
    let mut machine = opened_with(&input);
    for key in [Key::Left, Key::Right, Key::Up, Key::Down] {
        assert!(press(&mut machine, key).is_empty());
        assert_eq!(machine.view().focus, Some(Target::Planet(1)));
    }
}

#[test]
fn level_two_arrows_walk_the_previews_active_first_and_clamp() {
    let mut machine = OrbitMachine::new(1);
    machine.open(&OrbitInput {
        active_tab: 104,
        ..orbit_input(false)
    });
    machine.click(Target::Planet(1));
    assert_eq!(machine.view().focus, Some(Target::Preview(104)));
    assert!(
        press(&mut machine, Key::Left).is_empty(),
        "clamped at the first"
    );
    assert_eq!(machine.view().tab, Some(104));
    let mut seen = vec![104];
    for _ in 0..12 {
        press(&mut machine, Key::Right);
        let tab = machine.view().tab.expect("a focused tab");
        if seen.last() != Some(&tab) {
            seen.push(tab);
        }
    }
    assert_eq!(seen, vec![104, 101, 102, 103, 105, 106, 107, 108, 109, 110]);
    assert_eq!(machine.view().focus, Some(Target::Preview(110)));
    press(&mut machine, Key::Left);
    assert_eq!(machine.view().tab, Some(109));
    assert_eq!(machine.view().level, Level::Group);
}

#[test]
fn level_two_arrows_cross_from_the_first_ring_to_the_second_and_stop_at_the_last_shown() {
    let mut machine = opened_with(&big_group(30));
    machine.click(Target::Planet(1));
    for _ in 0..11 {
        press(&mut machine, Key::Right);
    }
    assert_eq!(
        machine.view().tab,
        Some(511),
        "the 12th and last of the first ring"
    );
    press(&mut machine, Key::Right);
    assert_eq!(
        machine.view().tab,
        Some(512),
        "the first of the second ring"
    );
    press(&mut machine, Key::Left);
    assert_eq!(machine.view().tab, Some(511));
    for _ in 0..40 {
        press(&mut machine, Key::Right);
    }
    assert_eq!(
        machine.view().tab,
        Some(523),
        "24 previews are shown, the rest is '+N more'"
    );
}

#[test]
fn level_two_up_and_down_do_nothing() {
    let mut machine = at_group(1);
    press(&mut machine, Key::Right);
    let before = machine.view();
    assert!(press(&mut machine, Key::Up).is_empty());
    assert!(press(&mut machine, Key::Down).is_empty());
    assert_eq!(machine.view(), before);
}

#[test]
fn level_three_left_and_right_show_the_previous_and_next_tab_of_the_group_and_clamp() {
    let mut machine = at_tab(104);
    let effects = press(&mut machine, Key::Right);
    assert_eq!(machine.view().tab, Some(105));
    assert_eq!(effects, zoomed(&machine));
    let effects = press(&mut machine, Key::Left);
    assert_eq!(machine.view().tab, Some(104));
    assert_eq!(effects, zoomed(&machine));
    for _ in 0..3 {
        press(&mut machine, Key::Left);
    }
    assert_eq!(machine.view().tab, Some(101));
    assert!(
        press(&mut machine, Key::Left).is_empty(),
        "clamped at the first tab"
    );
    for _ in 0..20 {
        press(&mut machine, Key::Right);
    }
    assert_eq!(machine.view().tab, Some(110));
    assert!(
        press(&mut machine, Key::Right).is_empty(),
        "clamped at the last tab"
    );
    assert_eq!(machine.view().level, Level::Tab);
    assert_eq!(machine.view().focus, Some(Target::Card));
}

#[test]
fn level_three_neighbours_follow_the_strip_not_the_level_two_order() {
    let mut machine = OrbitMachine::new(1);
    machine.open(&OrbitInput {
        active_tab: 104,
        ..orbit_input(false)
    });
    machine.click(Target::Planet(1));
    machine.click(Target::Preview(104));
    press(&mut machine, Key::Left);
    assert_eq!(
        machine.view().tab,
        Some(103),
        "tab 3 of 10, not the level-two neighbour 101"
    );
}

#[test]
fn level_three_up_and_down_do_nothing() {
    let mut machine = at_tab(104);
    let before = machine.view();
    assert!(press(&mut machine, Key::Up).is_empty());
    assert!(press(&mut machine, Key::Down).is_empty());
    assert_eq!(machine.view(), before);
}

#[test]
fn arrows_with_command_alt_or_control_move_nothing_except_command_up() {
    for mods in [
        Mods::COMMAND,
        Mods {
            alt: true,
            ..Mods::NONE
        },
        Mods {
            control: true,
            ..Mods::NONE
        },
    ] {
        for key in [Key::Left, Key::Right, Key::Down] {
            let mut machine = opened();
            let before = machine.view();
            assert!(machine.key(key, mods, KeyContext::default()).is_empty());
            assert_eq!(machine.view(), before, "{key:?} {mods:?}");
        }
    }
    let mut machine = opened();
    let alt_up = Mods {
        alt: true,
        ..Mods::NONE
    };
    assert!(
        machine
            .key(Key::Up, alt_up, KeyContext::default())
            .is_empty()
    );
}

#[test]
fn keys_that_belong_to_a_text_field_are_not_commands_while_it_has_the_focus() {
    for key in [
        Key::N,
        Key::Slash,
        Key::Char('x'),
        Key::Left,
        Key::Right,
        Key::Up,
        Key::Down,
    ] {
        let mut machine = opened();
        let before = machine.view();
        assert!(typing(&mut machine, key).is_empty(), "{key:?}");
        assert_eq!(machine.view(), before, "{key:?}");
    }
    // The keys that are Orbit's even in a field still work.
    let mut machine = at_tab(104);
    assert_eq!(
        typing(&mut machine, Key::Escape),
        vec![Effect::Close { activate: None }]
    );
    let effects = machine.key(
        Key::Up,
        Mods::COMMAND,
        KeyContext {
            text_input_focused: true,
        },
    );
    assert_eq!(machine.view().level, Level::Group);
    assert_eq!(effects, zoomed(&machine));
}

// ---------------------------------------------------------------------------------------------
// N, /, typed characters; the new-group popover

#[test]
fn n_opens_the_new_group_popover_for_the_focused_tab() {
    let mut machine = at_group(1);
    press(&mut machine, Key::Right);
    press(&mut machine, Key::Right);
    assert_eq!(
        press(&mut machine, Key::N),
        vec![Effect::OpenNewGroupPopover { tab: 103 }]
    );
    let popover = machine.view().popover.expect("a popover");
    assert_eq!(popover.tab, 103);
    assert!(popover.name.is_empty());
}

#[test]
fn n_at_level_three_uses_the_shown_tab() {
    let mut machine = at_tab(106);
    assert_eq!(
        press(&mut machine, Key::N),
        vec![Effect::OpenNewGroupPopover { tab: 106 }]
    );
}

#[test]
fn n_at_level_one_uses_the_active_tab_or_the_first_free_tab_of_the_focused_group() {
    let mut machine = opened();
    assert_eq!(
        press(&mut machine, Key::N),
        vec![Effect::OpenNewGroupPopover { tab: 101 }]
    );

    // The active tab is pinned (it lives in the pseudo planet, where Orbit opens): a pinned tab
    // cannot be taken, so use the first free tab of the focused group, else of any group.
    let mut tabs = orbit_tabs();
    tabs[0].pinned = true;
    let input = OrbitInput::new(&orbit_groups(false), &tabs, "P", "W", 101, false);
    let mut machine = opened_with(&input);
    assert_eq!(machine.view().focus, Some(Target::Planet(LOOSE_GROUP_ID)));
    assert_eq!(
        press(&mut machine, Key::N),
        vec![Effect::OpenNewGroupPopover { tab: 102 }]
    );
    press(&mut machine, Key::Escape);
    machine.click(Target::Planet(2));
    command(&mut machine, Key::Up);
    assert_eq!(machine.view().focus, Some(Target::Planet(2)));
    assert_eq!(
        press(&mut machine, Key::N),
        vec![Effect::OpenNewGroupPopover { tab: 111 }]
    );
}

#[test]
fn n_with_no_tab_to_take_does_nothing() {
    let mut machine = OrbitMachine::new(1);
    machine.open(&OrbitInput::default());
    assert!(press(&mut machine, Key::N).is_empty());
    assert!(machine.view().popover.is_none());
}

#[test]
fn n_while_the_popover_is_open_does_nothing_more() {
    let mut machine = at_group(1);
    press(&mut machine, Key::N);
    assert!(press(&mut machine, Key::N).is_empty());
    assert!(
        press(&mut machine, Key::Enter).is_empty(),
        "the popover is modal"
    );
    assert!(press(&mut machine, Key::Right).is_empty());
    assert!(command(&mut machine, Key::Up).is_empty());
    assert_eq!(machine.view().level, Level::Group);
    assert_eq!(machine.view().tab, Some(101));
}

#[test]
fn n_with_command_alt_or_control_is_not_ours() {
    let mut machine = opened();
    for mods in [
        Mods::COMMAND,
        Mods {
            alt: true,
            ..Mods::NONE
        },
        Mods {
            control: true,
            ..Mods::NONE
        },
    ] {
        assert!(machine.key(Key::N, mods, KeyContext::default()).is_empty());
    }
    assert!(machine.view().popover.is_none());
}

#[test]
fn confirming_the_popover_applies_create_group_with_the_name_and_colour() {
    let mut machine = at_group(1);
    press(&mut machine, Key::Right);
    press(&mut machine, Key::N);
    let effects = machine.popover_confirmed("  Plans ", "teal");
    assert_eq!(
        effects,
        vec![Effect::Apply(GroupOp::CreateGroup {
            tabs: vec![102],
            name: Some("Plans".into()),
            colour: Some("teal".into()),
        })]
    );
    assert!(machine.view().popover.is_none());
}

#[test]
fn confirming_with_no_name_or_colour_leaves_them_to_the_controller() {
    let mut machine = at_group(1);
    press(&mut machine, Key::N);
    let effects = machine.popover_confirmed("   ", "");
    assert_eq!(
        effects,
        vec![Effect::Apply(GroupOp::CreateGroup {
            tabs: vec![101],
            name: None,
            colour: None,
        })]
    );
}

#[test]
fn confirming_without_a_popover_does_nothing() {
    let mut machine = at_group(1);
    assert!(machine.popover_confirmed("x", "blue").is_empty());
}

#[test]
fn confirming_after_the_tab_vanished_closes_the_popover_and_applies_nothing() {
    let mut machine = at_group(1);
    press(&mut machine, Key::N);
    let mut input = orbit_input(false);
    input.tabs.retain(|t| t.id != 101);
    machine.rows_changed(&input);
    assert!(machine.popover_confirmed("x", "blue").is_empty());
    assert!(machine.view().popover.is_none());
}

#[test]
fn editing_the_popover_is_kept_in_the_view() {
    let mut machine = at_group(1);
    press(&mut machine, Key::N);
    machine.popover_edited("Trip", "rose");
    let popover = machine.view().popover.expect("a popover");
    assert_eq!(
        (popover.name.as_str(), popover.colour.as_str()),
        ("Trip", "rose")
    );
}

#[test]
fn slash_and_a_typed_character_focus_the_search_field() {
    for level in 0..3 {
        let mut machine = match level {
            0 => opened(),
            1 => at_group(1),
            _ => at_tab(104),
        };
        assert_eq!(press(&mut machine, Key::Slash), vec![Effect::FocusSearch]);
        assert_eq!(
            press(&mut machine, Key::Char('s')),
            vec![Effect::FocusSearch]
        );
    }
}

#[test]
fn control_characters_whitespace_and_shortcut_chords_do_not_start_a_search() {
    let mut machine = opened();
    assert!(press(&mut machine, Key::Char(' ')).is_empty());
    assert!(press(&mut machine, Key::Char('\u{7f}')).is_empty());
    assert!(command(&mut machine, Key::Char('c')).is_empty());
    assert!(
        machine
            .key(
                Key::Char('å'),
                Mods {
                    alt: true,
                    ..Mods::NONE
                },
                KeyContext::default()
            )
            .is_empty()
    );
    assert!(
        machine
            .key(Key::Slash, Mods::COMMAND, KeyContext::default())
            .is_empty()
    );
}

// ---------------------------------------------------------------------------------------------
// Pinch (D10): p = clamp((scale - 1) / 0.35, -1, 1); in at p = 1, out at p <= -0.7

fn pinch(machine: &mut OrbitMachine, target: Target, scale: f32) -> Vec<Effect> {
    machine.pinch_started(target);
    machine.pinch_updated(scale);
    machine.pinch_ended()
}

#[test]
fn pinch_in_commits_at_p_one_and_springs_back_below_it() {
    for (scale, commits) in [
        (1.35, true),
        (2.0, true),
        (5.0, true),
        (1.34, false),
        (1.2, false),
        (1.0, false),
    ] {
        let mut machine = opened();
        let effects = pinch(&mut machine, Target::Planet(2), scale);
        if commits {
            assert_eq!(machine.view().level, Level::Group, "scale {scale}");
            assert_eq!(machine.view().group, Some(2), "scale {scale}");
            assert_eq!(effects, zoomed(&machine), "scale {scale}");
        } else {
            assert_eq!(effects, vec![Effect::Springback], "scale {scale}");
            assert_eq!(machine.view().level, Level::Groups, "scale {scale}");
        }
        assert_eq!(machine.view().pinch, None, "scale {scale}");
    }
}

#[test]
fn pinch_out_commits_at_p_minus_point_seven_and_springs_back_above_it() {
    for (scale, commits) in [
        (0.75, true),
        (0.5, true),
        (0.1, true),
        (0.76, false),
        (0.9, false),
    ] {
        let mut machine = at_group(1);
        let effects = pinch(&mut machine, Target::Backdrop, scale);
        if commits {
            assert_eq!(machine.view().level, Level::Groups, "scale {scale}");
            assert_eq!(effects, zoomed(&machine), "scale {scale}");
        } else {
            assert_eq!(effects, vec![Effect::Springback], "scale {scale}");
            assert_eq!(machine.view().level, Level::Group, "scale {scale}");
        }
    }
}

#[test]
fn pinch_out_at_each_level_does_what_command_up_does() {
    let mut level3 = at_tab(104);
    pinch(&mut level3, Target::Card, 0.5);
    assert_eq!(
        (level3.view().level, level3.view().tab),
        (Level::Group, Some(104))
    );

    let mut level1 = opened();
    assert_eq!(
        pinch(&mut level1, Target::Backdrop, 0.5),
        vec![Effect::Close { activate: None }]
    );
}

#[test]
fn pinch_in_on_a_preview_goes_to_level_three_and_on_the_card_opens_the_tab() {
    let mut machine = at_group(1);
    pinch(&mut machine, Target::Preview(103), 1.5);
    assert_eq!(
        (machine.view().level, machine.view().tab),
        (Level::Tab, Some(103))
    );
    assert_eq!(
        pinch(&mut machine, Target::Card, 1.5),
        vec![Effect::Close {
            activate: Some(103)
        }]
    );
}

#[test]
fn pinch_updated_drives_the_view_progress_and_clamps_it() {
    let mut machine = opened();
    assert_eq!(machine.view().pinch, None);
    machine.pinch_started(Target::Planet(1));
    assert_eq!(machine.view().pinch, Some(0.0));
    machine.pinch_updated(1.175);
    let p = machine.view().pinch.expect("progress");
    assert!((p - 0.5).abs() < 1e-5, "{p}");
    machine.pinch_updated(9.0);
    assert_eq!(machine.view().pinch, Some(1.0));
    machine.pinch_updated(0.0);
    assert_eq!(machine.view().pinch, Some(-1.0));
    machine.pinch_updated(f32::NAN);
    assert_eq!(machine.view().pinch, Some(-1.0), "a NaN scale is ignored");
}

#[test]
fn the_pinch_target_is_the_one_at_started_even_if_the_pointer_moves() {
    let mut machine = opened();
    machine.pinch_started(Target::Planet(2));
    machine.hover(Target::Planet(4));
    machine.pinch_updated(1.3);
    machine.hover(Target::Planet(5));
    machine.pinch_updated(1.6);
    let effects = machine.pinch_ended();
    assert_eq!(machine.view().group, Some(2));
    assert_eq!(effects, zoomed(&machine));
}

#[test]
fn a_pinch_in_on_empty_space_or_the_sun_springs_back_and_never_picks_the_nearest_planet() {
    for target in [
        Target::Backdrop,
        Target::Sun,
        Target::NewGroup,
        Target::Planet(99),
    ] {
        let mut machine = opened();
        let before = machine.view();
        let effects = pinch(&mut machine, target, 2.0);
        assert_eq!(effects, vec![Effect::Springback], "{target:?}");
        assert_eq!(machine.view(), before, "{target:?}");
    }
    // At level 3 empty space is not "zoom out" for a pinch in either.
    let mut machine = at_tab(104);
    assert_eq!(
        pinch(&mut machine, Target::Backdrop, 2.0),
        vec![Effect::Springback]
    );
    assert_eq!(machine.view().level, Level::Tab);
}

#[test]
fn a_cancelled_gesture_springs_back_and_changes_nothing() {
    let mut machine = opened();
    machine.pinch_started(Target::Planet(2));
    machine.pinch_updated(2.0);
    assert_eq!(machine.pinch_cancelled(), vec![Effect::Springback]);
    assert_eq!(machine.view().pinch, None);
    assert_eq!(machine.view().level, Level::Groups);
    assert!(machine.pinch_ended().is_empty(), "the gesture is over");
    assert!(machine.pinch_cancelled().is_empty());
}

#[test]
fn a_second_gesture_while_the_first_is_up_is_ignored() {
    let mut machine = opened();
    machine.pinch_started(Target::Planet(2));
    machine.pinch_started(Target::Planet(3));
    machine.pinch_updated(1.5);
    machine.pinch_ended();
    assert_eq!(machine.view().group, Some(2), "the first gesture's target");
    // The next gesture, after the first ended, works.
    let mut machine = at_group(2);
    pinch(&mut machine, Target::Backdrop, 0.5);
    machine.pinch_started(Target::Planet(4));
    machine.pinch_updated(1.5);
    machine.pinch_ended();
    assert_eq!(machine.view().group, Some(4));
}

#[test]
fn pinch_calls_without_a_started_gesture_do_nothing() {
    let mut machine = opened();
    machine.pinch_updated(2.0);
    assert_eq!(machine.view().pinch, None);
    assert!(machine.pinch_ended().is_empty());
    assert_eq!(machine.view().level, Level::Groups);
}

#[test]
fn a_pinch_is_ignored_during_a_drag_and_while_the_popover_is_open() {
    let mut machine = opened();
    machine.drag_started(Source::Tab(102));
    machine.pinch_started(Target::Planet(2));
    assert_eq!(machine.view().pinch, None);
    machine.drag_cancelled();

    press(&mut machine, Key::N);
    machine.pinch_started(Target::Planet(2));
    assert_eq!(machine.view().pinch, None);
}

#[test]
fn a_click_during_a_pinch_is_ignored() {
    let mut machine = opened();
    machine.pinch_started(Target::Planet(2));
    assert!(machine.click(Target::Planet(3)).is_empty());
    assert_eq!(machine.view().level, Level::Groups);
}

// ---------------------------------------------------------------------------------------------
// Drag and drop (D11): the machine calls `drag::resolve` and passes its answer on

fn drop_on(machine: &mut OrbitMachine, source: Source, target: DropTarget) -> Vec<Effect> {
    machine.drag_started(source);
    machine.drag_over(Some(target));
    machine.drag_released()
}

#[test]
fn a_drag_in_flight_is_in_the_view_with_what_it_is_over() {
    let mut machine = opened();
    machine.drag_started(Source::Tab(102));
    let drag = machine.view().drag.expect("a drag");
    assert_eq!((drag.source, drag.over), (Source::Tab(102), None));
    let over = DropTarget::Group {
        group: 2,
        slot: Some(3),
    };
    machine.drag_over(Some(over));
    assert_eq!(machine.view().drag.expect("a drag").over, Some(over));
    machine.drag_over(None);
    assert_eq!(machine.view().drag.expect("a drag").over, None);
    machine.drag_cancelled();
    assert!(machine.view().drag.is_none());
}

#[test]
fn a_release_emits_resolve_with_the_decision_of_drag_resolve_for_that_source_and_target() {
    let input = orbit_input(false);
    let cases = [
        (
            Source::Tab(102),
            DropTarget::Group {
                group: 2,
                slot: Some(1),
            },
        ),
        (
            Source::Tab(102),
            DropTarget::Group {
                group: 1,
                slot: Some(5),
            },
        ),
        (
            Source::Tab(105),
            DropTarget::Group {
                group: 3,
                slot: None,
            },
        ),
        (Source::Tab(105), DropTarget::Empty),
        (Source::Tab(105), DropTarget::NewGroup),
        (
            Source::Group(1),
            DropTarget::Group {
                group: 4,
                slot: None,
            },
        ),
        (Source::Group(2), DropTarget::NewGroup),
        (Source::Group(2), DropTarget::Empty),
    ];
    for (source, target) in cases {
        let mut machine = opened();
        let effects = drop_on(&mut machine, source, target);
        let decision = drag::resolve(source, target, &input);
        assert_eq!(
            effects.first(),
            Some(&Effect::Resolve(decision.clone())),
            "{source:?} on {target:?}"
        );
        if let Decision::NewGroup { tab } = decision {
            assert_eq!(
                effects,
                vec![
                    Effect::Resolve(Decision::NewGroup { tab }),
                    Effect::OpenNewGroupPopover { tab }
                ],
                "{source:?} on {target:?}"
            );
            assert_eq!(machine.view().popover.map(|p| p.tab), Some(tab));
        } else {
            assert_eq!(effects.len(), 1, "{source:?} on {target:?}");
        }
        assert!(machine.view().drag.is_none(), "the drag is over");
    }
}

#[test]
fn a_release_over_nothing_resolves_nothing_and_ends_the_drag() {
    let mut machine = opened();
    machine.drag_started(Source::Tab(102));
    assert!(machine.drag_released().is_empty());
    assert!(machine.view().drag.is_none());
}

#[test]
fn a_release_without_a_drag_does_nothing() {
    let mut machine = opened();
    assert!(machine.drag_released().is_empty());
}

#[test]
fn a_cancelled_drag_resolves_nothing() {
    let mut machine = opened();
    machine.drag_started(Source::Tab(102));
    machine.drag_over(Some(DropTarget::Empty));
    machine.drag_cancelled();
    assert!(machine.drag_released().is_empty());
}

#[test]
fn a_drag_cannot_start_from_a_pinned_tab_the_loose_planet_or_an_unknown_id() {
    let mut tabs = orbit_tabs();
    tabs[0].pinned = true;
    let input = OrbitInput::new(&orbit_groups(false), &tabs, "P", "W", 101, false);
    let mut machine = opened_with(&input);
    for source in [
        Source::Tab(101),
        Source::Tab(9999),
        Source::Group(9999),
        Source::Group(LOOSE_GROUP_ID),
    ] {
        machine.drag_started(source);
        assert!(machine.view().drag.is_none(), "{source:?}");
    }
    machine.drag_started(Source::Tab(102));
    assert!(machine.view().drag.is_some());
}

#[test]
fn a_second_drag_while_one_is_in_flight_is_ignored() {
    let mut machine = opened();
    machine.drag_started(Source::Tab(102));
    machine.drag_started(Source::Group(3));
    assert_eq!(
        machine.view().drag.expect("a drag").source,
        Source::Tab(102)
    );
}

#[test]
fn while_a_drag_is_in_flight_only_escape_is_a_key_and_clicks_are_ignored() {
    let mut machine = at_group(1);
    machine.drag_started(Source::Tab(102));
    let before = machine.view();
    for key in [Key::Enter, Key::Right, Key::N, Key::Slash, Key::Char('x')] {
        assert!(press(&mut machine, key).is_empty(), "{key:?}");
    }
    assert!(command(&mut machine, Key::Up).is_empty());
    assert!(machine.click(Target::Preview(103)).is_empty());
    assert_eq!(machine.view(), before);
}

#[test]
fn a_drag_cannot_start_while_the_popover_is_open() {
    let mut machine = opened();
    press(&mut machine, Key::N);
    machine.drag_started(Source::Tab(102));
    assert!(machine.view().drag.is_none());
}

// ---------------------------------------------------------------------------------------------
// Reconcile with changed rows

fn without_tab(input: &OrbitInput, tab: i32) -> OrbitInput {
    let mut input = input.clone();
    input.tabs.retain(|t| t.id != tab);
    input
}

fn without_group(input: &OrbitInput, group: i32) -> OrbitInput {
    let mut input = input.clone();
    input.groups.retain(|g| g.id != group);
    input.tabs.retain(|t| t.group != group);
    input
}

#[test]
fn unchanged_rows_change_nothing_and_say_nothing() {
    for mut machine in [opened(), at_group(1), at_tab(104)] {
        let before = machine.view();
        assert!(machine.rows_changed(&orbit_input(false)).is_empty());
        assert_eq!(machine.view(), before);
    }
}

#[test]
fn a_group_that_disappears_while_shown_at_level_two_drops_to_level_one() {
    let mut machine = at_group(2);
    let effects = machine.rows_changed(&without_group(&orbit_input(false), 2));
    let view = machine.view();
    assert_eq!(view.level, Level::Groups);
    assert_eq!(view.tab, None);
    assert_eq!(
        view.group,
        Some(3),
        "the group that took its place in the order"
    );
    assert_eq!(view.focus, Some(Target::Planet(3)));
    assert_eq!(effects, zoomed(&machine));
}

#[test]
fn a_group_that_disappears_while_shown_at_level_three_drops_to_level_one() {
    let mut machine = at_tab(104);
    let effects = machine.rows_changed(&without_group(&orbit_input(false), 1));
    let view = machine.view();
    assert_eq!((view.level, view.tab), (Level::Groups, None));
    assert_eq!(view.group, Some(2), "the group that took its place");
    assert_eq!(effects, zoomed(&machine));
}

#[test]
fn the_focused_planet_that_disappears_moves_the_focus_to_its_nearest_neighbour() {
    let mut machine = opened();
    press(&mut machine, Key::Right);
    press(&mut machine, Key::Right);
    assert_eq!(machine.view().group, Some(3));
    let effects = machine.rows_changed(&without_group(&orbit_input(false), 3));
    assert_eq!(
        machine.view().focus,
        Some(Target::Planet(4)),
        "same place in the order"
    );
    assert!(effects.is_empty(), "the level did not change");

    let mut machine = opened();
    for _ in 0..4 {
        press(&mut machine, Key::Right);
    }
    machine.rows_changed(&without_group(&orbit_input(false), 5));
    assert_eq!(
        machine.view().focus,
        Some(Target::Planet(4)),
        "the last one: the one before"
    );
}

#[test]
fn a_planet_that_is_not_the_focused_one_may_go_without_moving_the_focus() {
    let mut machine = opened();
    press(&mut machine, Key::Right);
    machine.rows_changed(&without_group(&orbit_input(false), 4));
    assert_eq!(machine.view().focus, Some(Target::Planet(2)));
}

#[test]
fn a_tab_that_disappears_at_level_three_drops_to_level_two_on_its_neighbour() {
    let mut machine = at_tab(104);
    let effects = machine.rows_changed(&without_tab(&orbit_input(false), 104));
    let view = machine.view();
    assert_eq!((view.level, view.group), (Level::Group, Some(1)));
    assert_eq!(view.tab, Some(105), "the tab that took its place");
    assert_eq!(view.focus, Some(Target::Preview(105)));
    assert_eq!(effects, zoomed(&machine));

    let mut machine = at_tab(110);
    machine.rows_changed(&without_tab(&orbit_input(false), 110));
    assert_eq!(
        (machine.view().level, machine.view().tab),
        (Level::Group, Some(109))
    );
}

#[test]
fn the_last_tab_of_the_group_going_at_level_three_leaves_level_two_with_no_focus() {
    let mut input = orbit_input(false);
    input.tabs.retain(|t| t.group != 2 || t.id == 111);
    let mut machine = opened_with(&input);
    machine.click(Target::Planet(2));
    machine.click(Target::Preview(111));
    assert_eq!(machine.view().level, Level::Tab);
    machine.rows_changed(&without_tab(&input, 111));
    let view = machine.view();
    assert_eq!(
        (view.level, view.group, view.tab, view.focus),
        (Level::Group, Some(2), None, None)
    );
}

#[test]
fn a_tab_that_moved_to_another_group_leaves_level_three_of_the_old_one() {
    let mut machine = at_tab(104);
    let mut input = orbit_input(false);
    for tab in &mut input.tabs {
        if tab.id == 104 {
            tab.group = 2;
            tab.index_in_group = 8;
        }
    }
    machine.rows_changed(&input);
    let view = machine.view();
    assert_eq!((view.level, view.group), (Level::Group, Some(1)));
    assert_ne!(view.tab, Some(104));
    assert!(view.tab.is_some());
}

#[test]
fn the_focused_preview_that_disappears_moves_the_focus_to_its_neighbour_at_level_two() {
    let mut machine = at_group(1);
    for _ in 0..4 {
        press(&mut machine, Key::Right);
    }
    assert_eq!(machine.view().tab, Some(105));
    let effects = machine.rows_changed(&without_tab(&orbit_input(false), 105));
    assert_eq!(machine.view().tab, Some(106));
    assert_eq!(machine.view().focus, Some(Target::Preview(106)));
    assert!(effects.is_empty());
}

#[test]
fn no_tab_left_at_all_keeps_level_one_with_the_sun_only() {
    for mut machine in [at_group(1), at_tab(104)] {
        let mut input = orbit_input(false);
        input.tabs.clear();
        input.groups.clear();
        let effects = machine.rows_changed(&input);
        let view = machine.view();
        assert_eq!(
            (view.level, view.group, view.tab, view.focus),
            (Level::Groups, None, None, None)
        );
        assert_eq!(effects, zoomed(&machine));
    }
}

#[test]
fn groups_without_any_tab_stay_at_level_one_with_a_planet_focused() {
    let mut machine = at_group(2);
    let mut input = orbit_input(false);
    input.tabs.clear();
    machine.rows_changed(&input);
    assert_eq!(machine.view().level, Level::Groups);
    assert_eq!(machine.view().focus, Some(Target::Planet(2)));
}

#[test]
fn rows_arriving_after_none_give_level_one_a_focus() {
    let mut machine = OrbitMachine::new(1);
    machine.open(&OrbitInput::default());
    machine.rows_changed(&orbit_input(false));
    assert_eq!(machine.view().focus, Some(Target::Planet(1)));
    assert_eq!(machine.view().group, Some(1));
}

#[test]
fn a_drag_is_cancelled_when_its_source_is_gone_and_kept_when_it_is_not() {
    let mut machine = opened();
    machine.drag_started(Source::Tab(105));
    machine.rows_changed(&without_tab(&orbit_input(false), 106));
    assert!(machine.view().drag.is_some());
    machine.rows_changed(&without_tab(&orbit_input(false), 105));
    assert!(machine.view().drag.is_none());

    machine.drag_started(Source::Group(4));
    machine.rows_changed(&without_group(&orbit_input(false), 4));
    assert!(machine.view().drag.is_none());
}

#[test]
fn a_tab_that_became_pinned_cancels_its_drag() {
    let mut machine = opened();
    machine.drag_started(Source::Tab(105));
    let mut tabs = orbit_tabs();
    tabs[4].pinned = true;
    machine.rows_changed(&OrbitInput::new(
        &orbit_groups(false),
        &tabs,
        "P",
        "W",
        101,
        false,
    ));
    assert!(machine.view().drag.is_none());
}

#[test]
fn the_popover_closes_when_its_tab_is_gone() {
    let mut machine = at_group(1);
    press(&mut machine, Key::N);
    machine.rows_changed(&without_tab(&orbit_input(false), 102));
    assert!(
        machine.view().popover.is_some(),
        "tab 101 is the focused one and is still there"
    );
    machine.rows_changed(&without_tab(&orbit_input(false), 101));
    assert!(machine.view().popover.is_none());
}

#[test]
fn a_hover_on_something_that_is_gone_is_cleared() {
    let mut machine = opened();
    machine.hover(Target::Planet(4));
    machine.rows_changed(&without_group(&orbit_input(false), 2));
    assert_eq!(machine.view().hover, Some(Target::Planet(4)));
    machine.rows_changed(&without_group(&orbit_input(false), 4));
    assert_eq!(machine.view().hover, None);
    machine.hover(Target::Sun);
    machine.rows_changed(&orbit_input(false));
    assert_eq!(machine.view().hover, Some(Target::Sun));
}

#[test]
fn level_two_order_follows_a_changed_active_tab_without_moving_the_focus() {
    let mut machine = at_group(1);
    press(&mut machine, Key::Right);
    assert_eq!(machine.view().tab, Some(102));
    let input = OrbitInput {
        active_tab: 105,
        ..orbit_input(false)
    };
    machine.rows_changed(&input);
    assert_eq!(machine.view().tab, Some(102));
    press(&mut machine, Key::Left);
    press(&mut machine, Key::Left);
    assert_eq!(machine.view().tab, Some(105), "the new first");
}

// ---------------------------------------------------------------------------------------------
// Every (level, event) pair has a defined result

#[derive(Clone, Copy, Debug, PartialEq)]
enum Event {
    Click(Target),
    Press(Key),
    CommandUp,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Outcome {
    Nothing,
    /// The level changed to this one.
    Zoom(Level),
    /// Same level, other group, tab or focus.
    Moved,
    Close(Option<i32>),
    Popover,
    FocusSearch,
}

/// Every kind of target, with ids that exist in the fixture (Planet 2, Preview 103, Chip 2,
/// Neighbour 105). The match makes a new `Target` or `Key` variant fail to compile here.
fn every_event() -> Vec<Event> {
    fn _targets_are_listed(target: Target) {
        match target {
            Target::Planet(_)
            | Target::Sun
            | Target::Preview(_)
            | Target::Chip(_)
            | Target::NewGroup
            | Target::Card
            | Target::Neighbour(_)
            | Target::Backdrop => {}
        }
    }
    fn _keys_are_listed(key: Key) {
        match key {
            Key::Escape
            | Key::Enter
            | Key::Up
            | Key::Down
            | Key::Left
            | Key::Right
            | Key::N
            | Key::Slash
            | Key::Char(_) => {}
        }
    }
    let mut events: Vec<Event> = [
        Target::Planet(2),
        Target::Sun,
        Target::Preview(103),
        Target::Chip(2),
        Target::NewGroup,
        Target::Card,
        Target::Neighbour(105),
        Target::Backdrop,
    ]
    .into_iter()
    .map(Event::Click)
    .collect();
    events.extend(
        [
            Key::Escape,
            Key::Enter,
            Key::Up,
            Key::Down,
            Key::Left,
            Key::Right,
            Key::N,
            Key::Slash,
            Key::Char('x'),
        ]
        .into_iter()
        .map(Event::Press),
    );
    events.push(Event::CommandUp);
    events
}

fn classify(before: &View, after: &View, effects: &[Effect]) -> Outcome {
    for effect in effects {
        match effect {
            Effect::Close { activate } => return Outcome::Close(*activate),
            Effect::OpenNewGroupPopover { .. } => return Outcome::Popover,
            Effect::FocusSearch => return Outcome::FocusSearch,
            _ => {}
        }
    }
    if before.level != after.level {
        Outcome::Zoom(after.level)
    } else if before.group != after.group || before.tab != after.tab || before.focus != after.focus
    {
        Outcome::Moved
    } else {
        Outcome::Nothing
    }
}

fn run(level: u8, event: Event) -> Outcome {
    let mut machine = match level {
        1 => opened(),
        2 => at_group(1),
        _ => at_tab(104),
    };
    let before = machine.view();
    let effects = match event {
        Event::Click(target) => machine.click(target),
        Event::Press(key) => press(&mut machine, key),
        Event::CommandUp => command(&mut machine, Key::Up),
    };
    classify(&before, &machine.view(), &effects)
}

#[test]
fn every_level_and_event_pair_has_the_documented_result() {
    use Outcome::*;
    let closed = Close(None);
    // (event, level 1, level 2, level 3) for the fixture's Research with tab 104 at level 3.
    let table: Vec<(Event, Outcome, Outcome, Outcome)> = vec![
        (
            Event::Click(Target::Planet(2)),
            Zoom(Level::Group),
            Nothing,
            Nothing,
        ),
        (Event::Click(Target::Sun), Nothing, Nothing, Nothing),
        (
            Event::Click(Target::Preview(103)),
            Nothing,
            Zoom(Level::Tab),
            Nothing,
        ),
        (Event::Click(Target::Chip(2)), Nothing, Moved, Nothing),
        (Event::Click(Target::NewGroup), Nothing, Nothing, Nothing),
        (
            Event::Click(Target::Card),
            Nothing,
            Nothing,
            Close(Some(104)),
        ),
        (
            Event::Click(Target::Neighbour(105)),
            Nothing,
            Nothing,
            Moved,
        ),
        (
            Event::Click(Target::Backdrop),
            Nothing,
            Nothing,
            Zoom(Level::Group),
        ),
        (Event::Press(Key::Escape), closed, closed, closed),
        (
            Event::Press(Key::Enter),
            Zoom(Level::Group),
            Zoom(Level::Tab),
            Close(Some(104)),
        ),
        (Event::Press(Key::Up), Nothing, Nothing, Nothing),
        (Event::Press(Key::Down), Moved, Nothing, Nothing),
        (Event::Press(Key::Left), Nothing, Nothing, Moved),
        (Event::Press(Key::Right), Moved, Moved, Moved),
        (Event::Press(Key::N), Popover, Popover, Popover),
        (
            Event::Press(Key::Slash),
            FocusSearch,
            FocusSearch,
            FocusSearch,
        ),
        (
            Event::Press(Key::Char('x')),
            FocusSearch,
            FocusSearch,
            FocusSearch,
        ),
        (
            Event::CommandUp,
            closed,
            Zoom(Level::Groups),
            Zoom(Level::Group),
        ),
    ];
    let events = every_event();
    assert_eq!(
        table.len(),
        events.len(),
        "the table lists every event once"
    );
    for event in &events {
        assert!(
            table.iter().any(|row| row.0 == *event),
            "{event:?} is not in the table"
        );
    }
    for (event, level1, level2, level3) in table {
        assert_eq!(run(1, event), level1, "level 1, {event:?}");
        assert_eq!(run(2, event), level2, "level 2, {event:?}");
        assert_eq!(run(3, event), level3, "level 3, {event:?}");
    }
}

// ---------------------------------------------------------------------------------------------
// 1,000 random event sequences never leave an impossible state

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
    fn chance(&mut self, percent: u64) -> bool {
        self.below(100) < percent
    }
}

fn random_input(rng: &mut Rng) -> OrbitInput {
    let groups: Vec<GroupRow> = orbit_groups(false)
        .into_iter()
        .filter(|_| rng.chance(85))
        .collect();
    let mut tabs: Vec<TabRow> = orbit_tabs()
        .into_iter()
        .filter(|_| rng.chance(80))
        .collect();
    for tab in &mut tabs {
        tab.pinned = rng.chance(8);
    }
    let active = tabs
        .get(rng.below(tabs.len().max(1) as u64) as usize)
        .map_or(101, |t| t.id);
    OrbitInput::new(&groups, &tabs, "P", "W", active, false)
}

fn random_target(rng: &mut Rng) -> Target {
    let id = |rng: &mut Rng, base: i32, n: u64| base + rng.below(n) as i32;
    match rng.below(8) {
        0 => Target::Planet(if rng.chance(15) {
            LOOSE_GROUP_ID
        } else {
            id(rng, 0, 7)
        }),
        1 => Target::Sun,
        2 => Target::Preview(id(rng, 99, 45)),
        3 => Target::Chip(id(rng, 0, 7)),
        4 => Target::NewGroup,
        5 => Target::Card,
        6 => Target::Neighbour(id(rng, 99, 45)),
        _ => Target::Backdrop,
    }
}

fn random_key(rng: &mut Rng) -> Key {
    match rng.below(9) {
        0 => Key::Escape,
        1 => Key::Enter,
        2 => Key::Up,
        3 => Key::Down,
        4 => Key::Left,
        5 => Key::Right,
        6 => Key::N,
        7 => Key::Slash,
        _ => Key::Char(['x', ' ', 's', 'é'][rng.below(4) as usize]),
    }
}

fn random_drop(rng: &mut Rng) -> Option<DropTarget> {
    match rng.below(4) {
        0 => None,
        1 => Some(DropTarget::Empty),
        2 => Some(DropTarget::NewGroup),
        _ => Some(DropTarget::Group {
            group: rng.below(7) as i32,
            slot: if rng.chance(50) {
                Some(rng.below(12) as usize)
            } else {
                None
            },
        }),
    }
}

fn check_invariants(machine: &OrbitMachine, input: &OrbitInput, context: &str) {
    let view = machine.view();
    let group_ok = |g: i32| input.group(g).is_some();
    match view.level {
        Level::Tab => {
            let group = view
                .group
                .unwrap_or_else(|| panic!("level 3 without a group {context}"));
            let tab = view
                .tab
                .unwrap_or_else(|| panic!("level 3 without a tab {context}"));
            assert!(group_ok(group), "level 3 group {group} is gone {context}");
            assert_eq!(
                input.tab(tab).map(|t| t.group),
                Some(group),
                "level 3 tab {tab} {context}"
            );
            assert_eq!(view.focus, Some(Target::Card), "{context}");
        }
        Level::Group => {
            let group = view
                .group
                .unwrap_or_else(|| panic!("level 2 without a group {context}"));
            assert!(group_ok(group), "level 2 group {group} is gone {context}");
            match view.tab {
                Some(tab) => {
                    assert_eq!(input.tab(tab).map(|t| t.group), Some(group), "{context}");
                    assert_eq!(view.focus, Some(Target::Preview(tab)), "{context}");
                }
                None => {
                    assert_eq!(view.focus, None, "{context}");
                    assert!(
                        preview_order(input, group).is_empty(),
                        "no focus on a group with tabs {context}"
                    );
                }
            }
        }
        Level::Groups => {
            assert_eq!(view.tab, None, "{context}");
            match view.group {
                Some(group) => {
                    assert!(group_ok(group), "level 1 group {group} is gone {context}");
                    assert_eq!(view.focus, Some(Target::Planet(group)), "{context}");
                }
                None => {
                    assert!(
                        input.groups.is_empty(),
                        "no focus with groups present {context}"
                    );
                    assert_eq!(view.focus, None, "{context}");
                }
            }
        }
    }
    if let Some(drag) = &view.drag {
        match drag.source {
            Source::Tab(tab) => {
                assert!(
                    input.tab(tab).is_some() && !input.is_pinned(tab),
                    "drag of tab {tab} {context}"
                );
            }
            Source::Group(group) => {
                assert!(
                    group_ok(group) && group != LOOSE_GROUP_ID,
                    "drag of group {group} {context}"
                );
            }
        }
    }
    if let Some(popover) = &view.popover {
        assert!(
            input.tab(popover.tab).is_some(),
            "popover of a gone tab {context}"
        );
    }
    if let Some(p) = view.pinch {
        assert!((-1.0..=1.0).contains(&p), "pinch {p} {context}");
    }
    assert!(
        view.drag.is_none() || view.popover.is_none(),
        "a drag and a popover {context}"
    );
}

#[test]
fn a_thousand_random_event_sequences_never_leave_an_impossible_state() {
    let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
    for sequence in 0..1_000 {
        let mut input = if sequence % 10 == 0 {
            orbit_input(false)
        } else {
            random_input(&mut rng)
        };
        let mut machine = OrbitMachine::new(sequence);
        machine.open(&input);
        check_invariants(
            &machine,
            &input,
            &format!("after open, sequence {sequence}"),
        );
        let mut log: Vec<String> = Vec::new();
        for step in 0..60 {
            let what = match rng.below(15) {
                0..=2 => {
                    let target = random_target(&mut rng);
                    machine.click(target);
                    format!("click {target:?}")
                }
                3 => {
                    let target = random_target(&mut rng);
                    machine.hover(target);
                    format!("hover {target:?}")
                }
                4 | 5 => {
                    let key = random_key(&mut rng);
                    let mods = Mods {
                        command: rng.chance(20),
                        alt: rng.chance(5),
                        ..Mods::NONE
                    };
                    let context = KeyContext {
                        text_input_focused: rng.chance(15),
                    };
                    machine.key(key, mods, context);
                    format!("key {key:?} {mods:?} {context:?}")
                }
                6 => {
                    let target = random_target(&mut rng);
                    machine.pinch_started(target);
                    format!("pinch_started {target:?}")
                }
                7 => {
                    let scale = 0.2 + rng.below(220) as f32 / 100.0;
                    machine.pinch_updated(scale);
                    format!("pinch_updated {scale}")
                }
                8 => {
                    if rng.chance(70) {
                        machine.pinch_ended();
                        "pinch_ended".to_string()
                    } else {
                        machine.pinch_cancelled();
                        "pinch_cancelled".to_string()
                    }
                }
                9 => {
                    let source = if rng.chance(70) {
                        Source::Tab(99 + rng.below(45) as i32)
                    } else {
                        Source::Group(rng.below(7) as i32 - 1)
                    };
                    machine.drag_started(source);
                    format!("drag_started {source:?}")
                }
                10 => {
                    let over = random_drop(&mut rng);
                    machine.drag_over(over);
                    format!("drag_over {over:?}")
                }
                11 => {
                    if rng.chance(70) {
                        machine.drag_released();
                        "drag_released".to_string()
                    } else {
                        machine.drag_cancelled();
                        "drag_cancelled".to_string()
                    }
                }
                12 => {
                    if rng.chance(50) {
                        input = random_input(&mut rng);
                        machine.rows_changed(&input);
                        "rows_changed".to_string()
                    } else {
                        let query = ["", "servo", "zzz", "a"][rng.below(4) as usize];
                        machine.search_changed(query);
                        format!("search {query:?}")
                    }
                }
                13 => {
                    machine.popover_confirmed("n", "teal");
                    "popover_confirmed".to_string()
                }
                _ => {
                    machine.popover_edited("x", "rose");
                    "popover_edited".to_string()
                }
            };
            log.push(what.clone());
            check_invariants(
                &machine,
                &input,
                &format!("sequence {sequence} step {step} after `{what}`; history {log:?}"),
            );
        }
    }
}
