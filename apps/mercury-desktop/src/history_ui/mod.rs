//! The history lane's user interface: suggestions under the address field, the history window
//! (Cmd+Y) and the "Clear browsing data" dialog (Cmd+Shift+Delete). Everything goes through the
//! profile data service; nothing typed or listed leaves the local database.
//!
//! **Suggestions.** Each edit of the address field bumps a generation and (re)starts a
//! [`suggestions::SUGGEST_DEBOUNCE_MS`] timer; the store is asked when it fires, and a reply for
//! an older generation is dropped. Up and Down move the selection, Return opens it, Escape
//! closes the list (and is then not passed on, so a later Escape handler, such as the tabs
//! lane's revert, runs only when the list is closed); a click opens a row. When the top result's
//! host starts with what was typed, the rest of the host is added and selected, so typing goes
//! on over it; only typing completes, never deleting.
//!
//! **Diagnostic:** with `MERCURY_SHOW_HISTORY=1` in the environment the history window opens at
//! start-up, so `--screenshot` can capture it without input.

mod days;
mod listing;
mod suggestions;

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use mercury_store::{HistoryEntry, Suggestion};
use mercury_ui::{HistoryRow, MainWindow, SuggestionRow};
use slint::winit_030::winit::event::{ElementState, KeyEvent};
use slint::winit_030::winit::keyboard::{Key, NamedKey};
use slint::{ComponentHandle, ModelRc, SharedString, Timer, VecModel};
use talaria_core::{BrowserController, NavigationOrigin, TabActivation};

use crate::profile_data::{self, Area, ProfileDataError};
use crate::ui_sync::sync_ui;
use listing::{Line, PAGE, Period};
use suggestions::{Row, SUGGEST_DEBOUNCE_MS, SUGGESTION_ROWS};

/// How long typing in the history window's search field must pause before it searches.
const SEARCH_DEBOUNCE: Duration = Duration::from_millis(150);

thread_local! {
    static UI: RefCell<Option<Rc<HistoryUi>>> = const { RefCell::new(None) };
}

struct HistoryUi {
    window: slint::Weak<MainWindow>,
    controller: Rc<RefCell<BrowserController>>,
    suggest: RefCell<SuggestState>,
    suggest_timer: Timer,
    list: RefCell<ListState>,
    search_timer: Timer,
}

#[derive(Default)]
struct SuggestState {
    /// Bumped by every edit; a reply for an older one is stale.
    generation: u64,
    /// What the user typed, without any completion.
    typed: String,
    /// Whether the last edit added to the text (only then is the host completed).
    typing: bool,
    rows: Vec<Row>,
    selected: Option<usize>,
}

#[derive(Default)]
struct ListState {
    generation: u64,
    query: Option<String>,
    entries: Vec<HistoryEntry>,
    has_more: bool,
}

fn with_ui<R>(work: impl FnOnce(&Rc<HistoryUi>) -> R) -> Option<R> {
    let ui = UI.with(|current| current.borrow().clone())?;
    Some(work(&ui))
}

/// Connects the address field, the history window and the clear dialog. Call once, after the
/// profile data service has started.
pub(crate) fn connect(window: &MainWindow, controller: Rc<RefCell<BrowserController>>) {
    let ui = Rc::new(HistoryUi {
        window: window.as_weak(),
        controller,
        suggest: RefCell::default(),
        suggest_timer: Timer::default(),
        list: RefCell::default(),
        search_timer: Timer::default(),
    });
    UI.with(|current| *current.borrow_mut() = Some(Rc::clone(&ui)));

    window.on_address_edited(|text| {
        with_ui(|ui| ui.address_edited(text.as_str()));
    });
    window.on_suggestion_chosen(|index| {
        with_ui(|ui| ui.choose(usize::try_from(index).ok()));
    });
    window.on_history_search(|text| {
        with_ui(|ui| ui.search_edited(text.as_str()));
    });
    window.on_history_more(|| {
        with_ui(|ui| ui.load(false));
    });
    window.on_history_open_entry(|entry, new_tab| {
        with_ui(|ui| ui.open_entry(entry, new_tab));
    });
    window.on_history_delete_entry(|entry| {
        with_ui(|ui| ui.delete_entry(entry, false));
    });
    window.on_history_delete_site(|entry| {
        with_ui(|ui| ui.delete_entry(entry, true));
    });
    window.on_history_close(|| {
        with_ui(|ui| ui.close_history());
    });
    window.on_show_clear_dialog(show_clear_dialog);
    window.on_clear_cancel(|| {
        with_ui(|ui| ui.close_clear_dialog());
    });
    window.on_clear_confirm(|| {
        with_ui(|ui| ui.confirm_clear());
    });

    if let Some(data) = profile_data::handle() {
        // The handle is looked up inside, not captured (see `on_change`).
        data.on_change(Area::History, |_| {
            with_ui(|ui| ui.refresh_history());
        });
    }
    if std::env::var_os("MERCURY_SHOW_HISTORY").is_some_and(|value| value == "1") {
        show_history();
    }
}

