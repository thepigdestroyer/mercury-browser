// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! Camera and microphone permission prompts, "Remember for this site" and the per-tab
//! capture indicator (media-capture lane). Camera and microphone are asked for only on macOS,
//! where pages can use them; elsewhere they are denied without a prompt.
#![cfg(target_os = "macos")]

use std::time::Duration;

use talaria_core::{
    BrowserCommand, BrowserController, BrowserEvent, CaptureAccess, LoadState, PagePermission,
    PageRequest, PageRequestAnswer, PageRequestId, PageRequestKind, PageRequestReply,
    PermissionDecision, PermissionDecisions, PointerButton, PointerButtonAction,
    SessionPermissionDecisions, TabActivation, TabCommand, TabId,
};

const PAGE: &str = "https://meet.example/room";
const OTHER_PORT: &str = "https://meet.example:8443/room";

fn ready_controller() -> (BrowserController, TabId) {
    let mut controller = BrowserController::default();
    let tab_id = controller.active_tab_id();
    commit_page(&mut controller, tab_id, PAGE);
    (controller, tab_id)
}

fn commit_page(controller: &mut BrowserController, tab_id: TabId, url: &str) {
    controller.handle_event(
        tab_id,
        BrowserEvent::AddressChanged {
            url: url.to_owned(),
        },
    );
}

fn ask(
    controller: &mut BrowserController,
    tab_id: TabId,
    id: u64,
    url: &str,
    permission: PagePermission,
) {
    assert!(controller.handle_event(
        tab_id,
        BrowserEvent::PageRequested(PageRequest {
            id: PageRequestId::new(id),
            url: url.to_owned(),
            kind: PageRequestKind::Permission(permission),
            received_at: Duration::ZERO,
        }),
    ));
}

fn answer_command(id: u64, answer: PageRequestAnswer) -> BrowserCommand {
    BrowserCommand::AnswerPageRequest {
        id: PageRequestId::new(id),
        answer,
    }
}

fn drain(controller: &mut BrowserController) -> Vec<TabCommand> {
    std::iter::from_fn(|| controller.next_command()).collect()
}

fn answers(controller: &mut BrowserController) -> Vec<BrowserCommand> {
    drain(controller)
        .into_iter()
        .map(|command| command.command().clone())
        .filter(|command| matches!(command, BrowserCommand::AnswerPageRequest { .. }))
        .collect()
}

fn reply_front(controller: &mut BrowserController, tab_id: TabId, reply: PageRequestReply) {
    let id = controller
        .pending_page_request(tab_id)
        .expect("a request is shown")
        .id();
    assert!(controller.answer_page_request(tab_id, id, reply, Duration::from_secs(1)));
}

fn allow() -> PageRequestReply {
    PageRequestReply::new(PageRequestAnswer::Accept)
}

fn block() -> PageRequestReply {
    PageRequestReply::new(PageRequestAnswer::Decline)
}

fn access(camera: bool, microphone: bool) -> CaptureAccess {
    CaptureAccess { camera, microphone }
}

#[test]
fn camera_and_microphone_are_asked() {
    for permission in [PagePermission::Camera, PagePermission::Microphone] {
        let (mut controller, tab_id) = ready_controller();
        ask(&mut controller, tab_id, 1, PAGE, permission);

        assert!(permission.is_promptable());
        assert_eq!(
            controller
                .pending_page_request(tab_id)
                .map(|pending| pending.id().value()),
            Some(1)
        );
        assert!(drain(&mut controller).is_empty());
    }
}

#[test]
fn camera_and_microphone_from_one_page_are_asked_together() {
    let (mut controller, tab_id) = ready_controller();
    ask(&mut controller, tab_id, 1, PAGE, PagePermission::Microphone);
    ask(&mut controller, tab_id, 2, PAGE, PagePermission::Camera);

    let shown = controller.pending_page_request(tab_id).expect("shown");
    assert_eq!(shown.id().value(), 1);
    assert_eq!(shown.paired_permission(), Some(PagePermission::Camera));

    reply_front(&mut controller, tab_id, allow());
    assert_eq!(controller.pending_page_request(tab_id), None);
    assert_eq!(
        answers(&mut controller),
        vec![
            answer_command(1, PageRequestAnswer::Accept),
            answer_command(2, PageRequestAnswer::Accept),
        ]
    );
}

