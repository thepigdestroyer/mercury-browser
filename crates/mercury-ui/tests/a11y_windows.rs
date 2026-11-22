// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
#![cfg(feature = "test-support")]
//! The separate windows read the way an assistive technology reads them: the About window, the
//! import window, the settings window (with the filter-lists group and the Passwords section),
//! the bookmarks manager and the bookmarks preview. For each window and each distinct state the
//! reading order is checked against the labelling rule and the bounds VoiceOver is given, and
//! frozen in `tests/a11y-snapshots/windows-*.txt`; the controls are found by role and label and
//! pressed through their default action, which must do what a click does and nothing while the
//! control is disabled. The labels the desktop crate's tests look up are asserted to be unchanged.
//!
//! `cargo test --release -p mercury-ui --features test-support --test a11y_windows`; bless the
//! golden files with `MERCURY_A11Y_BLESS=1` in front of it.

mod common;

use std::cell::RefCell;
use std::rc::Rc;

use common::a11y::{self, Exempt, Node};
use i_slint_backend_testing::{AccessibleRole, ElementHandle, ElementRoot};
use mercury_ui::{
    AboutLink, AboutWindow, BookmarkBarItem, BookmarkMenuRow, BookmarkRowKind, BookmarksManager,
    BookmarksPreview, ImportPage, ImportProfileRow, ImportSummaryLine, ImportWindow, LoginListRow,
    PasswordManagerUi, SearchEngineRow, SettingsWindow,
};
use slint::platform::{PointerEventButton, WindowEvent};
use slint::{ComponentHandle, ModelRc, SharedString, VecModel};

use AccessibleRole as Role;

/// What the callbacks of a window were called with, in order.
type Log = Rc<RefCell<Vec<String>>>;

fn log() -> Log {
    Rc::new(RefCell::new(Vec::new()))
}

fn entries(log: &Log) -> Vec<String> {
    log.borrow().clone()
}

fn strings(values: &[&str]) -> ModelRc<SharedString> {
    ModelRc::new(VecModel::from(
        values
            .iter()
            .map(|value| SharedString::from(*value))
            .collect::<Vec<_>>(),
    ))
}

/// Reads the window the way VoiceOver does, checks the labelling rule and the bounds, and
/// compares the reading order with `tests/a11y-snapshots/<snapshot>.txt`.
fn freeze<W: ComponentHandle + ElementRoot>(window: &W, snapshot: &str) -> Vec<Node> {
    freeze_except(window, snapshot, &[])
}

fn freeze_except<W: ComponentHandle + ElementRoot>(
    window: &W,
    snapshot: &str,
    exemptions: &[Exempt],
) -> Vec<Node> {
    let nodes = a11y::reading_order(window);
    a11y::assert_labelled_except(&nodes, snapshot, exemptions);
    a11y::assert_bounds(&nodes, a11y::window_size(window));
    a11y::assert_snapshot(snapshot, &nodes);
    nodes
}

fn press(window: &impl ElementRoot, role: Role, label: &str) {
    a11y::control(window, role, label).invoke_accessible_default_action();
}

fn names(nodes: &[Node], role: Role) -> Vec<String> {
    nodes
        .iter()
        .filter(|node| node.role == role)
        .map(|node| node.label.clone())
        .collect()
}

fn find<'a>(nodes: &'a [Node], role: Role, label: &str) -> &'a Node {
    a11y::node_in(nodes, role, label)
}

fn absent(nodes: &[Node], role: Role, label: &str) -> bool {
    !nodes
        .iter()
        .any(|node| node.role == role && node.label == label)
}

// ---------------------------------------------------------------------------------------------
// The About window
// ---------------------------------------------------------------------------------------------

fn about_window(media_line: &str) -> AboutWindow {
    a11y::init();
    let window = AboutWindow::new().expect("a window");
    window.set_version_line("Version 0.1.0".into());
    window.set_engine_line("Servo 0.8.0 web engine".into());
    window.set_media_line(media_line.into());
    window.set_copyright_line(
        "Copyright © 2026 Mrinal Joshi and the Mercury Browser contributors. Portions © 2026 \
         Kayk Caputo (Hermes Browser)."
            .into(),
    );
    window.set_licence_line(
        "Source-available: free for non-commercial use (PolyForm Noncommercial 1.0.0; MPL-2.0 \
         for parts from Hermes and Servo)."
            .into(),
    );
    window.set_credit_line("Began as a fork of Hermes Browser by Kayk Caputo.".into());
    a11y::open_window(&window, 380.0, 420.0);
    // The window opens as tall as its content wants: 420 px where the system's font fits the
    // lines in that, more where it does not.
    a11y::open_window(&window, 380.0, window.get_wanted_height());
    window
}

/// The attribution Slint's licence requires is an image the std widget labels itself; it is a
/// meaningful image, read as such, so the rule that no image is exposed lets it through.
const SLINT_ATTRIBUTION: &[Exempt] = &[Exempt {
    role: Role::Image,
    label: "#MadeWithSlint",
    reason: "the AboutSlint widget (Slint Royalty-free Licence 2.0 section 2(a)) labels its own badge",
}];

#[test]
fn the_about_window_reads_its_lines_then_its_three_links_then_the_attribution() {
    let window = about_window("GStreamer 1.26.1 media framework");

    let nodes = freeze_except(&window, "windows-about", SLINT_ATTRIBUTION);

    // The decorative logo is hidden; the attribution that must stay is there, with the words a
    // sighted user sees under it.
    assert_eq!(names(&nodes, Role::Image), ["#MadeWithSlint"]);
    assert!(
        nodes
            .iter()
            .any(|node| node.role == Role::Text && node.label.contains("https://slint.dev/")),
        "the AboutSlint attribution text is read"
    );
    let buttons = names(&nodes, Role::Button);
    assert_eq!(buttons, ["Licence", "Third-party notices", "Source code"]);
    // Each link says what pressing it does, which its short caption does not.
    for (label, description) in [
        ("Licence", "Opens the licence terms in a new tab."),
        (
            "Third-party notices",
            "Opens the list of third-party notices in a new tab.",
        ),
        (
            "Source code",
            "Opens the source code of this version in a new tab.",
        ),
    ] {
        let link = find(&nodes, Role::Button, label);
        assert_eq!(link.description, description, "{label}");
        assert!(link.enabled, "{label}");
    }
}

#[test]
fn the_about_window_names_gstreamer_only_when_the_build_recorded_it() {
    let window = about_window("");

    let nodes = a11y::reading_order(&window);

    assert!(
        nodes.iter().all(|node| !node.label.contains("GStreamer")),
        "no media line without a version"
    );
    a11y::assert_labelled_except(
        &nodes,
        "the About window without a media line",
        SLINT_ATTRIBUTION,
    );
}

#[test]
fn pressing_a_link_of_the_about_window_opens_what_a_click_opens() {
    let window = about_window("");
    let opened = Rc::new(RefCell::new(Vec::new()));
    window.on_open_link({
        let opened = Rc::clone(&opened);
        move |link| opened.borrow_mut().push(link)
    });

    press(&window, Role::Button, "Licence");
    press(&window, Role::Button, "Third-party notices");
    press(&window, Role::Button, "Source code");

    assert_eq!(
        *opened.borrow(),
        [
            AboutLink::Licence,
            AboutLink::ThirdPartyNotices,
            AboutLink::SourceCode
        ]
    );
}

#[test]
fn the_about_links_have_the_text_rectangles_the_screen_reader_draws_its_cursor_in() {
    let window = about_window("");
    let nodes = a11y::reading_order(&window);

    for label in ["Licence", "Third-party notices", "Source code"] {
        let link = find(&nodes, Role::Button, label);
        let rect = link.rect();
        assert!(rect.width > 20.0 && rect.height > 8.0, "{label}: {rect:?}");
    }
    // Side by side, "Licence" first, then the notices, then the source.
    let licence = find(&nodes, Role::Button, "Licence").rect();
    let notices = find(&nodes, Role::Button, "Third-party notices").rect();
    let source = find(&nodes, Role::Button, "Source code").rect();
    assert!(licence.right() < notices.x);
    assert!(notices.right() < source.x);
    assert!((licence.y - notices.y).abs() < 4.0);
    assert!((notices.y - source.y).abs() < 4.0);
}

// ---------------------------------------------------------------------------------------------
// The import window
// ---------------------------------------------------------------------------------------------

fn profile(
    browser: &str,
    name: &str,
    holds: &str,
    bookmarks: bool,
    history: bool,
) -> ImportProfileRow {
    ImportProfileRow {
        browser: browser.into(),
        name: name.into(),
        holds: holds.into(),
        bookmarks,
        history,
    }
}

