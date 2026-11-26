// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
use talaria_core::{BackList, Direction, POPUP_ROWS, PopupAction, row_label};

fn urls(list: &[&str]) -> Vec<String> {
    list.iter().map(|url| (*url).to_owned()).collect()
}

fn labels(popup: &talaria_core::HistoryPopup) -> Vec<&str> {
    popup.rows.iter().map(|row| row.label.as_str()).collect()
}

fn steps(popup: &talaria_core::HistoryPopup) -> Vec<i32> {
    popup.rows.iter().map(|row| row.steps).collect()
}

#[test]
fn a_back_popup_lists_the_entries_before_the_current_one_nearest_first() {
    let mut list = BackList::default();
    list.set_entries(
        urls(&[
            "https://a.test/",
            "https://b.test/",
            "https://c.test/",
            "https://d.test/",
        ]),
        3,
    );

    let popup = list.popup(Direction::Back);

    assert_eq!(labels(&popup), ["c.test/", "b.test/", "a.test/"]);
    assert_eq!(steps(&popup), [-1, -2, -3]);
    assert!(!popup.truncated);
    assert!(list.popup(Direction::Forward).is_empty());
    assert!(list.popup(Direction::Forward).is_empty());
}

#[test]
fn a_forward_popup_lists_the_next_entries_in_order() {
    let mut list = BackList::default();
    list.set_entries(
        urls(&[
            "https://a.test/",
            "https://b.test/",
            "https://c.test/",
            "https://d.test/",
        ]),
        0,
    );

    let popup = list.popup(Direction::Forward);

    assert_eq!(steps(&popup), [1, 2, 3]);
    assert_eq!(labels(&popup), ["b.test/", "c.test/", "d.test/"]);
    assert!(list.popup(Direction::Back).is_empty());
}

#[test]
fn the_popup_stops_at_fifteen_rows_and_says_so() {
    let many: Vec<String> = (0..40).map(|i| format!("https://site{i}.test/")).collect();
    let mut list = BackList::default();
    list.set_entries(many.clone(), 39);

    let popup = list.popup(Direction::Back);

    assert_eq!(popup.rows.len(), POPUP_ROWS);
    assert_eq!(POPUP_ROWS, 15);
    assert_eq!(steps(&popup), (-15..=-1).rev().collect::<Vec<i32>>());
    assert!(popup.truncated);

    let mut shorter = BackList::default();
    shorter.set_entries(many[..16].to_vec(), 15);
    let popup = shorter.popup(Direction::Back);
    assert_eq!(popup.rows.len(), 15);
    assert!(!popup.truncated);
}

#[test]
fn the_last_action_is_show_full_history() {
    let mut list = BackList::default();
    list.set_entries(urls(&["https://a.test/", "https://b.test/"]), 1);
    let popup = list.popup(Direction::Back);

    assert_eq!(
        popup.action(popup.rows.len()),
        Some(PopupAction::ShowFullHistory)
    );
    assert_eq!(popup.action(0), Some(PopupAction::Traverse(-1)));
    assert_eq!(popup.action(popup.rows.len() + 1), None);
    assert_eq!(
        talaria_core::HistoryPopup::SHOW_FULL_HISTORY_LABEL,
        "Show full history"
    );
}

#[test]
fn traversal_out_of_range_is_ignored() {
    let mut list = BackList::default();
    list.set_entries(urls(&["https://a.test/", "https://b.test/"]), 1);
    let popup = list.popup(Direction::Back);

    assert_eq!(popup.action(1), Some(PopupAction::ShowFullHistory));
    assert_eq!(popup.action(2), None);
    for row in 0..popup.rows.len() {
        match popup.action(row) {
            Some(PopupAction::Traverse(steps)) => {
                assert!(steps < 0 && steps.unsigned_abs() as usize <= 1);
            }
            other => panic!("row {row} should traverse, got {other:?}"),
        }
    }
}

#[test]
fn titles_follow_their_address_across_list_changes() {
    let mut list = BackList::default();
    list.set_entries(urls(&["https://a.test/", "https://b.test/"]), 1);
    list.set_current_title(Some("B page".to_owned()));
    list.set_entries(
        urls(&["https://a.test/", "https://b.test/", "https://c.test/"]),
        2,
    );

    let popup = list.popup(Direction::Back);
    assert_eq!(popup.rows[0].label, "B page");
    assert_eq!(popup.rows[0].steps, -1);

    list.set_entries(
        urls(&["https://a.test/", "https://b.test/", "https://c.test/"]),
        1,
    );
    list.set_current_title(None);
    list.set_entries(
        urls(&["https://a.test/", "https://b.test/", "https://c.test/"]),
        2,
    );
    let popup = list.popup(Direction::Back);
    assert_eq!(popup.rows[0].label, "b.test/");
}

#[test]
fn row_label_prefers_the_title_then_the_address_without_its_scheme() {
    assert_eq!(
        row_label("https://example.com/a/b", None),
        "example.com/a/b"
    );
    assert_eq!(
        row_label("https://example.com/a", Some("  Hello  ")),
        "Hello"
    );
    assert_eq!(row_label("", None), "New tab");
    assert_eq!(row_label("about:blank", None), "New tab");
}

#[test]
fn row_label_elides_long_titles_and_addresses_to_sixty_characters() {
    let long_title = "t".repeat(200);
    let title = row_label("https://example.com/", Some(&long_title));
    assert_eq!(title.chars().count(), 60);
    assert!(title.ends_with('…'));

    let long_url = format!("https://example.com/{}", "p".repeat(200));
    let label = row_label(&long_url, None);
    assert!(label.chars().count() <= 60);
    assert!(label.starts_with("example.com/"));
    assert!(label.ends_with('p'));
    assert!(label.contains('…'));
}

#[test]
fn current_is_clamped_to_the_list() {
    let mut list = BackList::default();
    list.set_entries(urls(&["https://a.test/"]), 7);

    assert_eq!(list.current(), 0);
    assert!(!list.can_go_back());
    assert!(!list.can_go_forward());

    let mut empty = BackList::default();
    empty.set_entries(Vec::new(), 3);
    assert_eq!(empty.current(), 0);
    assert!(empty.entries().is_empty());
}