#[test]
fn a_refusal_of_the_pair_refuses_both() {
    let (mut controller, tab_id) = ready_controller();
    ask(&mut controller, tab_id, 1, PAGE, PagePermission::Camera);
    ask(&mut controller, tab_id, 2, PAGE, PagePermission::Microphone);

    reply_front(&mut controller, tab_id, block());
    assert_eq!(
        answers(&mut controller),
        vec![
            answer_command(1, PageRequestAnswer::Decline),
            answer_command(2, PageRequestAnswer::Decline),
        ]
    );
    assert_eq!(controller.capture_access(tab_id), CaptureAccess::default());
}

#[test]
fn only_camera_and_microphone_of_the_same_origin_are_paired() {
    let (mut controller, tab_id) = ready_controller();
    ask(&mut controller, tab_id, 1, PAGE, PagePermission::Camera);
    ask(
        &mut controller,
        tab_id,
        2,
        OTHER_PORT,
        PagePermission::Microphone,
    );
    ask(
        &mut controller,
        tab_id,
        3,
        PAGE,
        PagePermission::PersistentStorage,
    );
    ask(&mut controller, tab_id, 4, PAGE, PagePermission::Camera);

    let shown = controller.pending_page_request(tab_id).expect("shown");
    assert_eq!(shown.paired_permission(), None);

    reply_front(&mut controller, tab_id, allow());
    assert_eq!(
        answers(&mut controller),
        vec![answer_command(1, PageRequestAnswer::Accept)]
    );
    assert_eq!(
        controller
            .pending_page_request(tab_id)
            .map(|pending| pending.id().value()),
        Some(2)
    );
}

#[test]
fn remembering_the_pair_answers_both_kinds_for_that_origin_only() {
    let (mut controller, tab_id) = ready_controller();
    ask(&mut controller, tab_id, 1, PAGE, PagePermission::Camera);
    ask(&mut controller, tab_id, 2, PAGE, PagePermission::Microphone);
    reply_front(&mut controller, tab_id, allow().remembering_for_site());
    drain(&mut controller);

    let other = controller
        .open_tab_with(None, TabActivation::Foreground)
        .expect("a tab opens");
    commit_page(&mut controller, other, PAGE);
    ask(&mut controller, other, 3, PAGE, PagePermission::Camera);
    ask(&mut controller, other, 4, PAGE, PagePermission::Microphone);
    assert_eq!(
        answers(&mut controller),
        vec![
            answer_command(3, PageRequestAnswer::Accept),
            answer_command(4, PageRequestAnswer::Accept),
        ]
    );
    assert_eq!(controller.capture_access(other), access(true, true));

    // Another port is another origin: it is asked.
    ask(
        &mut controller,
        other,
        5,
        OTHER_PORT,
        PagePermission::Camera,
    );
    assert!(answers(&mut controller).is_empty());
}

#[test]
fn remembering_is_per_device_kind() {
    let (mut controller, tab_id) = ready_controller();
    ask(&mut controller, tab_id, 1, PAGE, PagePermission::Camera);
    reply_front(&mut controller, tab_id, allow().remembering_for_site());
    drain(&mut controller);

    ask(&mut controller, tab_id, 2, PAGE, PagePermission::Microphone);
    assert!(answers(&mut controller).is_empty());
    assert_eq!(
        controller
            .pending_page_request(tab_id)
            .map(|pending| pending.id().value()),
        Some(2)
    );
}

#[test]
fn a_remembered_refusal_answers_without_granting_access() {
    let (mut controller, tab_id) = ready_controller();
    ask(&mut controller, tab_id, 1, PAGE, PagePermission::Camera);
    reply_front(&mut controller, tab_id, block().remembering_for_site());
    drain(&mut controller);

    ask(&mut controller, tab_id, 2, PAGE, PagePermission::Camera);
    assert_eq!(
        answers(&mut controller),
        vec![answer_command(2, PageRequestAnswer::Decline)]
    );
    assert_eq!(controller.capture_access(tab_id), CaptureAccess::default());
}

