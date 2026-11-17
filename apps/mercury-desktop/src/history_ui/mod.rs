//! The history lane's user interface: suggestions under the address field, the history window
//! (Cmd+Y) and the "Clear browsing data" dialog (Cmd+Shift+Delete). Everything goes through the
//! profile data service; nothing typed or listed leaves the local database.
//!
//! **Suggestions.** Each edit of the address field starts a new generation and (re)starts a
//! [`suggestions::SUGGEST_DEBOUNCE_MS`] timer; the store is asked when it fires. Closing the
//! list (the field losing focus, Return, Escape, a choice) also starts a new generation, so a
//! reply for an older one is dropped and a closed list never comes back. Up and Down move the
//! selection, Return opens it, Escape closes the list (and is then not passed on, so a later
//! Escape handler, such as the tabs lane's revert, runs only when the list is closed); a click
//! opens a row. A typed or bookmarked host that starts with what was typed is completed inline
//! (see [`suggestions::inline_completion`]); only typing completes, never deleting.
//!
//! **Diagnostic:** in an automated run (`--screenshot`, `--background`) with
//! `MERCURY_SHOW_HISTORY=1` in the environment, the history window opens at start-up, so it can
//! be captured without input; with `MERCURY_SHOW_SUGGESTIONS=<text>`, the address field takes
//! focus and holds `<text>` as if typed 1.5 s after start-up (once the page has loaded), so the
//! suggestion list can be captured.

mod days;
mod listing;
mod suggestions;

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use mercury_store::{HistoryEntry, Suggestion};
use mercury_ui::{HistoryRow, MainWindow, SuggestionRow};
use slint::winit_030::WinitWindowAccessor;
use slint::winit_030::winit::event::{ElementState, KeyEvent};
use slint::winit_030::winit::keyboard::{Key, NamedKey};
use slint::{ComponentHandle, ModelRc, SharedString, Timer, VecModel};
use talaria_core::{BrowserController, NavigationOrigin, TabActivation};

use crate::profile_data::{self, Area, ProfileDataError};
use crate::ui_sync::sync_ui;
use listing::{Line, PAGE, Period, Removed};
use suggestions::{Edit, Row, SUGGEST_DEBOUNCE_MS, SUGGESTION_ROWS, SearchRow, Session};

/// How long typing in the history window's search field must pause before it searches.
const SEARCH_DEBOUNCE: Duration = Duration::from_millis(150);

thread_local! {
    static UI: RefCell<Option<Rc<HistoryUi>>> = const { RefCell::new(None) };
}

struct HistoryUi {
    window: slint::Weak<MainWindow>,
    controller: Rc<RefCell<BrowserController>>,
    suggest: RefCell<Session>,
    suggest_timer: Timer,
    list: RefCell<ListState>,
    search_timer: Timer,
    /// A `--screenshot` or `--background` run, which never takes focus.
    automated: bool,
}

#[derive(Default)]
struct ListState {
    generation: u64,
    query: Option<String>,
    entries: Vec<HistoryEntry>,
    has_more: bool,
    /// A read is on its way; a history change meanwhile waits for it (`refresh_waiting`).
    loading: bool,
    refresh_waiting: bool,
}

/// What a listing read is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Load {
    /// A new list: opening the window, or a new search.
    First,
    /// "Show more": the next page after what is shown.
    More,
    /// A history change: the newest page, merged with the older pages shown.
    Refresh,
}

fn with_ui<R>(work: impl FnOnce(&Rc<HistoryUi>) -> R) -> Option<R> {
    let ui = UI.with(|current| current.borrow().clone())?;
    Some(work(&ui))
}

