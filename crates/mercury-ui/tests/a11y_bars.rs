// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
#![cfg(feature = "test-support")]
//! What VoiceOver (or any assistive technology) is given for the browser's bars, lists and
//! panels: the history window and its "Clear browsing data" dialog, the suggestions under the
//! address field, the tab strip, its menu and tab search, the bookmarks bar and the star's
//! popover, the downloads button, popover, panel, quit question and offer bar, the find bar,
//! the session bar, the browser menu and the two notices under the toolbar.
//!
//! Each state is shown on a real `MainWindow`; its reading order (or, for a popup, the subtree
//! of the popup, because the test backend visits a popup's content before the item it hangs
//! from) is checked against the labelling rule, the bounds VoiceOver is given, and the golden
//! file in `tests/a11y-snapshots/bars-*.txt`. Default actions are pressed with their effect
//! asserted. The test backend is in-process: nothing reaches the operating system. The one
//! synthesised pointer event is the right click that opens a tab's menu, which has no other way
//! open (the strip's own function is not exposed by the window).

mod common;

use std::cell::RefCell;
use std::rc::Rc;

use common::a11y::{
    self, Node, Rect, assert_bounds, assert_inside, assert_labelled_except, assert_rect,
    assert_snapshot, check_snapshot, control, control_in, node_in, reading_order, snapshot_dir,
    subtree, window_size,
};
use common::a11y::{Exempt, dump};
use i_slint_backend_testing::AccessibleRole as Role;
use mercury_ui::{
    BookmarkBarItem, BookmarkMenuRow, BookmarkRowKind, BrowserMenuCommand, BrowserMenuEntry,
    BrowserTab, DownloadAction, DownloadRow, HistoryRow, MainWindow, SuggestionRow, TabAudioState,
    TabMenuCommand, TabSearchRow,
};
use slint::platform::PointerEventButton;
use slint::{ComponentHandle, ModelRc, SharedString, VecModel};

const WIDTH: f32 = 1180.0;
const HEIGHT: f32 = 760.0;

type Log = Rc<RefCell<Vec<String>>>;

/// A node's `accessible-item-index` and `accessible-item-count`.
type Item = Option<(usize, usize)>;

fn window() -> MainWindow {
    a11y::init();
    let window = MainWindow::new().unwrap();
    a11y::open_window(&window, WIDTH, HEIGHT);
    window
}

fn model<T: Clone + 'static>(items: Vec<T>) -> ModelRc<T> {
    ModelRc::new(VecModel::from(items))
}

fn take(log: &Log) -> Vec<String> {
    std::mem::take(&mut *log.borrow_mut())
}

/// Logs a callback's name and arguments into `log`.
macro_rules! record {
    ($window:expr, $log:expr, $on:ident, || $text:expr) => {{
        let log = Rc::clone(&$log);
        $window.$on(move || log.borrow_mut().push($text.to_string()));
    }};
    ($window:expr, $log:expr, $on:ident, |$($arg:ident),+| $text:expr) => {{
        let log = Rc::clone(&$log);
        $window.$on(move |$($arg),+| log.borrow_mut().push($text));
    }};
}

/// The nodes of the component with this role and label: the element and what is inside it.
fn nodes_of(window: &MainWindow, role: Role, label: &str) -> Vec<Node> {
    subtree(&control(window, role, label))
}

/// Every node with this role and label (a row's buttons share their labels with the other rows').
fn all<'a>(nodes: &'a [Node], role: Role, label: &str) -> Vec<&'a Node> {
    nodes
        .iter()
        .filter(|n| n.role == role && n.label == label)
        .collect()
}

fn exempt(role: Role, label: &'static str, reason: &'static str) -> Exempt {
    Exempt {
        role,
        label,
        reason,
    }
}

/// The checks every state goes through: the labelling rule (with the exemptions the state
/// needs, each with its reason), the bounds, and the golden file.
fn check(name: &str, window: &MainWindow, nodes: &[Node], exemptions: &[Exempt]) {
    assert_labelled_except(nodes, name, exemptions);
    assert_bounds(nodes, window_size(window));
    assert_snapshot(name, nodes);
}

/// Everything in `nodes` lies inside its first node: the control inside its landmark or overlay.
fn assert_inside_first(nodes: &[Node]) {
    for node in &nodes[1..] {
        assert_inside(node, &nodes[0]);
    }
}

fn press(nodes: &[Node], role: Role, label: &str) {
    control_in(nodes, role, label).invoke_accessible_default_action();
}

// ---------------------------------------------------------------------------------------------
// The history window

fn visit(entry: i32, time: &str, title: &str, url: &str, host: &str) -> HistoryRow {
    HistoryRow {
        time: time.into(),
        title: title.into(),
        url: url.into(),
        host: host.into(),
        entry,
        ..HistoryRow::default()
    }
}

fn day(label: &str) -> HistoryRow {
    HistoryRow {
        is_day: true,
        label: label.into(),
        ..HistoryRow::default()
    }
}

fn history_rows() -> Vec<HistoryRow> {
    vec![
        day("Today"),
        visit(
            0,
            "10:42",
            "Example Domain",
            "https://example.com/",
            "example.com",
        ),
        visit(
            1,
            "09:15",
            "Rust documentation",
            "https://doc.rust-lang.org/std/",
            "doc.rust-lang.org",
        ),
        visit(2, "08:03", "", "file:///tmp/notes.txt", ""),
        day("Yesterday"),
        visit(3, "21:30", "Servo", "https://servo.org/", "servo.org"),
        HistoryRow {
            is_more: true,
            ..HistoryRow::default()
        },
    ]
}

const HISTORY_EXEMPT: [(&str, &str); 2] = [
    (
        "Open in new tab",
        "each visit has this button; the description names the page",
    ),
    (
        "Delete",
        "each visit has this button; the description names the page",
    ),
];

fn history_exemptions() -> Vec<Exempt> {
    HISTORY_EXEMPT
        .iter()
        .map(|(label, reason)| exempt(Role::Button, label, reason))
        .collect()
}

fn history_window() -> MainWindow {
    let window = window();
    window.set_history_rows(model(history_rows()));
    window.set_history_open(true);
    window
}

#[test]
fn the_history_window_reads_as_a_list_of_visits() {
    let window = history_window();
    let nodes = nodes_of(&window, Role::Region, "History");
    check("bars-history-panel", &window, &nodes, &history_exemptions());
    assert_inside_first(&nodes);

    // The panel fills the page area (x 0, the window's width), under the tab strip.
    let panel = &nodes[0];
    assert_eq!(panel.rect().x, 0.0);
    assert_eq!(panel.rect().width, WIDTH);

    // Visits are list items named by their title, or by their address when they have none.
    let visits: Vec<&Node> = nodes.iter().filter(|n| n.role == Role::ListItem).collect();
    assert_eq!(visits.len(), 4);
    assert_eq!(visits[0].label, "Example Domain");
    assert_eq!(visits[0].description, "https://example.com/, 10:42");
    assert_eq!(visits[2].label, "file:///tmp/notes.txt");
    // The delete-everything button names the host, and exists only for a visit that has one.
    node_in(
        &nodes,
        Role::Button,
        "Delete all visits to doc.rust-lang.org",
    );
    assert_eq!(all(&nodes, Role::Button, "Delete").len(), 4);
    assert!(
        !nodes
            .iter()
            .any(|n| n.label.starts_with("Delete all visits to ") && n.label.ends_with(' '))
    );
    // Day headings are plain text, once.
    assert_eq!(all(&nodes, Role::Text, "Today").len(), 1);
    assert_eq!(all(&nodes, Role::Text, "Yesterday").len(), 1);
    // The heading "History" is the region's label: it is not read again.
    assert!(all(&nodes, Role::Text, "History").is_empty());
}

#[test]
fn the_history_search_field_says_what_it_is_for() {
    let window = history_window();
    let nodes = nodes_of(&window, Role::Region, "History");
    let field = node_in(&nodes, Role::TextInput, "Search history");
    assert_eq!(field.placeholder(), "Search history");
    assert_eq!(field.value, "");
    // What the visible placeholder says is not read as a text of its own.
    assert!(all(&nodes, Role::Text, "Search history").is_empty());

    window.set_history_query("rust".into());
    let nodes = nodes_of(&window, Role::Region, "History");
    assert_eq!(
        node_in(&nodes, Role::TextInput, "Search history").value,
        "rust"
    );
}

#[test]
fn the_history_message_replaces_the_list() {
    let window = window();
    window.set_history_open(true);
    window.set_history_query("zzz".into());
    window.set_history_message("Nothing in your history matches your search.".into());
    let nodes = nodes_of(&window, Role::Region, "History");
    check("bars-history-panel-message", &window, &nodes, &[]);
    assert!(nodes.iter().all(|n| n.role != Role::List));
    node_in(
        &nodes,
        Role::Text,
        "Nothing in your history matches your search.",
    );
}

/// `count` visits under one heading, then "Show more", as the browser lists a page of history.
fn long_history_rows(count: i32) -> Vec<HistoryRow> {
    let mut rows = vec![day("Today")];
    for entry in 0..count {
        rows.push(visit(
            entry,
            "10:00",
            &format!("Page {entry}"),
            &format!("https://example.com/{entry}"),
            "example.com",
        ));
    }
    rows.push(HistoryRow {
        is_more: true,
        ..HistoryRow::default()
    });
    rows
}

/// The visits an assistive technology can reach in the history window: label and place.
fn reachable_visits(window: &MainWindow) -> Vec<(String, Item)> {
    nodes_of(window, Role::Region, "History")
        .iter()
        .filter(|node| node.role == Role::ListItem)
        .map(|node| (node.label.clone(), node.item))
        .collect()
}

#[test]
fn a_visits_place_counts_the_visits_and_not_the_headings_or_show_more() {
    // Rows: heading, three visits, heading, one visit, "Show more". The visits are 1 to 4 of 4,
    // not 1 to 5 of 7 as the rows are numbered.
    let window = history_window();
    let places: Vec<Item> = reachable_visits(&window)
        .into_iter()
        .map(|(_, item)| item)
        .collect();
    assert_eq!(
        places,
        [Some((0, 4)), Some((1, 4)), Some((2, 4)), Some((3, 4))]
    );

    // The same without "Show more", and with one visit only.
    let mut rows = history_rows();
    rows.pop();
    window.set_history_rows(model(rows));
    assert_eq!(
        reachable_visits(&window).last().map(|(_, item)| *item),
        Some(Some((3, 4)))
    );
    window.set_history_rows(model(vec![
        day("Today"),
        visit(0, "10:42", "Only", "https://a.test/", "a.test"),
    ]));
    assert_eq!(
        reachable_visits(&window),
        [("Only".to_string(), Some((0, 1)))]
    );

    // A second page: the count grows with the visits that were loaded.
    window.set_history_rows(model(long_history_rows(120)));
    assert!(
        reachable_visits(&window)
            .iter()
            .all(|(_, item)| item.is_some_and(|(_, count)| count == 120))
    );
}