#[test]
fn a_grant_shows_access_until_a_new_document_starts() {
    let (mut controller, tab_id) = ready_controller();
    assert_eq!(controller.capture_access(tab_id), CaptureAccess::default());
    assert!(controller.capture_access(tab_id).is_empty());

    ask(&mut controller, tab_id, 1, PAGE, PagePermission::Camera);
    reply_front(&mut controller, tab_id, allow());
    assert_eq!(controller.capture_access(tab_id), access(true, false));
    assert_eq!(
        controller.capture_origin(tab_id).as_deref(),
        Some("meet.example")
    );

    ask(&mut controller, tab_id, 2, PAGE, PagePermission::Microphone);
    reply_front(&mut controller, tab_id, allow());
    assert_eq!(controller.capture_access(tab_id), access(true, true));

    controller.handle_event(tab_id, BrowserEvent::PageLoadStarted);
    assert_eq!(controller.capture_access(tab_id), CaptureAccess::default());
    assert_eq!(controller.capture_origin(tab_id), None);
}

#[test]
fn a_crash_ends_access() {
    let (mut controller, tab_id) = ready_controller();
    ask(&mut controller, tab_id, 1, PAGE, PagePermission::Microphone);
    reply_front(&mut controller, tab_id, allow());
    assert_eq!(controller.capture_access(tab_id), access(false, true));

    controller.handle_event(
        tab_id,
        BrowserEvent::LoadStateChanged {
            state: LoadState::Failed {
                message: "crashed".to_owned(),
            },
        },
    );
    controller.handle_event(tab_id, BrowserEvent::PageCrashed);
    assert_eq!(controller.capture_access(tab_id), CaptureAccess::default());
}

#[test]
fn an_address_bar_error_keeps_access() {
    let (mut controller, tab_id) = ready_controller();
    ask(&mut controller, tab_id, 1, PAGE, PagePermission::Camera);
    reply_front(&mut controller, tab_id, allow());

    // What the address bar reports for an address it cannot use: no navigation happened, the
    // page is still there and may still be capturing.
    controller.handle_event(
        tab_id,
        BrowserEvent::LoadStateChanged {
            state: LoadState::Failed {
                message: "no search engine is set".to_owned(),
            },
        },
    );
    assert_eq!(controller.capture_access(tab_id), access(true, false));
}

#[test]
fn a_remembered_grant_shows_access_in_the_tab_it_answers() {
    let (mut controller, tab_id) = ready_controller();
    let mut decisions = SessionPermissionDecisions::default();
    decisions.remember(
        "https://meet.example",
        PagePermission::Camera,
        PermissionDecision::Allow,
    );
    controller.set_permission_decisions(Box::new(decisions));

    ask(&mut controller, tab_id, 1, PAGE, PagePermission::Camera);
    assert_eq!(
        answers(&mut controller),
        vec![answer_command(1, PageRequestAnswer::Accept)]
    );
    assert_eq!(controller.capture_access(tab_id), access(true, false));
}

#[test]
fn a_waiting_request_answered_by_a_remembered_grant_shows_access_in_its_tab() {
    let (mut controller, tab_id) = ready_controller();
    let other = controller
        .open_tab_with(None, TabActivation::Background)
        .expect("a tab opens");
    commit_page(&mut controller, other, PAGE);
    ask(&mut controller, other, 1, PAGE, PagePermission::Camera);
    ask(&mut controller, tab_id, 2, PAGE, PagePermission::Camera);

    reply_front(&mut controller, tab_id, allow().remembering_for_site());
    assert_eq!(controller.capture_access(tab_id), access(true, false));
    assert_eq!(controller.capture_access(other), access(true, false));
}

