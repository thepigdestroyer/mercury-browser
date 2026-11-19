// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! Tab groups in the session file: the fields, the bounded decoding, the sanitising rules of the
//! groups-core brief (section 7) and `pop_closed_run`.

use mercury_settings::{
    ClosedTab, GroupSnapshot, MAX_CLOSED_TABS, MAX_GROUP_NAME_BYTES, MAX_GROUPS_PER_WINDOW,
    MAX_RESTORED_TABS, PreviousSession, SessionSnapshot, SessionStore, TabSnapshot, WindowSnapshot,
};

/// The largest group id the interface can carry (`i32::MAX`); the session keeps the counter
/// at or below it.
const MAX_COUNTER: u64 = i32::MAX as u64;

fn tab(n: usize) -> TabSnapshot {
    TabSnapshot {
        url: format!("https://site{n}.example/"),
        title: format!("site {n}"),
        ..TabSnapshot::default()
    }
}

fn in_group(n: usize, group: u64) -> TabSnapshot {
    TabSnapshot {
        group: Some(group),
        ..tab(n)
    }
}

fn pinned_in_group(n: usize, group: u64) -> TabSnapshot {
    TabSnapshot {
        pinned: true,
        ..in_group(n, group)
    }
}

fn pinned_first() -> TabSnapshot {
    TabSnapshot {
        pinned: true,
        ..tab(0)
    }
}

fn bad_in_group(group: u64) -> TabSnapshot {
    TabSnapshot {
        url: "javascript:alert(1)".to_owned(),
        group: Some(group),
        ..TabSnapshot::default()
    }
}

fn group(id: u64, name: &str) -> GroupSnapshot {
    GroupSnapshot {
        id,
        name: name.to_owned(),
        colour: "blue".to_owned(),
        key: None,
    }
}

fn keyed(id: u64, name: &str, key: &str) -> GroupSnapshot {
    GroupSnapshot {
        key: Some(key.to_owned()),
        ..group(id, name)
    }
}

fn window(tabs: Vec<TabSnapshot>, groups: Vec<GroupSnapshot>) -> WindowSnapshot {
    WindowSnapshot {
        tabs,
        groups,
        ..WindowSnapshot::default()
    }
}

fn snapshot(windows: Vec<WindowSnapshot>) -> SessionSnapshot {
    SessionSnapshot {
        windows,
        ..SessionSnapshot::default()
    }
}

/// The only window of `window`'s snapshot, sanitized.
fn settled(window: WindowSnapshot) -> WindowSnapshot {
    let mut windows = snapshot(vec![window]).sanitized().windows;
    assert_eq!(windows.len(), 1, "the window kept its tabs");
    windows.remove(0)
}

/// One tab in group 1 with the given group.
fn settled_group(group: GroupSnapshot) -> GroupSnapshot {
    let id = group.id;
    let mut window = settled(window(vec![in_group(1, id)], vec![group]));
    assert_eq!(window.groups.len(), 1, "the group kept its tab");
    window.groups.remove(0)
}

fn names(window: &WindowSnapshot) -> Vec<&str> {
    window.groups.iter().map(|g| g.name.as_str()).collect()
}

fn group_ids(window: &WindowSnapshot) -> Vec<u64> {
    window.groups.iter().map(|g| g.id).collect()
}

fn tab_groups(window: &WindowSnapshot) -> Vec<Option<u64>> {
    window.tabs.iter().map(|tab| tab.group).collect()
}

fn decode(json: &str) -> SessionSnapshot {
    serde_json::from_str(json).expect("deserialise")
}

// ---------------------------------------------------------------------------------------------
// The fields and the file

#[test]
fn the_limits_are_64_groups_and_64_name_bytes() {
    assert_eq!(MAX_GROUPS_PER_WINDOW, 64);
    assert_eq!(MAX_GROUP_NAME_BYTES, 64);
}

#[test]
fn groups_survive_a_serde_round_trip() {
    let shared = GroupSnapshot {
        id: 9,
        name: "Shopping \u{1f6d2}".to_owned(),
        colour: "#2f6fd6".to_owned(),
        key: None,
    };
    let original = SessionSnapshot {
        windows: vec![WindowSnapshot {
            tabs: vec![
                in_group(1, 7),
                in_group(2, 7),
                tab(3),
                in_group(4, 9),
                pinned_first(),
            ],
            active_tab: 2,
            groups: vec![keyed(7, "Research", "KeyA"), shared.clone()],
            next_group_id: 10,
            dock_shrunk: Some(true),
            dial_corner: Some("bottom_left".to_owned()),
            ..WindowSnapshot::default()
        }],
        closed_tabs: vec![ClosedTab {
            tab: in_group(5, 9),
            group: Some(shared),
            window: 0,
            index: 3,
            closed_at_ms: 1_763_200_000_000,
        }],
        saved_at_ms: 1_763_200_005_000,
    };

    let json = serde_json::to_string(&original).expect("serialise");
    let back = decode(&json);

    assert_eq!(back, original);
    assert_eq!(
        back.sanitized(),
        original,
        "a clean snapshot with groups is unchanged"
    );
}