/// Opens the history window (Cmd+Y), or focuses its search field if it is open.
pub(crate) fn show_history() {
    with_ui(|ui| ui.show_history());
}

/// Opens "Clear browsing data" (Cmd+Shift+Delete, and the history window's button).
pub(crate) fn show_clear_dialog() {
    with_ui(|ui| {
        let Some(window) = ui.window.upgrade() else {
            return;
        };
        window.set_clear_period(0);
        window.set_clear_history(true);
        window.set_clear_cookies(false);
        window.set_clear_cache(false);
        window.set_clear_busy(false);
        window.set_clear_dialog_open(true);
        window.window().request_redraw();
    });
}

/// The suggestion list's keys while the address field has focus and the list is open: Up and
/// Down move, Return opens the selected row, Escape closes the list. Returns true if the key
/// was used and must not reach the address field.
pub(crate) fn handle_key(window: &MainWindow, event: &KeyEvent) -> bool {
    if event.state != ElementState::Pressed
        || !window.get_address_focused()
        || !window.get_suggestions_open()
    {
        return false;
    }
    with_ui(|ui| match &event.logical_key {
        Key::Named(NamedKey::ArrowDown) => {
            ui.move_selection(true);
            true
        }
        Key::Named(NamedKey::ArrowUp) => {
            ui.move_selection(false);
            true
        }
        Key::Named(NamedKey::Escape) => {
            ui.close_suggestions();
            true
        }
        Key::Named(NamedKey::Enter) => {
            let selected = ui.suggest.borrow().selected;
            match selected {
                Some(index) => {
                    ui.choose(Some(index));
                    true
                }
                // The address field navigates to what it holds.
                None => {
                    ui.close_suggestions();
                    false
                }
            }
        }
        _ => false,
    })
    .unwrap_or(false)
}

fn slint_rows(rows: &[Row]) -> ModelRc<SuggestionRow> {
    ModelRc::new(VecModel::from(
        rows.iter()
            .map(|row| SuggestionRow {
                is_search: row.is_search,
                is_bookmark: row.is_bookmark,
                title_before: row.title.before.as_str().into(),
                title_match: row.title.matched.as_str().into(),
                title_after: row.title.after.as_str().into(),
                url_before: row.url.before.as_str().into(),
                url_match: row.url.matched.as_str().into(),
                url_after: row.url.after.as_str().into(),
            })
            .collect::<Vec<_>>(),
    ))
}

fn slint_lines(lines: &[Line], has_more: bool) -> ModelRc<HistoryRow> {
    let mut rows = lines
        .iter()
        .map(|line| match line {
            Line::Day(label) => HistoryRow {
                is_day: true,
                label: label.as_str().into(),
                ..HistoryRow::default()
            },
            Line::Visit {
                entry,
                time,
                title,
                url,
                host,
            } => HistoryRow {
                time: time.as_str().into(),
                title: title.as_str().into(),
                url: url.as_str().into(),
                host: host.as_str().into(),
                entry: i32::try_from(*entry).unwrap_or(i32::MAX),
                ..HistoryRow::default()
            },
        })
        .collect::<Vec<_>>();
    if has_more {
        rows.push(HistoryRow {
            is_more: true,
            ..HistoryRow::default()
        });
    }
    ModelRc::new(VecModel::from(rows))
}

impl HistoryUi {
    // Suggestions.

    fn address_edited(self: &Rc<Self>, text: &str) {
        let generation = {
            let mut state = self.suggest.borrow_mut();
            state.typing = suggestions::extends(&state.typed, text);
            state.typed = text.to_owned();
            state.selected = None;
            state.generation += 1;
            state.generation
        };
        let Some(window) = self.window.upgrade() else {
            return;
        };
        window.set_suggestion_selected(-1);
        if text.trim().is_empty() {
            self.close_suggestions();
            return;
        }
        let ui = Rc::downgrade(self);
        self.suggest_timer.start(
            slint::TimerMode::SingleShot,
            Duration::from_millis(SUGGEST_DEBOUNCE_MS),
            move || {
                if let Some(ui) = ui.upgrade() {
                    ui.ask_for_suggestions(generation);
                }
            },
        );
    }

    fn ask_for_suggestions(self: &Rc<Self>, generation: u64) {
        let typed = self.suggest.borrow().typed.clone();
        let Some(data) = profile_data::handle() else {
            self.show_suggestions(generation, &[]);
            return;
        };
        let ui = Rc::downgrade(self);
        data.suggest(typed, SUGGESTION_ROWS, move |result| {
            let Some(ui) = ui.upgrade() else {
                return;
            };
            // Without the database the search row is still offered.
            let suggestions = result.unwrap_or_else(|error| {
                log::debug!("no suggestions: {error}");
                Vec::new()
            });
            ui.show_suggestions(generation, &suggestions);
        });
    }

