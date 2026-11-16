use talaria_core::{
    BrowserCommand, BrowserController, BrowserEvent, LoadState, RestoredTab, TabCommand, TabId,
};

fn restored(url: Option<&str>, title: &str, zoom_percent: u16) -> RestoredTab {
    RestoredTab {
        url: url.map(str::to_owned),
        title: title.to_owned(),
        zoom_percent,
    }
}

fn tab_ids(controller: &BrowserController) -> Vec<TabId> {
    controller.tabs().map(|tab| tab.id()).collect()
}

fn state_of(controller: &BrowserController, tab_id: TabId) -> (String, String, u16, LoadState) {
    let tab = controller
        .tabs()
        .find(|tab| tab.id() == tab_id)
        .expect("the tab exists");
    let state = tab.state();
    (
        state.url().to_owned(),
        state.page_title().to_owned(),
        state.zoom_percent(),
        state.load_state().clone(),
    )
}

#[test]
fn a_restored_tab_opens_in_the_background_showing_its_saved_address_and_title() {
    let mut controller = BrowserController::default();
    let first = controller.active_tab_id();

    let restored_id = controller
        .open_restored_tab(1, &restored(Some("https://a.test/page"), "Page A", 150))
        .expect("the address resolves");

    assert_eq!(controller.active_tab_id(), first);
    assert_eq!(tab_ids(&controller), vec![first, restored_id]);
    assert_eq!(
        state_of(&controller, restored_id),
        (
            "https://a.test/page".to_owned(),
            "Page A".to_owned(),
            150,
            LoadState::Idle
        )
    );
}

#[test]
fn a_restored_tab_does_not_load_until_it_is_first_selected() {
    let mut controller = BrowserController::default();
    let restored_id = controller
        .open_restored_tab(1, &restored(Some("https://a.test/"), "A", 100))
        .unwrap();

    assert!(controller.is_load_deferred(restored_id));
    assert_eq!(controller.take_initial_url(restored_id), None);
    assert!(
        controller.is_load_deferred(restored_id),
        "asking for the URL early must not use it up"
    );

    assert!(controller.select_tab(restored_id));

    assert!(!controller.is_load_deferred(restored_id));
    assert_eq!(
        controller.take_initial_url(restored_id),
        Some("https://a.test/".to_owned())
    );
    assert_eq!(
        state_of(&controller, restored_id).3,
        LoadState::Loading,
        "once handed to the engine the tab shows that it is loading"
    );
}

#[test]
fn a_restored_tab_stays_loaded_after_it_is_left() {
    let mut controller = BrowserController::default();
    let first = controller.active_tab_id();
    let restored_id = controller
        .open_restored_tab(1, &restored(Some("https://a.test/"), "A", 100))
        .unwrap();
    controller.select_tab(restored_id);
    let _url = controller.take_initial_url(restored_id);

    controller.select_tab(first);

    assert!(!controller.is_load_deferred(restored_id));
    assert_eq!(controller.take_initial_url(restored_id), None);
}

#[test]
fn selecting_a_restored_tab_by_cycling_also_lets_it_load() {
    let mut controller = BrowserController::default();
    let restored_id = controller
        .open_restored_tab(1, &restored(Some("https://a.test/"), "A", 100))
        .unwrap();

    controller.select_next_tab(false);

    assert_eq!(controller.active_tab_id(), restored_id);
    assert!(!controller.is_load_deferred(restored_id));
}

#[test]
fn a_selected_restored_tab_that_was_left_before_the_engine_took_it_waits_again() {
    let mut controller = BrowserController::default();
    let first = controller.active_tab_id();
    let restored_id = controller
        .open_restored_tab(1, &restored(Some("https://a.test/"), "A", 100))
        .unwrap();

    controller.select_tab(restored_id);
    controller.select_tab(first);

    assert!(controller.is_load_deferred(restored_id));
}

#[test]
fn commands_for_a_waiting_restored_tab_are_held_and_pointer_input_is_dropped() {
    let mut controller = BrowserController::default();
    let restored_id = controller
        .open_restored_tab(1, &restored(Some("https://a.test/"), "A", 100))
        .unwrap();

    assert!(controller.queue_command_for(restored_id, BrowserCommand::Reload));
    assert!(controller.queue_command_for(restored_id, BrowserCommand::PointerLeft));
    assert!(!controller.has_pending_commands());
    assert_eq!(controller.next_command(), None);

    controller.select_tab(restored_id);
    let _url = controller.take_initial_url(restored_id);
    assert!(
        !controller.has_pending_commands(),
        "a web view that has reported nothing yet takes no commands"
    );
    controller.handle_event(
        restored_id,
        BrowserEvent::AddressChanged {
            url: "https://a.test/".to_owned(),
        },
    );

    assert_eq!(
        controller.next_command(),
        Some(TabCommand::new(restored_id, BrowserCommand::Reload))
    );
    assert_eq!(controller.next_command(), None);
}

