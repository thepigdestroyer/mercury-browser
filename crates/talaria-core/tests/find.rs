// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
use talaria_core::{
    BrowserCommand, BrowserController, BrowserEvent, EndFindRequest, FindMatches, FindOutcome,
    FindRequest, FindSelection, FindStatus, FindStep, TabCommand, TabId,
};

fn ready_controller() -> (BrowserController, TabId) {
    let mut controller = BrowserController::default();
    let tab_id = controller.active_tab_id();
    controller.handle_event(
        tab_id,
        BrowserEvent::AddressChanged {
            url: "https://a.test/page".to_owned(),
        },
    );
    (controller, tab_id)
}

fn drain(controller: &mut BrowserController) -> Vec<BrowserCommand> {
    std::iter::from_fn(|| controller.next_command())
        .map(TabCommand::into_command)
        .collect()
}

fn matches(count: usize, current: Option<usize>) -> FindOutcome {
    FindOutcome::Matches(FindMatches {
        count,
        current,
        truncated: false,
    })
}

/// A search started by typing, the debounce or a selection read.
fn search(id: u64, query: &str, step: FindStep, current: Option<usize>) -> BrowserCommand {
    BrowserCommand::Find(FindRequest {
        id,
        query: query.to_owned(),
        match_case: false,
        step,
        repeat: 1,
        thorough: false,
        current,
    })
}

/// A search started by Return, a button or a shortcut.
fn explicit(id: u64, query: &str, step: FindStep, current: Option<usize>) -> BrowserCommand {
    explicit_repeat(id, query, step, 1, current)
}

fn explicit_repeat(
    id: u64,
    query: &str,
    step: FindStep,
    repeat: u32,
    current: Option<usize>,
) -> BrowserCommand {
    BrowserCommand::Find(FindRequest {
        id,
        query: query.to_owned(),
        match_case: false,
        step,
        repeat,
        thorough: true,
        current,
    })
}

fn answer(
    controller: &mut BrowserController,
    tab_id: TabId,
    request_id: u64,
    outcome: FindOutcome,
) {
    controller.handle_event(
        tab_id,
        BrowserEvent::FindResult {
            request_id,
            outcome,
        },
    );
}

/// Opens the bar, answers the selection read with no selection, and types `query`.
fn open_and_search(controller: &mut BrowserController, tab_id: TabId, query: &str) -> u64 {
    assert!(controller.open_find());
    let [BrowserCommand::BeginFind { request_id }] = drain(controller)[..] else {
        panic!("opening the bar reads the page's selection");
    };
    controller.handle_event(
        tab_id,
        BrowserEvent::FindSelectionRead {
            request_id,
            selection: None,
        },
    );
    controller.set_find_query(query);
    controller.run_due_find();
    let commands = drain(controller);
    let [BrowserCommand::Find(FindRequest { id, .. })] = commands[..] else {
        panic!("a typed query is searched: {commands:?}");
    };
    id
}

#[test]
fn the_bar_starts_closed_and_empty() {
    let (controller, _) = ready_controller();
    let find = controller.active_state().find();

    assert!(!find.is_open());
    assert_eq!(find.query(), "");
    assert!(!find.match_case());
    assert_eq!(find.status(), &FindStatus::Idle);
}

#[test]
fn opening_reads_the_selection_once() {
    let (mut controller, _) = ready_controller();

    assert!(controller.open_find());
    assert!(!controller.open_find(), "already open");

    assert_eq!(
        drain(&mut controller),
        vec![BrowserCommand::BeginFind { request_id: 1 }]
    );
    assert!(controller.active_state().find().is_open());
}

#[test]
fn the_page_selection_becomes_the_query_and_is_searched() {
    let (mut controller, tab_id) = ready_controller();
    controller.open_find();
    drain(&mut controller);

    controller.handle_event(
        tab_id,
        BrowserEvent::FindSelectionRead {
            request_id: 1,
            selection: Some(FindSelection {
                text: "needle".to_owned(),
                saved: Some("0.1.0:4|0.1.0:10".to_owned()),
            }),
        },
    );

    assert_eq!(controller.active_state().find().query(), "needle");
    assert_eq!(
        drain(&mut controller),
        vec![search(2, "needle", FindStep::Here, None)]
    );
}

