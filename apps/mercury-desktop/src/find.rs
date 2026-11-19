// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! Find in page: connects the find bar to the active tab's find state, and the `--find` diagnostic.
//! The search itself is a script run in the page (`talaria-servo`'s `find_in_page`).

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::{Duration, Instant};

use mercury_ui::MainWindow;
use slint::{ComponentHandle, Timer, TimerMode};
#[cfg(feature = "diagnostics")]
use talaria_core::LoadState;
use talaria_core::{BrowserCommand, BrowserController, BrowserState, FindState, FindStatus, TabId};

#[cfg(feature = "diagnostics")]
use crate::cli::FindDiagnostic;

/// How long typing pauses before the query is searched. Return searches at once.
pub(crate) const TYPING_DEBOUNCE: Duration = Duration::from_millis(150);
/// How long the page may take to answer a search before the bar says it is not responding.
pub(crate) const ANSWER_WAIT: Duration = Duration::from_secs(2);
const ANSWER_POLL_INTERVAL: Duration = Duration::from_millis(250);
#[cfg(feature = "diagnostics")]
const DIAGNOSTIC_POLL_INTERVAL: Duration = Duration::from_millis(50);

/// What the bar shows next to the field. A search cut short at its limit knows only a lower
/// bound on the count and not the match's position.
pub(crate) fn status_text(status: &FindStatus, query_from_other_page: bool) -> String {
    match status {
        FindStatus::Idle if query_from_other_page => "Press Return to search".to_owned(),
        FindStatus::Idle => String::new(),
        FindStatus::Matches(matches) => match (matches.count, matches.current, matches.truncated) {
            (0, _, false) => "No matches".to_owned(),
            (0, _, true) => "No matches so far".to_owned(),
            (count, Some(current), false) => format!("{} of {count}", current + 1),
            (count, _, _) => format!("{count}+ matches"),
        },
        FindStatus::Failed => "Search failed".to_owned(),
        FindStatus::NotResponding => "Page not responding".to_owned(),
    }
}

fn has_no_matches(status: &FindStatus) -> bool {
    matches!(status, FindStatus::Matches(matches) if matches.count == 0 && !matches.truncated)
}

/// What the overdue timer does after a look at the active tab.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WatchStep {
    /// Nothing is awaited: the timer stops until the next search.
    Stop,
    Wait,
    /// This search has gone unanswered for `ANSWER_WAIT`.
    Overdue(TabId, u64),
}

/// Notices a search the page has not answered for `ANSWER_WAIT`.
#[derive(Debug, Default)]
pub(crate) struct OverdueWatch {
    waiting: Option<(TabId, u64, Instant)>,
    reported: bool,
}

impl OverdueWatch {
    /// `awaited` is the active tab's unanswered search.
    pub(crate) fn poll(&mut self, tab_id: TabId, awaited: Option<u64>, now: Instant) -> WatchStep {
        let Some(request_id) = awaited else {
            self.waiting = None;
            return WatchStep::Stop;
        };
        match self.waiting {
            Some((waiting_tab, waiting_id, since))
                if waiting_tab == tab_id && waiting_id == request_id =>
            {
                if self.reported || now.saturating_duration_since(since) < ANSWER_WAIT {
                    return WatchStep::Wait;
                }
                self.reported = true;
                WatchStep::Overdue(tab_id, request_id)
            }
            _ => {
                self.waiting = Some((tab_id, request_id, now));
                self.reported = false;
                WatchStep::Wait
            }
        }
    }
}

thread_local! {
    /// The overdue timer runs only while a search is awaited.
    static ANSWER_WATCH: RefCell<Option<AnswerWatch>> = const { RefCell::new(None) };
}

struct AnswerWatch {
    timer: Rc<Timer>,
    controller: Rc<RefCell<BrowserController>>,
    window: slint::Weak<MainWindow>,
}