fn summary_line(text: &str, problem: bool) -> ImportSummaryLine {
    ImportSummaryLine {
        text: text.into(),
        problem,
    }
}

fn import_window(page: ImportPage) -> ImportWindow {
    a11y::init();
    let window = ImportWindow::new().expect("a window");
    window.set_profiles(ModelRc::new(VecModel::from(vec![
        profile(
            "Firefox",
            "default-release",
            "Bookmarks and history",
            true,
            true,
        ),
        profile("Chrome", "Work", "Bookmarks and history", true, true),
        profile("Chrome", "Reading", "Bookmarks", true, false),
        profile("Arc", "Default", "History", false, true),
    ])));
    window.set_selected(1);
    window.set_import_bookmarks(true);
    window.set_import_history(true);
    window.set_page(page);
    a11y::open_window(&window, 560.0, 560.0);
    window
}

fn import_log(window: &ImportWindow) -> Log {
    let log = log();
    window.on_start_import({
        let log = Rc::clone(&log);
        move || log.borrow_mut().push("start-import".into())
    });
    window.on_cancel_import({
        let log = Rc::clone(&log);
        move || log.borrow_mut().push("cancel-import".into())
    });
    window.on_continue_import({
        let log = Rc::clone(&log);
        move || log.borrow_mut().push("continue-import".into())
    });
    window.on_import_from_file({
        let log = Rc::clone(&log);
        move || log.borrow_mut().push("import-from-file".into())
    });
    window.on_close_window({
        let log = Rc::clone(&log);
        move || log.borrow_mut().push("close-window".into())
    });
    log
}

#[test]
fn the_import_window_says_it_is_looking_and_shows_an_indeterminate_bar() {
    let window = import_window(ImportPage::Looking);

    let nodes = freeze(&window, "windows-import-looking");

    let bar = find(&nodes, Role::ProgressIndicator, "Search progress");
    assert_eq!(
        bar.value, "",
        "no value while the search has no end in sight"
    );
}

#[test]
fn the_import_window_lists_the_profiles_as_rows_with_the_selection() {
    let window = import_window(ImportPage::Choose);

    let nodes = freeze(&window, "windows-import-choose");

    let list = find(&nodes, Role::List, "Browser profiles");
    let rows: Vec<&Node> = nodes.iter().filter(|n| n.role == Role::ListItem).collect();
    assert_eq!(rows.len(), 4);
    for (index, row) in rows.iter().enumerate() {
        assert_eq!(row.item, Some((index, 4)), "{}", row.label);
        assert_eq!(row.selected, Some(index == 1), "{}", row.label);
        a11y::assert_inside(row, list);
    }
    assert_eq!(
        rows[2].label, "Chrome, Reading, Bookmarks",
        "the row says the browser, the profile and what it holds, once"
    );
}

#[test]
fn the_import_window_with_an_unfinished_import_offers_to_continue_it() {
    let window = import_window(ImportPage::Choose);
    window
        .set_unfinished_notice("The import from Firefox — default-release did not finish.".into());
    window.set_selected(2);
    window.set_import_history(false);

    let nodes = freeze(&window, "windows-import-choose-unfinished");

    let history = find(&nodes, Role::Checkbox, "History");
    assert!(!history.enabled, "this profile holds no history");
    assert_eq!(
        history.description,
        "This profile has no history to import."
    );
    assert_eq!(history.checked, Some(false));
    let bookmarks = find(&nodes, Role::Checkbox, "Bookmarks");
    assert!(bookmarks.enabled);
    assert_eq!(bookmarks.checked, Some(true));
}

#[test]
fn pressing_a_profile_row_selects_it_and_follows_what_it_holds() {
    let window = import_window(ImportPage::Choose);

    press(&window, Role::ListItem, "Arc, Default, History");

    assert_eq!(window.get_selected(), 3);
    assert!(
        !window.get_import_bookmarks(),
        "an Arc profile here has no bookmarks"
    );
    assert!(window.get_import_history());
    let nodes = a11y::reading_order(&window);
    let selected: Vec<String> = nodes
        .iter()
        .filter(|node| node.role == Role::ListItem && node.selected == Some(true))
        .map(|node| node.label.clone())
        .collect();
    assert_eq!(
        selected,
        ["Arc, Default, History"],
        "exactly one row is selected"
    );
    assert!(!find(&nodes, Role::Checkbox, "Bookmarks").enabled);
}

#[test]
fn the_check_boxes_toggle_through_their_default_action_unless_disabled() {
    let window = import_window(ImportPage::Choose);

    press(&window, Role::Checkbox, "History");
    assert!(!window.get_import_history());
    press(&window, Role::Checkbox, "History");
    assert!(window.get_import_history());

    // A profile without bookmarks: the disabled box does not move.
    press(&window, Role::ListItem, "Arc, Default, History");
    assert!(!window.get_import_bookmarks());
    press(&window, Role::Checkbox, "Bookmarks");
    assert!(!window.get_import_bookmarks());
}

#[test]
fn the_import_and_cancel_buttons_do_what_a_click_does_and_import_waits_for_a_choice() {
    let window = import_window(ImportPage::Choose);
    let log = import_log(&window);

    press(&window, Role::Button, "Import");
    press(&window, Role::Button, "Cancel");
    assert_eq!(entries(&log), ["start-import", "close-window"]);

    // Nothing ticked: Import is disabled, says what to do, and does nothing.
    window.set_import_bookmarks(false);
    window.set_import_history(false);
    let nodes = a11y::reading_order(&window);
    let import = find(&nodes, Role::Button, "Import");
    assert!(!import.enabled);
    assert_eq!(import.description, "Tick Bookmarks or History first.");
    press(&window, Role::Button, "Import");
    assert_eq!(entries(&log), ["start-import", "close-window"]);
}

#[test]
fn the_unfinished_import_can_be_continued_from_the_keyboard_less_path() {
    let window = import_window(ImportPage::Choose);
    window
        .set_unfinished_notice("The import from Firefox — default-release did not finish.".into());
    let log = import_log(&window);

    press(&window, Role::Button, "Continue the import");

    assert_eq!(entries(&log), ["continue-import"]);
}

#[test]
fn the_import_window_with_no_browser_offers_a_file_and_closing() {
    let window = import_window(ImportPage::NothingFound);
    window.set_can_import_from_file(true);

    let nodes = freeze(&window, "windows-import-nothing-found");

    assert!(find(&nodes, Role::Button, "Import bookmarks from a file…").enabled);
    let log = import_log(&window);
    press(&window, Role::Button, "Import bookmarks from a file…");
    press(&window, Role::Button, "Close");
    assert_eq!(entries(&log), ["import-from-file", "close-window"]);
}

#[test]
fn the_import_from_a_file_button_does_nothing_while_it_is_disabled() {
    let window = import_window(ImportPage::NothingFound);
    window.set_can_import_from_file(false);
    let log = import_log(&window);

    let nodes = a11y::reading_order(&window);
    assert!(!find(&nodes, Role::Button, "Import bookmarks from a file…").enabled);
    press(&window, Role::Button, "Import bookmarks from a file…");

    assert!(entries(&log).is_empty());
}

#[test]
fn the_import_window_reports_progress_as_a_bar_with_a_value() {
    let window = import_window(ImportPage::Importing);
    window.set_progress_title("Importing history from Firefox — default-release…".into());
    window.set_progress_detail("12,000 of 48,000 visits".into());
    window.set_progress_known(true);
    window.set_progress(0.25);

    let nodes = freeze(&window, "windows-import-importing");

    let bar = find(&nodes, Role::ProgressIndicator, "Import progress");
    assert_eq!(bar.value, "0.25");
    assert_eq!(bar.handle.accessible_value_minimum(), Some(0.0));
    assert_eq!(bar.handle.accessible_value_maximum(), Some(1.0));
    let log = import_log(&window);
    press(&window, Role::Button, "Cancel");
    assert_eq!(entries(&log), ["cancel-import"]);
}

#[test]
fn the_import_window_while_stopping_has_no_value_and_a_disabled_cancel_that_does_nothing() {
    let window = import_window(ImportPage::Importing);
    window.set_progress_title("Importing bookmarks from Chrome — Work…".into());
    window.set_progress_known(false);
    window.set_cancelling(true);
    let log = import_log(&window);

    let nodes = freeze(&window, "windows-import-stopping");

    assert_eq!(
        find(&nodes, Role::ProgressIndicator, "Import progress").value,
        ""
    );
    assert!(!find(&nodes, Role::Button, "Cancel").enabled);
    press(&window, Role::Button, "Cancel");
    assert!(entries(&log).is_empty());
}

