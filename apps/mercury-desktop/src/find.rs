//! Find in page: connects the find bar to the active tab's find state, and the `--find` diagnostic.
//! The search itself is a script run in the page (`talaria-servo`'s `find_in_page`).

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Duration;

use mercury_ui::MainWindow;
use slint::{ComponentHandle, Timer, TimerMode};
use talaria_core::{
    BrowserCommand, BrowserController, BrowserState, FindState, FindStatus, LoadState, TabId,
};

use crate::cli::FindDiagnostic;

/// How long typing pauses before the query is searched. Return searches at once.
pub(crate) const TYPING_DEBOUNCE: Duration = Duration::from_millis(150);
const DIAGNOSTIC_POLL_INTERVAL: Duration = Duration::from_millis(50);

/// What the bar shows next to the field.
pub(crate) fn status_text(status: &FindStatus) -> String {
    match status {
        FindStatus::Idle => String::new(),
        FindStatus::Matches(matches) => match matches.current {
            None => "No matches".to_owned(),
            Some(current) => format!(
                "{} of {}{}",
                current + 1,
                matches.count,
                if matches.truncated { "+" } else { "" }
            ),
        },
        FindStatus::Failed => "Search failed".to_owned(),
    }
}

fn has_no_matches(status: &FindStatus) -> bool {
    matches!(status, FindStatus::Matches(matches) if matches.count == 0)
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
    window.set_find_status(status_text(find.status()).into());
    window.set_find_no_matches(has_no_matches(find.status()));
    window.set_find_match_case(find.match_case());
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
    if forward {
        controller.borrow_mut().find_next();
    } else {
        controller.borrow_mut().find_previous();
    }
    sync_find_bar(window, controller.borrow().active_state());
    crate::engine::request_pump();
}

pub(crate) fn connect_find_bar(window: &MainWindow, controller: Rc<RefCell<BrowserController>>) {
    let debounce = Rc::new(Timer::default());

    let edited_controller = Rc::clone(&controller);
    let edited_window = window.as_weak();
    let edited_debounce = Rc::clone(&debounce);
    window.on_find_edited(move |text| {
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
            run_due_search(&controller, tab_id, &window);
        });
    });

    let next_controller = Rc::clone(&controller);
    let next_window = window.as_weak();
    let next_debounce = Rc::clone(&debounce);
    window.on_find_next(move || {
        next_debounce.stop();
        step_from_bar(&next_controller, &next_window, true);
    });

    let previous_controller = Rc::clone(&controller);
    let previous_window = window.as_weak();
    let previous_debounce = Rc::clone(&debounce);
    window.on_find_previous(move || {
        previous_debounce.stop();
        step_from_bar(&previous_controller, &previous_window, false);
    });

    let case_controller = Rc::clone(&controller);
    let case_window = window.as_weak();
    let case_debounce = Rc::clone(&debounce);
    window.on_find_toggle_match_case(move || {
        case_debounce.stop();
        case_controller.borrow_mut().toggle_find_match_case();
        refresh(&case_controller, &case_window);
    });

    let close_window = window.as_weak();
    window.on_find_close(move || {
        debounce.stop();
        controller.borrow_mut().close_find();
        refresh(&controller, &close_window);
    });
}

fn step_from_bar(
    controller: &RefCell<BrowserController>,
    window: &slint::Weak<MainWindow>,
    forward: bool,
) {
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
    controller.borrow_mut().run_due_find();
    refresh(controller, window);
}

fn refresh(controller: &RefCell<BrowserController>, window: &slint::Weak<MainWindow>) {
    if let Some(window) = window.upgrade() {
        sync_find_bar(&window, controller.borrow().active_state());
        window.window().request_redraw();
    }
    crate::engine::request_pump();
}

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
    timer.start(TimerMode::Repeated, DIAGNOSTIC_POLL_INTERVAL, move || {
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
            run_diagnostic_steps(&mut controller.borrow_mut(), &diagnostic);
            sync_find_bar(&window, controller.borrow().active_state());
            window.window().request_redraw();
            crate::engine::request_pump();
            return;
        }
        let controller = controller.borrow();
        let find = controller.active_state().find();
        if find.status() != &FindStatus::Idle && !find.is_awaiting_answer() {
            eprintln!(
                "mercury-browser: find diagnostic: \"{}\"{} {}",
                find.query(),
                if find.match_case() {
                    " (match case)"
                } else {
                    ""
                },
                status_text(find.status())
            );
            reported = true;
        }
    });
    timer
}

fn run_diagnostic_steps(controller: &mut BrowserController, diagnostic: &FindDiagnostic) {
    controller.open_find();
    if !diagnostic.query.is_empty() {
        controller.set_find_query(&diagnostic.query);
        controller.run_due_find();
    }
    for _ in 0..diagnostic.steps.unsigned_abs() {
        if diagnostic.steps > 0 {
            controller.find_next();
        } else {
            controller.find_previous();
        }
    }
}

#[cfg(test)]
mod tests {
    use talaria_core::{BrowserEvent, FindMatches, FindRequest, FindStep, TabCommand};

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
        assert_eq!(status_text(&FindStatus::Idle), "");
        assert_eq!(status_text(&matches(7, Some(2), false)), "3 of 7");
        assert_eq!(status_text(&matches(1000, Some(0), true)), "1 of 1000+");
        assert_eq!(status_text(&matches(0, None, false)), "No matches");
        assert_eq!(status_text(&FindStatus::Failed), "Search failed");
    }

    #[test]
    fn only_an_answer_with_no_matches_marks_the_field() {
        assert!(has_no_matches(&matches(0, None, false)));
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
    fn the_diagnostic_searches_then_steps_at_once() {
        let mut controller = BrowserController::default();
        let tab_id = controller.active_tab_id();
        controller.handle_event(
            tab_id,
            BrowserEvent::AddressChanged {
                url: "https://a.test/".to_owned(),
            },
        );

        run_diagnostic_steps(
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
        assert_eq!(
            steps,
            vec![
                None,
                Some(FindStep::Here),
                Some(FindStep::Previous),
                Some(FindStep::Previous)
            ]
        );
    }
}