#[test]
fn a_long_history_gives_assistive_technology_only_the_rows_on_screen() {
    // KNOWN LIMIT, frozen here so that a change to it is deliberate. The history list is a
    // ListView, which builds only the rows that are on screen, and the accessibility tree has
    // only the rows that exist. Slint's adapter answers no scroll action (it ignores every action
    // but click, focus, increment, decrement, set value and expand), so an assistive technology
    // cannot ask for the rows below. What a person with VoiceOver reaches without scrolling
    // with the wheel is the first screenful; the search field narrows the list. Follow-up:
    // keyboard scrolling (arrow and page keys, scrolling to the focused row).
    let window = window();
    window.set_history_rows(model(long_history_rows(200)));
    window.set_history_open(true);

    let reachable = reachable_visits(&window);
    assert!(
        !reachable.is_empty() && reachable.len() < 30,
        "the rows on a {HEIGHT} pixel window are about fifteen, not {}",
        reachable.len()
    );
    // From the top, in order, each with its place among all 200 visits loaded.
    for (index, (label, item)) in reachable.iter().enumerate() {
        assert_eq!(label, &format!("Page {index}"));
        assert_eq!(*item, Some((index, 200)));
    }
    // The last visit and "Show more" are not in the tree: nothing to find, nothing to press.
    let nodes = nodes_of(&window, Role::Region, "History");
    assert!(all(&nodes, Role::ListItem, "Page 199").is_empty());
    assert!(all(&nodes, Role::Button, "Show more").is_empty());
    // The list says what it is, whatever it holds.
    node_in(&nodes, Role::List, "Visited pages");

    // Scrolling with the wheel (a pointer, which an assistive technology does not have) brings
    // the rows below into the tree, and takes the first ones out.
    let list = node_in(&nodes, Role::List, "Visited pages").rect();
    let position =
        slint::LogicalPosition::new(list.x + list.width / 2.0, list.y + list.height / 2.0);
    window
        .window()
        .dispatch_event(slint::platform::WindowEvent::PointerScrolled {
            position,
            delta_x: 0.0,
            delta_y: -1_000.0,
        });
    let scrolled = reachable_visits(&window);
    let (first_label, first_item) = scrolled.first().expect("rows after scrolling").clone();
    assert_ne!(first_label, "Page 0", "scrolling changed which rows exist");
    let first_index = first_item.expect("a place").0;
    assert!(
        first_index > 0,
        "the rows after a scroll start further down"
    );
    assert_eq!(first_label, format!("Page {first_index}"));
    assert!(
        !scrolled.iter().any(|(label, _)| label == "Page 0"),
        "the first rows left the tree with the scroll"
    );
}

#[test]
fn the_history_buttons_do_what_a_click_does() {
    let window = history_window();
    let log = Log::default();
    record!(
        window,
        log,
        on_history_open_entry,
        |entry, new_tab| format!("open {entry} {new_tab}")
    );
    record!(window, log, on_history_delete_entry, |entry| format!(
        "delete {entry}"
    ));
    record!(window, log, on_history_delete_site, |entry| format!(
        "delete-site {entry}"
    ));
    record!(window, log, on_history_more, || "more");
    record!(window, log, on_history_close, || "close");
    record!(window, log, on_show_clear_dialog, || "clear-dialog");
    let nodes = nodes_of(&window, Role::Region, "History");

    press(&nodes, Role::Button, "Clear browsing data");
    press(&nodes, Role::Button, "Close history");
    press(&nodes, Role::Button, "Show more");
    // The second visit's buttons, found by their description (the page's title) and label.
    let second = |label: &str| -> Node {
        let found: Vec<&Node> = all(&nodes, Role::Button, label)
            .into_iter()
            .filter(|n| n.description == "Rust documentation")
            .collect();
        assert_eq!(found.len(), 1, "{label}");
        found[0].clone()
    };
    second("Open in new tab").press();
    second("Delete").press();
    second("Delete all visits to doc.rust-lang.org").press();
    // A visit pressed as a whole opens it in this tab.
    node_in(&nodes, Role::ListItem, "Rust documentation").press();

    assert_eq!(
        take(&log),
        [
            "clear-dialog",
            "close",
            "more",
            "open 1 true",
            "delete 1",
            "delete-site 1",
            "open 1 false",
        ]
    );
}

// ---------------------------------------------------------------------------------------------
// "Clear browsing data"

fn clear_dialog() -> MainWindow {
    let window = window();
    window.set_clear_dialog_open(true);
    window
}

#[test]
fn the_clear_dialog_reads_as_a_group_of_choices() {
    let window = clear_dialog();
    let nodes = nodes_of(&window, Role::Groupbox, "Clear browsing data");
    check("bars-clear-dialog", &window, &nodes, &[]);
    assert_inside_first(&nodes);

    // The four time ranges are radio buttons, one of them checked, "n of 4".
    let ranges: Vec<&Node> = nodes
        .iter()
        .filter(|n| n.role == Role::RadioButton)
        .collect();
    assert_eq!(
        ranges.iter().map(|n| n.label.as_str()).collect::<Vec<_>>(),
        ["Last hour", "Today", "Last 7 days", "All time"]
    );
    assert_eq!(
        ranges.iter().map(|n| n.checked).collect::<Vec<_>>(),
        [Some(true), Some(false), Some(false), Some(false)]
    );
    assert_eq!(ranges[2].item, Some((2, 4)));
    // What to clear: three check boxes; only the history is on.
    let boxes: Vec<&Node> = nodes.iter().filter(|n| n.role == Role::Checkbox).collect();
    assert_eq!(
        boxes
            .iter()
            .map(|n| (n.label.as_str(), n.checked))
            .collect::<Vec<_>>(),
        [
            ("Browsing and download history", Some(true)),
            ("Cookies and site data", Some(false)),
            ("Cached files", Some(false)),
        ]
    );
    // The heading is the group's label, not read again.
    assert!(all(&nodes, Role::Text, "Clear browsing data").is_empty());
    node_in(&nodes, Role::Text, "Time range");
    node_in(&nodes, Role::Text, "What to clear");
    assert!(node_in(&nodes, Role::Button, "Clear data").enabled);
    assert!(node_in(&nodes, Role::Button, "Cancel").enabled);
}

#[test]
fn the_clear_dialog_follows_what_is_chosen() {
    let window = clear_dialog();
    window.set_clear_period(1);
    window.set_clear_cookies(true);
    let nodes = nodes_of(&window, Role::Groupbox, "Clear browsing data");
    check("bars-clear-dialog-cookies-today", &window, &nodes, &[]);
    // Cookies by date cannot be cleared: the explanation is read.
    assert!(
        nodes
            .iter()
            .any(|n| n.role == Role::Text && n.label.starts_with("Cookies, site data and cached"))
    );

    // Nothing to clear: the button says it cannot be used.
    window.set_clear_cookies(false);
    window.set_clear_history(false);
    let nodes = nodes_of(&window, Role::Groupbox, "Clear browsing data");
    check("bars-clear-dialog-nothing-chosen", &window, &nodes, &[]);
    assert!(!node_in(&nodes, Role::Button, "Clear data").enabled);

    // While it runs: the button is "Clearing", unavailable.
    window.set_clear_history(true);
    window.set_clear_busy(true);
    let nodes = nodes_of(&window, Role::Groupbox, "Clear browsing data");
    check("bars-clear-dialog-busy", &window, &nodes, &[]);
    assert!(!node_in(&nodes, Role::Button, "Clearing").enabled);
}

#[test]
fn the_clear_dialog_choices_and_buttons_do_what_a_click_does() {
    let window = clear_dialog();
    let log = Log::default();
    record!(window, log, on_clear_confirm, || "confirm");
    record!(window, log, on_clear_cancel, || "cancel");
    let nodes = nodes_of(&window, Role::Groupbox, "Clear browsing data");

    press(&nodes, Role::RadioButton, "Last 7 days");
    assert_eq!(window.get_clear_period(), 2);
    let nodes = nodes_of(&window, Role::Groupbox, "Clear browsing data");
    assert_eq!(
        node_in(&nodes, Role::RadioButton, "Last 7 days").checked,
        Some(true)
    );
    assert_eq!(
        node_in(&nodes, Role::RadioButton, "Last hour").checked,
        Some(false)
    );

    press(&nodes, Role::Checkbox, "Cookies and site data");
    assert!(window.get_clear_cookies());
    press(&nodes, Role::Checkbox, "Browsing and download history");
    assert!(!window.get_clear_history());
    let nodes = nodes_of(&window, Role::Groupbox, "Clear browsing data");
    assert_eq!(
        node_in(&nodes, Role::Checkbox, "Cookies and site data").checked,
        Some(true)
    );
    assert_eq!(
        node_in(&nodes, Role::Checkbox, "Browsing and download history").checked,
        Some(false)
    );

    press(&nodes, Role::Button, "Clear data");
    press(&nodes, Role::Button, "Cancel");
    assert_eq!(take(&log), ["confirm", "cancel"]);

    // Nothing to clear, then clearing: the button does nothing.
    window.set_clear_cookies(false);
    window.set_clear_history(false);
    let nodes = nodes_of(&window, Role::Groupbox, "Clear browsing data");
    assert_eq!(
        control_in(&nodes, Role::Button, "Clear data").accessible_enabled(),
        Some(false)
    );
    press(&nodes, Role::Button, "Clear data");
    window.set_clear_history(true);
    window.set_clear_busy(true);
    let nodes = nodes_of(&window, Role::Groupbox, "Clear browsing data");
    press(&nodes, Role::Button, "Clearing");
    assert_eq!(take(&log), Vec::<String>::new());
}

#[test]
fn the_clear_dialog_card_is_where_it_is_drawn() {
    // The card sets a drop shadow and the buttons an opacity: Slint wraps both, and the bounds
    // VoiceOver is given must still be the drawn ones.
    let window = clear_dialog();
    let nodes = nodes_of(&window, Role::Groupbox, "Clear browsing data");
    let card = nodes[0].rect();
    assert_eq!(card.width, 420.0);
    assert_eq!(card.x, (WIDTH - 420.0) / 2.0);
    assert_eq!(card.y, ((HEIGHT - card.height) / 3.0).max(16.0));
    // The rows are 32 px high, 6 px apart, inside the card's 22 px padding.
    let hour = node_in(&nodes, Role::RadioButton, "Last hour").rect();
    let today = node_in(&nodes, Role::RadioButton, "Today").rect();
    assert_eq!(hour.height, 32.0);
    assert_eq!(hour.x, card.x + 22.0);
    assert_eq!(hour.width, 420.0 - 44.0);
    assert_eq!(today.y, hour.y + 32.0 + 6.0);
    // The two buttons are 32 px high, 8 px apart, flush with the right padding, also while one
    // of them is dimmed (opacity 0.45).
    for busy in [false, true] {
        window.set_clear_busy(busy);
        let nodes = nodes_of(&window, Role::Groupbox, "Clear browsing data");
        let cancel = node_in(&nodes, Role::Button, "Cancel").rect();
        let clear = nodes
            .iter()
            .find(|n| n.role == Role::Button && n.label != "Cancel")
            .expect("the clear button")
            .rect();
        assert_eq!(cancel.height, 32.0);
        assert_eq!(clear.height, 32.0);
        assert_eq!(cancel.right() + 8.0, clear.x);
        assert_eq!(clear.right(), card.right() - 22.0);
    }
}

// ---------------------------------------------------------------------------------------------
// Suggestions under the address field

fn suggestion(title: &str, url: &str, is_search: bool, is_bookmark: bool) -> SuggestionRow {
    SuggestionRow {
        is_search,
        is_bookmark,
        title_before: title.into(),
        url_before: url.into(),
        ..SuggestionRow::default()
    }
}