/// Starts the overdue timer if a search is awaited and it is not running. Called after every find
/// command is queued; answers that queue the next search arrive while it runs.
pub(crate) fn watch_for_answers() {
    ANSWER_WATCH.with(|watch| {
        let watch = watch.borrow();
        let Some(watch) = watch.as_ref() else {
            return;
        };
        if watch.timer.running()
            || watch
                .controller
                .borrow()
                .active_state()
                .find()
                .awaited_search()
                .is_none()
        {
            return;
        }
        let controller = Rc::clone(&watch.controller);
        let window = watch.window.clone();
        let timer = Rc::downgrade(&watch.timer);
        let mut overdue = OverdueWatch::default();
        watch
            .timer
            .start(TimerMode::Repeated, ANSWER_POLL_INTERVAL, move || {
                let _slint = crate::callback_scope::InSlintCallback::enter();
                let (tab_id, awaited) = {
                    let controller = controller.borrow();
                    (
                        controller.active_tab_id(),
                        controller.active_state().find().awaited_search(),
                    )
                };
                match overdue.poll(tab_id, awaited, Instant::now()) {
                    WatchStep::Wait => {}
                    WatchStep::Stop => {
                        if let Some(timer) = timer.upgrade() {
                            timer.stop();
                        }
                    }
                    WatchStep::Overdue(tab_id, request_id) => {
                        controller
                            .borrow_mut()
                            .find_answer_overdue(tab_id, request_id);
                        if let Some(window) = window.upgrade() {
                            sync_find_bar(&window, controller.borrow().active_state());
                            window.window().request_redraw();
                        }
                    }
                }
            });
    });
}

/// Cmd+G / Cmd+Shift+G with nothing to search for behave like Cmd+F.
pub(crate) fn step_opens_the_bar(find: &FindState) -> bool {
    find.query().is_empty()
}

pub(crate) fn sync_find_bar(window: &MainWindow, state: &BrowserState) {
    let find = state.find();
    window.set_find_open(find.is_open());
    if window.get_find_query().as_str() != find.query() {
        // Another tab's query, or the page's selection read when the bar opened: shown selected,
        // so typing replaces it.
        window.set_find_query(find.query().into());
        if window.get_find_field_focused() {
            window.invoke_find_select_all();
        }
    }
    let status = status_text(find.status(), find.is_query_from_other_page());
    if find.is_query_from_other_page()
        && window.get_find_status().as_str() != status
        && window.get_find_field_focused()
    {
        // The page changed under a focused bar: the kept query is shown selected, so typing
        // replaces it and Return searches the new page.
        window.invoke_find_select_all();
    }
    window.set_find_status(status.into());
    window.set_find_no_matches(has_no_matches(find.status()));
    window.set_find_match_case(find.match_case());
    crate::pdf::sync_find_status(window); // pdf-viewer lane: a PDF in front shows the viewer's count
}

/// Cmd+F: the user's own action, so the field takes focus, with its text selected.
pub(crate) fn open_find_bar(
    window: &MainWindow,
    controller: &RefCell<BrowserController>,
    web_view_focused: &Cell<bool>,
) {
    if web_view_focused.replace(false) {
        controller
            .borrow_mut()
            .queue_command(BrowserCommand::FocusChanged { focused: false });
    }
    controller.borrow_mut().open_find();
    sync_find_bar(window, controller.borrow().active_state());
    window.invoke_find_focus_field();
    crate::engine::request_pump();
    watch_for_answers();
}

/// Cmd+G / Cmd+Shift+G. Focus stays where it is.
pub(crate) fn step_find(
    window: &MainWindow,
    controller: &RefCell<BrowserController>,
    web_view_focused: &Cell<bool>,
    forward: bool,
) {
    if step_opens_the_bar(controller.borrow().active_state().find()) {
        open_find_bar(window, controller, web_view_focused);
        return;
    }
    // pdf-viewer lane: a PDF in front is searched by the viewer
    let tab_id = controller.borrow().active_tab_id();
    if crate::pdf::step(controller, tab_id, forward) {
        sync_find_bar(window, controller.borrow().active_state());
        return;
    }
    if forward {
        controller.borrow_mut().find_next();
    } else {
        controller.borrow_mut().find_previous();
    }
    sync_find_bar(window, controller.borrow().active_state());
    crate::engine::request_pump();
    watch_for_answers();
}