#[test]
fn the_file_names_the_new_fields() {
    let original = snapshot(vec![WindowSnapshot {
        tabs: vec![in_group(1, 7)],
        groups: vec![keyed(7, "Research", "KeyA")],
        next_group_id: 8,
        dock_shrunk: Some(false),
        dial_corner: Some("top_right".to_owned()),
        ..WindowSnapshot::default()
    }]);

    let value = serde_json::to_value(&original).expect("serialise");
    let window = &value["windows"][0];

    assert_eq!(window["tabs"][0]["group"], 7);
    assert_eq!(window["groups"][0]["id"], 7);
    assert_eq!(window["groups"][0]["name"], "Research");
    assert_eq!(window["groups"][0]["colour"], "blue");
    assert_eq!(window["groups"][0]["key"], "KeyA");
    assert_eq!(window["next_group_id"], 8);
    assert_eq!(window["dock_shrunk"], false);
    assert_eq!(window["dial_corner"], "top_right");
}

#[test]
fn a_snapshot_without_groups_writes_none_of_the_new_fields() {
    let session = SessionSnapshot {
        windows: vec![window(vec![tab(1)], Vec::new())],
        closed_tabs: vec![ClosedTab {
            tab: tab(2),
            ..ClosedTab::default()
        }],
        ..SessionSnapshot::default()
    };

    let json = serde_json::to_string(&session).expect("serialise");

    for field in [
        "group",
        "groups",
        "next_group_id",
        "dock_shrunk",
        "dial_corner",
    ] {
        assert!(
            !json.contains(&format!("\"{field}\"")),
            "{field} is written only when it says something: {json}"
        );
    }
}

#[test]
fn an_old_file_loads_without_groups() {
    let old = r#"{
      "windows": [{
        "tabs": [
          {"url": "https://example.org/", "title": "Example", "zoom_percent": 125, "pinned": true},
          {"url": "about:blank", "title": "", "zoom_percent": 100, "pinned": false}
        ],
        "active_tab": 1,
        "bounds": {"x": 80, "y": 60, "width": 1280, "height": 800},
        "maximized": false,
        "fullscreen": false
      }],
      "closed_tabs": [{
        "tab": {"url": "https://www.rust-lang.org/", "title": "Rust", "zoom_percent": 100, "pinned": false},
        "window": 0, "index": 2, "closed_at_ms": 1763200000000
      }],
      "saved_at_ms": 1763200005000
    }"#;

    let session = decode(old);
    let window = &session.windows[0];

    assert!(window.groups.is_empty());
    assert_eq!(window.next_group_id, 0);
    assert_eq!(window.dock_shrunk, None);
    assert_eq!(window.dial_corner, None);
    assert_eq!(tab_groups(window), vec![None, None]);
    assert_eq!(session.closed_tabs[0].group, None);
    assert_eq!(session.closed_tabs[0].tab.group, None);
    assert_eq!(session.clone().sanitized(), session, "and nothing changes");
}

#[test]
fn the_new_fields_default_when_a_group_is_partly_written() {
    let session = decode(
        r#"{"windows": [{"tabs": [{"url": "https://a.example/", "group": 3}],
            "groups": [{"id": 3}]}]}"#,
    );

    assert_eq!(
        session.windows[0].groups,
        vec![GroupSnapshot {
            id: 3,
            name: String::new(),
            colour: String::new(),
            key: None,
        }]
    );
    let settled = session.sanitized();
    assert_eq!(names(&settled.windows[0]), vec!["Group 1"]);
    assert_eq!(settled.windows[0].groups[0].colour, "");
}

// ---------------------------------------------------------------------------------------------
// Rule 5 and 6: references, pinned tabs, empty groups

#[test]
fn a_tab_naming_a_missing_group_is_loose() {
    let window = settled(window(
        vec![in_group(1, 7), in_group(2, 3), tab(3), in_group(4, 7)],
        vec![group(7, "Seven")],
    ));

    assert_eq!(tab_groups(&window), vec![Some(7), None, None, Some(7)]);
    assert_eq!(group_ids(&window), vec![7]);
}

#[test]
fn a_tab_naming_a_group_in_a_window_without_groups_is_loose() {
    let window = settled(window(vec![in_group(1, 7), tab(2)], Vec::new()));

    assert_eq!(tab_groups(&window), vec![None, None]);
    assert!(window.groups.is_empty());
}

#[test]
fn a_pinned_tab_loses_its_group() {
    let window = settled(window(
        vec![pinned_in_group(1, 7), in_group(2, 7)],
        vec![group(7, "Seven")],
    ));

    assert_eq!(tab_groups(&window), vec![None, Some(7)]);
    assert!(window.tabs[0].pinned);
    assert_eq!(group_ids(&window), vec![7]);
}

#[test]
fn a_group_with_only_pinned_tabs_is_dropped() {
    let window = settled(window(
        vec![pinned_in_group(1, 7), tab(2)],
        vec![group(7, "Seven")],
    ));

    assert_eq!(tab_groups(&window), vec![None, None]);
    assert!(window.groups.is_empty());
}

#[test]
fn a_group_without_tabs_is_dropped() {
    let window = settled(window(
        vec![in_group(1, 8), tab(2)],
        vec![group(7, "Empty"), group(8, "Used"), group(9, "Also empty")],
    ));

    assert_eq!(group_ids(&window), vec![8]);
}

#[test]
fn a_group_whose_tabs_all_had_bad_urls_is_dropped() {
    let window = settled(window(
        vec![bad_in_group(7), tab(1), in_group(2, 8), bad_in_group(8)],
        vec![group(7, "Gone"), group(8, "Kept")],
    ));

    assert_eq!(window.tabs.len(), 2);
    assert_eq!(group_ids(&window), vec![8]);
    assert_eq!(tab_groups(&window), vec![None, Some(8)]);
}