#[test]
fn the_import_summary_reads_each_line_and_says_which_are_problems() {
    let window = import_window(ImportPage::Summary);
    window.set_summary(ModelRc::new(VecModel::from(vec![
        summary_line(
            "Imported 1,204 bookmarks and 38,912 visits from Firefox — default-release.",
            false,
        ),
        summary_line(
            "12 bookmarks and 1,000 visits were already in Mercury Browser and were left out.",
            false,
        ),
        summary_line(
            "2 visits could not be kept (for example, an address that is too long or a date in the future).",
            true,
        ),
    ])));

    let nodes = freeze(&window, "windows-import-summary");

    let problems: Vec<&Node> = nodes
        .iter()
        .filter(|node| node.role == Role::Text && node.label.starts_with("Problem: "))
        .collect();
    assert_eq!(problems.len(), 1);
    assert!(
        nodes
            .iter()
            .all(|node| !node.label.contains('⚠') && !node.label.contains('✓')),
        "the drawn marks are not read"
    );
    let log = import_log(&window);
    press(&window, Role::Button, "Done");
    assert_eq!(entries(&log), ["close-window"]);
}

#[test]
fn a_stopped_import_summary_is_frozen_too() {
    let window = import_window(ImportPage::Summary);
    window.set_summary_title("Import stopped".into());
    window.set_summary(ModelRc::new(VecModel::from(vec![
        summary_line("Imported 500 visits from Chrome — Work.", false),
        summary_line(
            "Bookmarks could not be imported: Chrome kept writing to its files while they were being read. Quit Chrome and try again.",
            true,
        ),
        summary_line(
            "The import of history stopped before it finished: 500 of 38,912 had been imported.",
            true,
        ),
    ])));

    let nodes = freeze(&window, "windows-import-summary-stopped");

    assert_eq!(
        nodes
            .iter()
            .filter(|node| node.label.starts_with("Problem: "))
            .count(),
        2
    );
}

// ---------------------------------------------------------------------------------------------
// The settings window
// ---------------------------------------------------------------------------------------------

const SETTINGS_SIZE: (f32, f32) = (900.0, 2600.0);

fn engine(id: &str, name: &str, keyword: &str, custom: bool) -> SearchEngineRow {
    SearchEngineRow {
        id: id.into(),
        name: name.into(),
        keyword: keyword.into(),
        custom,
    }
}

fn login(site: &str, username: &str, kind: &str) -> LoginListRow {
    LoginListRow {
        site: site.into(),
        username: username.into(),
        kind: kind.into(),
    }
}

fn settings_window(section: i32) -> SettingsWindow {
    a11y::init();
    let window = SettingsWindow::new().expect("a window");
    window.set_startup_index(0);
    window.set_homepage("https://example.org/".into());
    window.set_engine_choices(strings(&["DuckDuckGo", "Bing", "Docs", "Rust docs"]));
    window.set_engine_index(0);
    window.set_engines(ModelRc::new(VecModel::from(vec![
        engine("duckduckgo", "DuckDuckGo", "@duckduckgo", false),
        engine("bing", "Bing", "@bing", false),
        engine("custom-docs", "Docs", "", true),
        engine("custom-rust-docs", "Rust docs", "@rs", true),
    ])));
    window.set_zoom_choices(strings(&["80%", "90%", "100%", "110%", "120%"]));
    window.set_zoom_index(2);
    window.set_theme_index(0);
    window.set_show_bookmarks_bar(true);
    window.set_block_ads_and_trackers(true);
    window.set_update_filter_lists(true);
    window.set_filter_lists_publishers(
        "Once a day, Mercury Browser asks easylist.to and raw.githubusercontent.com (GitHub) for newer versions of these lists."
            .into(),
    );
    window.set_filter_lists_status("Last updated 17 Nov 2026, 09:12.".into());
    window.set_filter_lists(strings(&[
        "EasyList: version 202611170805, from 17 Nov 2026, 09:12 (updated)",
        "EasyPrivacy: version 202611152034, from 15 Nov 2026, 21:46 (included with Mercury Browser)",
    ]));
    window.set_filter_lists_can_update(true);
    window.set_block_popups(true);
    window.set_clear_history_on_exit(true);
    window.set_clear_browsing_data_available(true);
    window.set_offer_to_save_passwords(true);
    window.set_never_save_origins(strings(&["https://bank.example", "http://localhost:8080"]));
    window.set_experimental_web_features(true);
    window.set_gpu_canvas(true);
    window.set_section(section);
    a11y::open_window(&window, SETTINGS_SIZE.0, SETTINGS_SIZE.1);
    window
}

/// The Passwords section with the logins of the desktop crate's tests plus an HTTP sign-in.
fn passwords_window() -> SettingsWindow {
    let window = settings_window(4);
    let ui = window.global::<PasswordManagerUi>();
    ui.set_storage_note("Saved in your login keychain.".into());
    ui.set_rows(ModelRc::new(VecModel::from(vec![
        login("a.test", "alice", ""),
        login("b.test", "bob", ""),
        login("http://127.0.0.1:8123", "carol", "HTTP sign-in"),
    ])));
    ui.set_total(3);
    window
}

fn settings_log(window: &SettingsWindow) -> Log {
    let log = log();
    window.on_toggled({
        let log = Rc::clone(&log);
        move |name, on| log.borrow_mut().push(format!("toggled {name} {on}"))
    });
    window.on_chosen({
        let log = Rc::clone(&log);
        move |name, index| log.borrow_mut().push(format!("chosen {name} {index}"))
    });
    window.on_commit_texts({
        let log = Rc::clone(&log);
        move || log.borrow_mut().push("commit-texts".into())
    });
    window.on_add_engine({
        let log = Rc::clone(&log);
        move || log.borrow_mut().push("add-engine".into())
    });
    window.on_edit_engine({
        let log = Rc::clone(&log);
        move |id| log.borrow_mut().push(format!("edit-engine {id}"))
    });
    window.on_remove_engine({
        let log = Rc::clone(&log);
        move |id| log.borrow_mut().push(format!("remove-engine {id}"))
    });
    window.on_save_engine({
        let log = Rc::clone(&log);
        move || log.borrow_mut().push("save-engine".into())
    });
    window.on_cancel_engine({
        let log = Rc::clone(&log);
        move || log.borrow_mut().push("cancel-engine".into())
    });
    window.on_remove_never_save({
        let log = Rc::clone(&log);
        move |index| log.borrow_mut().push(format!("remove-never-save {index}"))
    });
    window.on_clear_browsing_data({
        let log = Rc::clone(&log);
        move || log.borrow_mut().push("clear-browsing-data".into())
    });
    window.on_update_filter_lists_now({
        let log = Rc::clone(&log);
        move || log.borrow_mut().push("update-filter-lists-now".into())
    });
    log
}

const SECTIONS: [&str; 6] = [
    "General",
    "Appearance",
    "Privacy and security",
    "Downloads",
    "Passwords",
    "Advanced",
];

#[test]
fn the_settings_sidebar_is_a_tab_list_of_six_tabs_with_the_chosen_one_selected() {
    let window = settings_window(0);

    let nodes = freeze(&window, "windows-settings-general");

    let list = find(&nodes, Role::TabList, "Settings sections");
    let tabs: Vec<&Node> = nodes.iter().filter(|n| n.role == Role::Tab).collect();
    assert_eq!(
        tabs.iter().map(|t| t.label.as_str()).collect::<Vec<_>>(),
        SECTIONS
    );
    for (index, tab) in tabs.iter().enumerate() {
        assert_eq!(tab.item, Some((index, 6)), "{}", tab.label);
        assert_eq!(tab.selected, Some(index == 0), "{}", tab.label);
        a11y::assert_inside(tab, list);
    }
    // The sidebar's own title and each tab's caption are not read on top of the tab.
    assert_eq!(
        nodes
            .iter()
            .filter(|n| n.role == Role::Text && SECTIONS.contains(&n.label.as_str()))
            .count(),
        1,
        "the section's name is read once, as the heading, not again on the tab"
    );
}