    fn show_suggestions(&self, generation: u64, suggestions: &[Suggestion]) {
        let Some(window) = self.window.upgrade() else {
            return;
        };
        let mut state = self.suggest.borrow_mut();
        if state.generation != generation {
            return;
        }
        state.rows = suggestions::rows(&state.typed, suggestions);
        state.selected = None;
        window.set_suggestions(slint_rows(&state.rows));
        window.set_suggestion_selected(-1);
        window.set_suggestions_open(!state.rows.is_empty());
        // Complete the host only if the field still holds exactly what was typed.
        let completion = suggestions
            .first()
            .filter(|_| state.typing && window.get_address() == state.typed.as_str())
            .and_then(|top| suggestions::inline_completion(&state.typed, top));
        if let Some(completion) = completion {
            let typed_length = state.typed.len();
            let text = format!("{}{completion}", state.typed);
            let length = text.len();
            drop(state);
            window.invoke_complete_address(
                SharedString::from(text),
                i32::try_from(typed_length).unwrap_or(0),
                i32::try_from(length).unwrap_or(0),
            );
        }
        window.window().request_redraw();
    }

    fn move_selection(&self, down: bool) {
        let Some(window) = self.window.upgrade() else {
            return;
        };
        let mut state = self.suggest.borrow_mut();
        state.selected = suggestions::move_selection(state.selected, state.rows.len(), down);
        window.set_suggestion_selected(
            state
                .selected
                .and_then(|index| i32::try_from(index).ok())
                .unwrap_or(-1),
        );
        window.window().request_redraw();
    }

    fn close_suggestions(&self) {
        self.suggest_timer.stop();
        let mut state = self.suggest.borrow_mut();
        // A reply still on its way is now stale.
        state.generation += 1;
        state.selected = None;
        if let Some(window) = self.window.upgrade() {
            window.set_suggestions_open(false);
            window.set_suggestion_selected(-1);
            window.window().request_redraw();
        }
    }

    /// Opens a row: the search row searches, the others go to their page. Like typing it.
    fn choose(&self, index: Option<usize>) {
        let target = index.and_then(|index| {
            self.suggest
                .borrow()
                .rows
                .get(index)
                .map(|row| row.target.clone())
        });
        self.close_suggestions();
        if let (Some(target), Some(window)) = (target, self.window.upgrade()) {
            window.invoke_navigate(target.into());
        }
    }

    // The history window.

    fn show_history(self: &Rc<Self>) {
        let Some(window) = self.window.upgrade() else {
            return;
        };
        if !window.get_history_open() {
            self.list.borrow_mut().query = None;
            window.set_history_query(SharedString::default());
            window.set_history_open(true);
            self.load(true);
        }
        window.window().request_redraw();
    }

    fn close_history(&self) {
        if let Some(window) = self.window.upgrade() {
            window.set_history_open(false);
            window.window().request_redraw();
        }
    }

    fn search_edited(self: &Rc<Self>, text: &str) {
        let query = Some(text.trim().to_owned()).filter(|query| !query.is_empty());
        self.list.borrow_mut().query = query;
        let ui = Rc::downgrade(self);
        self.search_timer
            .start(slint::TimerMode::SingleShot, SEARCH_DEBOUNCE, move || {
                if let Some(ui) = ui.upgrade() {
                    ui.load(true);
                }
            });
    }

    /// A history change while the window is open: list again as many visits as are shown.
    fn refresh_history(self: &Rc<Self>) {
        if self
            .window
            .upgrade()
            .is_some_and(|window| window.get_history_open())
        {
            self.load(true);
        }
    }

    /// Lists the first page again (`reset`), or the next page after what is listed.
    fn load(self: &Rc<Self>, reset: bool) {
        let (generation, query, cursor, limit) = {
            let mut list = self.list.borrow_mut();
            list.generation += 1;
            let shown = u32::try_from(list.entries.len()).unwrap_or(u32::MAX);
            let cursor = if reset {
                None
            } else {
                list.entries.last().map(HistoryEntry::cursor)
            };
            let limit = if reset { shown.max(PAGE) } else { PAGE };
            (list.generation, list.query.clone(), cursor, limit)
        };
        let Some(data) = profile_data::handle() else {
            self.show_history_message("History is not available in this run.");
            return;
        };
        let ui = Rc::downgrade(self);
        data.history(query, cursor, limit, move |result| {
            if let Some(ui) = ui.upgrade() {
                ui.show_listing(generation, reset, limit, result);
            }
        });
    }

