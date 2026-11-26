// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! Orbit's drag and drop (brief D11, item 3): the payload that crosses a real `DataTransfer`, the
//! one resolver, the slot to final place conversion, `can_accept` and the controller mapping.
//! Pure Rust: no window, no ring-bands output.

use std::rc::Rc;

use mercury_ui::TabRow;
use mercury_ui::orbit_view::drag::{
    Decision, DropTarget, GroupOp, OrbitPayload, PayloadError, Source, can_accept, drag_text,
    drop_target_of, lit_target, resolve, slot_to_final_index, source_of, target_from_id,
};
use mercury_ui::orbit_view::fixtures::{orbit_groups, orbit_input, orbit_tabs};
use mercury_ui::orbit_view::{LOOSE_GROUP_ID, OrbitInput, Target};
use slint::{DataTransfer, SharedString};

const SESSION: u64 = 7;

/// Research (group 1) holds tabs 101 to 110, Lisbon trip (group 2) holds 111 to 118.
fn input() -> OrbitInput {
    orbit_input(false)
}

/// Tab 101 pinned and a loose tab 900: the pseudo planet holds both (D7), Research has 9 tabs.
fn input_with_loose() -> OrbitInput {
    let mut tabs = orbit_tabs();
    tabs[0].pinned = true;
    tabs.push(TabRow {
        id: 900,
        group: -1,
        title: "A loose page".into(),
        ..tabs[1].clone()
    });
    OrbitInput::new(&orbit_groups(false), &tabs, "P", "W", 102, false)
}

fn payload(source: Source) -> OrbitPayload {
    OrbitPayload {
        session: SESSION,
        source,
    }
}

fn group(group: i32, slot: Option<usize>) -> DropTarget {
    DropTarget::Group { group, slot }
}

// --- the payload -------------------------------------------------------------------------------

#[test]
fn a_tab_payload_round_trips_through_a_real_data_transfer() {
    let sent = payload(Source::Tab(104));
    let data = sent.to_data_transfer("AccessKit guide");
    assert_eq!(OrbitPayload::from_data_transfer(&data, SESSION), Ok(sent));
}

#[test]
fn a_group_payload_round_trips_through_a_real_data_transfer() {
    let sent = payload(Source::Group(3));
    let data = sent.to_data_transfer("Reading");
    assert_eq!(OrbitPayload::from_data_transfer(&data, SESSION), Ok(sent));
}

#[test]
fn the_loose_planet_is_a_valid_group_payload() {
    let sent = payload(Source::Group(LOOSE_GROUP_ID));
    let data = sent.to_data_transfer("Loose tabs");
    assert_eq!(OrbitPayload::from_data_transfer(&data, SESSION), Ok(sent));
}

#[test]
fn a_data_transfer_with_nothing_in_it_is_not_orbit() {
    assert_eq!(
        OrbitPayload::from_data_transfer(&DataTransfer::default(), SESSION),
        Err(PayloadError::NotOrbit)
    );
}

#[test]
fn plain_text_alone_is_rejected() {
    let data = DataTransfer::from(SharedString::from("AccessKit guide"));
    assert_eq!(
        OrbitPayload::from_data_transfer(&data, SESSION),
        Err(PayloadError::NotOrbit)
    );
}

#[test]
fn text_that_looks_like_a_payload_is_rejected() {
    for text in [
        "mercury-orbit:tab:12",
        "mercury-orbit:group:3",
        "orbit:tab:104",
        "tab:104",
        "{\"session\":7,\"source\":{\"Tab\":104}}",
    ] {
        let data = DataTransfer::from(SharedString::from(text));
        assert_eq!(
            OrbitPayload::from_data_transfer(&data, SESSION),
            Err(PayloadError::NotOrbit),
            "{text}"
        );
    }
}

#[test]
fn text_next_to_a_real_payload_does_not_change_what_is_read() {
    // The text is for other apps; only the user data counts.
    let sent = payload(Source::Tab(104));
    let data = sent.to_data_transfer("mercury-orbit:tab:999");
    assert_eq!(OrbitPayload::from_data_transfer(&data, SESSION), Ok(sent));
}

#[test]
fn an_image_alone_is_rejected() {
    let data = DataTransfer::from(mercury_ui::orbit_view::fixtures::skeleton_picture(false));
    assert_eq!(
        OrbitPayload::from_data_transfer(&data, SESSION),
        Err(PayloadError::NotOrbit)
    );
}

#[test]
fn a_payload_of_an_older_session_is_rejected() {
    let old = OrbitPayload {
        session: SESSION - 1,
        source: Source::Tab(104),
    };
    let data = old.to_data_transfer("AccessKit guide");
    assert_eq!(
        OrbitPayload::from_data_transfer(&data, SESSION),
        Err(PayloadError::StaleSession)
    );
    // The same data is fine for the Orbit that made it.
    assert_eq!(
        OrbitPayload::from_data_transfer(&data, SESSION - 1),
        Ok(old)
    );
}