#[test]
fn pressing_a_sidebar_tab_commits_the_texts_and_changes_the_section() {
    let window = settings_window(0);
    let log = settings_log(&window);

    for (index, name) in SECTIONS.iter().enumerate().skip(1) {
        press(&window, Role::Tab, name);
        assert_eq!(
            window.get_section(),
            i32::try_from(index).unwrap(),
            "{name}"
        );
        let nodes = a11y::reading_order(&window);
        let selected: Vec<&str> = nodes
            .iter()
            .filter(|n| n.role == Role::Tab && n.selected == Some(true))
            .map(|n| n.label.as_str())
            .collect();
        assert_eq!(selected, [*name]);
    }
    assert_eq!(
        entries(&log),
        vec!["commit-texts"; SECTIONS.len() - 1],
        "each change of section first commits the typed texts"
    );
}

#[test]
fn the_general_section_labels_every_field_and_reads_each_engine_once() {
    let window = settings_window(0);
    let nodes = a11y::reading_order(&window);

    for label in ["On start-up, open", "Default page zoom", "Search engine"] {
        let combobox = find(&nodes, Role::Combobox, label);
        assert_eq!(
            combobox.handle.accessible_expandable(),
            Some(true),
            "{label}"
        );
    }
    assert_eq!(
        find(&nodes, Role::Combobox, "Search engine").value,
        "DuckDuckGo"
    );
    let home = find(&nodes, Role::TextInput, "Home page");
    assert_eq!(home.value, "https://example.org/");
    assert_eq!(home.placeholder(), "https://example.com/ (empty for none)");
    // One text for each engine, with its keyword, and buttons only for the custom ones.
    for line in [
        "DuckDuckGo, keyword @duckduckgo",
        "Bing, keyword @bing",
        "Docs, no keyword",
        "Rust docs, keyword @rs",
    ] {
        find(&nodes, Role::Text, line);
    }
    assert_eq!(
        names(&nodes, Role::Button)
            .into_iter()
            .filter(|label| label.starts_with("Edit ") || label.starts_with("Remove "))
            .collect::<Vec<_>>(),
        [
            "Edit Docs",
            "Remove Docs",
            "Edit Rust docs",
            "Remove Rust docs"
        ]
    );
    let suggestions = find(
        &nodes,
        Role::Checkbox,
        "Show search suggestions while typing",
    );
    assert!(!suggestions.enabled);
}

#[test]
fn the_engine_buttons_name_their_engine_and_do_what_a_click_does() {
    let window = settings_window(0);
    let log = settings_log(&window);

    press(&window, Role::Button, "Edit Rust docs");
    press(&window, Role::Button, "Remove Docs");
    press(&window, Role::Button, "Add search engine…");

    assert_eq!(
        entries(&log),
        [
            "edit-engine custom-rust-docs",
            "remove-engine custom-docs",
            "add-engine"
        ]
    );
}

#[test]
fn adding_an_engine_does_nothing_at_the_limit_and_says_why() {
    let window = settings_window(0);
    window.set_can_add_engine(false);
    let log = settings_log(&window);

    let nodes = a11y::reading_order(&window);
    let add = find(&nodes, Role::Button, "Add search engine…");
    assert!(!add.enabled);
    assert_eq!(
        add.description,
        "The limit of custom search engines is reached."
    );
    press(&window, Role::Button, "Add search engine…");

    assert!(entries(&log).is_empty());
}

#[test]
fn the_search_engine_form_is_a_labelled_group_with_labelled_fields() {
    let window = settings_window(0);
    window.set_editor_open(true);
    window.set_editor_name("Broken engine".into());
    window.set_editor_url("https://search.example/find".into());
    window.set_editor_problem("The template has no {searchTerms}.".into());
    window.set_editor_warning(
        "This address starts with http: what you search for could be read on the way.".into(),
    );
    window.set_homepage_problem(
        "Enter an http, https, file or about address, without a user name or password.".into(),
    );

    let nodes = freeze(&window, "windows-settings-general-editor");

    let form = find(&nodes, Role::Groupbox, "Add a search engine");
    for label in ["Engine name", "Engine keyword", "Engine URL"] {
        a11y::assert_inside(find(&nodes, Role::TextInput, label), form);
    }
    assert_eq!(
        find(&nodes, Role::TextInput, "Engine URL").description,
        "Write {searchTerms} where the search text goes."
    );
    // While editing an existing engine the group says so.
    window.set_editor_id("custom-docs".into());
    let nodes = a11y::reading_order(&window);
    find(&nodes, Role::Groupbox, "Edit search engine");

    let log = settings_log(&window);
    press(&window, Role::Button, "Save");
    press(&window, Role::Button, "Cancel");
    assert_eq!(entries(&log), ["save-engine", "cancel-engine"]);
}

#[test]
fn the_appearance_section_is_frozen_and_its_boxes_toggle() {
    let window = settings_window(1);

    let nodes = freeze(&window, "windows-settings-appearance");

    assert_eq!(
        find(&nodes, Role::Combobox, "Theme").value,
        "Follow the system"
    );
    let log = settings_log(&window);
    press(&window, Role::Checkbox, "Show the home button");
    press(&window, Role::Checkbox, "Show the bookmarks bar");
    assert_eq!(
        entries(&log),
        [
            "toggled show-home-button true",
            "toggled show-bookmarks-bar false"
        ]
    );
}

#[test]
fn the_privacy_section_groups_the_filter_lists_and_labels_each_control() {
    let window = settings_window(2);

    let nodes = freeze(&window, "windows-settings-privacy");

    let group = find(&nodes, Role::Groupbox, "Filter lists");
    for (role, label) in [
        (Role::Checkbox, "Update filter lists automatically"),
        (Role::Button, "Update now"),
    ] {
        a11y::assert_inside(find(&nodes, role, label), group);
    }
    for line in [
        "EasyList: version 202611170805, from 17 Nov 2026, 09:12 (updated)",
        "EasyPrivacy: version 202611152034, from 15 Nov 2026, 21:46 (included with Mercury Browser)",
    ] {
        a11y::assert_inside(find(&nodes, Role::Text, line), group);
    }
    assert_eq!(
        find(&nodes, Role::Button, "Update now").description,
        "Downloads newer versions of the filter lists."
    );
    for label in [
        "Block ads and trackers",
        "Block pop-up windows",
        "Load pages over HTTPS only",
        "Browsing and download history",
        "Cookies and site data",
        "Cached files",
    ] {
        find(&nodes, Role::Checkbox, label);
    }
}

#[test]
fn the_privacy_controls_do_what_a_click_does_and_disabled_ones_do_nothing() {
    let window = settings_window(2);
    let log = settings_log(&window);

    press(&window, Role::Checkbox, "Update filter lists automatically");
    press(&window, Role::Button, "Update now");
    press(&window, Role::Button, "Clear browsing data…");
    assert_eq!(
        entries(&log),
        [
            "toggled update-filter-lists false",
            "update-filter-lists-now",
            "clear-browsing-data"
        ]
    );

    window.set_filter_lists_can_update(false);
    window.set_clear_browsing_data_available(false);
    let nodes = a11y::reading_order(&window);
    let update = find(&nodes, Role::Button, "Update now");
    assert!(!update.enabled);
    assert_eq!(
        update.description,
        "Not available now; the text beside it says why."
    );
    assert!(!find(&nodes, Role::Button, "Clear browsing data…").enabled);
    press(&window, Role::Button, "Update now");
    press(&window, Role::Button, "Clear browsing data…");
    assert_eq!(entries(&log).len(), 3, "nothing more happened");
}

#[test]
fn the_downloads_and_advanced_sections_are_frozen_with_their_fields_labelled() {
    let window = settings_window(3);
    window.set_download_directory("/Users/example/Downloads".into());
    window.set_download_directory_problem("That folder does not exist.".into());

    let nodes = freeze(&window, "windows-settings-downloads");

    assert_eq!(
        find(&nodes, Role::TextInput, "Save downloads in").value,
        "/Users/example/Downloads"
    );
    find(&nodes, Role::Checkbox, "Ask where to save each download");
    // The PDF setting is a labelled choice, and says what the browser does by default.
    assert_eq!(
        find(&nodes, Role::Combobox, "PDF files").value,
        "Open in Mercury Browser"
    );

    let window = settings_window(5);
    window.set_language("en-GB".into());
    let nodes = freeze(&window, "windows-settings-advanced");
    assert_eq!(find(&nodes, Role::TextInput, "Language").value, "en-GB");
    find(&nodes, Role::Checkbox, "Experimental web features");
    find(&nodes, Role::Checkbox, "Draw canvases on the GPU");
}