fn suggestion_window() -> MainWindow {
    let window = window();
    window.invoke_focus_address_bar();
    window.set_address("exa".into());
    window.set_suggestions(model(vec![
        suggestion("Search for exa", "", true, false),
        SuggestionRow {
            title_before: "".into(),
            title_match: "Exa".into(),
            title_after: "mple Domain".into(),
            url_before: "https://".into(),
            url_match: "exa".into(),
            url_after: "mple.com/".into(),
            ..SuggestionRow::default()
        },
        suggestion("Example docs", "https://docs.example.com/", false, true),
    ]));
    window.set_suggestion_selected(1);
    window.set_suggestions_open(true);
    assert!(window.get_address_focused(), "the address field has focus");
    window
}

#[test]
fn the_suggestions_read_as_a_list_with_the_selected_row() {
    let window = suggestion_window();
    let nodes = nodes_of(&window, Role::List, "Suggestions");
    check("bars-suggestions", &window, &nodes, &[]);
    assert_inside_first(&nodes);

    let rows: Vec<&Node> = nodes.iter().filter(|n| n.role == Role::ListItem).collect();
    assert_eq!(rows.len(), 3);
    // The emphasised pieces are one label; the kind is in the description.
    assert_eq!(rows[0].label, "Search for exa");
    assert_eq!(rows[0].description, "Search suggestion");
    assert_eq!(rows[1].label, "Example Domain");
    assert_eq!(rows[1].description, "History, https://example.com/");
    assert_eq!(rows[2].description, "Bookmark, https://docs.example.com/");
    assert_eq!(
        rows.iter().map(|n| n.selected).collect::<Vec<_>>(),
        [Some(false), Some(true), Some(false)]
    );
    assert_eq!(
        rows.iter().map(|n| n.item).collect::<Vec<_>>(),
        [Some((0, 3)), Some((1, 3)), Some((2, 3))]
    );
    // Nothing else is read: the star, the clock and the magnifier are hidden.
    assert_eq!(nodes.len(), 4, "{}", dump(&nodes));

    // The list hangs under the address field, 6 px below it, as wide as it.
    let bar = node_in(&reading_order(&window), Role::Search, "Address bar").rect();
    let list = nodes[0].rect();
    assert_eq!(list.x, bar.x);
    assert_eq!(list.width, bar.width);
    assert_eq!(list.y, bar.bottom() + 6.0);
    assert_eq!(list.height, 3.0 * 46.0 + 12.0);
    // The first row is 6 px inside the list's padding.
    assert_rect(
        rows[0],
        Rect::new(list.x + 6.0, list.y + 6.0, list.width - 12.0, 46.0),
    );
}

#[test]
fn the_suggestions_exist_only_while_the_address_field_has_focus() {
    let window = suggestion_window();
    assert!(
        reading_order(&window)
            .iter()
            .any(|n| n.role == Role::List && n.label == "Suggestions")
    );
    // Losing focus closes the list (the application does it on `address-blurred`; the markup
    // alone already hides the list while the field has no focus).
    window.set_is_new_tab(false);
    window.set_web_view_visible(true);
    window.invoke_downloads_focus_web_view();
    assert!(!window.get_address_focused());
    assert!(
        reading_order(&window)
            .iter()
            .all(|n| !(n.role == Role::List && n.label == "Suggestions"))
    );
}

#[test]
fn pressing_a_suggestion_chooses_it() {
    let window = suggestion_window();
    let log = Log::default();
    record!(window, log, on_suggestion_chosen, |index| format!(
        "chosen {index}"
    ));
    let nodes = nodes_of(&window, Role::List, "Suggestions");
    press(&nodes, Role::ListItem, "Example docs");
    press(&nodes, Role::ListItem, "Search for exa");
    assert_eq!(take(&log), ["chosen 2", "chosen 0"]);
}

// ---------------------------------------------------------------------------------------------
// The tab strip

fn tab(id: i32, title: &str, active: bool, pinned: bool, audio: TabAudioState) -> BrowserTab {
    BrowserTab {
        title: title.into(),
        active,
        id,
        pinned,
        audio,
        url: format!("https://site{id}.test/page").into(),
        ..BrowserTab::default()
    }
}

/// Two pinned tabs (one playing, one muted), then three others: a plain one, the active one
/// (loading, playing) and a muted one.
fn strip_tabs() -> Vec<BrowserTab> {
    let mut slint_docs = tab(
        4,
        "Slint documentation",
        true,
        false,
        TabAudioState::Playing,
    );
    slint_docs.loading = true;
    vec![
        tab(1, "Mail", false, true, TabAudioState::Playing),
        tab(2, "Calendar", false, true, TabAudioState::Muted),
        tab(
            3,
            "Servo, the embeddable engine",
            false,
            false,
            TabAudioState::Silent,
        ),
        slint_docs,
        tab(5, "Rust Weekly", false, false, TabAudioState::Muted),
    ]
}

fn strip_window(tabs: Vec<BrowserTab>, pinned: i32) -> MainWindow {
    let window = window();
    let active = tabs.iter().position(|tab| tab.active).unwrap_or(0);
    window.set_tabs(model(tabs));
    window.set_pinned_tab_count(pinned);
    window.set_active_tab_index(active as i32);
    window.set_is_new_tab(false);
    window.set_address("https://docs.slint.dev/".into());
    window
}

fn strip_exemptions() -> Vec<Exempt> {
    vec![exempt(
        Role::Button,
        "Close tab",
        "one in each tab; its description names the tab",
    )]
}

#[test]
fn the_tab_strip_reads_as_a_list_of_tabs() {
    let window = strip_window(strip_tabs(), 2);
    let nodes = nodes_of(&window, Role::TabList, "Tabs");
    check("bars-tab-strip", &window, &nodes, &strip_exemptions());
    assert_inside_first(&nodes);

    let tabs: Vec<&Node> = nodes.iter().filter(|n| n.role == Role::Tab).collect();
    assert_eq!(tabs.len(), 5);
    assert_eq!(
        tabs.iter()
            .map(|n| (n.label.as_str(), n.description.as_str()))
            .collect::<Vec<_>>(),
        [
            ("Mail", "Pinned tab, Playing sound"),
            ("Calendar", "Pinned tab, Muted"),
            ("Servo, the embeddable engine", ""),
            ("Slint documentation", "Loading, Playing sound"),
            ("Rust Weekly", "Muted"),
        ]
    );
    assert_eq!(
        tabs.iter().map(|n| n.selected).collect::<Vec<_>>(),
        [
            Some(false),
            Some(false),
            Some(false),
            Some(true),
            Some(false)
        ]
    );
    assert_eq!(
        tabs.iter().map(|n| n.item).collect::<Vec<_>>(),
        (0..5).map(|i| Some((i, 5))).collect::<Vec<_>>()
    );
    // Each speaker names its tab, and the page's right to override a mute is its description.
    let playing = node_in(&nodes, Role::Button, "Mute tab: Slint documentation");
    assert_eq!(playing.description, "The page can override this");
    let muted = node_in(&nodes, Role::Button, "Unmute tab: Calendar");
    assert_eq!(muted.description, "");
    // Each close button's description names its tab.
    let close = all(&nodes, Role::Button, "Close tab");
    assert_eq!(
        close
            .iter()
            .map(|n| n.description.as_str())
            .collect::<Vec<_>>(),
        [
            "Servo, the embeddable engine",
            "Slint documentation",
            "Rust Weekly"
        ]
    );
    // No glyph is read: the pinned tabs' icons, the loading dots and the cross are hidden.
    assert!(
        nodes
            .iter()
            .all(|n| n.role != Role::Text && n.role != Role::Image),
        "{}",
        dump(&nodes)
    );
}

#[test]
fn a_tab_without_a_title_is_still_named() {
    let window = strip_window(vec![tab(1, "", true, false, TabAudioState::Silent)], 0);
    let nodes = nodes_of(&window, Role::TabList, "Tabs");
    assert_labelled_except(&nodes, "an untitled tab", &strip_exemptions());
    node_in(&nodes, Role::Tab, "Untitled tab");
}

#[test]
fn the_tab_strip_and_its_buttons_do_what_a_click_does() {
    let window = strip_window(strip_tabs(), 2);
    let log = Log::default();
    record!(window, log, on_select_tab, |index| format!(
        "select {index}"
    ));
    record!(window, log, on_close_tab, |index| format!("close {index}"));
    record!(window, log, on_toggle_tab_muted, |id| format!(
        "toggle-muted {id}"
    ));
    let nodes = nodes_of(&window, Role::TabList, "Tabs");

    press(&nodes, Role::Tab, "Calendar");
    // The close button of the last tab, found by what it describes.
    all(&nodes, Role::Button, "Close tab")
        .into_iter()
        .find(|n| n.description == "Rust Weekly")
        .expect("the last tab's close button")
        .press();
    press(&nodes, Role::Button, "Mute tab: Slint documentation");
    press(&nodes, Role::Button, "Unmute tab: Calendar");
    assert_eq!(
        take(&log),
        ["select 1", "close 4", "toggle-muted 4", "toggle-muted 2"]
    );
}

#[test]
fn the_tab_strip_is_laid_out_where_it_says() {
    // Each tab sets a drop shadow property (nothing drawn for a tab that is not being dragged),
    // which makes Slint wrap it: the rectangle given to VoiceOver must still be the tab's own.
    let window = strip_window(strip_tabs(), 2);
    let nodes = nodes_of(&window, Role::TabList, "Tabs");
    let tabs: Vec<Rect> = nodes
        .iter()
        .filter(|n| n.role == Role::Tab)
        .map(|n| n.rect())
        .collect();
    assert_eq!(tabs.len(), 5);
    // Pinned tabs are 40 px wide, the others share the room (184 px at most), 6 px apart.
    assert_eq!(tabs[0].width, 40.0);
    assert_eq!(tabs[1].width, 40.0);
    assert_eq!(tabs[1].x, tabs[0].x + 46.0);
    assert_eq!(tabs[2].width, 184.0);
    assert_eq!(tabs[2].x, tabs[1].x + 46.0);
    assert_eq!(tabs[3].x, tabs[2].x + 184.0 + 6.0);
    assert_eq!(tabs[4].x, tabs[3].x + 184.0 + 6.0);
    let strip = nodes[0].rect();
    assert_eq!(tabs[0].x, strip.x);
    for tab in &tabs {
        assert!(tab.height > 0.0 && tab.y >= strip.y && tab.bottom() <= strip.bottom() + 0.01);
    }
    // The close button is 22 px square, 8 px from the tab's right edge, centred vertically.
    let close = nodes
        .iter()
        .filter(|n| n.role == Role::Button && n.label == "Close tab")
        .map(|n| n.rect())
        .collect::<Vec<_>>();
    assert_eq!(close.len(), 3);
    for (button, tab) in close.iter().zip(&tabs[2..]) {
        assert_eq!((button.width, button.height), (22.0, 22.0));
        assert_eq!(button.right(), tab.right() - 8.0);
        assert_eq!(button.y + button.height / 2.0, tab.y + tab.height / 2.0);
    }
}