pub(crate) fn connect_find_bar(window: &MainWindow, controller: Rc<RefCell<BrowserController>>) {
    let debounce = Rc::new(Timer::default());

    let edited_controller = Rc::clone(&controller);
    let edited_window = window.as_weak();
    let edited_debounce = Rc::clone(&debounce);
    window.on_find_edited(move |text| {
        let _slint = crate::callback_scope::InSlintCallback::enter();
        let tab_id = {
            let mut controller = edited_controller.borrow_mut();
            controller.set_find_query(text.as_str());
            controller.active_tab_id()
        };
        if let Some(window) = edited_window.upgrade() {
            sync_find_bar(&window, edited_controller.borrow().active_state());
        }
        let controller = Rc::clone(&edited_controller);
        let window = edited_window.clone();
        edited_debounce.start(TimerMode::SingleShot, TYPING_DEBOUNCE, move || {
            let _slint = crate::callback_scope::InSlintCallback::enter();
            run_due_search(&controller, tab_id, &window);
        });
    });

    // A page that does not answer is reported; nothing more is sent to it until it does.
    ANSWER_WATCH.with(|watch| {
        *watch.borrow_mut() = Some(AnswerWatch {
            timer: Rc::new(Timer::default()),
            controller: Rc::clone(&controller),
            window: window.as_weak(),
        });
    });

    let next_controller = Rc::clone(&controller);
    let next_window = window.as_weak();
    let next_debounce = Rc::clone(&debounce);
    window.on_find_next(move || {
        let _slint = crate::callback_scope::InSlintCallback::enter();
        next_debounce.stop();
        step_from_bar(&next_controller, &next_window, true);
    });

    let previous_controller = Rc::clone(&controller);
    let previous_window = window.as_weak();
    let previous_debounce = Rc::clone(&debounce);
    window.on_find_previous(move || {
        let _slint = crate::callback_scope::InSlintCallback::enter();
        previous_debounce.stop();
        step_from_bar(&previous_controller, &previous_window, false);
    });

    let case_controller = Rc::clone(&controller);
    let case_window = window.as_weak();
    let case_debounce = Rc::clone(&debounce);
    window.on_find_toggle_match_case(move || {
        let _slint = crate::callback_scope::InSlintCallback::enter();
        case_debounce.stop();
        case_controller.borrow_mut().toggle_find_match_case();
        // pdf-viewer lane: a PDF in front is searched again with the new option
        let tab_id = case_controller.borrow().active_tab_id();
        crate::pdf::search(&case_controller, tab_id);
        refresh(&case_controller, &case_window);
    });

    let close_window = window.as_weak();
    window.on_find_close(move || {
        let _slint = crate::callback_scope::InSlintCallback::enter();
        debounce.stop();
        let tab_id = controller.borrow().active_tab_id(); // pdf-viewer lane
        controller.borrow_mut().close_find();
        crate::pdf::find_closed(tab_id); // pdf-viewer lane: the highlights go
        refresh(&controller, &close_window);
    });
}

fn step_from_bar(
    controller: &RefCell<BrowserController>,
    window: &slint::Weak<MainWindow>,
    forward: bool,
) {
    // pdf-viewer lane: a PDF in front is searched by the viewer
    let tab_id = controller.borrow().active_tab_id();
    if crate::pdf::step(controller, tab_id, forward) {
        refresh(controller, window);
        return;
    }
    if forward {
        controller.borrow_mut().find_next();
    } else {
        controller.borrow_mut().find_previous();
    }
    refresh(controller, window);
}

/// The debounce fired. A tab switch in the meantime leaves the search due on the tab that was
/// typed in; Return there searches it.
fn run_due_search(
    controller: &RefCell<BrowserController>,
    tab_id: TabId,
    window: &slint::Weak<MainWindow>,
) {
    if controller.borrow().active_tab_id() != tab_id {
        return;
    }
    // pdf-viewer lane: a PDF in front is searched by the viewer
    if crate::pdf::search(controller, tab_id) {
        refresh(controller, window);
        return;
    }
    controller.borrow_mut().run_due_find();
    refresh(controller, window);
}

fn refresh(controller: &RefCell<BrowserController>, window: &slint::Weak<MainWindow>) {
    if let Some(window) = window.upgrade() {
        sync_find_bar(&window, controller.borrow().active_state());
        window.window().request_redraw();
    }
    crate::engine::request_pump();
    watch_for_answers();
}