#[test]
fn another_type_of_user_data_is_rejected() {
    let others: Vec<Rc<dyn std::any::Any>> = vec![
        Rc::new(104_i32),
        Rc::new(String::from("mercury-orbit:tab:104")),
        Rc::new((SESSION, 104_i32)),
        Rc::new(Some(payload(Source::Tab(104)))),
        Rc::new(Vec::<OrbitPayload>::new()),
    ];
    for user in others {
        let mut data = DataTransfer::default();
        data.set_user_data(user);
        assert_eq!(
            OrbitPayload::from_data_transfer(&data, SESSION),
            Err(PayloadError::NotOrbit)
        );
    }
}

#[test]
fn ids_that_are_not_ids_are_rejected() {
    for source in [
        Source::Tab(-1),
        Source::Tab(i32::MIN),
        Source::Tab(LOOSE_GROUP_ID),
        Source::Group(-1),
        Source::Group(-3),
        Source::Group(i32::MIN),
    ] {
        let data = payload(source).to_data_transfer("x");
        assert_eq!(
            OrbitPayload::from_data_transfer(&data, SESSION),
            Err(PayloadError::BadId),
            "{source:?}"
        );
    }
    for source in [
        Source::Tab(0),
        Source::Tab(i32::MAX),
        Source::Group(0),
        Source::Group(i32::MAX),
    ] {
        let data = payload(source).to_data_transfer("x");
        assert_eq!(
            OrbitPayload::from_data_transfer(&data, SESSION),
            Ok(payload(source)),
            "{source:?}"
        );
    }
}

#[test]
fn a_stale_session_wins_over_a_bad_id() {
    let old = OrbitPayload {
        session: 1,
        source: Source::Tab(-5),
    };
    let data = old.to_data_transfer("x");
    assert_eq!(
        OrbitPayload::from_data_transfer(&data, SESSION),
        Err(PayloadError::StaleSession)
    );
}

// --- the text other apps see -------------------------------------------------------------------

#[test]
fn the_plain_text_is_the_tab_title_or_the_group_name() {
    let input = input();
    for tab in &input.tabs {
        let text = drag_text(Source::Tab(tab.id), &input);
        assert_eq!(text, tab.title.as_str());
        let data = payload(Source::Tab(tab.id)).to_data_transfer(&text);
        assert_eq!(data.plain_text().ok().as_deref(), Some(tab.title.as_str()));
    }
    for group in &input.groups {
        let text = drag_text(Source::Group(group.id), &input);
        assert_eq!(text, group.name.as_str());
        let data = payload(Source::Group(group.id)).to_data_transfer(&text);
        assert_eq!(data.plain_text().ok().as_deref(), Some(group.name.as_str()));
    }
}

#[test]
fn the_plain_text_is_never_a_url() {
    let mut tabs = orbit_tabs();
    // A page without a title is shown by its address.
    tabs[3].title = "https://a11y.example.net/guide?id=1".into();
    tabs[4].title = "www.example.net/forum".into();
    tabs[5].title = "file:///Users/me/notes.html".into();
    let input = OrbitInput::new(&orbit_groups(false), &tabs, "P", "W", 101, false);
    for index in [3, 4, 5] {
        let id = tabs[index].id;
        let text = drag_text(Source::Tab(id), &input);
        assert!(!text.contains("://"), "{text}");
        assert!(!text.starts_with("www."), "{text}");
        assert_eq!(text, tabs[index].host.as_str(), "falls back to the host");
        let data = payload(Source::Tab(id)).to_data_transfer(&text);
        let carried = data.plain_text().map(|t| t.to_string()).unwrap_or_default();
        assert!(!carried.contains("://"), "{carried}");
    }
}

fn input_with_title(title: &str, host: &str) -> OrbitInput {
    let mut tabs = orbit_tabs();
    tabs[3].title = title.into();
    tabs[3].host = host.into();
    OrbitInput::new(&orbit_groups(false), &tabs, "P", "W", 101, false)
}