/// Connects the address field, the history window and the clear dialog. Call once, after the
/// profile data service has started. `automated`: a `--screenshot` or `--background` run.
pub(crate) fn connect(
    window: &MainWindow,
    controller: Rc<RefCell<BrowserController>>,
    automated: bool,
) {
    let ui = Rc::new(HistoryUi {
        window: window.as_weak(),
        controller,
        suggest: RefCell::default(),
        suggest_timer: Timer::default(),
        list: RefCell::default(),
        search_timer: Timer::default(),
        automated,
    });
    UI.with(|current| *current.borrow_mut() = Some(Rc::clone(&ui)));

    window.on_address_edited(|text| {
        let _slint = crate::callback_scope::InSlintCallback::enter();
        with_ui(|ui| ui.address_edited(text.as_str()));
    });
    window.on_address_blurred(|| {
        let _slint = crate::callback_scope::InSlintCallback::enter();
        with_ui(|ui| ui.close_suggestions());
    });
    window.on_suggestion_chosen(|index| {
        let _slint = crate::callback_scope::InSlintCallback::enter();
        with_ui(|ui| ui.choose(usize::try_from(index).ok()));
    });
    window.on_history_search(|text| {
        let _slint = crate::callback_scope::InSlintCallback::enter();
        with_ui(|ui| ui.search_edited(text.as_str()));
    });
    window.on_history_more(|| {
        let _slint = crate::callback_scope::InSlintCallback::enter();
        with_ui(|ui| ui.load(Load::More));
    });
    window.on_history_open_entry(|entry, new_tab| {
        let _slint = crate::callback_scope::InSlintCallback::enter();
        with_ui(|ui| ui.open_entry(entry, new_tab));
    });
    window.on_history_delete_entry(|entry| {
        let _slint = crate::callback_scope::InSlintCallback::enter();
        with_ui(|ui| ui.delete_entry(entry, false));
    });
    window.on_history_delete_site(|entry| {
        let _slint = crate::callback_scope::InSlintCallback::enter();
        with_ui(|ui| ui.delete_entry(entry, true));
    });
    window.on_history_close(|| {
        let _slint = crate::callback_scope::InSlintCallback::enter();
        with_ui(|ui| ui.close_history());
    });
    window.on_show_clear_dialog(show_clear_dialog);
    window.on_clear_cancel(|| {
        let _slint = crate::callback_scope::InSlintCallback::enter();
        with_ui(|ui| ui.close_clear_dialog());
    });
    window.on_clear_confirm(|| {
        let _slint = crate::callback_scope::InSlintCallback::enter();
        with_ui(|ui| ui.confirm_clear());
    });

    if let Some(data) = profile_data::handle() {
        // The handle is looked up inside, not captured (see `on_change`).
        data.on_change(Area::History, |_| {
            with_ui(|ui| ui.refresh_history());
        });
    }
    if automated && std::env::var_os("MERCURY_SHOW_HISTORY").is_some_and(|value| value == "1") {
        show_history();
    }
    if let Some(text) = std::env::var("MERCURY_SHOW_SUGGESTIONS")
        .ok()
        .filter(|_| automated)
    {
        let window = window.as_weak();
        Timer::single_shot(Duration::from_millis(1500), move || {
            let _slint = crate::callback_scope::InSlintCallback::enter();
            if let Some(window) = window.upgrade() {
                window.invoke_focus_address_bar();
                window.set_address(text.as_str().into());
                with_ui(|ui| ui.address_edited(&text));
            }
        });
    }
}

/// Opens the history window (Cmd+Y), or focuses its search field if it is open.
pub(crate) fn show_history() {
    with_ui(|ui| ui.show_history());
}

/// Opens "Clear browsing data" (Cmd+Shift+Delete, the history window's button, and the settings
/// window's "Clear browsing data…", from which it brings the browser window forward).
pub(crate) fn show_clear_dialog() {
    let _slint = crate::callback_scope::InSlintCallback::enter();
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
        if !ui.automated {
            let _focused = window
                .window()
                .with_winit_window(|native| native.focus_window());
        }
        window.window().request_redraw();
    });
}