#[cfg(feature = "diagnostics")]
/// `--find`: once the active page has loaded, opens the bar (without taking focus), searches the
/// query (an empty one uses the page's selection, as Cmd+F does), presses Next or Previous
/// `steps` times at once, and prints what the bar shows when the last answer has arrived.
pub(crate) fn start_find_diagnostic(
    window: &MainWindow,
    controller: Rc<RefCell<BrowserController>>,
    diagnostic: FindDiagnostic,
) -> Timer {
    let weak_window = window.as_weak();
    let timer = Timer::default();
    let mut started = false;
    let mut reported = false;
    let mut steps_left = diagnostic.steps;
    timer.start(TimerMode::Repeated, DIAGNOSTIC_POLL_INTERVAL, move || {
        let _slint = crate::callback_scope::InSlintCallback::enter();
        if reported {
            return;
        }
        let Some(window) = weak_window.upgrade() else {
            return;
        };
        if !started {
            if !matches!(
                controller.borrow().active_state().load_state(),
                LoadState::Loaded
            ) {
                return;
            }
            started = true;
            start_diagnostic_search(&mut controller.borrow_mut(), &diagnostic);
            sync_find_bar(&window, controller.borrow().active_state());
            window.window().request_redraw();
            crate::engine::request_pump();
            watch_for_answers();
            return;
        }
        if controller
            .borrow()
            .active_state()
            .find()
            .is_awaiting_answer()
        {
            return;
        }
        // One press per answer, as a person pressing Return again would.
        if steps_left != 0 {
            press_diagnostic_step(&mut controller.borrow_mut(), &mut steps_left);
            sync_find_bar(&window, controller.borrow().active_state());
            crate::engine::request_pump();
            watch_for_answers();
            return;
        }
        let controller = controller.borrow();
        let find = controller.active_state().find();
        if find.status() != &FindStatus::Idle {
            eprintln!(
                "mercury-browser: find diagnostic: \"{}\"{} {}",
                find.query(),
                if find.match_case() {
                    " (match case)"
                } else {
                    ""
                },
                status_text(find.status(), find.is_query_from_other_page())
            );
            reported = true;
        }
    });
    timer
}

#[cfg(feature = "diagnostics")]
fn start_diagnostic_search(controller: &mut BrowserController, diagnostic: &FindDiagnostic) {
    controller.open_find();
    if !diagnostic.query.is_empty() {
        // As if typed, then Return pressed: an explicit search.
        controller.set_find_query(&diagnostic.query);
        controller.find_next();
    }
}