/// Eight tabs in a strip too narrow for the 184 px of a wide one (under 160 px each), with sound.
#[test]
fn narrow_tabs_keep_their_speaker_and_show_the_close_button_only_on_the_active_tab() {
    let sound =
        |id: i32, state: TabAudioState| tab(id, &format!("Tab {id}"), id == 3, false, state);
    let tabs: Vec<BrowserTab> = (1..=8)
        .map(|id| match id {
            2 => sound(id, TabAudioState::Playing),
            3 => sound(id, TabAudioState::Playing),
            5 => sound(id, TabAudioState::Muted),
            _ => sound(id, TabAudioState::Silent),
        })
        .collect();
    let window = strip_window(tabs, 0);
    let nodes = nodes_of(&window, Role::TabList, "Tabs");
    let width = node_in(&nodes, Role::Tab, "Tab 1").rect().width;
    assert!(
        (112.0..150.0).contains(&width),
        "{width} px: not a narrow tab"
    );
    check(
        "bars-tab-strip-narrow",
        &window,
        &nodes,
        &strip_exemptions(),
    );
    assert_inside_first(&nodes);
    // The speaker of a narrow tab is read after its tab, as in a wide one; the close button is
    // only drawn on the active tab, under the pointer or from 150 px: an inactive narrow tab has
    // none, whichever way it is read (the tab's menu closes it).
    node_in(&nodes, Role::Button, "Mute tab: Tab 2");
    node_in(&nodes, Role::Button, "Unmute tab: Tab 5");
    assert_eq!(all(&nodes, Role::Button, "Close tab").len(), 1);
}

#[test]
fn an_overflowing_strip_offers_all_tabs() {
    let tabs: Vec<BrowserTab> = (1..=14)
        .map(|id| {
            tab(
                id,
                &format!("Tab {id}"),
                id == 3,
                false,
                TabAudioState::Silent,
            )
        })
        .collect();
    let window = strip_window(tabs, 0);
    let nodes = nodes_of(&window, Role::TabList, "Tabs");
    node_in(&nodes, Role::Button, "Show all tabs");
    // Only the tabs that are scrolled into view are read: Slint clips the others away.
    assert_labelled_except(&nodes, "an overflowing strip", &strip_exemptions());

    // The button opens tab search.
    window.set_tab_search_rows(model(vec![TabSearchRow {
        id: 3,
        title: "Tab 3".into(),
        url: "https://site3.test/page".into(),
        active: true,
        ..TabSearchRow::default()
    }]));
    press(&nodes, Role::Button, "Show all tabs");
    node_in(&reading_order(&window), Role::Search, "Search tabs");
}

// ---------------------------------------------------------------------------------------------
// The tab menu (a popup of the strip)

fn open_tab_menu(window: &MainWindow, label: &str) -> Vec<Node> {
    let strip = nodes_of(window, Role::TabList, "Tabs");
    // In-process, on the testing platform: the only way to open a tab's menu.
    node_in(&strip, Role::Tab, label)
        .handle
        .mock_single_click(PointerEventButton::Right);
    nodes_of(window, Role::List, "Tab menu")
}

#[test]
fn the_tab_menu_reads_as_a_list_of_commands() {
    let window = strip_window(strip_tabs(), 2);
    let nodes = open_tab_menu(&window, "Slint documentation");
    check("bars-tab-menu", &window, &nodes, &[]);
    assert_inside_first(&nodes);
    let rows: Vec<(&str, bool, Item)> = nodes[1..]
        .iter()
        .map(|n| (n.label.as_str(), n.enabled, n.item))
        .collect();
    assert_eq!(
        rows,
        [
            ("Pin tab", true, Some((0, 4))),
            ("Duplicate tab", true, Some((1, 4))),
            ("Close tab", true, Some((2, 4))),
            ("Close other tabs", true, Some((3, 4))),
        ]
    );
    // The menu's "Close tab" is a list item, the tab's own is a button: no two controls share
    // role and label while the menu is open over the strip.
    let everything = reading_order(&window);
    assert_labelled_except(&everything, "a window with a tab menu", &strip_exemptions());
}

#[test]
fn the_tab_menu_says_what_cannot_be_done() {
    // A pinned tab, and the only unpinned tab left: "Unpin tab", and nothing to close beside it.
    let tabs = vec![
        tab(1, "Mail", true, true, TabAudioState::Silent),
        tab(2, "Docs", false, false, TabAudioState::Silent),
    ];
    let window = strip_window(tabs, 1);
    let nodes = open_tab_menu(&window, "Docs");
    check("bars-tab-menu-last-tab", &window, &nodes, &[]);
    assert!(!node_in(&nodes, Role::ListItem, "Close other tabs").enabled);
    assert!(node_in(&nodes, Role::ListItem, "Pin tab").enabled);

    // A pinned tab's menu offers "Unpin tab".
    let window = strip_window(
        vec![
            tab(1, "Mail", false, true, TabAudioState::Silent),
            tab(2, "Docs", true, false, TabAudioState::Silent),
            tab(3, "News", false, false, TabAudioState::Silent),
        ],
        1,
    );
    let nodes = open_tab_menu(&window, "Mail");
    node_in(&nodes, Role::ListItem, "Unpin tab");
    assert!(node_in(&nodes, Role::ListItem, "Close other tabs").enabled);
}

#[test]
fn the_tab_menu_rows_do_what_a_click_does() {
    let window = strip_window(strip_tabs(), 2);
    let log = Log::default();
    record!(window, log, on_tab_menu_activated, |id, command| format!(
        "{id} {command:?}"
    ));
    let nodes = open_tab_menu(&window, "Slint documentation");
    press(&nodes, Role::ListItem, "Duplicate tab");
    // Choosing closes the menu.
    assert!(
        reading_order(&window)
            .iter()
            .all(|n| !(n.role == Role::List && n.label == "Tab menu"))
    );
    let nodes = open_tab_menu(&window, "Rust Weekly");
    press(&nodes, Role::ListItem, "Close tab");
    let nodes = open_tab_menu(&window, "Rust Weekly");
    press(&nodes, Role::ListItem, "Pin tab");
    assert_eq!(
        take(&log),
        [
            format!("4 {:?}", TabMenuCommand::Duplicate),
            format!("5 {:?}", TabMenuCommand::Close),
            format!("5 {:?}", TabMenuCommand::Pin),
        ]
    );
}

// ---------------------------------------------------------------------------------------------
// Tab search (a popup of the window)

fn search_rows() -> Vec<TabSearchRow> {
    let row = |id: i32, title: &str, url: &str, pinned: bool, active: bool| TabSearchRow {
        id,
        title: title.into(),
        url: url.into(),
        pinned,
        active,
        ..TabSearchRow::default()
    };
    vec![
        row(1, "Mail", "https://mail.example.com/", true, false),
        row(
            2,
            "Slint documentation",
            "https://docs.slint.dev/",
            false,
            true,
        ),
        row(
            3,
            "Rust Weekly",
            "https://this-week-in-rust.org/",
            false,
            false,
        ),
        row(4, "", "", false, false),
    ]
}

#[test]
fn tab_search_reads_as_a_search_of_a_list_of_tabs() {
    let window = window();
    window.set_tab_search_rows(model(search_rows()));
    window.invoke_show_tab_search();
    let nodes = nodes_of(&window, Role::Search, "Search tabs");
    check("bars-tab-search", &window, &nodes, &[]);
    assert_inside_first(&nodes);

    let field = node_in(&nodes, Role::TextInput, "Search tabs");
    assert_eq!(field.placeholder(), "Search tabs");
    let list = node_in(&nodes, Role::List, "Open tabs");
    let rows: Vec<&Node> = nodes.iter().filter(|n| n.role == Role::ListItem).collect();
    assert_eq!(
        rows.iter()
            .map(|n| (n.label.as_str(), n.description.as_str(), n.selected))
            .collect::<Vec<_>>(),
        [
            ("Mail", "Pinned tab, https://mail.example.com/", Some(true)),
            (
                "Slint documentation",
                "Current tab, https://docs.slint.dev/",
                Some(false)
            ),
            ("Rust Weekly", "https://this-week-in-rust.org/", Some(false)),
            ("Untitled tab", "", Some(false)),
        ]
    );
    assert_eq!(rows[3].item, Some((3, 4)));
    for row in &rows {
        assert_inside(row, list);
    }
    // The panel is as wide as it is drawn and as high as its rows make it. (The test backend
    // gives a popup's content positions relative to the popup, and the adapter adds the popup's
    // own position: only sizes are checked for popups.)
    let panel = nodes[0].rect();
    assert_eq!(
        (panel.width, panel.height),
        (420.0, 8.0 + 34.0 + 6.0 + 4.0 * 46.0 + 8.0)
    );
    assert_eq!((panel.x, panel.y), (0.0, 0.0));
}

#[test]
fn tab_search_with_nothing_found_says_so() {
    let window = window();
    window.set_tab_search_rows(model(vec![]));
    window.invoke_show_tab_search();
    let nodes = nodes_of(&window, Role::Search, "Search tabs");
    check("bars-tab-search-nothing", &window, &nodes, &[]);
    node_in(&nodes, Role::Text, "No open tab matches");
}

#[test]
fn pressing_a_tab_in_tab_search_chooses_it() {
    let window = window();
    window.set_tab_search_rows(model(search_rows()));
    window.invoke_show_tab_search();
    let log = Log::default();
    record!(window, log, on_tab_search_chosen, |id| format!(
        "chosen {id}"
    ));
    let nodes = nodes_of(&window, Role::Search, "Search tabs");
    press(&nodes, Role::ListItem, "Rust Weekly");
    assert_eq!(take(&log), ["chosen 3"]);
    // Choosing closes the popup.
    assert!(
        reading_order(&window)
            .iter()
            .all(|n| !(n.role == Role::Search && n.label == "Search tabs"))
    );
}

// ---------------------------------------------------------------------------------------------
// The bookmarks bar, its menus and the star

fn bar_item(title: &str, is_folder: bool, x: f32, width: f32) -> BookmarkBarItem {
    BookmarkBarItem {
        title: title.into(),
        is_folder,
        x,
        width,
        shown: true,
    }
}

fn bookmarks_window(overflow: bool) -> MainWindow {
    let window = window();
    window.set_bookmarks_bar_items(model(vec![
        bar_item("Mercury", false, 0.0, 90.0),
        bar_item("News", true, 96.0, 70.0),
        bar_item("", false, 172.0, 90.0),
    ]));
    window.set_bookmarks_bar_overflow(overflow);
    window
}

fn menu_row(title: &str, url: &str, kind: BookmarkRowKind) -> BookmarkMenuRow {
    BookmarkMenuRow {
        title: title.into(),
        url: url.into(),
        kind,
        depth: 0,
    }
}

#[test]
fn the_bookmarks_bar_reads_as_a_group_of_buttons() {
    let window = bookmarks_window(true);
    let nodes = nodes_of(&window, Role::Groupbox, "Bookmarks bar");
    check("bars-bookmarks-bar", &window, &nodes, &[]);
    assert_inside_first(&nodes);

    let buttons: Vec<&Node> = nodes[1..]
        .iter()
        .filter(|n| n.label != "More bookmarks")
        .collect();
    assert_eq!(
        buttons
            .iter()
            .map(|n| (n.label.as_str(), n.description.as_str(), n.item))
            .collect::<Vec<_>>(),
        [
            ("Mercury", "", Some((0, 3))),
            ("News", "Folder", Some((1, 3))),
            ("(untitled)", "", Some((2, 3))),
        ]
    );
    node_in(&nodes, Role::Button, "More bookmarks");
    // Nothing is read twice: the icons and the drawn titles are hidden.
    assert!(
        nodes
            .iter()
            .all(|n| n.role != Role::Text && n.role != Role::Image)
    );
}