#[test]
fn without_a_selection_the_last_query_is_searched_again() {
    let (mut controller, tab_id) = ready_controller();
    let id = open_and_search(&mut controller, tab_id, "old");
    controller.handle_event(
        tab_id,
        BrowserEvent::FindResult {
            request_id: id,
            outcome: matches(2, Some(1)),
        },
    );
    controller.close_find();
    drain(&mut controller);

    controller.open_find();
    let [BrowserCommand::BeginFind { request_id }] = drain(&mut controller)[..] else {
        panic!("reopening reads the selection");
    };
    controller.handle_event(
        tab_id,
        BrowserEvent::FindSelectionRead {
            request_id,
            selection: None,
        },
    );

    assert_eq!(controller.active_state().find().query(), "old");
    assert_eq!(
        drain(&mut controller),
        vec![search(request_id + 1, "old", FindStep::Here, None)]
    );
}

#[test]
fn a_selection_read_after_the_user_typed_does_not_replace_the_query() {
    let (mut controller, tab_id) = ready_controller();
    controller.open_find();
    drain(&mut controller);
    controller.set_find_query("typed");

    controller.handle_event(
        tab_id,
        BrowserEvent::FindSelectionRead {
            request_id: 1,
            selection: Some(FindSelection {
                text: "page".to_owned(),
                saved: None,
            }),
        },
    );

    assert_eq!(controller.active_state().find().query(), "typed");
    assert_eq!(drain(&mut controller), vec![]);
}

#[test]
fn typing_is_searched_only_when_the_debounce_fires() {
    let (mut controller, tab_id) = ready_controller();
    let id = open_and_search(&mut controller, tab_id, "ne");
    answer(&mut controller, tab_id, id, matches(1, Some(0)));

    controller.set_find_query("nee");
    controller.set_find_query("need");
    assert_eq!(drain(&mut controller), vec![]);

    controller.run_due_find();
    controller.run_due_find();
    assert_eq!(
        drain(&mut controller),
        vec![search(3, "need", FindStep::Here, Some(0))]
    );
}

#[test]
fn next_and_previous_step_from_the_current_match() {
    let (mut controller, tab_id) = ready_controller();
    let id = open_and_search(&mut controller, tab_id, "x");
    controller.handle_event(
        tab_id,
        BrowserEvent::FindResult {
            request_id: id,
            outcome: matches(7, Some(2)),
        },
    );

    controller.find_next();
    assert_eq!(
        drain(&mut controller),
        vec![explicit(3, "x", FindStep::Next, Some(2))]
    );
    answer(&mut controller, tab_id, 3, matches(7, Some(3)));
    controller.find_previous();
    assert_eq!(
        drain(&mut controller),
        vec![explicit(4, "x", FindStep::Previous, Some(3))]
    );
}

#[test]
fn return_before_the_debounce_searches_the_new_query_from_here() {
    let (mut controller, tab_id) = ready_controller();
    open_and_search(&mut controller, tab_id, "a");
    controller.set_find_query("ab");

    answer(&mut controller, tab_id, 2, matches(2, Some(0)));
    controller.find_next();

    assert_eq!(
        drain(&mut controller),
        vec![explicit(3, "ab", FindStep::Here, Some(0))]
    );
    controller.run_due_find();
    assert_eq!(drain(&mut controller), vec![], "nothing left to debounce");
}

#[test]
fn answers_to_older_requests_are_ignored() {
    let (mut controller, tab_id) = ready_controller();
    let first = open_and_search(&mut controller, tab_id, "x");

    answer(&mut controller, tab_id, first + 7, matches(7, Some(0)));
    assert_eq!(controller.active_state().find().status(), &FindStatus::Idle);

    answer(&mut controller, tab_id, first, matches(7, Some(1)));
    assert_eq!(
        controller.active_state().find().status(),
        &FindStatus::Matches(FindMatches {
            count: 7,
            current: Some(1),
            truncated: false,
        })
    );
}