#[test]
fn a_scheme_less_address_as_title_falls_back_to_the_host() {
    for (title, host) in [
        ("example.com/a?b=1", "example.com"),
        ("localhost:8080/x", "localhost"),
        ("example.com?token=1", "example.com"),
        ("sub.example.org#top", "sub.example.org"),
        ("example.com/", "example.com"),
        ("192.168.0.1:3000/admin", "192.168.0.1"),
        ("example.com:8080/a b", "example.com"),
    ] {
        let input = input_with_title(title, host);
        let id = input.tabs[3].id;
        let text = drag_text(Source::Tab(id), &input);
        if title.contains(' ') {
            assert_eq!(text, title, "words with a space are kept: {title}");
            continue;
        }
        assert_eq!(text, host, "{title}");
        assert!(!text.contains(['/', '?', '#']), "{text}");
        let data = payload(Source::Tab(id)).to_data_transfer(&text);
        assert_eq!(data.plain_text().ok().as_deref(), Some(host), "{title}");
        // Given as the text itself, the address is withheld.
        let data = payload(Source::Tab(id)).to_data_transfer(title);
        assert!(!data.has_plain_text(), "{title}");
    }
}

#[test]
fn a_scheme_less_address_with_no_usable_host_gives_no_text() {
    for host in ["example.com/a?b=1", "", "localhost:8080/x", "http://h/"] {
        let input = input_with_title("example.com/a?b=1", host);
        assert_eq!(
            drag_text(Source::Tab(input.tabs[3].id), &input),
            "",
            "{host:?}"
        );
    }
}

#[test]
fn titles_that_are_words_or_a_bare_host_are_not_addresses() {
    for title in [
        "TCP/IP",
        "and/or",
        "v1.2",
        "node.js",
        "example.com",
        "Q3 plan: example.com/a?b=1",
        "3.5 / 5",
        "C# in depth",
        "What is localhost:8080?",
    ] {
        let input = input_with_title(title, "example.com");
        let text = drag_text(Source::Tab(input.tabs[3].id), &input);
        assert_eq!(text, title, "{title}");
    }
}

#[test]
fn a_www_host_is_a_fine_drag_text() {
    let input = input_with_title("https://www.example.com/x", "www.example.com");
    assert_eq!(
        drag_text(Source::Tab(input.tabs[3].id), &input),
        "www.example.com"
    );
    let input = input_with_title("www.example.com/x", "www.example.com");
    assert_eq!(
        drag_text(Source::Tab(input.tabs[3].id), &input),
        "www.example.com"
    );
}

#[test]
fn a_host_that_is_an_address_is_not_a_drag_text() {
    for host in [
        "https://example.com",
        "mailto:a@b.c",
        "javascript:alert(1)",
        "a b",
    ] {
        let input = input_with_title("https://example.com/x", host);
        assert_eq!(
            drag_text(Source::Tab(input.tabs[3].id), &input),
            "",
            "{host}"
        );
    }
}

#[test]
fn to_data_transfer_drops_a_url_given_as_text() {
    for url in [
        "https://example.org/a",
        "http://x.y",
        "ftp://h/f",
        "www.example.org",
    ] {
        let data = payload(Source::Tab(104)).to_data_transfer(url);
        assert!(!data.has_plain_text(), "{url}");
        // The payload itself is untouched.
        assert_eq!(
            OrbitPayload::from_data_transfer(&data, SESSION),
            Ok(payload(Source::Tab(104)))
        );
    }
}

#[test]
fn a_title_with_spaces_that_mentions_a_url_is_kept() {
    let mut tabs = orbit_tabs();
    tabs[0].title = "How to read https://example.org docs".into();
    let input = OrbitInput::new(&orbit_groups(false), &tabs, "P", "W", 101, false);
    assert_eq!(
        drag_text(Source::Tab(101), &input),
        "How to read https://example.org docs"
    );
}

#[test]
fn the_text_of_an_unknown_source_is_empty() {
    let input = input();
    assert_eq!(drag_text(Source::Tab(9999), &input), "");
    assert_eq!(drag_text(Source::Group(77), &input), "");
}

// --- slot to final index -----------------------------------------------------------------------

#[test]
fn slot_to_final_index_matches_removing_and_reinserting_in_a_five_tab_group() {
    // The slot is the insertion index before the tab is removed (slots 0 to 5 for 5 tabs).
    for current in 0..5usize {
        for slot in 0..=5usize {
            let original: Vec<usize> = (0..5).collect();
            let mut others = original.clone();
            others.remove(current);
            let before = original
                .iter()
                .filter(|&&i| i < slot && i != current)
                .count();
            let mut moved = others.clone();
            moved.insert(before, current);
            let expected = moved.iter().position(|&i| i == current).unwrap();
            assert_eq!(
                slot_to_final_index(Some(7), current, 7, slot),
                expected,
                "tab {current} dropped at slot {slot}"
            );
        }
    }
}