#[test]
fn an_empty_bookmarks_bar_says_what_goes_there() {
    let window = window();
    let nodes = nodes_of(&window, Role::Groupbox, "Bookmarks bar");
    check("bars-bookmarks-bar-empty", &window, &nodes, &[]);
    node_in(
        &nodes,
        Role::Text,
        "Bookmarks you put in the bookmarks bar appear here.",
    );
}

#[test]
fn the_bookmarks_bar_buttons_do_what_a_click_does() {
    let window = bookmarks_window(true);
    let log = Log::default();
    record!(
        window,
        log,
        on_bookmarks_bar_item_activated,
        |index, new_tab| { format!("item {index} {new_tab}") }
    );
    record!(window, log, on_bookmarks_bar_folder_opened, |index, x| {
        format!("folder {index} {x}")
    });
    record!(window, log, on_bookmarks_bar_overflow_opened, |x| {
        format!("overflow {x}")
    });
    let nodes = nodes_of(&window, Role::Groupbox, "Bookmarks bar");
    press(&nodes, Role::Button, "Mercury");
    press(&nodes, Role::Button, "News");
    press(&nodes, Role::Button, "(untitled)");
    press(&nodes, Role::Button, "More bookmarks");
    let bar = nodes[0].rect();
    let entries = take(&log);
    // The folder opens where its button is (relative to the bar, 10 px of padding plus its x);
    // the overflow button is at the bar's right end, 28 px wide and 10 px from the edge.
    assert_eq!(
        entries[..2],
        ["item 0 false".to_string(), "folder 1 106".to_string()]
    );
    assert_eq!(entries[2], "item 2 false");
    assert_eq!(entries[3], format!("overflow {}", bar.width - 10.0 - 28.0));
}

fn menu_rows() -> Vec<BookmarkMenuRow> {
    vec![
        menu_row("Bookmarks Bar", "", BookmarkRowKind::Back),
        menu_row(
            "Mercury",
            "https://mercury.example/",
            BookmarkRowKind::Bookmark,
        ),
        menu_row("News", "", BookmarkRowKind::Folder),
        menu_row("", "", BookmarkRowKind::Separator),
        menu_row("", "https://untitled.example/", BookmarkRowKind::Bookmark),
        menu_row("Empty", "", BookmarkRowKind::Empty),
    ]
}

#[test]
fn the_bookmarks_menu_reads_as_a_list() {
    let window = bookmarks_window(false);
    window.set_bookmarks_menu_rows(model(menu_rows()));
    window.invoke_show_bookmarks_menu(100.0);
    let nodes = nodes_of(&window, Role::List, "Bookmarks menu");
    check("bars-bookmarks-menu", &window, &nodes, &[]);
    assert_inside_first(&nodes);

    let rows: Vec<(&str, &str, &str, Item)> = nodes[1..]
        .iter()
        .map(|n| {
            (
                n.role_name(),
                n.label.as_str(),
                n.description.as_str(),
                n.item,
            )
        })
        .collect();
    assert_eq!(
        rows,
        [
            ("list-item", "Back to Bookmarks Bar", "", Some((0, 6))),
            (
                "list-item",
                "Mercury",
                "https://mercury.example/",
                Some((1, 6))
            ),
            ("list-item", "News", "Folder", Some((2, 6))),
            (
                "list-item",
                "(untitled)",
                "https://untitled.example/",
                Some((4, 6))
            ),
            // "Empty" is text, and the separator is not read at all.
            ("text", "Empty", "", None),
        ]
    );
    // The popup is 300 px wide and as high as its six 28 px rows and the padding make it.
    let list = nodes[0].rect();
    assert_eq!((list.width, list.height), (300.0, 6.0 * 28.0 + 12.0));
}

#[test]
fn the_bookmarks_menu_rows_do_what_a_click_does() {
    let window = bookmarks_window(false);
    window.set_bookmarks_menu_rows(model(menu_rows()));
    window.invoke_show_bookmarks_menu(100.0);
    let log = Log::default();
    record!(
        window,
        log,
        on_bookmarks_menu_row_activated,
        |index, new_tab| { format!("row {index} {new_tab}") }
    );
    let nodes = nodes_of(&window, Role::List, "Bookmarks menu");
    press(&nodes, Role::ListItem, "Back to Bookmarks Bar");
    press(&nodes, Role::ListItem, "Mercury");
    press(&nodes, Role::ListItem, "News");
    press(&nodes, Role::ListItem, "(untitled)");
    assert_eq!(
        take(&log),
        ["row 0 false", "row 1 false", "row 2 false", "row 4 false"]
    );
}

fn star_window(enabled: bool, filled: bool) -> MainWindow {
    let window = window();
    window.set_is_new_tab(false);
    window.set_address("https://example.com/".into());
    window.set_bookmark_star_enabled(enabled);
    window.set_bookmark_star_filled(filled);
    window
}

#[test]
fn the_star_says_whether_the_page_is_bookmarked() {
    let window = star_window(true, false);
    let nodes = reading_order(&window);
    let star = node_in(&nodes, Role::Button, "Bookmark this page");
    assert!(star.enabled);
    window.set_bookmark_star_filled(true);
    let nodes = reading_order(&window);
    assert!(
        nodes.iter().all(|n| n.label != "Bookmark this page"),
        "the label did not follow the state"
    );
    let star = node_in(&nodes, Role::Button, "Edit bookmark for this page");
    assert!(star.enabled);
    // No glyph is read: the star is a label, not "★".
    assert!(nodes.iter().all(|n| n.label != "★" && n.label != "☆"));

    // A disabled star (no page to bookmark) says so and does nothing.
    window.set_bookmark_star_enabled(false);
    let log = Log::default();
    record!(window, log, on_bookmark_star_clicked, || "star");
    let nodes = reading_order(&window);
    let star = node_in(&nodes, Role::Button, "Edit bookmark for this page");
    assert!(!star.enabled);
    star.press();
    assert_eq!(take(&log), Vec::<String>::new());
    window.set_bookmark_star_enabled(true);
    press(
        &reading_order(&window),
        Role::Button,
        "Edit bookmark for this page",
    );
    assert_eq!(take(&log), ["star"]);
}

#[test]
fn the_star_is_where_it_is_drawn() {
    // The star sets an opacity (0.32 while disabled): the bounds must not move with it.
    let window = star_window(true, false);
    let address_bar = node_in(&reading_order(&window), Role::Search, "Address bar").rect();
    let mut rects = Vec::new();
    for enabled in [true, false] {
        window.set_bookmark_star_enabled(enabled);
        let nodes = reading_order(&window);
        let star = node_in(&nodes, Role::Button, "Bookmark this page");
        assert_eq!((star.rect().width, star.rect().height), (30.0, 30.0));
        // 4 px from the right end of the address field, centred in it.
        assert_eq!(star.rect().right(), address_bar.right() - 4.0);
        assert!(
            (star.rect().y + 15.0 - (address_bar.y + address_bar.height / 2.0)).abs() <= 0.01,
            "{:?} {:?}",
            star.rect(),
            address_bar
        );
        rects.push(star.rect());
    }
    assert!(rects[0].same_as(&rects[1]));
}

/// Slint's own `ComboBox` draws its drop-down arrow as an `Image` it does not hide; the widget is
/// not ours to change. Every state with the folder drop-down lists it, with this reason.
const COMBOBOX_ARROW: [Exempt; 1] = [Exempt {
    role: Role::Image,
    label: "",
    reason: "the drop-down arrow of Slint's own ComboBox, which the widget does not hide",
}];

#[test]
fn the_bookmark_popover_reads_as_a_group() {
    let window = star_window(true, true);
    window.set_bookmark_popover_is_new(true);
    window.set_bookmark_popover_name("Example Domain".into());
    window.set_bookmark_popover_folders(model(vec![
        SharedString::from("Bookmarks bar"),
        "Other bookmarks".into(),
    ]));
    window.invoke_open_bookmark_popover();
    let nodes = nodes_of(&window, Role::Groupbox, "Bookmark added");
    check(
        "bars-bookmark-popover-new",
        &window,
        &nodes,
        &COMBOBOX_ARROW,
    );
    assert_inside_first(&nodes);
    assert_eq!(
        node_in(&nodes, Role::TextInput, "Bookmark name").value,
        "Example Domain"
    );
    node_in(&nodes, Role::Combobox, "Bookmark folder");
    node_in(&nodes, Role::Button, "Remove");
    node_in(&nodes, Role::Button, "Done");
    // The captions are read once: "Name" and "Folder" are the fields' labels.
    assert!(
        nodes.iter().all(|n| n.role != Role::Text),
        "{}",
        dump(&nodes)
    );

    // The popover is 340 x 196 px (the position of a popup's content is relative to the popup).
    assert_rect(&nodes[0], Rect::new(0.0, 0.0, 340.0, 196.0));

    window.invoke_close_bookmark_popover();
    window.set_bookmark_popover_is_new(false);
    window.invoke_open_bookmark_popover();
    let nodes = nodes_of(&window, Role::Groupbox, "Edit bookmark");
    check(
        "bars-bookmark-popover-edit",
        &window,
        &nodes,
        &COMBOBOX_ARROW,
    );
}

#[test]
fn the_bookmark_popover_buttons_do_what_a_click_does() {
    let window = star_window(true, true);
    window.set_bookmark_popover_folders(model(vec![SharedString::from("Bookmarks bar")]));
    let log = Log::default();
    record!(window, log, on_bookmark_popover_done, || "done");
    record!(window, log, on_bookmark_popover_remove, || "remove");
    window.invoke_open_bookmark_popover();
    let nodes = nodes_of(&window, Role::Groupbox, "Edit bookmark");
    press(&nodes, Role::Button, "Done");
    // Done closes the popover.
    assert!(
        reading_order(&window)
            .iter()
            .all(|n| n.label != "Edit bookmark")
    );
    window.invoke_open_bookmark_popover();
    let nodes = nodes_of(&window, Role::Groupbox, "Edit bookmark");
    press(&nodes, Role::Button, "Remove");
    assert_eq!(take(&log), ["done", "remove"]);
}

// ---------------------------------------------------------------------------------------------
// Downloads

fn download(guid: &str, name: &str, status: &str, progress: f32) -> DownloadRow {
    DownloadRow {
        guid: guid.into(),
        name: name.into(),
        status: status.into(),
        progress,
        ..DownloadRow::default()
    }
}

fn downloads() -> Vec<DownloadRow> {
    vec![
        DownloadRow {
            can_cancel: true,
            ..download("g1", "report.pdf", "4.2 MB of 9 MB", 0.46)
        },
        DownloadRow {
            can_open: true,
            can_show: true,
            can_remove: true,
            ..download("g2", "photo.jpg", "Done, 1.1 MB", -1.0)
        },
        DownloadRow {
            failed: true,
            can_retry: true,
            can_remove: true,
            ..download("g3", "setup.dmg", "Failed: no space left", -1.0)
        },
    ]
}

fn downloads_window() -> MainWindow {
    let window = window();
    window.set_downloads_button_visible(true);
    window.set_downloads_show_label("Show in Finder".into());
    window
}