#[test]
fn one_search_at_a_time_and_presses_meanwhile_add_up() {
    let (mut controller, tab_id) = ready_controller();
    let first = open_and_search(&mut controller, tab_id, "x");

    controller.find_next();
    controller.find_next();
    controller.find_next();
    controller.find_previous();
    assert_eq!(
        drain(&mut controller),
        vec![],
        "the page is still answering"
    );
    assert!(controller.active_state().find().is_awaiting_answer());

    answer(&mut controller, tab_id, first, matches(7, Some(0)));
    assert_eq!(
        drain(&mut controller),
        vec![explicit_repeat(first + 1, "x", FindStep::Next, 2, Some(0))]
    );
}

#[test]
fn presses_that_cancel_out_send_nothing() {
    let (mut controller, tab_id) = ready_controller();
    let first = open_and_search(&mut controller, tab_id, "x");
    controller.find_next();
    controller.find_previous();

    answer(&mut controller, tab_id, first, matches(7, Some(0)));

    assert_eq!(drain(&mut controller), vec![]);
    assert!(!controller.active_state().find().is_awaiting_answer());
}

#[test]
fn a_query_typed_meanwhile_is_searched_after_the_answer() {
    let (mut controller, tab_id) = ready_controller();
    let first = open_and_search(&mut controller, tab_id, "x");
    controller.find_next();
    controller.set_find_query("xy");
    controller.run_due_find();
    assert_eq!(drain(&mut controller), vec![]);

    answer(&mut controller, tab_id, first, matches(7, Some(0)));

    assert_eq!(
        drain(&mut controller),
        vec![search(first + 1, "xy", FindStep::Here, Some(0))]
    );
}

#[test]
fn a_page_that_does_not_answer_is_reported_and_gets_nothing_more() {
    let (mut controller, tab_id) = ready_controller();
    let first = open_and_search(&mut controller, tab_id, "x");
    assert_eq!(
        controller.active_state().find().awaited_search(),
        Some(first)
    );

    controller.find_answer_overdue(tab_id, first + 1);
    assert_eq!(controller.active_state().find().status(), &FindStatus::Idle);
    controller.find_answer_overdue(tab_id, first);
    assert_eq!(
        controller.active_state().find().status(),
        &FindStatus::NotResponding
    );

    controller.find_next();
    controller.set_find_query("xy");
    controller.run_due_find();
    assert_eq!(
        drain(&mut controller),
        vec![],
        "nothing is queued behind a stuck search"
    );

    answer(&mut controller, tab_id, first, matches(1, Some(0)));
    assert_eq!(
        drain(&mut controller),
        vec![search(first + 1, "xy", FindStep::Here, Some(0))]
    );
}

#[test]
fn closing_drops_a_stuck_search_so_reopening_works() {
    let (mut controller, tab_id) = ready_controller();
    let first = open_and_search(&mut controller, tab_id, "x");
    controller.find_answer_overdue(tab_id, first);
    controller.close_find();
    drain(&mut controller);

    controller.find_next();

    assert_eq!(
        drain(&mut controller),
        vec![explicit(first + 1, "x", FindStep::Next, None)]
    );
}

#[test]
fn a_new_query_keeps_the_last_status_until_it_is_answered() {
    let (mut controller, tab_id) = ready_controller();
    let id = open_and_search(&mut controller, tab_id, "x");
    controller.handle_event(
        tab_id,
        BrowserEvent::FindResult {
            request_id: id,
            outcome: matches(3, Some(0)),
        },
    );

    controller.set_find_query("xy");

    assert_eq!(
        controller.active_state().find().status(),
        &FindStatus::Matches(FindMatches {
            count: 3,
            current: Some(0),
            truncated: false,
        })
    );
}

#[test]
fn an_empty_query_is_not_searched() {
    let (mut controller, tab_id) = ready_controller();
    open_and_search(&mut controller, tab_id, "x");

    controller.set_find_query("");
    controller.run_due_find();
    controller.find_next();

    assert_eq!(drain(&mut controller), vec![]);
    assert_eq!(controller.active_state().find().status(), &FindStatus::Idle);
}