#[test]
fn slot_to_final_index_named_cases() {
    // Own slot and own slot + 1: the tab stays where it is.
    assert_eq!(slot_to_final_index(Some(1), 2, 1, 2), 2);
    assert_eq!(slot_to_final_index(Some(1), 2, 1, 3), 2);
    // After its own place: one less.
    assert_eq!(slot_to_final_index(Some(1), 2, 1, 4), 3);
    assert_eq!(slot_to_final_index(Some(1), 2, 1, 5), 4);
    // Before its own place: unchanged.
    assert_eq!(slot_to_final_index(Some(1), 2, 1, 0), 0);
    assert_eq!(slot_to_final_index(Some(1), 2, 1, 1), 1);
    // First and last of the group.
    assert_eq!(slot_to_final_index(Some(1), 0, 1, 5), 4);
    assert_eq!(slot_to_final_index(Some(1), 4, 1, 0), 0);
}

#[test]
fn slot_to_final_index_is_the_slot_for_another_group() {
    for current in 0..5usize {
        for slot in 0..=8usize {
            assert_eq!(slot_to_final_index(Some(1), current, 2, slot), slot);
            assert_eq!(slot_to_final_index(None, current, 2, slot), slot);
            assert_eq!(
                slot_to_final_index(Some(LOOSE_GROUP_ID), current, 2, slot),
                slot
            );
        }
    }
}

// --- resolve: tabs -----------------------------------------------------------------------------

#[test]
fn a_tab_dropped_on_another_group_moves_to_the_slot() {
    let input = input();
    assert_eq!(
        resolve(Source::Tab(104), group(2, Some(3)), &input),
        Decision::MoveTab {
            tab: 104,
            group: Some(2),
            index: 3
        }
    );
    assert_eq!(
        resolve(Source::Tab(104), group(2, Some(0)), &input),
        Decision::MoveTab {
            tab: 104,
            group: Some(2),
            index: 0
        }
    );
}

#[test]
fn a_tab_dropped_on_a_halo_or_a_chip_goes_to_the_end() {
    let input = input();
    assert_eq!(
        resolve(Source::Tab(104), group(2, None), &input),
        Decision::MoveTab {
            tab: 104,
            group: Some(2),
            index: 8
        }
    );
}

#[test]
fn a_slot_past_the_end_is_the_end() {
    let input = input();
    assert_eq!(
        resolve(Source::Tab(104), group(2, Some(8)), &input),
        Decision::MoveTab {
            tab: 104,
            group: Some(2),
            index: 8
        }
    );
    assert_eq!(
        resolve(Source::Tab(104), group(2, Some(500)), &input),
        Decision::MoveTab {
            tab: 104,
            group: Some(2),
            index: 8
        }
    );
}

#[test]
fn a_tab_dropped_in_its_own_group_follows_the_slot_rules() {
    let input = input();
    // Tab 104 is Research's index 3 of 10.
    let own = |slot| resolve(Source::Tab(104), group(1, Some(slot)), &input);
    assert_eq!(own(3), Decision::NoChange);
    assert_eq!(own(4), Decision::NoChange);
    assert_eq!(
        own(0),
        Decision::MoveTab {
            tab: 104,
            group: Some(1),
            index: 0
        }
    );
    assert_eq!(
        own(2),
        Decision::MoveTab {
            tab: 104,
            group: Some(1),
            index: 2
        }
    );
    assert_eq!(
        own(5),
        Decision::MoveTab {
            tab: 104,
            group: Some(1),
            index: 4
        }
    );
    assert_eq!(
        own(10),
        Decision::MoveTab {
            tab: 104,
            group: Some(1),
            index: 9
        }
    );
    assert_eq!(
        own(99),
        Decision::MoveTab {
            tab: 104,
            group: Some(1),
            index: 9
        }
    );
}

#[test]
fn the_first_and_last_tab_of_a_group_have_the_edge_cases_right() {
    let input = input();
    // 101 is Research's first, 110 its last.
    assert_eq!(
        resolve(Source::Tab(101), group(1, Some(0)), &input),
        Decision::NoChange
    );
    assert_eq!(
        resolve(Source::Tab(101), group(1, Some(1)), &input),
        Decision::NoChange
    );
    assert_eq!(
        resolve(Source::Tab(101), group(1, Some(10)), &input),
        Decision::MoveTab {
            tab: 101,
            group: Some(1),
            index: 9
        }
    );
    assert_eq!(
        resolve(Source::Tab(110), group(1, Some(9)), &input),
        Decision::NoChange
    );
    assert_eq!(
        resolve(Source::Tab(110), group(1, Some(10)), &input),
        Decision::NoChange
    );
    assert_eq!(
        resolve(Source::Tab(110), group(1, Some(0)), &input),
        Decision::MoveTab {
            tab: 110,
            group: Some(1),
            index: 0
        }
    );
}

#[test]
fn a_tab_dropped_on_its_own_planets_halo_changes_nothing() {
    let input = input();
    assert_eq!(
        resolve(Source::Tab(104), group(1, None), &input),
        Decision::NoChange
    );
}