#[test]
fn a_group_emptied_by_the_tab_cap_is_dropped_and_one_that_kept_a_tab_stays() {
    // 503 tabs: the cap keeps the active tab (the last) and the tabs nearest the end, so tabs 0,
    // 1 and 2 go. Group 1 had only tab 0; group 2 had tabs 1, 2 and 3, and keeps tab 3.
    let mut tabs: Vec<TabSnapshot> = (0..MAX_RESTORED_TABS + 3).map(tab).collect();
    tabs[0].group = Some(1);
    for tab in &mut tabs[1..=3] {
        tab.group = Some(2);
    }
    let window = settled(WindowSnapshot {
        tabs,
        groups: vec![group(1, "Went with the cap"), group(2, "Kept a tab")],
        active_tab: MAX_RESTORED_TABS + 2,
        ..WindowSnapshot::default()
    });

    assert_eq!(window.tabs.len(), MAX_RESTORED_TABS);
    assert_eq!(group_ids(&window), vec![2]);
    assert_eq!(window.tabs[0].url, "https://site3.example/");
    assert_eq!(window.tabs[0].group, Some(2));
    assert_eq!(window.tabs[1].group, None);
}

#[test]
fn the_cap_across_windows_keeps_the_groups_that_kept_tabs() {
    let first = WindowSnapshot {
        tabs: (0..300)
            .map(|n| in_group(n, 1))
            .chain(std::iter::once(tab(300)))
            .collect(),
        active_tab: 300,
        groups: vec![group(1, "First")],
        ..WindowSnapshot::default()
    };
    let second = WindowSnapshot {
        tabs: (0..300)
            .map(|n| in_group(n + 1000, 1))
            .chain(std::iter::once(tab(1300)))
            .collect(),
        active_tab: 300,
        groups: vec![group(1, "Second")],
        ..WindowSnapshot::default()
    };

    let settled = snapshot(vec![first, second]).sanitized();

    let total: usize = settled.windows.iter().map(|w| w.tabs.len()).sum();
    assert_eq!(total, MAX_RESTORED_TABS);
    for window in &settled.windows {
        let grouped = window.tabs.iter().filter(|t| t.group == Some(1)).count();
        assert!(grouped > 0, "each window kept some grouped tabs");
        assert_eq!(group_ids(window), vec![1]);
    }
}

// ---------------------------------------------------------------------------------------------
// Rule 1: ids and the 64 limit

#[test]
fn a_repeated_group_id_keeps_the_first_group() {
    let window = settled(window(
        vec![in_group(1, 7), in_group(2, 7), in_group(3, 8)],
        vec![group(7, "First"), group(7, "Second"), group(8, "Third")],
    ));

    assert_eq!(names(&window), vec!["First", "Third"]);
    assert_eq!(tab_groups(&window), vec![Some(7), Some(7), Some(8)]);
}

#[test]
fn sixty_five_groups_are_cut_to_sixty_four_on_decode() {
    let group_list: Vec<GroupSnapshot> = (1..=65).map(|id| group(id, &format!("g{id}"))).collect();
    let tabs: Vec<TabSnapshot> = (1..=65).map(|id| in_group(id as usize, id)).collect();
    let json = serde_json::to_string(&snapshot(vec![window(tabs, group_list)])).expect("serialise");

    let session = decode(&json);

    assert_eq!(session.windows[0].groups.len(), MAX_GROUPS_PER_WINDOW);
    let settled = session.sanitized();
    let window = &settled.windows[0];
    assert_eq!(window.groups.len(), MAX_GROUPS_PER_WINDOW);
    assert_eq!(window.groups[63].id, 64);
    assert_eq!(window.tabs[64].group, None, "the 65th group is gone");
    assert_eq!(window.tabs[63].group, Some(64));
}

#[test]
fn sanitizing_cuts_a_snapshot_built_in_memory_to_sixty_four_groups_too() {
    let groups: Vec<GroupSnapshot> = (1..=70).map(|id| group(id, "g")).collect();
    let tabs: Vec<TabSnapshot> = (1..=70).map(|id| in_group(id as usize, id)).collect();

    let window = settled(window(tabs, groups));

    assert_eq!(window.groups.len(), MAX_GROUPS_PER_WINDOW);
    assert_eq!(tab_groups(&window)[64..], vec![None::<u64>; 6]);
}

// ---------------------------------------------------------------------------------------------
// Rule 2: names

#[test]
fn names_lose_control_characters_and_outer_white_space() {
    let cleaned = |raw: &str| settled_group(group(1, raw)).name;

    assert_eq!(cleaned("  Work \t "), "Work");
    assert_eq!(cleaned("Wo\trk\u{7}\n"), "Work");
    assert_eq!(cleaned("\n  x"), "x");
    assert_eq!(cleaned("My  group"), "My  group", "inner spaces stay");
    assert_eq!(cleaned("Bücher \u{1f4da}"), "Bücher \u{1f4da}");
}

#[test]
fn an_empty_name_becomes_group_and_its_position() {
    let window = settled(window(
        vec![
            in_group(1, 1),
            in_group(2, 2),
            in_group(3, 3),
            in_group(4, 4),
        ],
        vec![
            group(1, "Named"),
            group(2, ""),
            group(3, "   "),
            group(4, "\u{7}\n"),
        ],
    ));

    assert_eq!(
        names(&window),
        vec!["Named", "Group 2", "Group 3", "Group 4"]
    );
}