    fn show_listing(
        &self,
        generation: u64,
        reset: bool,
        limit: u32,
        result: Result<Vec<HistoryEntry>, ProfileDataError>,
    ) {
        let Some(window) = self.window.upgrade() else {
            return;
        };
        let mut list = self.list.borrow_mut();
        if list.generation != generation {
            return;
        }
        let entries = match result {
            Ok(entries) => entries,
            Err(ProfileDataError::Unavailable(reason)) => {
                drop(list);
                self.show_history_message(&format!("History is not available: {reason}"));
                return;
            }
            Err(error) => {
                drop(list);
                self.show_history_message(&format!("History could not be read: {error}"));
                return;
            }
        };
        list.has_more = entries.len() as u64 >= u64::from(limit);
        if reset {
            list.entries = entries;
        } else {
            list.entries.extend(entries);
        }
        let message = match (&list.query, list.entries.is_empty()) {
            (_, false) => String::new(),
            (Some(query), true) => format!("No pages in your history match “{query}”."),
            (None, true) => "Pages you visit are listed here.".to_owned(),
        };
        let lines = listing::lines(&list.entries, profile_data::now_ms());
        window.set_history_rows(slint_lines(&lines, list.has_more));
        window.set_history_message(message.into());
        window.window().request_redraw();
    }

    fn show_history_message(&self, message: &str) {
        if let Some(window) = self.window.upgrade() {
            window.set_history_rows(ModelRc::default());
            window.set_history_message(message.into());
            window.window().request_redraw();
        }
    }

    fn entry(&self, index: i32) -> Option<HistoryEntry> {
        let index = usize::try_from(index).ok()?;
        self.list.borrow().entries.get(index).cloned()
    }

    /// Opens a listed visit's page in the current tab (closing the window) or a new one behind
    /// it (the window stays, to open more). Either counts as following a link.
    fn open_entry(&self, index: i32, new_tab: bool) {
        let (Some(entry), Some(window)) = (self.entry(index), self.window.upgrade()) else {
            return;
        };
        if new_tab {
            let opened = self
                .controller
                .borrow_mut()
                .open_tab_with(Some(&entry.url), TabActivation::Background);
            match opened {
                Ok(tab_id) => {
                    self.controller
                        .borrow_mut()
                        .set_navigation_origin(tab_id, NavigationOrigin::Link);
                    sync_ui(&window, &self.controller.borrow());
                }
                Err(error) => log::warn!("could not open {}: {error}", entry.url),
            }
        } else {
            window.set_history_open(false);
            window.invoke_navigate(entry.url.into());
            let tab_id = self.controller.borrow().active_tab_id();
            self.controller
                .borrow_mut()
                .set_navigation_origin(tab_id, NavigationOrigin::Link);
        }
        window.window().request_redraw();
    }

    /// Deletes a visit, or every visit of its site. The list follows through the change
    /// notification.
    fn delete_entry(&self, index: i32, whole_site: bool) {
        let (Some(entry), Some(data)) = (self.entry(index), profile_data::handle()) else {
            return;
        };
        let report = |result: Result<_, ProfileDataError>| {
            if let Err(error) = result {
                log::warn!("could not delete history: {error}");
            }
        };
        if whole_site {
            let Some(host) = url::Url::parse(&entry.url)
                .ok()
                .and_then(|url| url.host_str().map(str::to_owned))
            else {
                return;
            };
            data.delete_site(host, move |result| report(result.map(|_| ())));
        } else {
            data.delete_visit(entry.visit_id, move |result| report(result.map(|_| ())));
        }
    }

    // "Clear browsing data".

    fn close_clear_dialog(&self) {
        if let Some(window) = self.window.upgrade() {
            window.set_clear_dialog_open(false);
            window.set_clear_busy(false);
            window.window().request_redraw();
        }
    }

    fn confirm_clear(self: &Rc<Self>) {
        let Some(window) = self.window.upgrade() else {
            return;
        };
        let period = Period::from_index(window.get_clear_period());
        let kinds = talaria_servo::SiteDataKinds {
            cookies_and_site_data: window.get_clear_cookies(),
            cache: window.get_clear_cache(),
        };
        window.set_clear_busy(true);
        if (kinds.cookies_and_site_data || kinds.cache) && !crate::engine::clear_site_data(kinds) {
            log::warn!("cookies, site data or the cache could not be cleared");
        }
        let data = profile_data::handle().filter(|_| window.get_clear_history());
        let Some(data) = data else {
            self.close_clear_dialog();
            return;
        };
        let ui = Rc::downgrade(self);
        data.clear_history(period.range(profile_data::now_ms()), move |result| {
            match result {
                Ok(removed) => log::info!("cleared {removed} visits"),
                Err(error) => log::warn!("could not clear the history: {error}"),
            }
            if let Some(ui) = ui.upgrade() {
                ui.close_clear_dialog();
            }
        });
    }
}