#[test]
fn the_never_save_list_names_each_site_on_its_remove_button() {
    let window = settings_window(4);
    let log = settings_log(&window);

    let nodes = a11y::reading_order(&window);
    find(&nodes, Role::Button, "Remove https://bank.example");
    press(&window, Role::Button, "Remove http://localhost:8080");

    assert_eq!(entries(&log), ["remove-never-save 1"]);
}

// ---------------------------------------------------------------------------------------------
// The Passwords section of the settings window
// ---------------------------------------------------------------------------------------------

fn passwords_log(window: &SettingsWindow) -> Log {
    let ui = window.global::<PasswordManagerUi>();
    let log = log();
    macro_rules! plain {
        ($on:ident, $name:expr) => {
            ui.$on({
                let log = Rc::clone(&log);
                move || log.borrow_mut().push($name.into())
            });
        };
    }
    ui.on_open_login({
        let log = Rc::clone(&log);
        move |index| log.borrow_mut().push(format!("open-login {index}"))
    });
    plain!(on_close_login, "close-login");
    plain!(on_show_password, "show-password");
    plain!(on_hide_password, "hide-password");
    plain!(on_copy_username, "copy-username");
    plain!(on_copy_password, "copy-password");
    plain!(on_edit_login, "edit-login");
    plain!(on_generate_for_edit, "generate-for-edit");
    plain!(on_save_edit, "save-edit");
    plain!(on_cancel_edit, "cancel-edit");
    plain!(on_delete_login, "delete-login");
    plain!(on_confirm_delete, "confirm-delete");
    plain!(on_cancel_delete, "cancel-delete");
    plain!(on_generate, "generate");
    plain!(on_copy_generated, "copy-generated");
    plain!(on_import_logins, "import-logins");
    plain!(on_export_logins, "export-logins");
    log
}

/// The first login opened, as the desktop code sets it.
fn open_first_login(window: &SettingsWindow) {
    let ui = window.global::<PasswordManagerUi>();
    ui.set_selected(0);
    ui.set_detail_site("a.test".into());
    ui.set_detail_username("alice".into());
}

#[test]
fn the_saved_logins_are_rows_with_a_place_in_the_list_and_the_kind_as_description() {
    let window = passwords_window();

    let nodes = freeze(&window, "windows-settings-passwords-list");

    let rows: Vec<&Node> = nodes.iter().filter(|n| n.role == Role::ListItem).collect();
    assert_eq!(
        rows.iter().map(|r| r.label.as_str()).collect::<Vec<_>>(),
        [
            "a.test, alice",
            "b.test, bob",
            "http://127.0.0.1:8123, carol"
        ]
    );
    for (index, row) in rows.iter().enumerate() {
        assert_eq!(row.item, Some((index, 3)), "{}", row.label);
        assert_eq!(row.selected, Some(false), "{}", row.label);
        assert_eq!(row.handle.accessible_expandable(), Some(true));
        assert_eq!(row.handle.accessible_expanded(), Some(false));
    }
    assert_eq!(rows[2].description, "HTTP sign-in");
    // The search field, the two buttons and the generator are labelled as the desktop tests expect.
    find(&nodes, Role::TextInput, "Search saved logins");
    find(&nodes, Role::Button, "Import logins from a CSV file");
    find(&nodes, Role::Button, "Export logins to a CSV file");
    let length = find(&nodes, Role::Spinbox, "Password length");
    assert_eq!(length.handle.accessible_value_minimum(), Some(8.0));
    assert_eq!(length.handle.accessible_value_maximum(), Some(128.0));
    assert_eq!(length.value, "20");
    find(&nodes, Role::Checkbox, "Symbols");
    find(&nodes, Role::Button, "Generate a password");
}

#[test]
fn a_login_row_opens_and_closes_its_details_through_the_default_action() {
    let window = passwords_window();
    let log = passwords_log(&window);
    let ui = window.global::<PasswordManagerUi>();

    press(&window, Role::ListItem, "a.test, alice");
    assert_eq!(entries(&log), ["open-login 0"]);

    // The row that is open closes on a second press, as a second click does.
    open_first_login(&window);
    let nodes = a11y::reading_order(&window);
    let row = find(&nodes, Role::ListItem, "a.test, alice");
    assert_eq!(row.selected, Some(true));
    assert_eq!(row.handle.accessible_expanded(), Some(true));
    assert_eq!(ui.get_selected(), 0);
    press(&window, Role::ListItem, "a.test, alice");
    assert_eq!(entries(&log), ["open-login 0", "close-login"]);
}

#[test]
fn an_opened_login_reads_the_user_name_and_a_hidden_password_and_offers_its_buttons() {
    let window = passwords_window();
    open_first_login(&window);
    window
        .global::<PasswordManagerUi>()
        .set_status("Copied the user name.".into());

    let nodes = freeze(&window, "windows-settings-passwords-opened");

    find(&nodes, Role::Text, "User name: alice");
    find(&nodes, Role::Text, "Password, hidden");
    for label in [
        "Copy the user name",
        "Show the password",
        "Copy the password",
        "Delete the login",
        "Edit the login",
    ] {
        find(&nodes, Role::Button, label);
    }
    find(&nodes, Role::Text, "Copied the user name.");
}

#[test]
fn a_shown_password_is_in_the_reading_and_the_hide_button_takes_its_place() {
    let window = passwords_window();
    open_first_login(&window);
    let ui = window.global::<PasswordManagerUi>();
    ui.set_password_shown(true);
    ui.set_shown_password("fake-correct-horse".into());

    let nodes = freeze(&window, "windows-settings-passwords-revealed");

    find(&nodes, Role::Text, "Password: fake-correct-horse");
    assert!(absent(&nodes, Role::Button, "Show the password"));
    let log = passwords_log(&window);
    press(&window, Role::Button, "Hide the password");
    assert_eq!(entries(&log), ["hide-password"]);
}

#[test]
fn the_login_buttons_do_what_a_click_does() {
    let window = passwords_window();
    open_first_login(&window);
    let log = passwords_log(&window);

    press(&window, Role::Button, "Copy the user name");
    press(&window, Role::Button, "Show the password");
    press(&window, Role::Button, "Copy the password");
    press(&window, Role::Button, "Edit the login");
    press(&window, Role::Button, "Delete the login");
    press(&window, Role::Button, "Import logins from a CSV file");
    press(&window, Role::Button, "Export logins to a CSV file");

    assert_eq!(
        entries(&log),
        [
            "copy-username",
            "show-password",
            "copy-password",
            "edit-login",
            "delete-login",
            "import-logins",
            "export-logins"
        ]
    );
}

#[test]
fn editing_a_login_labels_the_new_fields_and_the_buttons_that_save_or_leave() {
    let window = passwords_window();
    open_first_login(&window);
    let ui = window.global::<PasswordManagerUi>();
    ui.set_editing(true);
    ui.set_edit_username("alice".into());
    ui.set_edit_problem("A login with this user name is already saved for this site.".into());

    let nodes = freeze(&window, "windows-settings-passwords-editing");

    assert_eq!(
        find(&nodes, Role::TextInput, "New user name").value,
        "alice"
    );
    let password = find(&nodes, Role::TextInput, "New password");
    assert_eq!(password.placeholder(), "Unchanged");
    let log = passwords_log(&window);
    press(&window, Role::Button, "Generate a new password");
    press(&window, Role::Button, "Save the login");
    press(&window, Role::Button, "Cancel");
    assert_eq!(
        entries(&log),
        ["generate-for-edit", "save-edit", "cancel-edit"]
    );
}

/// Types `text` into whatever has the keyboard focus, one key at a time.
fn type_text(window: &SettingsWindow, text: &str) {
    for character in text.chars() {
        let key = SharedString::from(character.to_string());
        window
            .window()
            .dispatch_event(WindowEvent::KeyPressed { text: key.clone() });
        window
            .window()
            .dispatch_event(WindowEvent::KeyReleased { text: key });
    }
}

/// Everything an assistive technology is given about the window, as one text: the reading order
/// with each node's label, description and flags, and each node's value and placeholder.
fn everything_read(window: &SettingsWindow) -> String {
    let nodes = a11y::reading_order(window);
    let mut all = a11y::dump(&nodes);
    for node in &nodes {
        all.push_str(&format!(
            "\n{}|{}|{}|{}",
            node.label,
            node.description,
            node.value,
            node.placeholder()
        ));
    }
    all
}

