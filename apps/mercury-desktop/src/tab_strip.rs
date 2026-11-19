// SPDX-License-Identifier: MPL-2.0
//! The tab strip's model and its commands (tabs-plus lane): pinned tabs, dragging, the tab menu,
//! "close other tabs", tab search and the Cmd+W rule for pinned tabs. The rules live in
//! `talaria_core::BrowserController`; this module turns them into what the window shows.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use mercury_ui::{BrowserMenuCommand, BrowserTab, MainWindow, TabMenuCommand, TabSearchRow};
use slint::{ComponentHandle, Image, Model, ModelRc, VecModel};
use talaria_core::{BrowserController, LoadState, PinnedCloseOutcome, TabId};

use crate::ui_sync::{slint_image_from_page_icon, sync_ui};

/// A tab id as the window carries it. Ids past `i32::MAX` never happen in practice (one per
/// opened tab); such a tab gets -1, which no command finds.
pub(crate) fn id_for_ui(tab_id: TabId) -> i32 {
    i32::try_from(tab_id.value()).unwrap_or(-1)
}

pub(crate) fn id_from_ui(id: i32) -> Option<TabId> {
    u64::try_from(id).ok().map(TabId::new)
}

/// The strip's rows, in strip order.
pub(crate) fn strip_rows(controller: &BrowserController) -> Vec<BrowserTab> {
    let active_tab_id = controller.active_tab_id();
    controller
        .tabs()
        .map(|tab| {
            let state = tab.state();
            let (icon, has_icon) = match state.page_icon() {
                Some(icon) => (slint_image_from_page_icon(icon), true),
                None => (Image::default(), false),
            };
            BrowserTab {
                title: state.page_title().into(),
                icon,
                has_icon,
                active: tab.id() == active_tab_id,
                loading: matches!(state.load_state(), LoadState::Loading),
                id: id_for_ui(tab.id()),
                pinned: tab.is_pinned(),
                url: state.url().into(),
                // media-session lane
                audio: crate::media_session::tab_audio_state(state.audio_indicator()),
            }
        })
        .collect()
}

/// Updates the window's tabs in place by tab id, so the strip's tab items (and a drag in progress)
/// survive a sync, tabs opening and closing included.
pub(crate) fn sync(window: &MainWindow, controller: &BrowserController) {
    let rows = strip_rows(controller);
    let model = window.get_tabs();
    match model.as_any().downcast_ref::<VecModel<BrowserTab>>() {
        // By tab id, never a reset: a drag in progress keeps its tab item.
        Some(tabs) => mercury_ui::update_tab_rows(tabs, rows),
        None => window.set_tabs(ModelRc::new(VecModel::from(rows))),
    }
    let active_tab_id = controller.active_tab_id();
    let active_index = controller
        .tabs()
        .position(|tab| tab.id() == active_tab_id)
        .unwrap_or(0);
    window.set_active_tab_index(i32::try_from(active_index).unwrap_or(i32::MAX));
    window.set_pinned_tab_count(i32::try_from(controller.pinned_tab_count()).unwrap_or(0));
    sync_browser_menu(window, controller);
    window.invoke_reveal_active_tab();
}

/// The browser menu's tab rows follow the active tab: "Pin Tab" or "Unpin Tab", and "Close Other
/// Tabs" only when there is another unpinned tab.
fn sync_browser_menu(window: &MainWindow, controller: &BrowserController) {
    let active_tab_id = controller.active_tab_id();
    let pinned = controller
        .tabs()
        .any(|tab| tab.id() == active_tab_id && tab.is_pinned());
    let can_close_others = controller
        .tabs()
        .any(|tab| tab.id() != active_tab_id && !tab.is_pinned());
    let entries = window.get_browser_menu_top_entries();
    for index in 0..entries.row_count() {
        let Some(mut entry) = entries.row_data(index) else {
            continue;
        };
        let before = entry.clone();
        match entry.command {
            BrowserMenuCommand::TogglePinTab => {
                entry.title = if pinned { "Unpin Tab" } else { "Pin Tab" }.into();
            }
            BrowserMenuCommand::CloseOtherTabs => entry.enabled = can_close_others,
            _ => continue,
        }
        if entry != before {
            entries.set_row_data(index, entry);
        }
    }
}