#[test]
fn toggling_match_case_searches_again_from_here() {
    let (mut controller, tab_id) = ready_controller();
    let id = open_and_search(&mut controller, tab_id, "X");
    controller.handle_event(
        tab_id,
        BrowserEvent::FindResult {
            request_id: id,
            outcome: matches(3, Some(1)),
        },
    );

    controller.toggle_find_match_case();

    assert!(controller.active_state().find().match_case());
    assert_ne!(
        controller.active_state().find().status(),
        &FindStatus::Idle,
        "no flicker"
    );
    assert_eq!(
        drain(&mut controller),
        vec![BrowserCommand::Find(FindRequest {
            id: id + 1,
            query: "X".to_owned(),
            match_case: true,
            step: FindStep::Here,
            repeat: 1,
            thorough: true,
            current: Some(1),
        })]
    );
}

#[test]
fn closing_restores_the_saved_selection_and_ignores_late_results() {
    let (mut controller, tab_id) = ready_controller();
    controller.open_find();
    drain(&mut controller);
    controller.handle_event(
        tab_id,
        BrowserEvent::FindSelectionRead {
            request_id: 1,
            selection: Some(FindSelection {
                text: String::new(),
                saved: Some("0.1:0|0.1:0".to_owned()),
            }),
        },
    );
    controller.set_find_query("x");
    controller.run_due_find();
    drain(&mut controller);

    controller.close_find();

    assert_eq!(
        drain(&mut controller),
        vec![BrowserCommand::EndFind(EndFindRequest {
            query: "x".to_owned(),
            match_case: false,
            restore_selection: Some("0.1:0|0.1:0".to_owned()),
            caret_left: false,
        })]
    );
    controller.handle_event(
        tab_id,
        BrowserEvent::FindResult {
            request_id: 2,
            outcome: matches(1, Some(0)),
        },
    );
    let find = controller.active_state().find();
    assert!(!find.is_open());
    assert_eq!(find.query(), "x", "the query is kept for next time");
    assert_eq!(find.status(), &FindStatus::Idle);
}

#[test]
fn closing_a_bar_that_never_searched_tells_the_page_nothing() {
    let (mut controller, _) = ready_controller();
    controller.open_find();
    drain(&mut controller);

    controller.close_find();
    controller.close_find();

    assert_eq!(drain(&mut controller), vec![]);
}

#[test]
fn navigating_clears_the_count_but_keeps_the_query() {
    let (mut controller, tab_id) = ready_controller();
    let id = open_and_search(&mut controller, tab_id, "x");
    controller.handle_event(
        tab_id,
        BrowserEvent::FindResult {
            request_id: id,
            outcome: matches(3, Some(0)),
        },
    );

    controller.handle_event(
        tab_id,
        BrowserEvent::AddressChanged {
            url: "https://b.test/".to_owned(),
        },
    );
    controller.handle_event(
        tab_id,
        BrowserEvent::FindResult {
            request_id: id,
            outcome: matches(3, Some(0)),
        },
    );

    let find = controller.active_state().find();
    assert!(find.is_open());
    assert_eq!(find.query(), "x");
    assert_eq!(find.status(), &FindStatus::Idle);
    assert!(find.is_query_from_other_page());
    controller.find_next();
    assert_eq!(
        drain(&mut controller),
        vec![explicit(id + 1, "x", FindStep::Here, None)],
        "Return searches the new page"
    );
    assert!(!controller.active_state().find().is_query_from_other_page());
}

#[test]
fn an_old_query_is_never_sent_to_a_new_document_by_itself() {
    let (mut controller, tab_id) = ready_controller();
    open_and_search(&mut controller, tab_id, "secret");
    controller.handle_event(
        tab_id,
        BrowserEvent::AddressChanged {
            url: "https://other.test/".to_owned(),
        },
    );

    // The debounce firing late, and closing and reopening the bar with no selection.
    controller.run_due_find();
    controller.close_find();
    controller.open_find();
    let commands = drain(&mut controller);
    let request_id = commands
        .iter()
        .find_map(|command| match command {
            BrowserCommand::BeginFind { request_id } => Some(*request_id),
            _ => None,
        })
        .expect("reopening reads the selection");
    controller.handle_event(
        tab_id,
        BrowserEvent::FindSelectionRead {
            request_id,
            selection: None,
        },
    );

    assert!(
        drain(&mut controller)
            .iter()
            .all(|command| !matches!(command, BrowserCommand::Find(_))),
        "the old query is not searched"
    );
    let find = controller.active_state().find();
    assert_eq!(find.query(), "secret", "it is shown, for the user to send");
    assert!(find.is_query_from_other_page());

    // Typing makes it the user's query for this page again.
    controller.set_find_query("secrets");
    controller.run_due_find();
    assert_eq!(drain(&mut controller).len(), 1);
}