#[test]
fn the_default_name_counts_the_groups_that_are_kept() {
    // The first group has no tab and is dropped, so the unnamed one is the first.
    let window = settled(window(
        vec![in_group(1, 2)],
        vec![group(1, "Empty"), group(2, "")],
    ));

    assert_eq!(names(&window), vec!["Group 1"]);
}

#[test]
fn a_name_is_cut_at_64_bytes_on_a_character_boundary() {
    let cleaned = |raw: &str| settled_group(group(1, raw)).name;

    assert_eq!(cleaned(&"a".repeat(64)), "a".repeat(64));
    assert_eq!(cleaned(&"a".repeat(70)), "a".repeat(64));
    // 40 two-byte characters: 32 fit.
    assert_eq!(cleaned(&"\u{e9}".repeat(40)), "\u{e9}".repeat(32));
    // 63 bytes, then a two-byte character that would end at byte 65.
    let straddling = format!("{}\u{e9}z", "a".repeat(63));
    assert_eq!(cleaned(&straddling), "a".repeat(63));
    // A four-byte character at the edge.
    let emoji = format!("{}\u{1f980}", "a".repeat(61));
    assert_eq!(cleaned(&emoji), "a".repeat(61));
}

#[test]
fn white_space_left_at_the_cut_is_trimmed_so_that_sanitizing_again_changes_nothing() {
    let raw = format!("{} tail", "a".repeat(63));
    let once = settled_group(group(1, &raw));

    assert_eq!(once.name, "a".repeat(63));
    assert_eq!(settled_group(once.clone()), once);
}

#[test]
fn a_one_megabyte_name_is_cut_to_64_bytes() {
    let long = "x".repeat(1024 * 1024);
    let json = serde_json::to_string(&snapshot(vec![window(
        vec![in_group(1, 1)],
        vec![group(1, &long)],
    )]))
    .expect("serialise");

    let settled = decode(&json).sanitized();

    assert_eq!(settled.windows[0].groups[0].name, "x".repeat(64));
}

// ---------------------------------------------------------------------------------------------
// Rule 3: colours

#[test]
fn colours_are_kept_as_lower_case_names_or_hex_and_cleared_otherwise() {
    let colour = |raw: &str| {
        settled_group(GroupSnapshot {
            colour: raw.to_owned(),
            ..group(1, "g")
        })
        .colour
    };

    for (raw, expected) in [
        ("blue", "blue"),
        ("brown", "brown"),
        ("BLUE", "blue"),
        ("Blue", "blue"),
        ("chartreuse", "chartreuse"),
        ("abcdefghijklmnop", "abcdefghijklmnop"),
        ("a", "a"),
        ("#2f6fd6", "#2f6fd6"),
        ("#2F6FD6", "#2f6fd6"),
        ("#ABCDEF", "#abcdef"),
        ("#000000", "#000000"),
        ("", ""),
        ("abcdefghijklmnopq", ""),
        ("blue1", ""),
        ("two words", ""),
        (" blue", ""),
        ("blue ", ""),
        ("bl\u{fc}e", ""),
        ("#2f6fd", ""),
        ("#2f6fd6f", ""),
        ("#gggggg", ""),
        ("#2f6fd\u{e9}", ""),
        ("2f6fd6", ""),
        ("#", ""),
        ("rgb(1,2,3)", ""),
    ] {
        assert_eq!(colour(raw), expected, "{raw:?}");
    }
    assert_eq!(colour(&"b".repeat(1024 * 1024)), "");
}

// ---------------------------------------------------------------------------------------------
// Rule 4: keys

#[test]
fn keys_are_kept_only_as_key_a_to_key_z() {
    let key = |raw: Option<&str>| {
        settled_group(GroupSnapshot {
            key: raw.map(str::to_owned),
            ..group(1, "g")
        })
        .key
    };

    for letter in 'A'..='Z' {
        let name = format!("Key{letter}");
        assert_eq!(key(Some(&name)), Some(name.clone()));
    }
    assert_eq!(key(None), None);
    for bad in [
        "",
        "keya",
        "KEYA",
        "Keya",
        "KeyAA",
        "Key",
        "Key1",
        "Key\u{c0}",
        "Digit1",
        "KeyA ",
        " KeyA",
        "A",
    ] {
        assert_eq!(key(Some(bad)), None, "{bad:?}");
    }
    assert_eq!(key(Some(&"K".repeat(1024 * 1024))), None);
}

#[test]
fn a_duplicate_key_is_cleared_on_the_later_group() {
    let window = settled(window(
        vec![
            in_group(1, 1),
            in_group(2, 2),
            in_group(3, 3),
            in_group(4, 4),
        ],
        vec![
            keyed(1, "One", "KeyA"),
            keyed(2, "Two", "KeyA"),
            keyed(3, "Three", "KeyB"),
            keyed(4, "Four", "KeyB"),
        ],
    ));

    let keys: Vec<Option<&str>> = window.groups.iter().map(|g| g.key.as_deref()).collect();
    assert_eq!(keys, vec![Some("KeyA"), None, Some("KeyB"), None]);
}

#[test]
fn a_key_is_not_lost_to_a_group_that_was_dropped() {
    // Group 1 has no tab and held KeyA first; group 2 keeps it.
    let window = settled(window(
        vec![in_group(1, 2)],
        vec![keyed(1, "Empty", "KeyA"), keyed(2, "Used", "KeyA")],
    ));

    assert_eq!(window.groups[0].key.as_deref(), Some("KeyA"));
}