/// The rows tab search shows for `query`.
pub(crate) fn search_rows(controller: &BrowserController, query: &str) -> Vec<TabSearchRow> {
    let matching = controller.matching_tabs(query);
    let active_tab_id = controller.active_tab_id();
    controller
        .tabs()
        .filter(|tab| matching.contains(&tab.id()))
        .map(|tab| {
            let state = tab.state();
            let (icon, has_icon) = match state.page_icon() {
                Some(icon) => (slint_image_from_page_icon(icon), true),
                None => (Image::default(), false),
            };
            TabSearchRow {
                id: id_for_ui(tab.id()),
                title: state.page_title().into(),
                url: state.url().into(),
                icon,
                has_icon,
                pinned: tab.is_pinned(),
                active: tab.id() == active_tab_id,
            }
        })
        .collect()
}

/// What a tab menu command does to the controller.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MenuOutcome {
    Changed,
    Unchanged,
    /// The tab was closed (or was the last tab): finish as any close.
    Closed(talaria_core::CloseTabOutcome),
    /// Duplicate the tab: the session lane opens the copy (`session::duplicate_tab`).
    Duplicate,
}

pub(crate) fn run_menu_command(
    controller: &mut BrowserController,
    tab_id: TabId,
    command: TabMenuCommand,
) -> MenuOutcome {
    let changed = |changed: bool| {
        if changed {
            MenuOutcome::Changed
        } else {
            MenuOutcome::Unchanged
        }
    };
    match command {
        TabMenuCommand::Pin => changed(controller.set_tab_pinned(tab_id, true)),
        TabMenuCommand::Unpin => changed(controller.set_tab_pinned(tab_id, false)),
        // The menu closes a pinned tab too: it is the explicit way to.
        TabMenuCommand::Close => MenuOutcome::Closed(controller.close_tab(tab_id)),
        TabMenuCommand::CloseOthers => changed(controller.close_other_tabs(tab_id) > 0),
        TabMenuCommand::Duplicate => MenuOutcome::Duplicate,
    }
}

/// Cmd+W (and File > Close Tab) on a pinned tab: selects the first unpinned tab, or opens a new
/// tab when every tab is pinned. Returns false for an unpinned tab, which closes as usual.
pub(crate) fn step_off_pinned_tab(
    window: &MainWindow,
    controller: &RefCell<BrowserController>,
    web_view_focused: &Cell<bool>,
    tab_id: TabId,
) -> bool {
    let outcome = controller.borrow_mut().step_off_pinned_tab(tab_id);
    match outcome {
        PinnedCloseOutcome::NotPinned => return false,
        PinnedCloseOutcome::SelectedUnpinned => sync_ui(window, &controller.borrow()),
        PinnedCloseOutcome::OpenedTab => {
            crate::shortcuts::unfocus_web_view(controller, web_view_focused);
            sync_ui(window, &controller.borrow());
            window.invoke_focus_address_bar();
        }
    }
    true
}

/// The menu bar's commands for the active tab (the keyboard path to the tab menu's items).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TabStripCommand {
    PinTab,
    UnpinTab,
    /// Pins the active tab, or unpins it (the browser menu's one row).
    TogglePinTab,
    CloseOtherTabs,
}

impl TabStripCommand {
    const fn tab_menu_command(self, active_tab_pinned: bool) -> TabMenuCommand {
        match self {
            Self::PinTab => TabMenuCommand::Pin,
            Self::UnpinTab => TabMenuCommand::Unpin,
            Self::TogglePinTab if active_tab_pinned => TabMenuCommand::Unpin,
            Self::TogglePinTab => TabMenuCommand::Pin,
            Self::CloseOtherTabs => TabMenuCommand::CloseOthers,
        }
    }
}

pub(crate) fn run_strip_command(
    window: &MainWindow,
    controller: &RefCell<BrowserController>,
    command: TabStripCommand,
) {
    let active_tab_id = controller.borrow().active_tab_id();
    let active_tab_pinned = controller
        .borrow()
        .tabs()
        .any(|tab| tab.id() == active_tab_id && tab.is_pinned());
    let outcome = run_menu_command(
        &mut controller.borrow_mut(),
        active_tab_id,
        command.tab_menu_command(active_tab_pinned),
    );
    if outcome == MenuOutcome::Changed {
        sync_ui(window, &controller.borrow());
        window.window().request_redraw();
    }
}