#[test]
fn stop_forgets_the_sites_answers_and_reloads_and_access_lasts_until_the_new_document() {
    let (mut controller, tab_id) = ready_controller();
    ask(&mut controller, tab_id, 1, PAGE, PagePermission::Camera);
    ask(&mut controller, tab_id, 2, PAGE, PagePermission::Microphone);
    reply_front(&mut controller, tab_id, allow().remembering_for_site());
    drain(&mut controller);

    assert!(controller.stop_capture(tab_id));
    // The page still runs until the reload replaces it: the marks stay.
    assert_eq!(controller.capture_access(tab_id), access(true, true));
    assert!(!controller.capture_stop_refused(tab_id));
    let commands = drain(&mut controller)
        .into_iter()
        .map(|command| command.command().clone())
        .collect::<Vec<_>>();
    assert_eq!(commands, vec![BrowserCommand::Reload]);
    controller.handle_event(tab_id, BrowserEvent::PageLoadStarted);
    assert_eq!(controller.capture_access(tab_id), CaptureAccess::default());

    // Asked again, both kinds.
    ask(&mut controller, tab_id, 3, PAGE, PagePermission::Camera);
    ask(&mut controller, tab_id, 4, PAGE, PagePermission::Microphone);
    assert!(answers(&mut controller).is_empty());
    assert_eq!(
        controller
            .pending_page_request(tab_id)
            .and_then(|pending| pending.paired_permission()),
        Some(PagePermission::Microphone)
    );
}

#[test]
fn stop_without_access_does_nothing() {
    let (mut controller, tab_id) = ready_controller();
    assert!(!controller.stop_capture(tab_id));
    assert!(drain(&mut controller).is_empty());
}

#[test]
fn the_session_store_forgets_one_kind_for_one_origin() {
    let mut decisions = SessionPermissionDecisions::default();
    for permission in [PagePermission::Camera, PagePermission::Microphone] {
        decisions.remember("https://a.example", permission, PermissionDecision::Allow);
        decisions.remember("https://b.example", permission, PermissionDecision::Allow);
    }
    decisions.forget("https://a.example", PagePermission::Camera);

    assert_eq!(
        decisions.decision("https://a.example", PagePermission::Camera),
        None
    );
    assert_eq!(
        decisions.decision("https://a.example", PagePermission::Microphone),
        Some(PermissionDecision::Allow)
    );
    assert_eq!(
        decisions.decision("https://b.example", PagePermission::Camera),
        Some(PermissionDecision::Allow)
    );
}

#[test]
fn a_reload_the_page_keeps_from_happening_after_stop_leaves_access_shown_and_says_so() {
    let (mut controller, tab_id) = ready_controller();
    ask(&mut controller, tab_id, 1, PAGE, PagePermission::Camera);
    reply_front(&mut controller, tab_id, allow());
    // The user clicked in the page, so its leave prompt is shown.
    controller.queue_command(BrowserCommand::PointerButton {
        action: PointerButtonAction::Down,
        button: PointerButton::Left,
        x: 0.0,
        y: 0.0,
    });
    drain(&mut controller);

    assert!(controller.stop_capture(tab_id));
    assert!(controller.handle_event(
        tab_id,
        BrowserEvent::PageRequested(PageRequest {
            id: PageRequestId::new(9),
            url: PAGE.to_owned(),
            kind: PageRequestKind::LeavePage,
            received_at: Duration::ZERO,
        }),
    ));
    let leave = controller
        .pending_page_request(tab_id)
        .expect("the leave prompt");
    assert!(leave.is_reload_prompt());
    // Stay.
    reply_front(&mut controller, tab_id, block());

    assert_eq!(controller.capture_access(tab_id), access(true, false));
    assert!(controller.capture_stop_refused(tab_id));
    // Stop again: another reload is asked for.
    assert!(controller.stop_capture(tab_id));
    assert!(!controller.capture_stop_refused(tab_id));
    controller.handle_event(tab_id, BrowserEvent::PageLoadStarted);
    assert!(!controller.capture_stop_refused(tab_id));
    assert_eq!(controller.capture_access(tab_id), CaptureAccess::default());
}