#[test]
fn the_downloads_button_says_what_the_ring_and_the_dot_show() {
    let window = downloads_window();
    // One button seen at four moments. A node reads its state when it is dumped, so each moment
    // is dumped as it is, and the golden file is the four lines together.
    let mut text = String::new();
    let mut seen = |window: &MainWindow, node: &Node| {
        assert_labelled_except(std::slice::from_ref(node), "a downloads button", &[]);
        assert_bounds(std::slice::from_ref(node), window_size(window));
        text.push_str(&dump(std::slice::from_ref(node)));
    };
    let button = |window: &MainWindow, label: &str| -> Node {
        node_in(&reading_order(window), Role::Button, label).clone()
    };

    // Idle.
    let idle = button(&window, "Downloads");
    assert_eq!(idle.description, "");
    assert_eq!(idle.handle.accessible_expandable(), Some(true));
    assert_eq!(idle.handle.accessible_expanded(), Some(false));
    seen(&window, &idle);

    // A download is running, 46 percent done, and something started since the list was opened.
    window.set_downloads_active(true);
    window.set_downloads_progress(0.46);
    window.set_downloads_attention(true);
    let running = button(&window, "Downloads, in progress");
    assert_eq!(
        running.description,
        "46 percent done. Something changed since you last looked"
    );
    seen(&window, &running);

    // The size is unknown: no percentage.
    window.set_downloads_progress(2.0);
    let unknown = button(&window, "Downloads, in progress");
    assert_eq!(
        unknown.description,
        "Something changed since you last looked"
    );
    seen(&window, &unknown);

    // The popover is open: the button says so, and the dot is gone.
    window.set_downloads_progress(0.9);
    window.set_downloads_popover_open(true);
    let open = button(&window, "Downloads, in progress");
    assert_eq!(open.description, "90 percent done");
    assert_eq!(open.handle.accessible_expanded(), Some(true));
    seen(&window, &open);
    let bless = std::env::var("MERCURY_A11Y_BLESS").is_ok_and(|value| value == "1");
    check_snapshot(&snapshot_dir(), "bars-downloads-button", &text, bless);
}

#[test]
fn pressing_the_downloads_button_toggles_the_list() {
    let window = downloads_window();
    let log = Log::default();
    record!(window, log, on_downloads_toggle_popover, || "toggle");
    press(&reading_order(&window), Role::Button, "Downloads");
    assert_eq!(take(&log), ["toggle"]);
    // It is where the toolbar puts it: 36 x 34 px, in the toolbar.
    let nodes = reading_order(&window);
    let button = node_in(&nodes, Role::Button, "Downloads").rect();
    let toolbar = node_in(&nodes, Role::Navigation, "Toolbar").rect();
    assert_eq!((button.width, button.height), (36.0, 34.0));
    assert!(toolbar.contains(&button), "{button:?} {toolbar:?}");
}

fn downloads_exemptions() -> Vec<Exempt> {
    Vec::new()
}

#[test]
fn the_downloads_popover_reads_as_a_list_of_downloads() {
    let window = downloads_window();
    window.set_downloads_recent(model(downloads()));
    window.set_downloads_popover_open(true);
    let nodes = nodes_of(&window, Role::List, "Downloads");
    check(
        "bars-downloads-popover",
        &window,
        &nodes,
        &downloads_exemptions(),
    );
    assert_inside_first(&nodes);

    let rows: Vec<(&str, Option<(usize, usize)>)> = nodes
        .iter()
        .filter(|n| n.role == Role::ListItem)
        .map(|n| (n.label.as_str(), n.item))
        .collect();
    assert_eq!(
        rows,
        [
            ("report.pdf, 4.2 MB of 9 MB", Some((0, 3))),
            ("photo.jpg, Done, 1.1 MB", Some((1, 3))),
            ("setup.dmg, Failed: no space left", Some((2, 3))),
        ]
    );
    // Each button of a row names the download, so two rows' buttons never read alike.
    node_in(&nodes, Role::Button, "Cancel: report.pdf");
    node_in(&nodes, Role::Button, "Open: photo.jpg");
    node_in(&nodes, Role::Button, "Show in Finder: photo.jpg");
    node_in(&nodes, Role::Button, "Remove: photo.jpg");
    node_in(&nodes, Role::Button, "Retry: setup.dmg");
    node_in(&nodes, Role::Button, "Remove: setup.dmg");
    node_in(&nodes, Role::Button, "Clear list");
    node_in(&nodes, Role::Button, "Show all downloads");
    // The heading is the list's label, the rows' texts are their labels: nothing reads twice.
    assert!(
        nodes.iter().all(|n| n.role != Role::Text),
        "{}",
        dump(&nodes)
    );

    // The popover hangs 6 px under the downloads button, ending at its right edge: 460 px wide.
    let button = node_in(&reading_order(&window), Role::Button, "Downloads").rect();
    let popover = nodes[0].rect();
    assert_eq!(popover.width, 460.0);
    assert_eq!(popover.y, button.bottom() + 6.0);
    assert_eq!(popover.x, (button.right() - 460.0).max(8.0));
}

#[test]
fn an_empty_downloads_popover_says_so() {
    let window = downloads_window();
    window.set_downloads_popover_open(true);
    let nodes = nodes_of(&window, Role::List, "Downloads");
    check("bars-downloads-popover-empty", &window, &nodes, &[]);
    node_in(&nodes, Role::Text, "No downloads");
}

#[test]
fn the_downloads_panel_reads_as_a_list_of_downloads() {
    let window = downloads_window();
    window.set_downloads_all(model(downloads()));
    window.set_downloads_can_show_more(true);
    window.set_downloads_panel_open(true);
    let nodes = nodes_of(&window, Role::List, "Downloads");
    check(
        "bars-downloads-panel",
        &window,
        &nodes,
        &downloads_exemptions(),
    );
    assert_inside_first(&nodes);
    node_in(&nodes, Role::Button, "Clear list");
    node_in(&nodes, Role::Button, "Close");
    node_in(&nodes, Role::Button, "Show more");

    window.set_downloads_all(model(vec![]));
    let nodes = nodes_of(&window, Role::List, "Downloads");
    check("bars-downloads-panel-empty", &window, &nodes, &[]);
}

#[test]
fn the_downloads_buttons_do_what_a_click_does() {
    let window = downloads_window();
    window.set_downloads_recent(model(downloads()));
    window.set_downloads_popover_open(true);
    let log = Log::default();
    record!(window, log, on_downloads_action, |guid, action| format!(
        "{guid} {action:?}"
    ));
    record!(window, log, on_downloads_clear_list, || "clear-list");
    record!(window, log, on_downloads_show_all, || "show-all");
    let nodes = nodes_of(&window, Role::List, "Downloads");
    press(&nodes, Role::Button, "Cancel: report.pdf");
    press(&nodes, Role::Button, "Open: photo.jpg");
    press(&nodes, Role::Button, "Show in Finder: photo.jpg");
    press(&nodes, Role::Button, "Remove: photo.jpg");
    press(&nodes, Role::Button, "Retry: setup.dmg");
    press(&nodes, Role::Button, "Clear list");
    press(&nodes, Role::Button, "Show all downloads");
    assert_eq!(
        take(&log),
        [
            format!("g1 {:?}", DownloadAction::Cancel),
            format!("g2 {:?}", DownloadAction::Open),
            format!("g2 {:?}", DownloadAction::Show),
            format!("g2 {:?}", DownloadAction::Remove),
            format!("g3 {:?}", DownloadAction::Retry),
            "clear-list".to_string(),
            "show-all".to_string(),
        ]
    );

    window.set_downloads_popover_open(false);
    window.set_downloads_all(model(downloads()));
    window.set_downloads_can_show_more(true);
    window.set_downloads_panel_open(true);
    record!(window, log, on_downloads_show_more, || "show-more");
    record!(window, log, on_downloads_close_panel, || "close-panel");
    let nodes = nodes_of(&window, Role::List, "Downloads");
    press(&nodes, Role::Button, "Show more");
    press(&nodes, Role::Button, "Close");
    assert_eq!(take(&log), ["show-more", "close-panel"]);
}

#[test]
fn the_quit_question_reads_as_a_group_with_its_two_answers() {
    let window = window();
    window.set_quit_prompt_title("Cancel 2 downloads and quit?".into());
    window.set_quit_prompt_body("report.pdf and 1 more are still downloading.".into());
    window.set_quit_prompt_confirm_label("Cancel downloads and quit".into());
    window.set_quit_prompt_keep_label("Keep downloading".into());
    window.set_quit_prompt_open(true);
    let log = Log::default();
    record!(window, log, on_quit_prompt_confirm, || "confirm");
    record!(window, log, on_quit_prompt_keep, || "keep");
    let nodes = nodes_of(&window, Role::Groupbox, "Cancel 2 downloads and quit?");
    check("bars-quit-prompt", &window, &nodes, &[]);
    assert_eq!(
        nodes[0].description,
        "report.pdf and 1 more are still downloading."
    );
    assert!(nodes.iter().all(|n| n.role != Role::Text));
    // The card is a drop-shadowed Rectangle of its own: its buttons are 26 px high, 8 px apart,
    // flush with the 22 px of padding of the 420 px wide card in the middle of the window.
    let confirm = node_in(&nodes, Role::Button, "Cancel downloads and quit").rect();
    let keep = node_in(&nodes, Role::Button, "Keep downloading").rect();
    assert_eq!((confirm.height, keep.height), (26.0, 26.0));
    assert_eq!(confirm.right() + 8.0, keep.x);
    assert_eq!(keep.right(), (WIDTH + 420.0) / 2.0 - 22.0);
    press(&nodes, Role::Button, "Cancel downloads and quit");
    press(&nodes, Role::Button, "Keep downloading");
    assert_eq!(take(&log), ["confirm", "keep"]);
}

#[test]
fn the_download_offer_bar_names_the_site_and_has_three_answers() {
    let window = window();
    window.set_is_new_tab(false);
    window.set_download_offer_text("example.com wants to download a file.".into());
    window.set_download_offer_detail("From files.example.net".into());
    window.set_download_offer_visible(true);
    let log = Log::default();
    record!(window, log, on_download_offer_download, || "download");
    record!(window, log, on_download_offer_block, || "block");
    record!(window, log, on_download_offer_dismiss, || "dismiss");
    let nodes = nodes_of(
        &window,
        Role::Groupbox,
        "example.com wants to download a file.",
    );
    check("bars-download-offer", &window, &nodes, &[]);
    assert_inside_first(&nodes);
    assert_eq!(nodes[0].description, "From files.example.net");
    press(&nodes, Role::Button, "Download");
    press(&nodes, Role::Button, "Block");
    press(&nodes, Role::Button, "Not now");
    assert_eq!(take(&log), ["download", "block", "dismiss"]);

    // From the page's own host there is no detail, and no dangling full stop in the label.
    window.set_download_offer_detail("".into());
    let nodes = nodes_of(
        &window,
        Role::Groupbox,
        "example.com wants to download a file.",
    );
    assert_eq!(nodes[0].description, "");
    check("bars-download-offer-same-host", &window, &nodes, &[]);
}

// ---------------------------------------------------------------------------------------------
// The find bar

fn find_window() -> MainWindow {
    let window = window();
    window.set_is_new_tab(false);
    window.set_find_open(true);
    window
}