#[test]
fn an_invalid_key_does_not_take_up_a_valid_one() {
    let window = settled(window(
        vec![in_group(1, 1), in_group(2, 2)],
        vec![keyed(1, "One", "keya"), keyed(2, "Two", "KeyA")],
    ));

    assert_eq!(window.groups[0].key, None);
    assert_eq!(window.groups[1].key.as_deref(), Some("KeyA"));
}

// ---------------------------------------------------------------------------------------------
// Rule 7 and 8: the id counter, the dial corner

fn counter(next_group_id: u64, groups: Vec<GroupSnapshot>, tabs: Vec<TabSnapshot>) -> u64 {
    settled(WindowSnapshot {
        tabs,
        groups,
        next_group_id,
        ..WindowSnapshot::default()
    })
    .next_group_id
}

#[test]
fn the_id_counter_stays_above_every_group_id() {
    // No groups: the counter is left alone.
    assert_eq!(counter(0, Vec::new(), vec![tab(1)]), 0);
    assert_eq!(counter(5, Vec::new(), vec![tab(1)]), 5);
    // Groups 3 and 9, counter 4: it must be above 9.
    let two = || vec![group(3, "a"), group(9, "b")];
    let tabs = || vec![in_group(1, 3), in_group(2, 9)];
    assert_eq!(counter(4, two(), tabs()), 10);
    assert_eq!(counter(0, two(), tabs()), 10);
    // A counter already above it stays.
    assert_eq!(counter(20, two(), tabs()), 20);
    // Id 0 still gives a counter of at least 1.
    assert_eq!(counter(0, vec![group(0, "zero")], vec![in_group(1, 0)]), 1);
}

#[test]
fn a_dropped_group_does_not_lower_or_raise_the_counter() {
    // Group 50 has no tab: dropped. The counter 12 (the id the next group would get) stays,
    // so an id in the closed list can never name a different group later.
    let kept = counter(
        12,
        vec![group(3, "kept"), group(50, "dropped")],
        vec![in_group(1, 3)],
    );
    assert_eq!(kept, 12);
}

#[test]
fn the_id_counter_never_passes_i32_max() {
    assert_eq!(counter(u64::MAX, Vec::new(), vec![tab(1)]), MAX_COUNTER);
    assert_eq!(
        counter(0, vec![group(u64::MAX, "big")], vec![in_group(1, u64::MAX)]),
        MAX_COUNTER
    );
    assert_eq!(
        counter(
            0,
            vec![group(MAX_COUNTER, "edge")],
            vec![in_group(1, MAX_COUNTER)]
        ),
        MAX_COUNTER
    );
    assert_eq!(
        counter(
            3,
            vec![group(MAX_COUNTER - 1, "edge")],
            vec![in_group(1, MAX_COUNTER - 1)]
        ),
        MAX_COUNTER
    );
}

#[test]
fn the_dial_corner_is_one_of_four_names_or_nothing() {
    let corner = |raw: Option<&str>| {
        settled(WindowSnapshot {
            tabs: vec![tab(1)],
            dial_corner: raw.map(str::to_owned),
            ..WindowSnapshot::default()
        })
        .dial_corner
    };

    for name in ["top_left", "top_right", "bottom_left", "bottom_right"] {
        assert_eq!(corner(Some(name)).as_deref(), Some(name));
    }
    assert_eq!(corner(None), None);
    for bad in [
        "",
        "center",
        "TopLeft",
        "top-left",
        "TOP_LEFT",
        "top_left ",
        "left",
    ] {
        assert_eq!(corner(Some(bad)), None, "{bad:?}");
    }
    assert_eq!(corner(Some(&"t".repeat(1024 * 1024))), None);
}

#[test]
fn the_dock_flag_and_the_corner_are_carried_through_unchanged() {
    for shrunk in [None, Some(false), Some(true)] {
        let window = settled(WindowSnapshot {
            tabs: vec![tab(1)],
            dock_shrunk: shrunk,
            dial_corner: Some("bottom_right".to_owned()),
            ..WindowSnapshot::default()
        });
        assert_eq!(window.dock_shrunk, shrunk);
        assert_eq!(window.dial_corner.as_deref(), Some("bottom_right"));
    }
}

// ---------------------------------------------------------------------------------------------
// Idempotence

fn messy() -> SessionSnapshot {
    let mut closed_with_group = ClosedTab {
        tab: in_group(40, 99),
        group: Some(GroupSnapshot {
            id: 99,
            name: format!("  {}\u{7}  ", "\u{e9}".repeat(50)),
            colour: "#ABCDEF".to_owned(),
            key: Some("keyq".to_owned()),
        }),
        window: 0,
        index: 4,
        closed_at_ms: 77,
    };
    closed_with_group.tab.title = "t".repeat(2000);
    let pinned_closed = ClosedTab {
        tab: pinned_in_group(41, 98),
        group: Some(group(98, "p")),
        ..ClosedTab::default()
    };
    SessionSnapshot {
        windows: vec![
            WindowSnapshot {
                tabs: vec![
                    in_group(1, 1),
                    bad_in_group(2),
                    pinned_in_group(3, 3),
                    in_group(4, 3),
                    in_group(5, 77),
                    in_group(6, 4),
                    tab(7),
                ],
                active_tab: 1,
                groups: vec![
                    GroupSnapshot {
                        id: 1,
                        name: format!("{} z", "a".repeat(63)),
                        colour: "Blue".to_owned(),
                        key: Some("KeyA".to_owned()),
                    },
                    keyed(2, "dropped", "KeyB"),
                    keyed(3, "", "KeyA"),
                    GroupSnapshot {
                        id: 3,
                        name: "duplicate id".to_owned(),
                        colour: String::new(),
                        key: None,
                    },
                    GroupSnapshot {
                        id: u64::MAX,
                        name: "\u{7}".to_owned(),
                        colour: "###".to_owned(),
                        key: Some("KeyZ".to_owned()),
                    },
                    keyed(4, "four", "nope"),
                ],
                next_group_id: 2,
                dock_shrunk: Some(true),
                dial_corner: Some("Middle".to_owned()),
                ..WindowSnapshot::default()
            },
            WindowSnapshot {
                tabs: vec![in_group(8, 5)],
                groups: vec![group(5, "other window")],
                next_group_id: u64::MAX,
                ..WindowSnapshot::default()
            },
        ],
        closed_tabs: vec![closed_with_group, pinned_closed],
        saved_at_ms: 5,
    }
}