#[cfg(feature = "diagnostics")]
fn press_diagnostic_step(controller: &mut BrowserController, steps_left: &mut i32) {
    if *steps_left > 0 {
        controller.find_next();
        *steps_left -= 1;
    } else if *steps_left < 0 {
        controller.find_previous();
        *steps_left += 1;
    }
}

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use talaria_core::FindMatches;
    #[cfg(feature = "diagnostics")]
    use talaria_core::{BrowserEvent, FindRequest, FindStep, TabCommand};

    use super::*;

    fn matches(count: usize, current: Option<usize>, truncated: bool) -> FindStatus {
        FindStatus::Matches(FindMatches {
            count,
            current,
            truncated,
        })
    }

    #[test]
    fn the_status_says_which_match_of_how_many() {
        assert_eq!(status_text(&FindStatus::Idle, false), "");
        assert_eq!(status_text(&matches(7, Some(2), false), false), "3 of 7");
        assert_eq!(status_text(&matches(0, None, false), false), "No matches");
        assert_eq!(status_text(&FindStatus::Failed, false), "Search failed");
    }

    #[test]
    fn a_search_cut_short_gives_a_lower_bound_and_no_position() {
        assert_eq!(
            status_text(&matches(1000, None, true), false),
            "1000+ matches"
        );
        assert_eq!(status_text(&matches(1, None, true), false), "1+ matches");
        assert_eq!(
            status_text(&matches(0, None, true), false),
            "No matches so far"
        );
    }

    #[test]
    fn a_stuck_page_and_a_query_from_another_page_are_said_plainly() {
        assert_eq!(
            status_text(&FindStatus::NotResponding, false),
            "Page not responding"
        );
        assert_eq!(
            status_text(&FindStatus::Idle, true),
            "Press Return to search"
        );
    }

    #[test]
    fn only_a_complete_answer_with_no_matches_marks_the_field() {
        assert!(has_no_matches(&matches(0, None, false)));
        assert!(!has_no_matches(&matches(0, None, true)));
        assert!(!has_no_matches(&matches(2, Some(0), false)));
        assert!(!has_no_matches(&FindStatus::Idle));
        assert!(!has_no_matches(&FindStatus::Failed));
    }

    #[test]
    fn stepping_without_a_query_opens_the_bar_instead() {
        let mut controller = BrowserController::default();
        assert!(step_opens_the_bar(controller.active_state().find()));

        controller.set_find_query("x");
        assert!(!step_opens_the_bar(controller.active_state().find()));
    }

    #[test]
    fn an_answer_missing_for_the_whole_wait_is_overdue_once() {
        let tab = TabId::new(1);
        let start = Instant::now();
        let mut watch = OverdueWatch::default();

        assert_eq!(watch.poll(tab, Some(4), start), WatchStep::Wait);
        assert_eq!(
            watch.poll(tab, Some(4), start + Duration::from_millis(1999)),
            WatchStep::Wait
        );
        assert_eq!(
            watch.poll(tab, Some(4), start + ANSWER_WAIT),
            WatchStep::Overdue(tab, 4)
        );
        assert_eq!(
            watch.poll(tab, Some(4), start + ANSWER_WAIT * 2),
            WatchStep::Wait,
            "reported once"
        );
    }

    #[test]
    fn a_new_request_or_tab_restarts_the_wait() {
        let start = Instant::now();
        let mut watch = OverdueWatch::default();
        watch.poll(TabId::new(1), Some(4), start);

        assert_eq!(
            watch.poll(TabId::new(1), Some(5), start + ANSWER_WAIT),
            WatchStep::Wait
        );
        assert_eq!(
            watch.poll(TabId::new(2), Some(5), start + ANSWER_WAIT),
            WatchStep::Wait
        );
        assert_eq!(
            watch.poll(TabId::new(2), Some(5), start + ANSWER_WAIT * 3),
            WatchStep::Overdue(TabId::new(2), 5)
        );
    }

    #[test]
    fn the_watch_stops_when_nothing_is_awaited() {
        let start = Instant::now();
        let mut watch = OverdueWatch::default();
        watch.poll(TabId::new(1), Some(4), start);

        assert_eq!(watch.poll(TabId::new(1), None, start), WatchStep::Stop);
        assert_eq!(
            watch.poll(TabId::new(1), Some(4), start + ANSWER_WAIT),
            WatchStep::Wait,
            "a later wait starts afresh"
        );
    }

    #[cfg(feature = "diagnostics")]
    #[test]
    fn the_diagnostic_searches_then_waits_to_step() {
        let mut controller = BrowserController::default();
        let tab_id = controller.active_tab_id();
        controller.handle_event(
            tab_id,
            BrowserEvent::AddressChanged {
                url: "https://a.test/".to_owned(),
            },
        );

        start_diagnostic_search(
            &mut controller,
            &FindDiagnostic {
                query: "x".to_owned(),
                steps: -2,
            },
        );

        let commands = std::iter::from_fn(|| controller.next_command())
            .map(TabCommand::into_command)
            .collect::<Vec<_>>();
        let steps = commands
            .iter()
            .map(|command| match command {
                BrowserCommand::BeginFind { .. } => None,
                BrowserCommand::Find(FindRequest { step, .. }) => Some(*step),
                other => panic!("unexpected {other:?}"),
            })
            .collect::<Vec<_>>();
        assert_eq!(steps, vec![None, Some(FindStep::Here)]);
        assert!(
            controller.active_state().find().is_awaiting_answer(),
            "the steps wait for the first answer"
        );

        let mut steps_left = -2;
        press_diagnostic_step(&mut controller, &mut steps_left);
        press_diagnostic_step(&mut controller, &mut steps_left);
        press_diagnostic_step(&mut controller, &mut steps_left);
        assert_eq!(steps_left, 0, "one press per call, and none past the count");
    }
}