/// Opens tab search (Shift+Cmd+A, Window > Search Tabs…, the strip's "all tabs" button).
pub(crate) fn show_tab_search(window: &MainWindow) {
    window.invoke_show_tab_search();
}

pub(crate) fn connect(window: &MainWindow, controller: Rc<RefCell<BrowserController>>) {
    let move_controller = Rc::clone(&controller);
    let move_window = window.as_weak();
    window.on_move_tab(move |id, index, pinned| {
        let _slint = crate::callback_scope::InSlintCallback::enter();
        let (Some(tab_id), Ok(index)) = (id_from_ui(id), usize::try_from(index)) else {
            return;
        };
        let moved = move_controller.borrow_mut().move_tab(tab_id, index, pinned);
        if let Some(window) = move_window.upgrade() {
            // Even unmoved: the dragged tab goes back to its place.
            if moved {
                sync_ui(&window, &move_controller.borrow());
            }
            window.window().request_redraw();
        }
    });

    let menu_controller = Rc::clone(&controller);
    let menu_window = window.as_weak();
    window.on_tab_menu_activated(move |id, command| {
        let _slint = crate::callback_scope::InSlintCallback::enter();
        let Some(tab_id) = id_from_ui(id) else {
            return;
        };
        let outcome = run_menu_command(&mut menu_controller.borrow_mut(), tab_id, command);
        let Some(window) = menu_window.upgrade() else {
            return;
        };
        match outcome {
            MenuOutcome::Changed => sync_ui(&window, &menu_controller.borrow()),
            MenuOutcome::Unchanged => {}
            MenuOutcome::Duplicate => {
                // The copy follows the tab the menu belongs to; the session lane opens it.
                if menu_controller.borrow_mut().select_tab(tab_id) {
                    crate::session::duplicate_tab(&window, &menu_controller);
                }
            }
            MenuOutcome::Closed(outcome) => {
                crate::tab_close::finish_closing_tab(&window, &menu_controller.borrow(), outcome);
            }
        }
        window.window().request_redraw();
    });

    let search_controller = Rc::clone(&controller);
    let search_window = window.as_weak();
    window.on_tab_search_edited(move |query| {
        let _slint = crate::callback_scope::InSlintCallback::enter();
        if let Some(window) = search_window.upgrade() {
            let rows = search_rows(&search_controller.borrow(), &query);
            window.set_tab_search_rows(ModelRc::new(VecModel::from(rows)));
        }
    });

    let chosen_controller = controller;
    let chosen_window = window.as_weak();
    window.on_tab_search_chosen(move |id| {
        let _slint = crate::callback_scope::InSlintCallback::enter();
        let Some(tab_id) = id_from_ui(id) else {
            return;
        };
        if chosen_controller.borrow_mut().select_tab(tab_id)
            && let Some(window) = chosen_window.upgrade()
        {
            sync_ui(&window, &chosen_controller.borrow());
            window.window().request_redraw();
        }
    });
}

#[cfg(test)]
mod tests {
    use talaria_core::{BrowserEvent, CloseTabOutcome, TabActivation};

    use super::*;

    fn controller_with_tabs(count: usize) -> (BrowserController, Vec<TabId>) {
        let mut controller = BrowserController::default();
        let mut ids = vec![controller.active_tab_id()];
        for _ in 1..count {
            ids.push(
                controller
                    .open_tab_with(None, TabActivation::Background)
                    .expect("a blank tab opens"),
            );
        }
        (controller, ids)
    }

    #[test]
    fn tab_ids_survive_the_trip_through_the_window() {
        let tab_id = TabId::new(41);
        assert_eq!(id_from_ui(id_for_ui(tab_id)), Some(tab_id));
        assert_eq!(id_from_ui(-1), None);
        assert_eq!(id_for_ui(TabId::new(u64::MAX)), -1);
    }