#[test]
fn a_tab_dropped_on_new_group_or_empty_space_makes_a_group() {
    let input = input();
    assert_eq!(
        resolve(Source::Tab(104), DropTarget::NewGroup, &input),
        Decision::NewGroup { tab: 104 }
    );
    assert_eq!(
        resolve(Source::Tab(104), DropTarget::Empty, &input),
        Decision::NewGroup { tab: 104 }
    );
}

#[test]
fn unknown_ids_are_rejected() {
    let input = input();
    let targets = [
        group(1, None),
        group(2, Some(2)),
        DropTarget::NewGroup,
        DropTarget::Empty,
    ];
    for target in targets {
        assert_eq!(
            resolve(Source::Tab(9999), target, &input),
            Decision::Reject,
            "{target:?}"
        );
        assert_eq!(
            resolve(Source::Group(77), target, &input),
            Decision::Reject,
            "{target:?}"
        );
    }
    assert_eq!(
        resolve(Source::Tab(104), group(77, None), &input),
        Decision::Reject
    );
    assert_eq!(
        resolve(Source::Tab(104), group(77, Some(1)), &input),
        Decision::Reject
    );
    assert_eq!(
        resolve(Source::Group(1), group(77, None), &input),
        Decision::Reject
    );
}

#[test]
fn nothing_resolves_against_no_rows() {
    let input = OrbitInput::default();
    assert_eq!(
        resolve(Source::Tab(104), DropTarget::NewGroup, &input),
        Decision::Reject
    );
    assert_eq!(
        resolve(Source::Group(1), group(2, None), &input),
        Decision::Reject
    );
}

// --- resolve: the loose planet and pinned tabs -------------------------------------------------

#[test]
fn a_tab_dropped_on_the_loose_planet_becomes_loose() {
    let input = input_with_loose();
    // The pseudo planet holds 101 (pinned) and 900.
    assert_eq!(
        resolve(Source::Tab(104), group(LOOSE_GROUP_ID, None), &input),
        Decision::MoveTab {
            tab: 104,
            group: None,
            index: 2
        }
    );
    assert_eq!(
        resolve(Source::Tab(104), group(LOOSE_GROUP_ID, Some(1)), &input),
        Decision::MoveTab {
            tab: 104,
            group: None,
            index: 1
        }
    );
    assert_eq!(
        resolve(Source::Tab(104), group(LOOSE_GROUP_ID, Some(40)), &input),
        Decision::MoveTab {
            tab: 104,
            group: None,
            index: 2
        }
    );
}

#[test]
fn a_loose_tab_dropped_on_the_loose_planet_changes_nothing() {
    let input = input_with_loose();
    for slot in [None, Some(0), Some(1), Some(2)] {
        assert_eq!(
            resolve(Source::Tab(900), group(LOOSE_GROUP_ID, slot), &input),
            Decision::NoChange,
            "{slot:?}"
        );
    }
}

#[test]
fn a_loose_tab_can_join_a_group() {
    let input = input_with_loose();
    // Research has 9 tabs now (101 is pinned).
    assert_eq!(
        resolve(Source::Tab(900), group(1, Some(3)), &input),
        Decision::MoveTab {
            tab: 900,
            group: Some(1),
            index: 3
        }
    );
    assert_eq!(
        resolve(Source::Tab(900), group(1, None), &input),
        Decision::MoveTab {
            tab: 900,
            group: Some(1),
            index: 9
        }
    );
}

#[test]
fn a_loose_tab_dropped_on_new_group_makes_a_group() {
    let input = input_with_loose();
    assert_eq!(
        resolve(Source::Tab(900), DropTarget::NewGroup, &input),
        Decision::NewGroup { tab: 900 }
    );
}

#[test]
fn a_pinned_source_is_rejected_everywhere() {
    let input = input_with_loose();
    assert!(input.is_pinned(101));
    let targets = [
        group(1, Some(2)),
        group(2, None),
        group(LOOSE_GROUP_ID, None),
        group(LOOSE_GROUP_ID, Some(0)),
        DropTarget::NewGroup,
        DropTarget::Empty,
    ];
    for target in targets {
        assert_eq!(
            resolve(Source::Tab(101), target, &input),
            Decision::Reject,
            "{target:?}"
        );
    }
}

// --- resolve: groups ---------------------------------------------------------------------------

#[test]
fn a_group_dropped_on_another_planet_takes_its_place_in_the_order() {
    let input = input();
    // Group order is Research 0, Lisbon trip 1, Reading 2, Shopping 3, Work 4.
    assert_eq!(
        resolve(Source::Group(2), group(1, None), &input),
        Decision::ReorderGroup { group: 2, index: 0 }
    );
    assert_eq!(
        resolve(Source::Group(1), group(3, None), &input),
        Decision::ReorderGroup { group: 1, index: 2 }
    );
    assert_eq!(
        resolve(Source::Group(5), group(2, None), &input),
        Decision::ReorderGroup { group: 5, index: 1 }
    );
}

