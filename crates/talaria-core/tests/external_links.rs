// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
// openurl: links a page asks the system to open in another app.

use std::time::Duration;

use talaria_core::{
    BrowserController, BrowserEvent, CloseTabOutcome, ExternalLinkRequest, PageRequest,
    PageRequestId, PageRequestKind, TabActivation, TabId,
};

fn requested(url: &str, page_url: Option<&str>) -> BrowserEvent {
    BrowserEvent::ExternalLinkRequested(ExternalLinkRequest {
        id: PageRequestId::new(7),
        url: url.to_owned(),
        page_url: page_url.map(str::to_owned),
        received_at: Duration::from_secs(2),
    })
}

#[test]
fn a_request_is_queued_for_its_tab_and_taken_once() {
    let mut controller = BrowserController::default();
    let tab_id = controller.active_tab_id();
    let state_before = controller.active_state().clone();

    let changed = controller.handle_event(
        tab_id,
        requested("mailto:a@example.com", Some("https://example.com/")),
    );

    assert!(!changed, "nothing the window shows has changed");
    assert_eq!(controller.active_state(), &state_before);
    assert_eq!(
        controller.take_external_link_requests(),
        vec![(
            tab_id,
            ExternalLinkRequest {
                id: PageRequestId::new(7),
                url: "mailto:a@example.com".to_owned(),
                page_url: Some("https://example.com/".to_owned()),
                received_at: Duration::from_secs(2),
            }
        )]
    );
    assert_eq!(controller.take_external_link_requests(), vec![]);
}

#[test]
fn requests_keep_their_order_across_tabs() {
    let mut controller = BrowserController::default();
    let first = controller.active_tab_id();
    let second = controller
        .open_tab_with(None, TabActivation::Background)
        .expect("a blank tab opens");

    controller.handle_event(second, requested("tel:1", None));
    controller.handle_event(first, requested("tel:2", None));

    let tabs = controller
        .take_external_link_requests()
        .into_iter()
        .map(|(tab_id, request)| (tab_id, request.url))
        .collect::<Vec<_>>();
    assert_eq!(
        tabs,
        vec![(second, "tel:1".to_owned()), (first, "tel:2".to_owned())]
    );
}

#[test]
fn a_request_from_an_unknown_tab_is_dropped() {
    let mut controller = BrowserController::default();

    assert!(!controller.handle_event(TabId::new(99), requested("tel:1", None)));
    assert_eq!(controller.take_external_link_requests(), vec![]);
}

#[test]
fn closing_a_tab_drops_its_requests() {
    let mut controller = BrowserController::default();
    let first = controller.active_tab_id();
    let second = controller.open_tab();
    controller.handle_event(first, requested("tel:1", None));
    controller.handle_event(second, requested("tel:2", None));

    assert_eq!(controller.close_tab(second), CloseTabOutcome::Closed);

    let urls = controller
        .take_external_link_requests()
        .into_iter()
        .map(|(_, request)| request.url)
        .collect::<Vec<_>>();
    assert_eq!(urls, vec!["tel:1".to_owned()]);
}

fn confirmation(url: &str) -> BrowserEvent {
    BrowserEvent::PageRequested(PageRequest {
        id: PageRequestId::new(9),
        url: "https://example.com/contact".to_owned(),
        kind: PageRequestKind::OpenExternally {
            url: url.to_owned(),
            scheme: "mailto".to_owned(),
            app: Some("Mail".to_owned()),
            always_label: Some("Always allow example.com to open mailto links".to_owned()),
        },
        received_at: Duration::ZERO,
    })
}

#[test]
fn the_confirmation_waits_with_the_tabs_other_requests_and_goes_with_its_page() {
    let mut controller = BrowserController::default();
    let tab_id = controller.active_tab_id();
    controller.handle_event(tab_id, confirmation("mailto:a@example.com"));

    let pending = controller
        .pending_page_request(tab_id)
        .expect("the confirmation waits");
    assert_eq!(pending.origin().label(), "example.com");
    assert!(matches!(
        pending.kind(),
        PageRequestKind::OpenExternally { url, .. } if url == "mailto:a@example.com"
    ));

    controller.handle_event(tab_id, BrowserEvent::PageLoadStarted);
    assert_eq!(controller.pending_page_request(tab_id), None);
}

#[test]
fn the_address_shown_in_the_confirmation_is_sanitised() {
    let mut controller = BrowserController::default();
    let tab_id = controller.active_tab_id();
    let long = format!("mailto:a@example.com?body=\u{202e}{}\n", "x".repeat(5_000));
    controller.handle_event(tab_id, confirmation(&long));

    let pending = controller.pending_page_request(tab_id).expect("pending");
    let PageRequestKind::OpenExternally { url, .. } = pending.kind() else {
        panic!("not a confirmation");
    };
    assert!(!url.contains('\u{202e}'));
    assert!(!url.contains('\n'));
    assert!(url.chars().count() <= 2_001, "{}", url.chars().count());
}