#[test]
fn the_find_bar_has_five_controls_and_a_status() {
    let window = find_window();
    let nodes = nodes_of(&window, Role::Search, "Find bar");
    check("bars-find-bar", &window, &nodes, &[]);
    assert_inside_first(&nodes);
    let field = node_in(&nodes, Role::TextInput, "Find in page");
    assert_eq!(field.placeholder(), "Find in page");
    // Nothing typed: Previous and Next cannot be used.
    assert!(!node_in(&nodes, Role::Button, "Previous match").enabled);
    assert!(!node_in(&nodes, Role::Button, "Next match").enabled);
    assert!(node_in(&nodes, Role::Button, "Match case").enabled);
    assert!(node_in(&nodes, Role::Button, "Close find bar").enabled);
    assert_eq!(
        node_in(&nodes, Role::Button, "Match case").checked,
        Some(false)
    );
    // The five controls, and no glyph: ‹ › Aa × are hidden.
    assert_eq!(
        nodes
            .iter()
            .filter(|n| n.role == Role::Button || n.role == Role::TextInput)
            .count(),
        5
    );
    assert!(
        !nodes
            .iter()
            .any(|n| matches!(n.label.as_str(), "‹" | "›" | "Aa" | "×"))
    );
}

#[test]
fn the_find_bar_follows_the_search() {
    let window = find_window();
    window.set_find_query("slint".into());
    window.set_find_status("3 of 7".into());
    window.set_find_match_case(true);
    let nodes = nodes_of(&window, Role::Search, "Find bar");
    check("bars-find-bar-matches", &window, &nodes, &[]);
    assert_eq!(
        node_in(&nodes, Role::TextInput, "Find in page").value,
        "slint"
    );
    node_in(&nodes, Role::Text, "3 of 7");
    assert!(node_in(&nodes, Role::Button, "Previous match").enabled);
    assert!(node_in(&nodes, Role::Button, "Next match").enabled);
    assert_eq!(
        node_in(&nodes, Role::Button, "Match case").checked,
        Some(true)
    );

    window.set_find_query("zzz".into());
    window.set_find_status("No matches".into());
    window.set_find_no_matches(true);
    window.set_find_match_case(false);
    let nodes = nodes_of(&window, Role::Search, "Find bar");
    check("bars-find-bar-no-matches", &window, &nodes, &[]);
    node_in(&nodes, Role::Text, "No matches");
}

#[test]
fn the_find_bar_buttons_do_what_a_click_does() {
    let window = find_window();
    let log = Log::default();
    record!(window, log, on_find_next, || "next");
    record!(window, log, on_find_previous, || "previous");
    record!(window, log, on_find_close, || "close");
    record!(window, log, on_find_toggle_match_case, || "match-case");
    let nodes = nodes_of(&window, Role::Search, "Find bar");
    // Nothing typed: Previous and Next do nothing.
    press(&nodes, Role::Button, "Previous match");
    press(&nodes, Role::Button, "Next match");
    assert_eq!(take(&log), Vec::<String>::new());

    window.set_find_query("slint".into());
    let nodes = nodes_of(&window, Role::Search, "Find bar");
    press(&nodes, Role::Button, "Previous match");
    press(&nodes, Role::Button, "Next match");
    press(&nodes, Role::Button, "Match case");
    press(&nodes, Role::Button, "Close find bar");
    assert_eq!(take(&log), ["previous", "next", "match-case", "close"]);

    // Typing through the field's accessible value reaches `find-edited`.
    let edited = Log::default();
    record!(window, edited, on_find_edited, |text| format!(
        "edited {text}"
    ));
    control_in(&nodes, Role::TextInput, "Find in page").set_accessible_value("rust");
    assert_eq!(take(&edited), ["edited rust"]);
}

#[test]
fn the_find_bar_buttons_are_where_they_are_drawn() {
    // The buttons dim with an opacity: the bounds must not move with it. 30 x 28 px (the match
    // case button 34 px wide), 6 px apart.
    let window = find_window();
    for query in ["", "slint"] {
        window.set_find_query(query.into());
        let nodes = nodes_of(&window, Role::Search, "Find bar");
        let rect = |label: &str| node_in(&nodes, Role::Button, label).rect();
        let (previous, next, case, close) = (
            rect("Previous match"),
            rect("Next match"),
            rect("Match case"),
            rect("Close find bar"),
        );
        assert_eq!((previous.width, previous.height), (30.0, 28.0));
        assert_eq!((next.width, next.height), (30.0, 28.0));
        assert_eq!((case.width, case.height), (34.0, 28.0));
        assert_eq!((close.width, close.height), (30.0, 28.0));
        assert_eq!(previous.right() + 6.0, next.x);
        assert_eq!(next.right() + 6.0, case.x);
        assert_eq!(close.right(), nodes[0].rect().right() - 14.0);
        assert_eq!(previous.y, next.y);
        assert_eq!(previous.y, close.y);
    }
}

// ---------------------------------------------------------------------------------------------
// The session bar

fn session_window(sessions: &[&str], notice: &str) -> MainWindow {
    let window = window();
    window.set_recovery_sessions(model(
        sessions
            .iter()
            .map(|name| SharedString::from(*name))
            .collect(),
    ));
    window.set_session_notice(notice.into());
    window
}

const RESTORE_OFFER: &str = "Mercury Browser didn't close properly. Restore your tabs?";

#[test]
fn the_session_bar_offers_to_restore_the_tabs() {
    let window = session_window(&["Today, 10:42 (5 tabs)"], "");
    let nodes = nodes_of(&window, Role::Groupbox, RESTORE_OFFER);
    check("bars-session-bar-one", &window, &nodes, &[]);
    assert_inside_first(&nodes);
    node_in(&nodes, Role::Button, "Restore");
    // "Dismiss" alone is the label of the notices' button, which tests find by it.
    node_in(&nodes, Role::Button, "Dismiss restore offer");
    assert!(nodes.iter().all(|n| n.label != "Dismiss"));
    assert!(nodes.iter().all(|n| n.role != Role::Text));

    let window = session_window(&["Today, 10:42 (5 tabs)", "Yesterday, 18:03 (2 tabs)"], "");
    let nodes = nodes_of(&window, Role::Groupbox, RESTORE_OFFER);
    check("bars-session-bar-several", &window, &nodes, &[]);
    node_in(&nodes, Role::Button, "Choose a session");
}

#[test]
fn the_session_bar_can_show_a_notice() {
    let window = session_window(&[], "Your tabs from last time were restored.");
    let nodes = nodes_of(
        &window,
        Role::Groupbox,
        "Your tabs from last time were restored.",
    );
    check("bars-session-bar-notice", &window, &nodes, &[]);
    node_in(&nodes, Role::Button, "OK");
}

#[test]
fn the_session_bar_buttons_do_what_a_click_does() {
    let window = session_window(&["Today, 10:42 (5 tabs)", "Yesterday, 18:03 (2 tabs)"], "");
    let log = Log::default();
    record!(window, log, on_restore_recovered_session, |index| format!(
        "restore {index}"
    ));
    record!(window, log, on_dismiss_recovered_sessions, || "dismiss");
    let nodes = nodes_of(&window, Role::Groupbox, RESTORE_OFFER);
    press(&nodes, Role::Button, "Restore");
    press(&nodes, Role::Button, "Dismiss restore offer");
    assert_eq!(take(&log), ["restore 0", "dismiss"]);

    // "Choose a session" opens the chooser, a list named by its heading; a row restores it.
    press(&nodes, Role::Button, "Choose a session");
    let chooser = nodes_of(&window, Role::List, "Restore which session?");
    check("bars-session-chooser", &window, &chooser, &[]);
    assert_inside_first(&chooser);
    assert_eq!(
        chooser[1..]
            .iter()
            .map(|n| (n.label.as_str(), n.item))
            .collect::<Vec<_>>(),
        [
            ("Today, 10:42 (5 tabs)", Some((0, 2))),
            ("Yesterday, 18:03 (2 tabs)", Some((1, 2))),
        ]
    );
    press(&chooser, Role::ListItem, "Yesterday, 18:03 (2 tabs)");
    assert_eq!(take(&log), ["restore 1"]);
    assert!(
        reading_order(&window)
            .iter()
            .all(|n| n.label != "Restore which session?"),
        "choosing closes the chooser"
    );

    let notice = session_window(&[], "Your tabs from last time were restored.");
    let log = Log::default();
    record!(notice, log, on_dismiss_session_notice, || "ok");
    press(
        &nodes_of(
            &notice,
            Role::Groupbox,
            "Your tabs from last time were restored.",
        ),
        Role::Button,
        "OK",
    );
    assert_eq!(take(&log), ["ok"]);
}

// ---------------------------------------------------------------------------------------------
// The browser menu

fn menu_entry(
    title: &str,
    shortcut: &str,
    enabled: bool,
    command: BrowserMenuCommand,
    separator_before: bool,
) -> BrowserMenuEntry {
    BrowserMenuEntry {
        title: title.into(),
        shortcut: shortcut.into(),
        enabled,
        command,
        separator_before,
    }
}

fn menu_window() -> MainWindow {
    let window = window();
    window.set_browser_menu_top_entries(model(vec![
        menu_entry("New tab", "⌘T", true, BrowserMenuCommand::NewTab, false),
        menu_entry("Find in page…", "⌘F", true, BrowserMenuCommand::Find, false),
        menu_entry(
            "Close other tabs",
            "",
            false,
            BrowserMenuCommand::CloseOtherTabs,
            true,
        ),
    ]));
    window.set_browser_menu_bottom_entries(model(vec![
        menu_entry("Settings…", "⌘,", true, BrowserMenuCommand::Settings, false),
        menu_entry(
            "About Mercury Browser",
            "",
            true,
            BrowserMenuCommand::About,
            false,
        ),
        menu_entry("Quit", "⌘Q", true, BrowserMenuCommand::Quit, true),
    ]));
    window.set_zoom_percent(125);
    window.invoke_show_browser_menu();
    window
}

#[test]
fn the_browser_menu_reads_as_a_list_of_commands() {
    let window = menu_window();
    let nodes = nodes_of(&window, Role::List, "Browser menu");
    check("bars-browser-menu", &window, &nodes, &[]);
    assert_inside_first(&nodes);

    // Rows keep their shortcut as the description; a disabled row says so.
    let new_tab = node_in(&nodes, Role::Button, "New tab");
    assert_eq!(new_tab.description, "⌘T");
    assert!(new_tab.enabled);
    assert!(!node_in(&nodes, Role::Button, "Close other tabs").enabled);
    // The zoom row: two buttons around the current size, which is the value of "Actual size".
    node_in(&nodes, Role::Button, "Zoom out");
    node_in(&nodes, Role::Button, "Zoom in");
    assert_eq!(node_in(&nodes, Role::Button, "Actual size").value, "125%");
    // No text is read besides the controls: the captions and shortcuts are hidden.
    assert!(
        nodes.iter().all(|n| n.role != Role::Text),
        "{}",
        dump(&nodes)
    );

    // The popup is 264 px wide. Its rows are 30 px high; the zoom row 34 px; the separators 9 px.
    let list = nodes[0].rect();
    assert_eq!(list.width, 264.0);
    assert_eq!(list.x, 0.0);
    for row in nodes[1..]
        .iter()
        .filter(|n| n.label != "Zoom out" && n.label != "Zoom in" && n.label != "Actual size")
    {
        assert_eq!(
            (row.rect().x, row.rect().width),
            (6.0, 264.0 - 12.0),
            "{}",
            dump(&nodes)
        );
        assert_eq!(row.rect().height, 30.0);
    }
}