#[test]
fn a_password_typed_into_the_edit_form_never_reaches_assistive_technology() {
    // The std `LineEdit` bound its accessible value to its text, which the AccessKit adapter hands
    // on for a password field too: macOS gave the typed password to every assistive technology
    // as the field's AXValue. The field of this form must give nothing of it, whichever way the
    // text got there and whether the dots or the characters are drawn.
    let window = passwords_window();
    open_first_login(&window);
    let ui = window.global::<PasswordManagerUi>();
    ui.set_editing(true);
    let field = || a11y::control(&window, Role::TextInput, "New password");
    let value_read = || {
        find(
            &a11y::reading_order(&window),
            Role::TextInput,
            "New password",
        )
        .value
        .clone()
    };
    assert_eq!(value_read(), "");

    // Typed with the keyboard, after a click in the field.
    let typed = "fake-typed-secret-1";
    field().mock_single_click(PointerEventButton::Left);
    type_text(&window, typed);
    assert_eq!(
        ui.get_edit_password(),
        typed,
        "the keys reach the text the browser reads"
    );
    assert_eq!(value_read(), "");
    assert!(
        !everything_read(&window).contains(typed),
        "the typed password is in what is read:\n{}",
        everything_read(&window)
    );

    // Revealed: the characters are drawn, and still nothing more is read.
    press(&window, Role::Button, "Show the new password");
    let nodes = a11y::reading_order(&window);
    assert!(absent(&nodes, Role::Button, "Show the new password"));
    find(&nodes, Role::Button, "Hide the new password");
    assert_eq!(value_read(), "");
    assert!(!everything_read(&window).contains(typed));
    let nodes = freeze(&window, "windows-settings-passwords-editing-typed");
    assert_eq!(find(&nodes, Role::TextInput, "New password").value, "");

    // Put there by an assistive technology (as VoiceOver's dictation does): it works, and what
    // it set is not read back either.
    let set = "fake-set-secret-2";
    field().set_accessible_value(set);
    assert_eq!(ui.get_edit_password(), set);
    assert_eq!(value_read(), "");
    assert!(!everything_read(&window).contains(set));

    // Put there by the browser (the Generate button, which the desktop crate fills in).
    let generated = "fake-generated-secret-3";
    ui.set_edit_password(generated.into());
    assert_eq!(value_read(), "");
    assert!(!everything_read(&window).contains(generated));
}

#[test]
fn the_reveal_button_of_the_new_password_is_there_while_there_is_text_to_reveal() {
    let window = passwords_window();
    open_first_login(&window);
    let ui = window.global::<PasswordManagerUi>();
    ui.set_editing(true);

    let nodes = a11y::reading_order(&window);
    assert!(absent(&nodes, Role::Button, "Show the new password"));
    assert!(absent(&nodes, Role::Button, "Hide the new password"));

    ui.set_edit_password("fake-text".into());
    let nodes = a11y::reading_order(&window);
    let show = find(&nodes, Role::Button, "Show the new password");
    // Inside the field it belongs to, on its right.
    let field = find(&nodes, Role::TextInput, "New password");
    assert!(
        show.rect().x >= field.rect().right() - 0.01,
        "the reveal button is beside the typed text: {:?} and {:?}",
        show.rect(),
        field.rect()
    );
    assert!(show.rect().width > 0.0 && show.rect().height > 0.0);

    // A click in the reported rectangle does what the default action does.
    show.handle.mock_single_click(PointerEventButton::Left);
    let nodes = a11y::reading_order(&window);
    find(&nodes, Role::Button, "Hide the new password");
    press(&window, Role::Button, "Hide the new password");
    let nodes = a11y::reading_order(&window);
    find(&nodes, Role::Button, "Show the new password");

    // Emptied again: nothing left to reveal.
    ui.set_edit_password("".into());
    let nodes = a11y::reading_order(&window);
    assert!(absent(&nodes, Role::Button, "Show the new password"));
}

#[test]
fn deleting_a_login_asks_first_and_the_answers_are_named_by_what_they_do() {
    let window = passwords_window();
    open_first_login(&window);
    let ui = window.global::<PasswordManagerUi>();
    ui.set_confirming_delete(true);

    let nodes = freeze(&window, "windows-settings-passwords-delete");

    find(
        &nodes,
        Role::Text,
        "Delete the login for a.test? This cannot be undone.",
    );
    let log = passwords_log(&window);
    press(&window, Role::Button, "Keep the login");
    press(&window, Role::Button, "Delete the login now");
    assert_eq!(entries(&log), ["cancel-delete", "confirm-delete"]);
}

#[test]
fn the_generator_reads_the_generated_password_and_copies_it() {
    let window = passwords_window();
    window
        .global::<PasswordManagerUi>()
        .set_generated("vH7#qP2-kZ9wT4mX@cE6".into());
    let log = passwords_log(&window);

    let nodes = a11y::reading_order(&window);
    find(
        &nodes,
        Role::Text,
        "Generated password: vH7#qP2-kZ9wT4mX@cE6",
    );
    press(&window, Role::Button, "Generate a password");
    press(&window, Role::Button, "Copy the generated password");
    assert_eq!(entries(&log), ["generate", "copy-generated"]);
}

#[test]
fn exporting_is_disabled_and_does_nothing_without_saved_logins() {
    let window = settings_window(4);
    let ui = window.global::<PasswordManagerUi>();
    ui.set_total(0);
    let log = passwords_log(&window);

    let nodes = freeze(&window, "windows-settings-passwords-empty");

    assert!(!find(&nodes, Role::Button, "Export logins to a CSV file").enabled);
    assert!(find(&nodes, Role::Button, "Import logins from a CSV file").enabled);
    press(&window, Role::Button, "Export logins to a CSV file");
    assert!(entries(&log).is_empty());
}

#[test]
fn the_settings_and_passwords_labels_the_desktop_tests_look_up_are_unchanged() {
    // `apps/mercury-desktop/src/settings/window.rs` and `passwords/section.rs` find their controls
    // with `find_by_accessible_label` and take the first one that is not a text. Every label they
    // use must still exist, once among the non-text elements, with the role they rely on.
    let non_text = |window: &SettingsWindow, label: &str| -> Vec<ElementHandle> {
        ElementHandle::find_by_accessible_label(window, label)
            .filter(|element| element.accessible_role() != Some(Role::Text))
            .collect()
    };
    let check = |window: &SettingsWindow, role: Role, labels: &[&str]| {
        for label in labels {
            let found = non_text(window, label);
            assert_eq!(
                found.len(),
                1,
                "{label:?}: {} non-text elements",
                found.len()
            );
            assert_eq!(found[0].accessible_role(), Some(role), "{label:?}");
        }
    };

    let general = settings_window(0);
    check(&general, Role::Tab, &SECTIONS);
    check(&general, Role::TextInput, &["Home page"]);
    check(&general, Role::Combobox, &["Search engine"]);
    check(&general, Role::Button, &["Remove Docs"]);

    let privacy = settings_window(2);
    check(
        &privacy,
        Role::Checkbox,
        &[
            "Block ads and trackers",
            "Update filter lists automatically",
        ],
    );

    let advanced = settings_window(5);
    check(&advanced, Role::TextInput, &["Language"]);

    let passwords = passwords_window();
    check(&passwords, Role::TextInput, &["Search saved logins"]);
    check(
        &passwords,
        Role::ListItem,
        &["a.test, alice", "b.test, bob"],
    );
    check(&passwords, Role::Button, &["Generate a password"]);
    open_first_login(&passwords);
    check(
        &passwords,
        Role::Button,
        &[
            "Show the password",
            "Copy the user name",
            "Copy the password",
            "Edit the login",
            "Delete the login",
        ],
    );
    let ui = passwords.global::<PasswordManagerUi>();
    ui.set_password_shown(true);
    check(&passwords, Role::Button, &["Hide the password"]);
    ui.set_password_shown(false);
    ui.set_editing(true);
    check(
        &passwords,
        Role::TextInput,
        &["New user name", "New password"],
    );
    check(&passwords, Role::Button, &["Save the login"]);
    ui.set_editing(false);
    ui.set_confirming_delete(true);
    check(&passwords, Role::Button, &["Delete the login now"]);
}

// ---------------------------------------------------------------------------------------------
// The bookmarks manager and the bookmarks preview
// ---------------------------------------------------------------------------------------------

fn menu_row(title: &str, url: &str, kind: BookmarkRowKind, depth: i32) -> BookmarkMenuRow {
    BookmarkMenuRow {
        title: title.into(),
        url: url.into(),
        kind,
        depth,
    }
}