#[test]
fn a_selection_on_the_new_page_is_searched() {
    let (mut controller, tab_id) = ready_controller();
    open_and_search(&mut controller, tab_id, "old");
    controller.close_find();
    controller.handle_event(
        tab_id,
        BrowserEvent::AddressChanged {
            url: "https://other.test/".to_owned(),
        },
    );
    drain(&mut controller);

    controller.open_find();
    let [BrowserCommand::BeginFind { request_id }] = drain(&mut controller)[..] else {
        panic!("opening reads the selection");
    };
    controller.handle_event(
        tab_id,
        BrowserEvent::FindSelectionRead {
            request_id,
            selection: Some(FindSelection {
                text: "fresh".to_owned(),
                saved: None,
            }),
        },
    );

    assert_eq!(
        drain(&mut controller),
        vec![search(request_id + 1, "fresh", FindStep::Here, None)]
    );
}

#[test]
fn a_reload_clears_the_count_and_drops_pending_answers() {
    let (mut controller, tab_id) = ready_controller();
    let id = open_and_search(&mut controller, tab_id, "x");
    answer(&mut controller, tab_id, id, matches(3, Some(0)));
    controller.handle_event(
        tab_id,
        BrowserEvent::LoadStateChanged {
            state: talaria_core::LoadState::Loaded,
        },
    );
    controller.find_next();
    drain(&mut controller);

    controller.handle_event(
        tab_id,
        BrowserEvent::LoadStateChanged {
            state: talaria_core::LoadState::Loading,
        },
    );

    let find = controller.active_state().find();
    assert_eq!(find.status(), &FindStatus::Idle);
    assert!(!find.is_awaiting_answer());
    answer(&mut controller, tab_id, id + 1, matches(3, Some(1)));
    assert_eq!(controller.active_state().find().status(), &FindStatus::Idle);
}

#[test]
fn navigating_forgets_a_pending_selection_read() {
    let (mut controller, tab_id) = ready_controller();
    controller.open_find();
    let [BrowserCommand::BeginFind { request_id }] = drain(&mut controller)[..] else {
        panic!("opening reads the selection");
    };

    controller.handle_event(
        tab_id,
        BrowserEvent::AddressChanged {
            url: "https://other.test/".to_owned(),
        },
    );
    assert!(!controller.active_state().find().is_awaiting_answer());
    controller.handle_event(
        tab_id,
        BrowserEvent::FindSelectionRead {
            request_id,
            selection: Some(FindSelection {
                text: "from the old page".to_owned(),
                saved: Some("1:0|1:1".to_owned()),
            }),
        },
    );

    assert_eq!(controller.active_state().find().query(), "");
    assert_eq!(drain(&mut controller), vec![]);
}

#[test]
fn a_fragment_change_keeps_the_count() {
    let (mut controller, tab_id) = ready_controller();
    let id = open_and_search(&mut controller, tab_id, "x");
    controller.handle_event(
        tab_id,
        BrowserEvent::FindResult {
            request_id: id,
            outcome: matches(3, Some(0)),
        },
    );

    controller.handle_event(
        tab_id,
        BrowserEvent::AddressChanged {
            url: "https://a.test/page#section".to_owned(),
        },
    );

    assert_ne!(controller.active_state().find().status(), &FindStatus::Idle);
}

#[test]
fn a_failed_page_clears_the_count() {
    let (mut controller, tab_id) = ready_controller();
    let id = open_and_search(&mut controller, tab_id, "x");
    controller.handle_event(
        tab_id,
        BrowserEvent::FindResult {
            request_id: id,
            outcome: matches(3, Some(0)),
        },
    );

    controller.handle_event(
        tab_id,
        BrowserEvent::LoadStateChanged {
            state: talaria_core::LoadState::Failed {
                message: "crashed".to_owned(),
            },
        },
    );

    assert_eq!(controller.active_state().find().status(), &FindStatus::Idle);
}