#[test]
fn sanitizing_twice_changes_nothing() {
    let once = messy().sanitized();
    let twice = once.clone().sanitized();

    assert_eq!(twice, once);
    // The mess did produce something to check.
    assert_eq!(names(&once.windows[0])[0], "a".repeat(63));
    assert_eq!(once.windows[0].next_group_id, 5);
    assert_eq!(once.windows[1].next_group_id, MAX_COUNTER);
}

#[test]
fn sanitizing_twice_changes_nothing_for_hostile_ids_and_counters() {
    let session = snapshot(vec![WindowSnapshot {
        tabs: vec![in_group(1, u64::MAX), in_group(2, u64::MAX - 1), tab(3)],
        groups: vec![group(u64::MAX, ""), group(u64::MAX - 1, "x")],
        next_group_id: u64::MAX,
        ..WindowSnapshot::default()
    }]);

    let once = session.sanitized();

    assert_eq!(once.clone().sanitized(), once);
    assert_eq!(once.windows[0].groups.len(), 2);
}

// ---------------------------------------------------------------------------------------------
// Rule 9: closed tabs

fn closed_in(n: usize, group: Option<GroupSnapshot>, index: usize, at: i64) -> ClosedTab {
    ClosedTab {
        tab: TabSnapshot {
            group: group.as_ref().map(|g| g.id),
            ..tab(n)
        },
        group,
        window: 0,
        index,
        closed_at_ms: at,
    }
}

#[test]
fn a_closed_tab_keeps_its_group_with_the_same_rules() {
    let session = SessionSnapshot {
        closed_tabs: vec![ClosedTab {
            // The tab says 99, the group says 5: the group wins.
            tab: in_group(1, 99),
            group: Some(GroupSnapshot {
                id: 5,
                name: format!("  Dirty\u{7} {}", "n".repeat(100)),
                colour: "BLUE".to_owned(),
                key: Some("keyx".to_owned()),
            }),
            window: 0,
            index: 2,
            closed_at_ms: 9,
        }],
        ..SessionSnapshot::default()
    }
    .sanitized();

    let closed = &session.closed_tabs[0];
    let group = closed.group.as_ref().expect("the group is kept");
    assert_eq!(group.id, 5);
    assert_eq!(group.name, format!("Dirty {}", "n".repeat(64 - 6)));
    assert_eq!(group.colour, "blue");
    assert_eq!(group.key, None);
    assert_eq!(closed.tab.group, Some(5));
}

#[test]
fn a_closed_tab_without_a_group_has_no_group_id_either() {
    let session = SessionSnapshot {
        closed_tabs: vec![ClosedTab {
            tab: in_group(1, 5),
            group: None,
            ..ClosedTab::default()
        }],
        ..SessionSnapshot::default()
    }
    .sanitized();

    assert_eq!(session.closed_tabs[0].group, None);
    assert_eq!(session.closed_tabs[0].tab.group, None);
}

#[test]
fn a_closed_group_with_an_empty_name_is_called_group_1() {
    let session = SessionSnapshot {
        closed_tabs: vec![closed_in(1, Some(group(5, " ")), 0, 1)],
        ..SessionSnapshot::default()
    }
    .sanitized();

    assert_eq!(
        session.closed_tabs[0].group.as_ref().expect("kept").name,
        "Group 1"
    );
}

#[test]
fn a_closed_pinned_tab_has_no_group() {
    let session = SessionSnapshot {
        closed_tabs: vec![ClosedTab {
            tab: pinned_in_group(1, 5),
            group: Some(group(5, "g")),
            ..ClosedTab::default()
        }],
        ..SessionSnapshot::default()
    }
    .sanitized();

    assert_eq!(session.closed_tabs[0].group, None);
    assert_eq!(session.closed_tabs[0].tab.group, None);
}

#[test]
fn a_closed_tab_with_a_bad_url_is_dropped_with_its_group() {
    let mut closed = closed_in(1, Some(group(5, "g")), 0, 1);
    closed.tab.url = "javascript:alert(1)".to_owned();

    let session = SessionSnapshot {
        closed_tabs: vec![closed, closed_in(2, Some(group(5, "g")), 1, 1)],
        ..SessionSnapshot::default()
    }
    .sanitized();

    assert_eq!(session.closed_tabs.len(), 1);
    assert_eq!(session.closed_tabs[0].tab.url, "https://site2.example/");
}