#[test]
fn the_slot_does_not_matter_when_a_group_is_dropped() {
    let input = input();
    for slot in [None, Some(0), Some(3), Some(99)] {
        assert_eq!(
            resolve(Source::Group(2), group(4, slot), &input),
            Decision::ReorderGroup { group: 2, index: 3 },
            "{slot:?}"
        );
    }
}

#[test]
fn a_group_dropped_on_itself_changes_nothing() {
    let input = input();
    for id in 1..=5 {
        assert_eq!(
            resolve(Source::Group(id), group(id, None), &input),
            Decision::NoChange
        );
        assert_eq!(
            resolve(Source::Group(id), group(id, Some(2)), &input),
            Decision::NoChange
        );
    }
}

#[test]
fn a_group_dropped_on_new_group_is_rejected_and_on_empty_space_changes_nothing() {
    let input = input();
    assert_eq!(
        resolve(Source::Group(2), DropTarget::NewGroup, &input),
        Decision::Reject
    );
    assert_eq!(
        resolve(Source::Group(2), DropTarget::Empty, &input),
        Decision::NoChange
    );
}

#[test]
fn the_loose_planet_is_not_reorderable() {
    let input = input_with_loose();
    // As a source ...
    assert_eq!(
        resolve(Source::Group(LOOSE_GROUP_ID), group(1, None), &input),
        Decision::Reject
    );
    assert_eq!(
        resolve(
            Source::Group(LOOSE_GROUP_ID),
            group(LOOSE_GROUP_ID, None),
            &input
        ),
        Decision::Reject
    );
    // ... and as the place to drop on.
    assert_eq!(
        resolve(Source::Group(1), group(LOOSE_GROUP_ID, None), &input),
        Decision::Reject
    );
    assert_eq!(
        resolve(Source::Group(1), group(LOOSE_GROUP_ID, Some(1)), &input),
        Decision::Reject
    );
}

#[test]
fn a_group_reorder_does_not_count_the_loose_planet() {
    let input = input_with_loose();
    // Loose is last in the rows (index 5); real groups keep their controller indexes.
    assert_eq!(
        resolve(Source::Group(5), group(1, None), &input),
        Decision::ReorderGroup { group: 5, index: 0 }
    );
}

// --- can_accept --------------------------------------------------------------------------------

#[test]
fn can_accept_for_every_kind_of_payload_and_target() {
    let plain = input();
    let loose = input_with_loose();
    // (name, input, payload, target, accepted)
    let table: Vec<(&str, &OrbitInput, Source, DropTarget, bool)> = vec![
        (
            "tab, own planet (halo)",
            &plain,
            Source::Tab(104),
            group(1, None),
            false,
        ),
        (
            "tab, own planet, own slot",
            &plain,
            Source::Tab(104),
            group(1, Some(3)),
            false,
        ),
        (
            "tab, own planet, another slot",
            &plain,
            Source::Tab(104),
            group(1, Some(7)),
            true,
        ),
        (
            "tab, another planet (halo)",
            &plain,
            Source::Tab(104),
            group(2, None),
            true,
        ),
        (
            "tab, another planet, slot",
            &plain,
            Source::Tab(104),
            group(2, Some(4)),
            true,
        ),
        (
            "tab, unknown planet",
            &plain,
            Source::Tab(104),
            group(77, None),
            false,
        ),
        (
            "tab, new group",
            &plain,
            Source::Tab(104),
            DropTarget::NewGroup,
            true,
        ),
        (
            "tab, empty space",
            &plain,
            Source::Tab(104),
            DropTarget::Empty,
            true,
        ),
        (
            "tab, loose planet",
            &loose,
            Source::Tab(104),
            group(LOOSE_GROUP_ID, None),
            true,
        ),
        (
            "loose tab, loose planet",
            &loose,
            Source::Tab(900),
            group(LOOSE_GROUP_ID, None),
            false,
        ),
        (
            "loose tab, a group",
            &loose,
            Source::Tab(900),
            group(2, None),
            true,
        ),
        (
            "pinned tab, a group",
            &loose,
            Source::Tab(101),
            group(2, None),
            false,
        ),
        (
            "pinned tab, loose planet",
            &loose,
            Source::Tab(101),
            group(LOOSE_GROUP_ID, None),
            false,
        ),
        (
            "pinned tab, new group",
            &loose,
            Source::Tab(101),
            DropTarget::NewGroup,
            false,
        ),
        (
            "pinned tab, empty space",
            &loose,
            Source::Tab(101),
            DropTarget::Empty,
            false,
        ),
        (
            "unknown tab, a group",
            &plain,
            Source::Tab(9999),
            group(2, None),
            false,
        ),
        (
            "unknown tab, new group",
            &plain,
            Source::Tab(9999),
            DropTarget::NewGroup,
            false,
        ),
        (
            "group, another planet",
            &plain,
            Source::Group(2),
            group(1, None),
            true,
        ),
        (
            "group, another planet, slot",
            &plain,
            Source::Group(2),
            group(4, Some(2)),
            true,
        ),
        (
            "group, itself",
            &plain,
            Source::Group(2),
            group(2, None),
            false,
        ),
        (
            "group, new group",
            &plain,
            Source::Group(2),
            DropTarget::NewGroup,
            false,
        ),
        (
            "group, empty space",
            &plain,
            Source::Group(2),
            DropTarget::Empty,
            false,
        ),
        (
            "group, loose planet",
            &loose,
            Source::Group(2),
            group(LOOSE_GROUP_ID, None),
            false,
        ),
        (
            "loose planet, a group",
            &loose,
            Source::Group(LOOSE_GROUP_ID),
            group(1, None),
            false,
        ),
        (
            "unknown group, a group",
            &plain,
            Source::Group(77),
            group(1, None),
            false,
        ),
    ];
    for (name, input, source, target, accepted) in table {
        assert_eq!(
            can_accept(&payload(source), target, input),
            accepted,
            "{name}"
        );
    }
}