/// The suggestion list's keys while the address field has focus: Up and Down move and Escape
/// closes the list while it is open; Return opens the selected row, and otherwise closes the
/// list (even one whose first reply has not come yet) and lets the field navigate. Returns true
/// if the key was used and must not reach the address field.
pub(crate) fn handle_key(window: &MainWindow, event: &KeyEvent) -> bool {
    if event.state != ElementState::Pressed || !window.get_address_focused() {
        return false;
    }
    with_ui(|ui| {
        let open = ui.suggest.borrow().is_open();
        match &event.logical_key {
            Key::Named(NamedKey::ArrowDown) if open => {
                ui.move_selection(true);
                true
            }
            Key::Named(NamedKey::ArrowUp) if open => {
                ui.move_selection(false);
                true
            }
            Key::Named(NamedKey::Escape) if open => {
                ui.close_suggestions();
                true
            }
            Key::Named(NamedKey::Enter) => {
                let choice = ui.suggest.borrow_mut().accept();
                ui.show_closed();
                match choice {
                    Some(choice) => {
                        ui.navigate(choice);
                        true
                    }
                    // The address field navigates to what it holds.
                    None => false,
                }
            }
            _ => false,
        }
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
        let edit = self.suggest.borrow_mut().edited(text);
        let Edit::Ask { generation, .. } = edit else {
            self.show_closed();
            return;
        };
        if let Some(window) = self.window.upgrade() {
            window.set_suggestion_selected(-1);
        }
        let ui = Rc::downgrade(self);
        self.suggest_timer.start(
            slint::TimerMode::SingleShot,
            Duration::from_millis(SUGGEST_DEBOUNCE_MS),
            move || {
                let _slint = crate::callback_scope::InSlintCallback::enter();
                if let Some(ui) = ui.upgrade() {
                    ui.ask_for_suggestions(generation);
                }
            },
        );
    }

    fn ask_for_suggestions(self: &Rc<Self>, generation: u64) {
        let Some(input) = self.suggest.borrow().pending_input(generation) else {
            return;
        };
        let Some(data) = profile_data::handle() else {
            self.show_suggestions(generation, &[]);
            return;
        };
        let ui = Rc::downgrade(self);
        data.suggest(input, SUGGESTION_ROWS, move |result| {
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

    /// The search row for `input`: the controller's search builder makes the URL (`None` if it
    /// cannot) and the settings name the engine.
    fn search_row(&self, input: &str) -> Option<SearchRow> {
        let url = self.controller.borrow().search_url(input).ok()?;
        let engine = crate::settings::search_engine_name(input.trim())?;
        Some(SearchRow {
            engine,
            url: url.into(),
        })
    }

    fn show_suggestions(&self, generation: u64, suggestions: &[Suggestion]) {
        let Some(window) = self.window.upgrade() else {
            return;
        };
        let field = window.get_address();
        let search = self
            .suggest
            .borrow()
            .pending_input(generation)
            .and_then(|input| self.search_row(&input));
        let shown =
            self.suggest
                .borrow_mut()
                .replied(generation, search, suggestions, field.as_str());
        // A stale reply: the list was closed or edited since.
        let Some(completion) = shown else {
            return;
        };
        {
            let session = self.suggest.borrow();
            window.set_suggestions(slint_rows(session.rows()));
            window.set_suggestion_selected(-1);
            window.set_suggestions_open(session.is_open());
        }
        if let Some(completion) = completion {
            window.invoke_complete_address(
                SharedString::from(completion.text),
                i32::try_from(completion.start).unwrap_or(0),
                i32::try_from(completion.end).unwrap_or(0),
            );
        }
        window.window().request_redraw();
    }

    fn move_selection(&self, down: bool) {
        let Some(window) = self.window.upgrade() else {
            return;
        };
        let selected = {
            let mut session = self.suggest.borrow_mut();
            session.move_selection(down);
            session.selected()
        };
        window.set_suggestion_selected(
            selected
                .and_then(|index| i32::try_from(index).ok())
                .unwrap_or(-1),
        );
        window.window().request_redraw();
    }

    fn close_suggestions(&self) {
        self.suggest_timer.stop();
        self.suggest.borrow_mut().close();
        self.show_closed();
    }

    /// Mirrors a closed session in the window.
    fn show_closed(&self) {
        if let Some(window) = self.window.upgrade() {
            window.set_suggestions_open(false);
            window.set_suggestion_selected(-1);
            window.window().request_redraw();
        }
    }

    /// A click on a row: the search row searches, the others go to their page. Like typing it.
    fn choose(&self, index: Option<usize>) {
        let choice = index.and_then(|index| self.suggest.borrow_mut().choose(index));
        self.suggest_timer.stop();
        self.show_closed();
        if let Some(choice) = choice {
            self.navigate(choice);
        }
    }

    /// Goes where a chosen row leads, like typing it; a page only ever reached by links counts
    /// as a link (see `Row::typed`).
    fn navigate(&self, choice: suggestions::Choice) {
        let Some(window) = self.window.upgrade() else {
            return;
        };
        window.invoke_navigate(choice.target.into());
        if !choice.typed {
            let tab_id = self.controller.borrow().active_tab_id();
            self.controller
                .borrow_mut()
                .set_navigation_origin(tab_id, NavigationOrigin::Link);
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
            // Without a database, say why at once rather than asking the service.
            match profile_data::handle().and_then(|data| data.unavailable_reason()) {
                Some(reason) => {
                    self.show_history_message(&format!("History is not available: {reason}"));
                }
                None => self.load(Load::First),
            }
        }
        window.window().request_redraw();
    }

    fn close_history(&self) {
        if let Some(window) = self.window.upgrade() {
            window.set_history_open(false);
            // The search field goes with the window.
            window.set_history_search_focused(false);
            window.window().request_redraw();
        }
    }

    fn search_edited(self: &Rc<Self>, text: &str) {
        let query = Some(text.trim().to_owned()).filter(|query| !query.is_empty());
        self.list.borrow_mut().query = query;
        let ui = Rc::downgrade(self);
        self.search_timer
            .start(slint::TimerMode::SingleShot, SEARCH_DEBOUNCE, move || {
                let _slint = crate::callback_scope::InSlintCallback::enter();
                if let Some(ui) = ui.upgrade() {
                    ui.load(Load::First);
                }
            });
    }

    /// A history change while the window is open: the newest page is read again and merged
    /// with the older pages shown. If a read is on its way (a "Show more"), it waits for it.
    fn refresh_history(self: &Rc<Self>) {
        if !self
            .window
            .upgrade()
            .is_some_and(|window| window.get_history_open())
        {
            return;
        }
        let loading = {
            let mut list = self.list.borrow_mut();
            list.refresh_waiting |= list.loading;
            list.loading
        };
        if !loading {
            self.load(Load::Refresh);
        }
    }

    fn load(self: &Rc<Self>, load: Load) {
        let (generation, query, cursor) = {
            let mut list = self.list.borrow_mut();
            list.generation += 1;
            list.loading = true;
            if load != Load::Refresh {
                list.refresh_waiting = false;
            }
            let cursor = match load {
                Load::More => list.entries.last().map(HistoryEntry::cursor),
                Load::First | Load::Refresh => None,
            };
            (list.generation, list.query.clone(), cursor)
        };
        let Some(data) = profile_data::handle() else {
            self.list.borrow_mut().loading = false;
            self.show_history_message("History is not available in this run.");
            return;
        };
        let ui = Rc::downgrade(self);
        data.history(query, cursor, PAGE, move |result| {
            if let Some(ui) = ui.upgrade() {
                ui.show_listing(generation, load, result);
            }
        });
    }

    fn show_listing(
        self: &Rc<Self>,
        generation: u64,
        load: Load,
        result: Result<Vec<HistoryEntry>, ProfileDataError>,
    ) {
        let Some(window) = self.window.upgrade() else {
            return;
        };
        let mut list = self.list.borrow_mut();
        if list.generation != generation {
            return;
        }
        list.loading = false;
        let entries = match result {
            Ok(entries) => entries,
            Err(error) => {
                drop(list);
                self.show_history_message(&match error {
                    ProfileDataError::Unavailable(reason) => {
                        format!("History is not available: {reason}")
                    }
                    error => format!("History could not be read: {error}"),
                });
                return;
            }
        };
        let page = PAGE as usize;
        match load {
            Load::First => {
                list.has_more = entries.len() >= page;
                list.entries = entries;
            }
            Load::More => {
                list.has_more = entries.len() >= page;
                list.entries.extend(entries);
            }
            Load::Refresh => {
                let (merged, has_more) =
                    listing::merge_refresh(&list.entries, entries, page, list.has_more);
                list.entries = merged;
                list.has_more = has_more;
            }
        }
        let refresh = std::mem::take(&mut list.refresh_waiting);
        drop(list);
        self.show_entries();
        if refresh {
            self.load(Load::Refresh);
        }
        window.window().request_redraw();
    }

    /// Shows the visits listed, or why there are none.
    fn show_entries(&self) {
        let Some(window) = self.window.upgrade() else {
            return;
        };
        let list = self.list.borrow();
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
            self.close_history();
            window.invoke_navigate(entry.url.into());
            let tab_id = self.controller.borrow().active_tab_id();
            self.controller
                .borrow_mut()
                .set_navigation_origin(tab_id, NavigationOrigin::Link);
        }
        window.window().request_redraw();
    }

    /// Deletes a visit, or every visit of its host; once the deletion is done the list loses
    /// them at once (the change notification then refreshes it).
    fn delete_entry(self: &Rc<Self>, index: i32, whole_site: bool) {
        let (Some(entry), Some(data)) = (self.entry(index), profile_data::handle()) else {
            return;
        };
        let removed = if whole_site {
            let Some(host) = url::Url::parse(&entry.url)
                .ok()
                .and_then(|url| url.host_str().map(str::to_owned))
            else {
                return;
            };
            Removed::Site(host)
        } else {
            Removed::Visit(entry.visit_id)
        };
        let target = removed.clone();
        let ui = Rc::downgrade(self);
        let done = move |result: Result<(), ProfileDataError>| match result {
            Ok(()) => {
                if let Some(ui) = ui.upgrade() {
                    listing::remove_visits(&mut ui.list.borrow_mut().entries, &removed);
                    ui.show_entries();
                }
            }
            Err(error) => log::warn!("could not delete history: {error}"),
        };
        match target {
            Removed::Site(host) => data.delete_site(host, move |result| done(result.map(|_| ()))),
            Removed::Visit(id) => data.delete_visit(id, move |result| done(result.map(|_| ()))),
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