#[test]
fn a_closed_group_does_not_need_a_live_group_or_a_window() {
    // The window list is empty; the closed tab's group stands on its own.
    let session = SessionSnapshot {
        closed_tabs: vec![closed_in(1, Some(keyed(5, "Alone", "KeyC")), 0, 1)],
        ..SessionSnapshot::default()
    }
    .sanitized();

    assert_eq!(
        session.closed_tabs[0].group,
        Some(keyed(5, "Alone", "KeyC"))
    );
}

// ---------------------------------------------------------------------------------------------
// pop_closed_run

fn indexes(run: &[ClosedTab]) -> Vec<usize> {
    run.iter().map(|closed| closed.index).collect()
}

fn with_closed(closed_tabs: Vec<ClosedTab>) -> SessionSnapshot {
    SessionSnapshot {
        closed_tabs,
        ..SessionSnapshot::default()
    }
}

#[test]
fn an_empty_list_pops_an_empty_run() {
    let mut session = SessionSnapshot::default();

    assert!(session.pop_closed_run().is_empty());
    assert!(session.closed_tabs.is_empty());
}

#[test]
fn a_tab_without_a_group_pops_alone_even_beside_an_entry_of_the_same_time() {
    let mut session = with_closed(vec![
        closed_in(1, None, 4, 100),
        closed_in(2, None, 3, 100),
        closed_in(3, Some(group(8, "g")), 2, 100),
    ]);

    let run = session.pop_closed_run();

    assert_eq!(indexes(&run), vec![4]);
    assert_eq!(session.closed_tabs.len(), 2);
}

#[test]
fn a_single_tab_of_a_group_pops_alone() {
    let mut session = with_closed(vec![
        closed_in(1, Some(group(8, "g")), 4, 100),
        closed_in(2, None, 3, 50),
    ]);

    let run = session.pop_closed_run();

    assert_eq!(indexes(&run), vec![4]);
    assert_eq!(run[0].group, Some(group(8, "g")));
    assert_eq!(session.closed_tabs.len(), 1);
}

#[test]
fn a_closed_group_pops_as_one_run_lowest_index_first() {
    // Closed in strip order 3, 4, 5, so the newest is index 5.
    let g = group(8, "g");
    let mut session = with_closed(vec![
        closed_in(5, Some(g.clone()), 5, 100),
        closed_in(4, Some(g.clone()), 3, 100),
        closed_in(3, Some(g.clone()), 4, 100),
        closed_in(2, None, 0, 90),
    ]);

    let run = session.pop_closed_run();

    assert_eq!(indexes(&run), vec![3, 4, 5]);
    assert_eq!(session.closed_tabs.len(), 1);
    assert_eq!(session.closed_tabs[0].tab.url, "https://site2.example/");
    // The next pop is the plain tab.
    assert_eq!(indexes(&session.pop_closed_run()), vec![0]);
    assert!(session.pop_closed_run().is_empty());
}

#[test]
fn a_run_stops_at_an_entry_of_another_group_with_the_same_time() {
    let mut session = with_closed(vec![
        closed_in(3, Some(group(8, "g")), 2, 100),
        closed_in(2, Some(group(8, "g")), 1, 100),
        closed_in(1, Some(group(9, "other")), 0, 100),
    ]);

    let run = session.pop_closed_run();

    assert_eq!(indexes(&run), vec![1, 2]);
    assert_eq!(session.closed_tabs.len(), 1);
}

#[test]
fn a_run_stops_at_an_entry_without_a_group_with_the_same_time() {
    let mut session = with_closed(vec![
        closed_in(2, Some(group(8, "g")), 1, 100),
        closed_in(1, None, 0, 100),
    ]);

    assert_eq!(indexes(&session.pop_closed_run()), vec![1]);
    assert_eq!(session.closed_tabs.len(), 1);
}

#[test]
fn a_run_stops_at_an_entry_of_the_same_group_closed_at_another_time() {
    let mut session = with_closed(vec![
        closed_in(2, Some(group(8, "g")), 1, 100),
        closed_in(1, Some(group(8, "g")), 0, 99),
    ]);

    assert_eq!(indexes(&session.pop_closed_run()), vec![1]);
    assert_eq!(session.closed_tabs.len(), 1);
}

#[test]
fn a_run_interleaved_with_another_group_pops_only_the_leading_entries() {
    let mut session = with_closed(vec![
        closed_in(3, Some(group(8, "a")), 1, 100),
        closed_in(2, Some(group(9, "b")), 0, 100),
        closed_in(1, Some(group(8, "a")), 2, 100),
    ]);

    assert_eq!(indexes(&session.pop_closed_run()), vec![1]);
    assert_eq!(session.closed_tabs.len(), 2);
    assert_eq!(indexes(&session.pop_closed_run()), vec![0]);
    assert_eq!(indexes(&session.pop_closed_run()), vec![2]);
}

#[test]
fn a_run_may_be_the_whole_list() {
    let g = group(8, "g");
    let mut session = with_closed(
        (0..MAX_CLOSED_TABS)
            .map(|n| closed_in(n, Some(g.clone()), MAX_CLOSED_TABS - n, 100))
            .collect(),
    );

    let run = session.pop_closed_run();

    assert_eq!(run.len(), MAX_CLOSED_TABS);
    assert!(session.closed_tabs.is_empty());
    assert!(run.windows(2).all(|pair| pair[0].index <= pair[1].index));
}

#[test]
fn pop_closed_still_pops_one_entry_of_a_run() {
    let g = group(8, "g");
    let mut session = with_closed(vec![
        closed_in(2, Some(g.clone()), 1, 100),
        closed_in(1, Some(g), 0, 100),
    ]);

    assert_eq!(session.pop_closed().expect("an entry").index, 1);
    assert_eq!(session.closed_tabs.len(), 1);
}

