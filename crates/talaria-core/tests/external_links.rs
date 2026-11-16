// openurl: links a page asks the system to open in another app.

use talaria_core::{BrowserController, BrowserEvent, ExternalLinkRequest, TabActivation, TabId};

fn requested(url: &str, page_url: Option<&str>) -> BrowserEvent {
    BrowserEvent::ExternalLinkRequested {
        url: url.to_owned(),
        page_url: page_url.map(str::to_owned),
    }
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
        vec![ExternalLinkRequest {
            tab_id,
            url: "mailto:a@example.com".to_owned(),
            page_url: Some("https://example.com/".to_owned()),
        }]
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
        .map(|request| (request.tab_id, request.url))
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

    assert!(controller.close_tab(second));

    let urls = controller
        .take_external_link_requests()
        .into_iter()
        .map(|request| request.url)
        .collect::<Vec<_>>();
    assert_eq!(urls, vec!["tel:1".to_owned()]);
}