#[test]
fn the_browser_menu_rows_do_what_a_click_does() {
    let window = menu_window();
    let log = Log::default();
    record!(window, log, on_browser_menu_activated, |command| format!(
        "{command:?}"
    ));
    let nodes = nodes_of(&window, Role::List, "Browser menu");
    press(&nodes, Role::Button, "Zoom in");
    press(&nodes, Role::Button, "Zoom out");
    press(&nodes, Role::Button, "Actual size");
    // The zoom buttons keep the menu open.
    let nodes = nodes_of(&window, Role::List, "Browser menu");
    press(&nodes, Role::Button, "Find in page…");
    // A disabled row does nothing.
    press(&nodes, Role::Button, "Close other tabs");
    assert_eq!(
        take(&log),
        [
            format!("{:?}", BrowserMenuCommand::ZoomIn),
            format!("{:?}", BrowserMenuCommand::ZoomOut),
            format!("{:?}", BrowserMenuCommand::ZoomReset),
            format!("{:?}", BrowserMenuCommand::Find),
        ]
    );
    // Choosing a row closes the menu.
    assert!(
        reading_order(&window)
            .iter()
            .all(|n| !(n.role == Role::List && n.label == "Browser menu"))
    );
}

// ---------------------------------------------------------------------------------------------
// The two notices under the toolbar, and the labels tests find them by

const SETTINGS_NOTICE: &str = "The settings file could not be read, so the defaults are used.";
const PASSWORD_NOTICE: &str = "Couldn't save the login for example.com.";

#[test]
fn the_settings_notice_is_a_group_with_a_dismiss_button() {
    let window = window();
    window.set_settings_notice(SETTINGS_NOTICE.into());
    let log = Log::default();
    record!(window, log, on_dismiss_settings_notice, || "dismiss");
    let nodes = nodes_of(&window, Role::Groupbox, SETTINGS_NOTICE);
    check("bars-settings-notice", &window, &nodes, &[]);
    assert_inside_first(&nodes);
    // The message is the group's label, once; the button has exactly this label.
    assert_eq!(nodes.len(), 2, "{}", dump(&nodes));
    press(&nodes, Role::Button, "Dismiss");
    assert_eq!(take(&log), ["dismiss"]);
}

#[test]
fn the_password_save_notice_is_a_group_with_retry_and_dismiss() {
    let window = window();
    window.set_password_save_problem(PASSWORD_NOTICE.into());
    let log = Log::default();
    record!(window, log, on_retry_password_save, || "retry");
    record!(window, log, on_dismiss_password_save_problem, || "dismiss");
    let nodes = nodes_of(&window, Role::Groupbox, PASSWORD_NOTICE);
    check("bars-password-save-notice", &window, &nodes, &[]);
    assert_inside_first(&nodes);
    assert_eq!(nodes.len(), 3, "{}", dump(&nodes));
    press(&nodes, Role::Button, "Retry");
    press(&nodes, Role::Button, "Dismiss");
    assert_eq!(take(&log), ["retry", "dismiss"]);
}

/// How many controls of the window, other than static text, have this label: what the desktop
/// tests count (`find_by_accessible_label`, less the texts).
fn controls_labelled(window: &MainWindow, label: &str) -> usize {
    i_slint_backend_testing::ElementHandle::find_by_accessible_label(window, label)
        .filter(|element| element.accessible_role() != Some(Role::Text))
        .count()
}

#[test]
fn only_the_notices_buttons_are_called_dismiss_and_retry() {
    // Every other bar and list on screen at once, with something to retry and dismiss in each.
    let window = window();
    window.set_is_new_tab(false);
    window.set_downloads_button_visible(true);
    window.set_downloads_all(model(downloads()));
    window.set_downloads_panel_open(true);
    window.set_recovery_sessions(model(vec![SharedString::from("Today, 10:42 (5 tabs)")]));
    window.set_find_open(true);
    window.set_download_offer_text("example.com wants to download a file.".into());
    window.set_download_offer_visible(true);
    window.set_history_rows(model(history_rows()));
    window.set_history_open(true);
    window.set_clear_dialog_open(true);
    window.set_tabs(model(strip_tabs()));
    window.set_pinned_tab_count(2);
    assert_eq!(controls_labelled(&window, "Retry"), 0);
    assert_eq!(controls_labelled(&window, "Dismiss"), 0);
    // All of it follows the labelling rule at once: no two controls of the bars share a label.
    assert_labelled_except(
        &reading_order(&window),
        "every bar and list at once",
        &[
            exempt(
                Role::Button,
                "Open in new tab",
                "each visit has this button; the description names the page",
            ),
            exempt(
                Role::Button,
                "Delete",
                "each visit has this button; the description names the page",
            ),
            exempt(
                Role::Button,
                "Close tab",
                "one in each tab; its description names the tab",
            ),
        ],
    );

    // The settings notice: one "Dismiss" and no "Retry".
    window.set_settings_notice(SETTINGS_NOTICE.into());
    assert_eq!(controls_labelled(&window, "Dismiss"), 1);
    assert_eq!(controls_labelled(&window, "Retry"), 0);
    window.set_settings_notice("".into());

    // The password notice: one of each.
    window.set_password_save_problem(PASSWORD_NOTICE.into());
    assert_eq!(controls_labelled(&window, "Dismiss"), 1);
    assert_eq!(controls_labelled(&window, "Retry"), 1);
    window.set_password_save_problem("".into());
    assert_eq!(controls_labelled(&window, "Dismiss"), 0);
    assert_eq!(controls_labelled(&window, "Retry"), 0);

    // And a window with the notice alone, as the desktop tests have it.
    let alone = self::window();
    alone.set_password_save_problem(PASSWORD_NOTICE.into());
    assert_eq!(controls_labelled(&alone, "Dismiss"), 1);
    assert_eq!(controls_labelled(&alone, "Retry"), 1);
}

// ---------------------------------------------------------------------------------------------
// Where the rows of every list with a shadow or an opacity are (risk 8 of the lane brief)

#[test]
fn the_rows_of_the_lists_are_where_they_are_drawn() {
    // Components that set a drop shadow or an opacity are wrapped by Slint; if a wrapper doubled
    // an offset, VoiceOver's cursor and hit test would be off by it. These are the drawn
    // measurements of the .slint files. Popups are measured from their own origin (the test
    // backend gives a popup's content positions relative to the popup).
    let approx = |actual: f32, expected: f32, what: &str| {
        assert!(
            (actual - expected).abs() <= 0.01,
            "{what}: {actual} is not {expected}"
        );
    };

    // The history window's visits: 44 px rows across the list; their buttons 28 px high, 4 px
    // apart, 8 px from the right edge.
    let window = history_window();
    let nodes = nodes_of(&window, Role::Region, "History");
    let list = node_in(&nodes, Role::List, "Visited pages").rect();
    let visits: Vec<&Node> = nodes.iter().filter(|n| n.role == Role::ListItem).collect();
    for visit in &visits {
        let row = visit.rect();
        approx(row.height, 44.0, "a visit's height");
        approx(row.x, list.x, "a visit's x");
        approx(row.width, list.width, "a visit's width");
    }
    let first = visits[0].rect();
    let buttons: Vec<Rect> = nodes
        .iter()
        .filter(|n| n.role == Role::Button && n.description == "Example Domain")
        .map(|n| n.rect())
        .collect();
    assert_eq!(buttons.len(), 3);
    for button in &buttons {
        approx(button.height, 28.0, "a row button's height");
        assert!(first.contains(button), "{button:?} is not in {first:?}");
    }
    approx(buttons[2].right(), first.right() - 8.0, "the delete button");
    approx(
        buttons[1].right() + 4.0,
        buttons[2].x,
        "the gap between buttons",
    );
    approx(
        buttons[0].right() + 4.0,
        buttons[1].x,
        "the gap between buttons",
    );

    // The tab menu: rows 188 px wide and 30 px high, 6 px from the edges; the separator before
    // "Close tab" is 9 px high.
    let window = strip_window(strip_tabs(), 2);
    let nodes = open_tab_menu(&window, "Slint documentation");
    let rows: Vec<Rect> = nodes[1..].iter().map(|n| n.rect()).collect();
    assert_eq!(nodes[0].rect().width, 200.0);
    for row in &rows {
        approx(row.x, 6.0, "a tab menu row's x");
        approx(row.width, 188.0, "a tab menu row's width");
        approx(row.height, 30.0, "a tab menu row's height");
    }
    let ys: Vec<f32> = rows.iter().map(|r| r.y).collect();
    assert_eq!(ys, [6.0, 36.0, 75.0, 105.0]);

    // The bookmarks menu: 28 px rows, 1 px inside the 300 px popup, the first 6 px down.
    let window = bookmarks_window(false);
    window.set_bookmarks_menu_rows(model(menu_rows()));
    window.invoke_show_bookmarks_menu(100.0);
    let nodes = nodes_of(&window, Role::List, "Bookmarks menu");
    let ys: Vec<f32> = nodes[1..].iter().map(|n| n.rect().y).collect();
    assert_eq!(ys, [6.0, 34.0, 62.0, 118.0, 146.0]);
    for row in &nodes[1..] {
        approx(row.rect().x, 1.0, "a bookmarks menu row's x");
        approx(row.rect().width, 298.0, "a bookmarks menu row's width");
        approx(row.rect().height, 28.0, "a bookmarks menu row's height");
    }

    // Tab search: 44 px rows, 8 px from the edges, 46 px apart, the first under the 34 px field.
    let window = window_with_search();
    let nodes = nodes_of(&window, Role::Search, "Search tabs");
    let rows: Vec<Rect> = nodes
        .iter()
        .filter(|n| n.role == Role::ListItem)
        .map(|n| n.rect())
        .collect();
    assert_eq!(rows.len(), 4);
    for (index, row) in rows.iter().enumerate() {
        approx(row.x, 8.0, "a tab search row's x");
        approx(row.width, 404.0, "a tab search row's width");
        approx(row.height, 44.0, "a tab search row's height");
        approx(row.y, 48.0 + 46.0 * index as f32, "a tab search row's y");
    }

    // The session chooser: 32 px rows, 6 px from the edges of the 340 px popup, 2 px apart.
    let window = session_window(&["Today, 10:42 (5 tabs)", "Yesterday, 18:03 (2 tabs)"], "");
    press(
        &nodes_of(&window, Role::Groupbox, RESTORE_OFFER),
        Role::Button,
        "Choose a session",
    );
    let nodes = nodes_of(&window, Role::List, "Restore which session?");
    assert_eq!(nodes[0].rect().width, 340.0);
    for (index, row) in nodes[1..].iter().enumerate() {
        approx(row.rect().x, 6.0, "a chooser row's x");
        approx(row.rect().width, 328.0, "a chooser row's width");
        approx(row.rect().height, 32.0, "a chooser row's height");
        approx(
            row.rect().y,
            34.0 + 34.0 * index as f32,
            "a chooser row's y",
        );
    }

    // The downloads popover: 64 px rows stacked under its 36 px heading, 8 px from its edges.
    let window = downloads_window();
    window.set_downloads_recent(model(downloads()));
    window.set_downloads_popover_open(true);
    let nodes = nodes_of(&window, Role::List, "Downloads");
    let popover = nodes[0].rect();
    for (index, row) in nodes
        .iter()
        .filter(|n| n.role == Role::ListItem)
        .map(|n| n.rect())
        .enumerate()
    {
        approx(row.x, popover.x + 8.0, "a download's x");
        approx(row.width, 444.0, "a download's width");
        approx(row.height, 64.0, "a download's height");
        approx(
            row.y,
            popover.y + 8.0 + 36.0 + 64.0 * index as f32,
            "a download's y",
        );
    }
}

fn window_with_search() -> MainWindow {
    let window = window();
    window.set_tab_search_rows(model(search_rows()));
    window.invoke_show_tab_search();
    window
}