// ---------------------------------------------------------------------------------------------
// The store keeps groups

#[test]
fn the_store_saves_and_restores_groups() {
    let dir = tempfile::tempdir().expect("a profile");
    let session = SessionSnapshot {
        windows: vec![WindowSnapshot {
            tabs: vec![in_group(1, 4), in_group(2, 4), tab(3), in_group(4, 6)],
            active_tab: 3,
            groups: vec![
                keyed(4, "Research", "KeyA"),
                GroupSnapshot {
                    colour: "#2f6fd6".to_owned(),
                    ..group(6, "Custom")
                },
            ],
            next_group_id: 12,
            dock_shrunk: Some(true),
            dial_corner: Some("top_left".to_owned()),
            ..WindowSnapshot::default()
        }],
        closed_tabs: vec![closed_in(5, Some(keyed(4, "Research", "KeyA")), 1, 1_000)],
        saved_at_ms: 1_763_200_005_000,
    };
    let store = SessionStore::new(dir.path());

    store.save(&session, true).expect("save");
    let found = SessionStore::new(dir.path())
        .begin_run()
        .expect("begin_run");

    assert_eq!(
        found.last,
        Some(PreviousSession {
            snapshot: session,
            exited_cleanly: true
        })
    );
}

#[test]
fn the_store_sanitizes_the_groups_of_a_file_it_reads() {
    let dir = tempfile::tempdir().expect("a profile");
    let file = r#"{
      "format_version": 1,
      "exited_cleanly": true,
      "windows": [{
        "tabs": [
          {"url": "https://a.example/", "group": 1},
          {"url": "https://b.example/", "group": 2}
        ],
        "groups": [
          {"id": 1, "name": "  Mine\u0007 ", "colour": "BLUE", "key": "KeyA"},
          {"id": 1, "name": "again"}
        ],
        "next_group_id": 0,
        "dial_corner": "middle"
      }],
      "saved_at_ms": 5
    }"#;
    std::fs::write(dir.path().join("session.json"), file).expect("write");

    let last = SessionStore::new(dir.path())
        .begin_run()
        .expect("begin_run")
        .last
        .expect("a session");

    let window = &last.snapshot.windows[0];
    assert_eq!(names(window), vec!["Mine"]);
    assert_eq!(window.groups[0].colour, "blue");
    assert_eq!(tab_groups(window), vec![Some(1), None]);
    assert_eq!(window.next_group_id, 2);
    assert_eq!(window.dial_corner, None);
}

// ---------------------------------------------------------------------------------------------
// Hostile input

#[test]
fn ten_thousand_groups_are_read_as_sixty_four() {
    let groups: Vec<String> = (1..=10_000)
        .map(|id| format!(r#"{{"id":{id},"name":"g{id}","colour":"blue"}}"#))
        .collect();
    let json = format!(
        r#"{{"windows": [{{"tabs": [{{"url": "https://a.example/", "group": 10000}}, {{"url": "https://b.example/", "group": 5}}],
            "groups": [{}]}}]}}"#,
        groups.join(",")
    );

    let session = decode(&json);
    assert_eq!(session.windows[0].groups.len(), MAX_GROUPS_PER_WINDOW);

    let settled = session.sanitized();
    let window = &settled.windows[0];
    assert_eq!(group_ids(window), vec![5]);
    assert_eq!(tab_groups(window), vec![None, Some(5)]);
}

#[test]
fn thousands_of_tabs_naming_one_group_are_cut_with_the_tab_cap() {
    let tabs: Vec<String> = (0..5_000)
        .map(|n| format!(r#"{{"url":"https://site{n}.example/","group":1}}"#))
        .collect();
    let json = format!(
        r#"{{"windows": [{{"tabs": [{}], "groups": [{{"id": 1, "name": "All"}}]}}]}}"#,
        tabs.join(",")
    );

    let session = decode(&json);
    assert_eq!(session.windows[0].tabs.len(), MAX_RESTORED_TABS);

    let settled = session.sanitized();
    let window = &settled.windows[0];
    assert_eq!(window.tabs.len(), MAX_RESTORED_TABS);
    assert_eq!(group_ids(window), vec![1]);
    assert!(window.tabs.iter().all(|tab| tab.group == Some(1)));
}

#[test]
fn ids_at_u64_max_are_read_and_kept_as_ids() {
    let window = settled(window(
        vec![in_group(1, u64::MAX), in_group(2, u64::MAX - 1)],
        vec![group(u64::MAX, "max"), group(u64::MAX - 1, "almost")],
    ));

    assert_eq!(group_ids(&window), vec![u64::MAX, u64::MAX - 1]);
    assert_eq!(
        tab_groups(&window),
        vec![Some(u64::MAX), Some(u64::MAX - 1)]
    );
    assert_eq!(window.next_group_id, MAX_COUNTER);
}

#[test]
fn a_negative_or_fractional_group_id_makes_the_file_unreadable_like_any_bad_number() {
    for bad in [
        r#"{"windows": [{"tabs": [{"url": "https://a.example/", "group": -1}]}]}"#,
        r#"{"windows": [{"tabs": [{"url": "https://a.example/", "group": 1.5}]}]}"#,
        r#"{"windows": [{"groups": [{"id": -3}]}]}"#,
    ] {
        assert!(
            serde_json::from_str::<SessionSnapshot>(bad).is_err(),
            "{bad}"
        );
    }
}