fn manager_window() -> BookmarksManager {
    a11y::init();
    let window = BookmarksManager::new().expect("a window");
    window.set_folders(ModelRc::new(VecModel::from(vec![
        menu_row("Bookmarks Bar", "", BookmarkRowKind::Folder, 0),
        menu_row("News", "", BookmarkRowKind::Folder, 1),
        menu_row("Local", "", BookmarkRowKind::Folder, 2),
        menu_row("", "", BookmarkRowKind::Folder, 1),
        menu_row("Other Bookmarks", "", BookmarkRowKind::Folder, 0),
    ])));
    window.set_folder_index(1);
    window.set_heading("News".into());
    window.set_rows(ModelRc::new(VecModel::from(vec![
        menu_row("Local", "", BookmarkRowKind::Folder, 0),
        menu_row(
            "BBC News",
            "https://www.bbc.co.uk/news",
            BookmarkRowKind::Bookmark,
            0,
        ),
        menu_row(
            "",
            "https://untitled.example/",
            BookmarkRowKind::Bookmark,
            0,
        ),
        menu_row("", "", BookmarkRowKind::Separator, 0),
        menu_row(
            "Hacker News",
            "https://news.ycombinator.com/",
            BookmarkRowKind::Bookmark,
            0,
        ),
    ])));
    window.set_row_index(1);
    window.set_move_targets(strings(&[
        "Bookmarks Bar",
        "News",
        "Local",
        "Other Bookmarks",
    ]));
    window.set_move_index(1);
    window.set_status("5 items.".into());
    a11y::open_window(&window, 920.0, 620.0);
    window
}

fn manager_log(window: &BookmarksManager) -> Log {
    let log = log();
    window.on_folder_selected({
        let log = Rc::clone(&log);
        move |index| log.borrow_mut().push(format!("folder-selected {index}"))
    });
    window.on_row_selected({
        let log = Rc::clone(&log);
        move |index| log.borrow_mut().push(format!("row-selected {index}"))
    });
    window.on_save_edits({
        let log = Rc::clone(&log);
        move || log.borrow_mut().push("save-edits".into())
    });
    window.on_new_folder({
        let log = Rc::clone(&log);
        move || log.borrow_mut().push("new-folder".into())
    });
    window.on_new_separator({
        let log = Rc::clone(&log);
        move || log.borrow_mut().push("new-separator".into())
    });
    window.on_delete_selected({
        let log = Rc::clone(&log);
        move || log.borrow_mut().push("delete-selected".into())
    });
    window.on_undo_delete({
        let log = Rc::clone(&log);
        move || log.borrow_mut().push("undo-delete".into())
    });
    window.on_move_up({
        let log = Rc::clone(&log);
        move || log.borrow_mut().push("move-up".into())
    });
    window.on_move_down({
        let log = Rc::clone(&log);
        move || log.borrow_mut().push("move-down".into())
    });
    window.on_move_to({
        let log = Rc::clone(&log);
        move |index| log.borrow_mut().push(format!("move-to {index}"))
    });
    window.on_import_file({
        let log = Rc::clone(&log);
        move || log.borrow_mut().push("import-file".into())
    });
    window.on_export_file({
        let log = Rc::clone(&log);
        move || log.borrow_mut().push("export-file".into())
    });
    log
}

#[test]
fn the_bookmarks_manager_reads_folders_then_contents_with_their_place_and_selection() {
    let window = manager_window();

    let nodes = freeze(&window, "windows-bookmarks-manager-folder");

    let folders = find(&nodes, Role::List, "Folders");
    let folder_rows = rows_of(&nodes, "Folders");
    assert_eq!(
        folder_rows
            .iter()
            .map(|r| r.label.as_str())
            .collect::<Vec<_>>(),
        [
            "Bookmarks Bar",
            "News",
            "Local",
            "(untitled)",
            "Other Bookmarks"
        ]
    );
    for (index, row) in folder_rows.iter().enumerate() {
        assert_eq!(row.item, Some((index, 5)), "{}", row.label);
        assert_eq!(row.selected, Some(index == 1), "{}", row.label);
        a11y::assert_inside(row, folders);
    }
    assert_eq!(folder_rows[0].description, "");
    assert_eq!(folder_rows[2].description, "Level 3");

    let contents = find(&nodes, Role::List, "Folder contents");
    let rows = rows_of(&nodes, "Folder contents");
    assert_eq!(rows.len(), 5);
    for (index, row) in rows.iter().enumerate() {
        assert_eq!(row.item, Some((index, 5)), "{}", row.label);
        assert_eq!(row.selected, Some(index == 1), "{}", row.label);
        a11y::assert_inside(row, contents);
    }
    // A folder says so, a bookmark gives its address, and a separator is named.
    assert_eq!(rows[0].description, "Folder");
    assert_eq!(rows[1].description, "https://www.bbc.co.uk/news");
    assert_eq!(rows[2].label, "(untitled)");
    assert_eq!(rows[3].label, "Separator");
}

/// The list items that follow the list labelled `list_label`, up to the next list.
fn rows_of<'a>(nodes: &'a [Node], list_label: &str) -> Vec<&'a Node> {
    let start = nodes
        .iter()
        .position(|n| n.role == Role::List && n.label == list_label)
        .unwrap_or_else(|| panic!("no list labelled {list_label:?}"));
    nodes[start + 1..]
        .iter()
        .take_while(|n| n.role != Role::List)
        .filter(|n| n.role == Role::ListItem)
        .collect()
}

/// The content row with this label and description: the folder tree has rows of the same names.
fn find_row<'a>(nodes: &'a [Node], label: &str, description: &str) -> &'a Node {
    let found: Vec<&Node> = nodes
        .iter()
        .filter(|n| n.role == Role::ListItem && n.label == label && n.description == description)
        .collect();
    assert_eq!(
        found.len(),
        1,
        "{label:?} / {description:?}: {} rows",
        found.len()
    );
    found[0]
}

#[test]
fn pressing_a_folder_or_a_row_of_the_manager_selects_it_as_a_click_does() {
    let window = manager_window();
    let log = manager_log(&window);

    press(&window, Role::ListItem, "Other Bookmarks");
    assert_eq!(entries(&log), ["folder-selected 4"]);

    let nodes = a11y::reading_order(&window);
    let hacker = find_row(&nodes, "Hacker News", "https://news.ycombinator.com/");
    hacker.press();
    assert_eq!(window.get_row_index(), 4);
    assert_eq!(entries(&log), ["folder-selected 4", "row-selected 4"]);
    let nodes = a11y::reading_order(&window);
    assert_eq!(
        find_row(&nodes, "Hacker News", "https://news.ycombinator.com/").selected,
        Some(true)
    );
    assert_eq!(
        find_row(&nodes, "BBC News", "https://www.bbc.co.uk/news").selected,
        Some(false)
    );
}

#[test]
fn the_manager_buttons_do_what_a_click_does() {
    let window = manager_window();
    window.set_can_undo(true);
    window.set_undo_label("Undo delete of “Weather”".into());
    window.set_can_move_up(true);
    window.set_can_move_down(true);
    let log = manager_log(&window);

    for label in [
        "New folder",
        "New separator",
        "Import bookmarks from file…",
        "Export bookmarks…",
        "Move up",
        "Move down",
        "Delete",
        "Undo delete of “Weather”",
    ] {
        press(&window, Role::Button, label);
    }

    assert_eq!(
        entries(&log),
        [
            "new-folder",
            "new-separator",
            "import-file",
            "export-file",
            "move-up",
            "move-down",
            "delete-selected",
            "undo-delete"
        ]
    );
}

#[test]
fn the_manager_buttons_that_cannot_act_say_so_and_do_nothing() {
    let window = manager_window();
    window.set_can_add(false);
    window.set_busy(true);
    window.set_row_index(-1);
    window.set_can_move_up(false);
    window.set_can_move_down(false);
    let log = manager_log(&window);

    let nodes = a11y::reading_order(&window);
    for label in [
        "New folder",
        "New separator",
        "Import bookmarks from file…",
        "Export bookmarks…",
        "Move up",
        "Move down",
        "Delete",
    ] {
        assert!(!find(&nodes, Role::Button, label).enabled, "{label}");
        press(&window, Role::Button, label);
    }

    assert!(entries(&log).is_empty(), "{:?}", entries(&log));
    assert!(
        absent(&nodes, Role::Button, "Undo delete"),
        "no undo button without an undo"
    );
}