#[test]
fn can_accept_says_yes_exactly_when_the_drop_would_change_something() {
    for input in [input(), input_with_loose()] {
        let mut sources: Vec<Source> = input.tabs.iter().map(|t| Source::Tab(t.id)).collect();
        sources.extend(input.groups.iter().map(|g| Source::Group(g.id)));
        sources.extend([Source::Tab(9999), Source::Group(77)]);
        let mut targets = vec![DropTarget::NewGroup, DropTarget::Empty, group(77, None)];
        for g in &input.groups {
            targets.push(group(g.id, None));
            targets.extend((0..=12).map(|slot| group(g.id, Some(slot))));
        }
        for source in &sources {
            for target in &targets {
                let changes = matches!(
                    resolve(*source, *target, &input),
                    Decision::MoveTab { .. }
                        | Decision::NewGroup { .. }
                        | Decision::ReorderGroup { .. }
                );
                assert_eq!(
                    can_accept(&payload(*source), *target, &input),
                    changes,
                    "{source:?} onto {target:?}"
                );
            }
        }
    }
}

#[test]
fn a_moved_tab_never_lands_outside_its_group() {
    let input = input();
    for tab in &input.tabs {
        for g in &input.groups {
            let len = input.tabs_of(g.id).len();
            for slot in (0..=len + 3).map(Some).chain([None]) {
                if let Decision::MoveTab {
                    group: Some(target),
                    index,
                    ..
                } = resolve(Source::Tab(tab.id), group(g.id, slot), &input)
                {
                    let own = tab.group == target;
                    let max = if own { len - 1 } else { len };
                    assert!(
                        index <= max,
                        "tab {} onto {} slot {slot:?} gave {index}",
                        tab.id,
                        g.id
                    );
                }
            }
        }
    }
}

// --- Decision -> GroupOp -----------------------------------------------------------------------

#[test]
fn a_decision_maps_to_one_controller_operation() {
    assert_eq!(
        Decision::MoveTab {
            tab: 104,
            group: Some(2),
            index: 3
        }
        .op(),
        Some(GroupOp::MoveTab {
            tab: 104,
            group: 2,
            index: 3
        })
    );
    assert_eq!(
        Decision::MoveTab {
            tab: 104,
            group: None,
            index: 2
        }
        .op(),
        Some(GroupOp::Ungroup { tab: 104 })
    );
    assert_eq!(
        Decision::ReorderGroup { group: 2, index: 0 }.op(),
        Some(GroupOp::ReorderGroup { group: 2, index: 0 })
    );
    assert_eq!(Decision::NewGroup { tab: 104 }.op(), None);
    assert_eq!(Decision::NoChange.op(), None);
    assert_eq!(Decision::Reject.op(), None);
}

#[test]
fn resolving_a_drop_on_the_loose_planet_ends_in_ungroup() {
    let input = input_with_loose();
    let decision = resolve(Source::Tab(104), group(LOOSE_GROUP_ID, None), &input);
    assert_eq!(decision.op(), Some(GroupOp::Ungroup { tab: 104 }));
}

#[test]
fn resolving_a_group_drop_ends_in_reorder() {
    let input = input();
    let decision = resolve(Source::Group(4), group(2, None), &input);
    assert_eq!(
        decision.op(),
        Some(GroupOp::ReorderGroup { group: 4, index: 1 })
    );
}