#[test]
fn a_failed_search_is_reported() {
    let (mut controller, tab_id) = ready_controller();
    let id = open_and_search(&mut controller, tab_id, "x");

    controller.handle_event(
        tab_id,
        BrowserEvent::FindResult {
            request_id: id,
            outcome: FindOutcome::Failed,
        },
    );

    assert_eq!(
        controller.active_state().find().status(),
        &FindStatus::Failed
    );
}

#[test]
fn each_tab_keeps_its_own_bar() {
    let (mut controller, first) = ready_controller();
    let id = open_and_search(&mut controller, first, "first");
    controller.handle_event(
        first,
        BrowserEvent::FindResult {
            request_id: id,
            outcome: matches(4, Some(2)),
        },
    );

    let second = controller.open_tab();
    assert!(!controller.active_state().find().is_open());
    assert_eq!(controller.active_state().find().query(), "");

    controller.select_tab(first);
    let find = controller.active_state().find();
    assert!(find.is_open());
    assert_eq!(find.query(), "first");
    assert_eq!(
        find.status(),
        &FindStatus::Matches(FindMatches {
            count: 4,
            current: Some(2),
            truncated: false,
        })
    );
    assert_ne!(first, second);
}

#[test]
fn find_commands_go_to_the_active_tab() {
    let (mut controller, first) = ready_controller();
    let second = controller.open_tab();
    controller.handle_event(
        second,
        BrowserEvent::AddressChanged {
            url: "https://b.test/".to_owned(),
        },
    );

    controller.open_find();

    let queued = controller.next_command().expect("a command was queued");
    assert_eq!(queued.tab_id(), second);
    assert!(
        !controller
            .tabs()
            .any(|tab| tab.id() == first && tab.state().find().is_open())
    );
}

#[test]
fn stepping_with_a_closed_bar_reopens_it_without_reading_the_selection() {
    let (mut controller, tab_id) = ready_controller();
    let id = open_and_search(&mut controller, tab_id, "x");
    controller.handle_event(
        tab_id,
        BrowserEvent::FindResult {
            request_id: id,
            outcome: matches(3, Some(0)),
        },
    );
    controller.close_find();
    drain(&mut controller);

    controller.find_next();

    assert!(controller.active_state().find().is_open());
    assert_eq!(
        drain(&mut controller),
        vec![explicit(id + 1, "x", FindStep::Next, None)]
    );
}

#[test]
fn stepping_with_no_query_does_nothing() {
    let (mut controller, _) = ready_controller();

    controller.find_next();
    controller.find_previous();

    assert_eq!(drain(&mut controller), vec![]);
    assert!(!controller.active_state().find().is_open());
}

#[test]
fn the_bar_knows_when_it_waits_for_the_page() {
    let (mut controller, tab_id) = ready_controller();
    controller.open_find();
    assert!(controller.active_state().find().is_awaiting_answer());

    let id = open_and_search_after_open(&mut controller, tab_id);
    assert!(controller.active_state().find().is_awaiting_answer());

    controller.handle_event(
        tab_id,
        BrowserEvent::FindResult {
            request_id: id,
            outcome: matches(1, Some(0)),
        },
    );
    assert!(!controller.active_state().find().is_awaiting_answer());
}

fn open_and_search_after_open(controller: &mut BrowserController, tab_id: TabId) -> u64 {
    drain(controller);
    controller.handle_event(
        tab_id,
        BrowserEvent::FindSelectionRead {
            request_id: 1,
            selection: None,
        },
    );
    controller.set_find_query("x");
    controller.run_due_find();
    let [BrowserCommand::Find(FindRequest { id, .. })] = drain(controller)[..] else {
        panic!("the query is searched");
    };
    id
}

fn cut_short_with_nothing() -> FindOutcome {
    FindOutcome::Matches(FindMatches {
        count: 0,
        current: None,
        truncated: true,
    })
}

#[test]
fn a_step_merged_into_a_waiting_typed_search_takes_the_larger_budget() {
    let (mut controller, tab_id) = ready_controller();
    let first = open_and_search(&mut controller, tab_id, "x");
    controller.set_find_query("xy");
    controller.run_due_find();
    controller.find_next();

    answer(&mut controller, tab_id, first, matches(1, Some(0)));

    assert_eq!(
        drain(&mut controller),
        vec![explicit(first + 1, "xy", FindStep::Here, Some(0))]
    );
}