#[test]
fn the_manager_editor_labels_its_fields_and_saves_through_its_buttons() {
    let window = manager_window();
    window.set_editing(true);
    window.set_editing_bookmark(true);
    window.set_edit_name("BBC News".into());
    window.set_edit_url("https://www.bbc.co.uk/news".into());
    window.set_problem("Enter a full address, such as https://example.com/.".into());
    window.set_can_undo(true);
    window.set_can_move_up(true);
    window.set_can_move_down(true);
    window.set_undo_label("Undo delete of “Weather”".into());

    let nodes = freeze(&window, "windows-bookmarks-manager-editing");

    assert_eq!(find(&nodes, Role::TextInput, "Name").value, "BBC News");
    assert_eq!(
        find(&nodes, Role::TextInput, "Address").value,
        "https://www.bbc.co.uk/news"
    );
    assert_eq!(
        find(&nodes, Role::Combobox, "Folder to move to").value,
        "News"
    );
    let log = manager_log(&window);
    press(&window, Role::Button, "Move");
    press(&window, Role::Button, "Save");
    assert_eq!(entries(&log), ["move-to 1", "save-edits"]);
}

#[test]
fn editing_a_folder_leaves_out_the_address_field() {
    let window = manager_window();
    window.set_row_index(0);
    window.set_editing(true);
    window.set_editing_bookmark(false);
    window.set_edit_name("Local".into());

    let nodes = freeze(&window, "windows-bookmarks-manager-editing-folder");

    assert!(absent(&nodes, Role::TextInput, "Address"));
    find(&nodes, Role::TextInput, "Name");
}

#[test]
fn the_manager_search_names_its_results_and_the_empty_cases_are_read() {
    let window = manager_window();
    window.set_searching(true);
    window.set_search_text("guardian".into());
    window.set_heading("1 result for “guardian”".into());
    window.set_rows(ModelRc::new(VecModel::from(vec![menu_row(
        "The Guardian",
        "https://www.theguardian.com/international",
        BookmarkRowKind::Bookmark,
        0,
    )])));
    window.set_row_index(-1);

    let nodes = freeze(&window, "windows-bookmarks-manager-search");

    assert_eq!(
        find(&nodes, Role::TextInput, "Search bookmarks").value,
        "guardian"
    );
    find(&nodes, Role::List, "Search results");
    assert!(absent(&nodes, Role::List, "Folder contents"));
    // The results replace the folder's contents, and no folder is drawn as selected, so none
    // is read as selected.
    let folder_rows = rows_of(&nodes, "Folders");
    assert_eq!(folder_rows.len(), 5);
    assert!(folder_rows.iter().all(|row| row.selected == Some(false)));
    let results = rows_of(&nodes, "Search results");
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].item, Some((0, 1)));

    // No match: the drawn sentence is read, in front of the empty list.
    window.set_rows(ModelRc::new(VecModel::from(Vec::<BookmarkMenuRow>::new())));
    window.set_heading("No results for “guardian”".into());
    let nodes = freeze(&window, "windows-bookmarks-manager-no-match");
    find(&nodes, Role::Text, "No bookmarks match.");
    assert!(rows_of(&nodes, "Search results").is_empty());
}

#[test]
fn an_empty_folder_of_the_manager_says_so() {
    let window = manager_window();
    window.set_folder_index(2);
    window.set_heading("Local".into());
    window.set_rows(ModelRc::new(VecModel::from(Vec::<BookmarkMenuRow>::new())));
    window.set_row_index(-1);
    window.set_status("0 items.".into());

    let nodes = freeze(&window, "windows-bookmarks-manager-empty");

    find(&nodes, Role::Text, "This folder is empty.");
    assert!(rows_of(&nodes, "Folder contents").is_empty());
    assert!(!find(&nodes, Role::Button, "Delete").enabled);
}

#[test]
fn the_bookmarks_preview_reads_the_bar_the_menu_and_the_popover() {
    a11y::init();
    let window = BookmarksPreview::new().expect("a window");
    let widths = 150.0;
    window.set_bar_items(ModelRc::new(VecModel::from(
        [
            ("Mercury Browser", false),
            ("News", true),
            ("Rust documentation", false),
        ]
        .iter()
        .enumerate()
        .map(|(index, (title, is_folder))| BookmarkBarItem {
            title: (*title).into(),
            is_folder: *is_folder,
            x: widths * index as f32,
            width: widths - 6.0,
            shown: true,
        })
        .collect::<Vec<_>>(),
    )));
    window.set_menu_rows(ModelRc::new(VecModel::from(vec![
        menu_row("News", "", BookmarkRowKind::Back, 0),
        menu_row("Local", "", BookmarkRowKind::Folder, 0),
        menu_row(
            "BBC News",
            "https://www.bbc.co.uk/news",
            BookmarkRowKind::Bookmark,
            0,
        ),
    ])));
    window.set_popover_name("The Guardian".into());
    window.set_popover_folders(strings(&["Bookmarks Bar", "News"]));
    window.set_popover_folder_index(1);
    a11y::open_window(&window, 760.0, 520.0);

    let nodes = freeze(&window, "windows-bookmarks-preview");

    // The three parts the preview shows are all there: the bar buttons (the titles the test gave),
    // the folder popup's rows and the star popover's fields.
    for (index, title) in ["Mercury Browser", "News", "Rust documentation"]
        .iter()
        .enumerate()
    {
        assert_eq!(
            find(&nodes, Role::Button, title).item,
            Some((index, 3)),
            "{title}"
        );
    }
    assert_eq!(
        find(&nodes, Role::TextInput, "Bookmark name").value,
        "The Guardian"
    );
    assert_eq!(
        find(&nodes, Role::Combobox, "Bookmark folder").value,
        "News"
    );
}

// ---------------------------------------------------------------------------------------------
// Text fields and pop-up menus, through the actions an assistive technology has for them
// ---------------------------------------------------------------------------------------------

#[test]
fn a_text_field_takes_what_the_set_value_action_gives_it() {
    // The settings window's Home page: the value reaches the property the browser reads.
    let settings = settings_window(0);
    a11y::control(&settings, Role::TextInput, "Home page").set_accessible_value("example.net");
    assert_eq!(settings.get_homepage(), "example.net");

    // The Passwords search: the value and the callback that narrows the list.
    let passwords = passwords_window();
    let ui = passwords.global::<PasswordManagerUi>();
    let log = log();
    ui.on_search_edited({
        let log = Rc::clone(&log);
        move |text| log.borrow_mut().push(format!("search-edited {text}"))
    });
    a11y::control(&passwords, Role::TextInput, "Search saved logins").set_accessible_value("bob");
    assert_eq!(ui.get_search(), "bob");
    assert_eq!(entries(&log), ["search-edited bob"]);

    // The bookmarks manager's search and its editor.
    let manager = manager_window();
    manager.set_editing(true);
    manager.set_editing_bookmark(true);
    let log = manager_search_log(&manager);
    a11y::control(&manager, Role::TextInput, "Search bookmarks").set_accessible_value("rust");
    a11y::control(&manager, Role::TextInput, "Name").set_accessible_value("Rust docs");
    assert_eq!(manager.get_search_text(), "rust");
    assert_eq!(manager.get_edit_name(), "Rust docs");
    assert_eq!(entries(&log), ["search-edited rust"]);
}

fn manager_search_log(window: &BookmarksManager) -> Log {
    let log = log();
    window.on_search_edited({
        let log = Rc::clone(&log);
        move |text| log.borrow_mut().push(format!("search-edited {text}"))
    });
    log
}

#[test]
fn a_combo_box_of_the_settings_window_opens_through_the_expand_action() {
    let window = settings_window(1);
    let theme = a11y::control(&window, Role::Combobox, "Theme");
    assert_eq!(theme.accessible_expandable(), Some(true));
    assert_eq!(theme.accessible_expanded(), Some(false));

    theme.invoke_accessible_expand_action();

    assert_eq!(theme.accessible_expanded(), Some(true));
}

#[test]
fn the_settings_notice_about_a_file_that_cannot_be_saved_is_read_before_the_section() {
    let window = settings_window(0);
    window.set_save_problem(
        "Your settings could not be read, so Mercury Browser is using its defaults.".into(),
    );

    let nodes = a11y::reading_order(&window);
    a11y::assert_labelled(&nodes, "the settings window with its notice");
    a11y::assert_bounds(&nodes, a11y::window_size(&window));

    let position = |role: Role, label: &str| {
        nodes
            .iter()
            .position(|n| n.role == role && n.label == label)
            .unwrap_or_else(|| panic!("no {role:?} {label:?}"))
    };
    let notice = position(
        Role::Text,
        "Your settings could not be read, so Mercury Browser is using its defaults.",
    );
    // After the sections, in front of the heading of the section it is shown over.
    assert!(position(Role::TabList, "Settings sections") < notice);
    assert!(notice < position(Role::Text, "General"));
}