    #[test]
    fn strip_rows_carry_the_pinned_flag_the_id_and_the_address() {
        let (mut controller, ids) = controller_with_tabs(2);
        assert!(controller.handle_event(
            ids[1],
            BrowserEvent::AddressChanged {
                url: "https://a.test/".into()
            }
        ));
        assert!(controller.set_tab_pinned(ids[1], true));

        let rows = strip_rows(&controller);

        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].id, id_for_ui(ids[1]));
        assert!(rows[0].pinned);
        assert_eq!(rows[0].url, "https://a.test/");
        assert!(!rows[1].pinned);
        assert!(rows[1].active);
    }

    #[test]
    fn tab_search_lists_the_matching_tabs() {
        let (mut controller, ids) = controller_with_tabs(3);
        assert!(controller.handle_event(
            ids[2],
            BrowserEvent::PageTitleChanged {
                title: Some("Servo".into())
            }
        ));

        let rows = search_rows(&controller, "servo");

        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].id, id_for_ui(ids[2]));
        assert_eq!(search_rows(&controller, "").len(), 3);
    }

    #[test]
    fn the_window_gets_the_pinned_count_and_rows_by_id() {
        crate::settings::tests::init_slint();
        let window = MainWindow::new().expect("the main window can be created");
        let (mut controller, ids) = controller_with_tabs(3);
        assert!(controller.set_tab_pinned(ids[2], true));

        sync(&window, &controller);

        assert_eq!(window.get_pinned_tab_count(), 1);
        let rows = window.get_tabs();
        let pinned = rows.iter().filter(|row| row.pinned).count();
        assert_eq!(pinned, 1);
        assert_eq!(
            rows.iter().map(|row| row.id).collect::<Vec<_>>(),
            [ids[2], ids[0], ids[1]].map(id_for_ui)
        );

        // A tab closes and another opens: the model is the same one, updated.
        let before = rows.clone();
        assert_eq!(controller.close_tab(ids[0]), CloseTabOutcome::Closed);
        controller.open_tab();
        sync(&window, &controller);
        assert!(before == window.get_tabs(), "the same model, not a new one");
        assert_eq!(window.get_tabs().row_count(), 3);
    }

    #[test]
    fn the_browser_menu_says_unpin_for_a_pinned_tab() {
        crate::settings::tests::init_slint();
        let window = MainWindow::new().expect("the main window can be created");
        let row = |command| mercury_ui::BrowserMenuEntry {
            title: "Pin Tab".into(),
            enabled: true,
            command,
            ..Default::default()
        };
        window.set_browser_menu_top_entries(ModelRc::new(VecModel::from(vec![
            row(BrowserMenuCommand::TogglePinTab),
            row(BrowserMenuCommand::CloseOtherTabs),
        ])));
        let (mut controller, ids) = controller_with_tabs(2);
        assert!(controller.set_tab_pinned(ids[1], true));
        assert!(controller.select_tab(ids[1]));

        sync(&window, &controller);

        let entries = window.get_browser_menu_top_entries();
        assert_eq!(
            entries.row_data(0).map(|entry| entry.title),
            Some("Unpin Tab".into())
        );
        assert_eq!(entries.row_data(1).map(|entry| entry.enabled), Some(true));

        assert!(controller.set_tab_pinned(ids[1], false));
        assert_eq!(controller.close_other_tabs(ids[1]), 1);
        sync(&window, &controller);
        let entries = window.get_browser_menu_top_entries();
        assert_eq!(
            entries.row_data(0).map(|entry| entry.title),
            Some("Pin Tab".into())
        );
        assert_eq!(entries.row_data(1).map(|entry| entry.enabled), Some(false));
    }

    #[test]
    fn the_tab_menu_pins_unpins_and_closes() {
        let (mut controller, ids) = controller_with_tabs(3);

        assert_eq!(
            run_menu_command(&mut controller, ids[2], TabMenuCommand::Pin),
            MenuOutcome::Changed
        );
        assert_eq!(
            run_menu_command(&mut controller, ids[2], TabMenuCommand::Pin),
            MenuOutcome::Unchanged
        );
        assert_eq!(
            run_menu_command(&mut controller, ids[1], TabMenuCommand::Duplicate),
            MenuOutcome::Duplicate
        );
        assert_eq!(
            run_menu_command(&mut controller, ids[0], TabMenuCommand::CloseOthers),
            MenuOutcome::Changed
        );
        assert_eq!(controller.tabs().len(), 2);
        // The menu closes a pinned tab, which the shortcut never does.
        assert_eq!(
            run_menu_command(&mut controller, ids[2], TabMenuCommand::Close),
            MenuOutcome::Closed(CloseTabOutcome::Closed)
        );
        assert_eq!(
            run_menu_command(&mut controller, ids[0], TabMenuCommand::Close),
            MenuOutcome::Closed(CloseTabOutcome::LastTab)
        );
    }
}