#[test]
fn a_remembered_refusal_of_one_kind_refuses_the_other_kind_of_the_same_call() {
    let (mut controller, tab_id) = ready_controller();
    let mut decisions = SessionPermissionDecisions::default();
    decisions.remember(
        "https://meet.example",
        PagePermission::Camera,
        PermissionDecision::Block,
    );
    controller.set_permission_decisions(Box::new(decisions));

    // The engine asks for the microphone, then the camera, of one getUserMedia call.
    ask(&mut controller, tab_id, 1, PAGE, PagePermission::Microphone);
    ask(&mut controller, tab_id, 2, PAGE, PagePermission::Camera);
    assert_eq!(
        answers(&mut controller),
        vec![
            answer_command(2, PageRequestAnswer::Decline),
            answer_command(1, PageRequestAnswer::Decline),
        ]
    );
    assert_eq!(controller.pending_page_request(tab_id), None);
}

fn leave_prompt(controller: &mut BrowserController, tab_id: TabId, id: u64) {
    assert!(controller.handle_event(
        tab_id,
        BrowserEvent::PageRequested(PageRequest {
            id: PageRequestId::new(id),
            url: PAGE.to_owned(),
            kind: PageRequestKind::LeavePage,
            received_at: Duration::ZERO,
        }),
    ));
}

fn click_in_page(controller: &mut BrowserController) {
    controller.queue_command(BrowserCommand::PointerButton {
        action: PointerButtonAction::Down,
        button: PointerButton::Left,
        x: 0.0,
        y: 0.0,
    });
}

#[test]
fn staying_on_a_page_the_user_leaves_after_stop_is_not_called_a_refused_stop() {
    let (mut controller, tab_id) = ready_controller();
    ask(&mut controller, tab_id, 1, PAGE, PagePermission::Camera);
    reply_front(&mut controller, tab_id, allow());
    click_in_page(&mut controller);
    assert!(controller.stop_capture(tab_id));
    drain(&mut controller);

    // Before the reload shows anything, the user goes to another address, and stays when the
    // page asks: that is not the page refusing Stop's reload.
    controller.queue_command(BrowserCommand::Navigate {
        url: "https://elsewhere.example/".to_owned(),
    });
    leave_prompt(&mut controller, tab_id, 9);
    let leave = controller
        .pending_page_request(tab_id)
        .expect("the leave prompt");
    assert!(!leave.is_reload_prompt());
    reply_front(&mut controller, tab_id, block());

    assert!(!controller.capture_stop_refused(tab_id));
    assert_eq!(controller.capture_access(tab_id), access(true, false));
}

#[test]
fn a_remembered_refusal_of_the_kind_asked_first_refuses_the_kind_asked_next() {
    let (mut controller, tab_id) = ready_controller();
    let mut decisions = SessionPermissionDecisions::default();
    decisions.remember(
        "https://meet.example",
        PagePermission::Microphone,
        PermissionDecision::Block,
    );
    controller.set_permission_decisions(Box::new(decisions));

    // The engine asks for the microphone, then the camera, of one getUserMedia call: the call
    // has already failed when the camera request arrives.
    ask(&mut controller, tab_id, 1, PAGE, PagePermission::Microphone);
    ask(&mut controller, tab_id, 2, PAGE, PagePermission::Camera);
    assert_eq!(
        answers(&mut controller),
        vec![
            answer_command(1, PageRequestAnswer::Decline),
            answer_command(2, PageRequestAnswer::Decline),
        ]
    );
    assert_eq!(controller.pending_page_request(tab_id), None);

    // A camera request of a later call is asked as usual.
    assert!(controller.handle_event(
        tab_id,
        BrowserEvent::PageRequested(PageRequest {
            id: PageRequestId::new(3),
            url: PAGE.to_owned(),
            kind: PageRequestKind::Permission(PagePermission::Camera),
            received_at: Duration::from_secs(5),
        }),
    ));
    assert_eq!(
        controller
            .pending_page_request(tab_id)
            .map(|pending| pending.id().value()),
        Some(3)
    );
}