#[test]
fn a_second_loading_state_does_not_clear_again() {
    let (mut controller, tab_id) = ready_controller();
    controller.handle_event(
        tab_id,
        BrowserEvent::LoadStateChanged {
            state: talaria_core::LoadState::Loading,
        },
    );
    let id = open_and_search(&mut controller, tab_id, "x");
    answer(&mut controller, tab_id, id, matches(2, Some(0)));

    // Servo reports `Started` and then `HeadParsed`, both `Loading`.
    controller.handle_event(
        tab_id,
        BrowserEvent::LoadStateChanged {
            state: talaria_core::LoadState::Loading,
        },
    );

    assert_eq!(
        controller.active_state().find().status(),
        &FindStatus::Matches(FindMatches {
            count: 2,
            current: Some(0),
            truncated: false,
        })
    );
}

#[test]
fn an_explicit_search_cut_short_with_nothing_found_goes_on_by_itself() {
    let (mut controller, tab_id) = ready_controller();
    let first = open_and_search(&mut controller, tab_id, "x");
    answer(&mut controller, tab_id, first, matches(1, Some(0)));
    controller.find_next();
    let mut id = first + 1;
    assert_eq!(
        drain(&mut controller),
        vec![explicit(id, "x", FindStep::Next, Some(0))]
    );

    // Four more rounds, then it stops: five in all.
    for _ in 0..4 {
        answer(&mut controller, tab_id, id, cut_short_with_nothing());
        id += 1;
        assert_eq!(
            drain(&mut controller),
            vec![explicit(id, "x", FindStep::Here, None)],
            "continues from where the last round stopped"
        );
    }
    answer(&mut controller, tab_id, id, cut_short_with_nothing());
    assert_eq!(drain(&mut controller), vec![]);
    assert_eq!(
        controller.active_state().find().status(),
        &cut_short_status()
    );
}

fn cut_short_status() -> FindStatus {
    FindStatus::Matches(FindMatches {
        count: 0,
        current: None,
        truncated: true,
    })
}

#[test]
fn typing_never_continues_by_itself_and_a_find_ends_the_rounds() {
    let (mut controller, tab_id) = ready_controller();
    let first = open_and_search(&mut controller, tab_id, "x");

    answer(&mut controller, tab_id, first, cut_short_with_nothing());
    assert_eq!(
        drain(&mut controller),
        vec![],
        "typing searches do not go on"
    );

    controller.find_next();
    let [BrowserCommand::Find(FindRequest { id, .. })] = drain(&mut controller)[..] else {
        panic!("Return searches");
    };
    answer(&mut controller, tab_id, id, matches(3, None));
    assert_eq!(drain(&mut controller), vec![], "a match ends the rounds");
}

#[test]
fn a_press_during_the_rounds_replaces_them() {
    let (mut controller, tab_id) = ready_controller();
    let first = open_and_search(&mut controller, tab_id, "x");
    answer(&mut controller, tab_id, first, matches(1, Some(0)));
    controller.find_next();
    answer(&mut controller, tab_id, first + 1, cut_short_with_nothing());
    drain(&mut controller);

    controller.find_previous();
    answer(&mut controller, tab_id, first + 2, cut_short_with_nothing());

    assert_eq!(
        drain(&mut controller),
        vec![explicit(first + 3, "x", FindStep::Previous, None)]
    );
}

#[test]
fn closing_after_a_caret_was_left_says_so() {
    let (mut controller, tab_id) = ready_controller();
    let first = open_and_search(&mut controller, tab_id, "x");
    answer(&mut controller, tab_id, first, matches(1, Some(0)));
    controller.find_next();
    answer(&mut controller, tab_id, first + 1, cut_short_with_nothing());
    drain(&mut controller);

    controller.close_find();

    assert_eq!(
        drain(&mut controller),
        vec![BrowserCommand::EndFind(EndFindRequest {
            query: "x".to_owned(),
            match_case: false,
            restore_selection: None,
            caret_left: true,
        })]
    );
}