#[test]
fn restored_tabs_are_inserted_at_their_index_clamped_to_the_strip() {
    let mut controller = BrowserController::default();
    let first = controller.active_tab_id();

    let at_end = controller
        .open_restored_tab(99, &restored(Some("https://end.test/"), "End", 100))
        .unwrap();
    let at_start = controller
        .open_restored_tab(0, &restored(Some("https://start.test/"), "Start", 100))
        .unwrap();
    let between = controller
        .open_restored_tab(2, &restored(Some("https://mid.test/"), "Mid", 100))
        .unwrap();

    assert_eq!(tab_ids(&controller), vec![at_start, first, between, at_end]);
}

#[test]
fn a_restored_new_tab_page_opens_blank_when_selected() {
    let mut controller = BrowserController::default();
    let restored_id = controller
        .open_restored_tab(1, &restored(None, "", 100))
        .unwrap();

    assert!(controller.is_load_deferred(restored_id));
    assert_eq!(
        state_of(&controller, restored_id),
        (String::new(), "New Tab".to_owned(), 100, LoadState::Idle)
    );

    controller.select_tab(restored_id);
    assert!(!controller.is_load_deferred(restored_id));
    assert_eq!(controller.take_initial_url(restored_id), None);
}

#[test]
fn an_unusable_restored_address_opens_no_tab() {
    let mut controller = BrowserController::default();

    let result = controller.open_restored_tab(1, &restored(Some("javascript:alert(1)"), "x", 100));

    assert!(result.is_err());
    assert_eq!(controller.tabs().len(), 1);
}

#[test]
fn a_restored_zoom_outside_the_steps_is_clamped() {
    let mut controller = BrowserController::default();
    let low = controller
        .open_restored_tab(1, &restored(Some("https://a.test/"), "A", 5))
        .unwrap();
    let high = controller
        .open_restored_tab(2, &restored(Some("https://b.test/"), "B", 9000))
        .unwrap();

    assert_eq!(state_of(&controller, low).2, 30);
    assert_eq!(state_of(&controller, high).2, 500);
}

#[test]
fn ordinary_tabs_are_never_deferred() {
    let mut controller = BrowserController::default();
    let first = controller.active_tab_id();
    let background = controller
        .open_tab_with(
            Some("https://b.test/"),
            talaria_core::TabActivation::Background,
        )
        .unwrap();

    assert!(!controller.is_load_deferred(first));
    assert!(!controller.is_load_deferred(background));
    assert!(!controller.is_load_deferred(TabId::new(999)));
}

#[test]
fn a_waiting_restored_tab_can_be_closed() {
    let mut controller = BrowserController::default();
    let first = controller.active_tab_id();
    let restored_id = controller
        .open_restored_tab(1, &restored(Some("https://a.test/"), "A", 100))
        .unwrap();

    assert!(controller.close_tab(restored_id));
    assert_eq!(tab_ids(&controller), vec![first]);
}

#[test]
fn a_waiting_tab_holds_only_the_latest_colour_scheme() {
    use talaria_core::ColorScheme;

    let mut controller = BrowserController::default();
    let restored_id = controller
        .open_restored_tab(1, &restored(Some("https://a.test/"), "A", 100))
        .unwrap();
    controller.queue_command_for(restored_id, BrowserCommand::Reload);
    for scheme in [ColorScheme::Dark, ColorScheme::Light, ColorScheme::Dark] {
        controller.set_color_scheme(scheme);
    }

    controller.select_tab(restored_id);
    let _url = controller.take_initial_url(restored_id);
    controller.handle_event(
        restored_id,
        BrowserEvent::AddressChanged {
            url: "https://a.test/".to_owned(),
        },
    );
    let held = std::iter::from_fn(|| controller.next_command())
        .filter(|queued| queued.tab_id() == restored_id)
        .map(TabCommand::into_command)
        .collect::<Vec<_>>();

    assert_eq!(
        held,
        vec![
            BrowserCommand::Reload,
            BrowserCommand::SetColorScheme(ColorScheme::Dark),
        ]
    );
}

#[test]
fn a_restored_tab_is_known_to_need_its_page_until_the_engine_takes_it() {
    let mut controller = BrowserController::default();
    let first = controller.active_tab_id();
    let restored_id = controller
        .open_restored_tab(1, &restored(Some("https://a.test/"), "A", 100))
        .unwrap();

    assert!(controller.is_restored_without_page(restored_id));
    controller.select_tab(restored_id);
    assert!(
        controller.is_restored_without_page(restored_id),
        "selected, but the engine has not created its page yet"
    );
    let _url = controller.take_initial_url(restored_id);
    assert!(!controller.is_restored_without_page(restored_id));
    assert!(!controller.is_restored_without_page(first));
    assert!(!controller.is_restored_without_page(TabId::new(999)));
}