// --- ids and kinds of the Slint side (moved from the desktop glue) -----------------------------

#[test]
fn a_drag_area_kind_and_id_give_the_source() {
    assert_eq!(source_of("tab", 104), Some(Source::Tab(104)));
    assert_eq!(source_of("group", 3), Some(Source::Group(3)));
    assert_eq!(
        source_of("group", LOOSE_GROUP_ID),
        Some(Source::Group(LOOSE_GROUP_ID))
    );
    for kind in ["", "Tab", "tabs", "new-group", "empty", "planet"] {
        assert_eq!(source_of(kind, 1), None, "{kind:?}");
    }
}

#[test]
fn a_drop_area_kind_id_and_slot_give_the_target() {
    assert_eq!(drop_target_of("group", 2, -1), Some(group(2, None)));
    assert_eq!(drop_target_of("group", 2, 0), Some(group(2, Some(0))));
    assert_eq!(drop_target_of("group", 2, 7), Some(group(2, Some(7))));
    assert_eq!(drop_target_of("group", 2, i32::MIN), Some(group(2, None)));
    assert_eq!(
        drop_target_of("new-group", -1, -1),
        Some(DropTarget::NewGroup)
    );
    assert_eq!(drop_target_of("empty", -1, 4), Some(DropTarget::Empty));
    for kind in ["", "tab", "Group", "new_group", "planet"] {
        assert_eq!(drop_target_of(kind, 1, 1), None, "{kind:?}");
    }
}

#[test]
fn a_negative_slint_id_is_the_backdrop() {
    assert_eq!(target_from_id(-1, Target::Planet), Target::Backdrop);
    assert_eq!(target_from_id(i32::MIN, Target::Preview), Target::Backdrop);
    assert_eq!(target_from_id(0, Target::Planet), Target::Planet(0));
    assert_eq!(target_from_id(7, Target::Preview), Target::Preview(7));
    assert_eq!(target_from_id(7, Target::Chip), Target::Chip(7));
    assert_eq!(target_from_id(7, Target::Neighbour), Target::Neighbour(7));
}

#[test]
fn only_a_target_that_would_change_something_is_lit() {
    let input = input();
    // A tab of Research over Lisbon trip: lit; over its own halo: nothing to change.
    assert_eq!(
        lit_target(Source::Tab(104), SESSION, Some(group(2, None)), &input),
        Some(group(2, None))
    );
    assert_eq!(
        lit_target(Source::Tab(104), SESSION, Some(group(1, None)), &input),
        None
    );
    assert_eq!(
        lit_target(
            Source::Tab(104),
            SESSION,
            Some(DropTarget::NewGroup),
            &input
        ),
        Some(DropTarget::NewGroup)
    );
    assert_eq!(
        lit_target(Source::Tab(104), SESSION, Some(DropTarget::Empty), &input),
        Some(DropTarget::Empty)
    );
    // A group: reorder is lit, "New group" and itself are not.
    assert_eq!(
        lit_target(Source::Group(2), SESSION, Some(group(4, None)), &input),
        Some(group(4, None))
    );
    assert_eq!(
        lit_target(
            Source::Group(2),
            SESSION,
            Some(DropTarget::NewGroup),
            &input
        ),
        None
    );
    assert_eq!(
        lit_target(Source::Group(2), SESSION, Some(group(2, None)), &input),
        None
    );
    // No target, a stale source, a pinned tab.
    assert_eq!(lit_target(Source::Tab(104), SESSION, None, &input), None);
    assert_eq!(
        lit_target(Source::Tab(9999), SESSION, Some(group(2, None)), &input),
        None
    );
    let loose = input_with_loose();
    assert_eq!(
        lit_target(Source::Tab(101), SESSION, Some(group(2, None)), &loose),
        None,
        "tab 101 is pinned"
    );
}

#[test]
fn lit_target_agrees_with_can_accept_for_every_pair() {
    let input = input();
    let mut sources: Vec<Source> = input.tabs.iter().map(|t| Source::Tab(t.id)).collect();
    sources.extend(input.groups.iter().map(|g| Source::Group(g.id)));
    let mut targets = vec![DropTarget::NewGroup, DropTarget::Empty];
    for g in &input.groups {
        targets.push(group(g.id, None));
        targets.extend((0..=11).map(|slot| group(g.id, Some(slot))));
    }
    for source in &sources {
        for target in &targets {
            let lit = lit_target(*source, SESSION, Some(*target), &input);
            let accepted = can_accept(&payload(*source), *target, &input);
            assert_eq!(lit, accepted.then_some(*target), "{source:?} on {target:?}");
        }
    }
}
