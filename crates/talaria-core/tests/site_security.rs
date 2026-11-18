// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

mod common;

use common::test_search;
use talaria_core::{
    BrowserCommand, BrowserController, BrowserEvent, ErrorPageKind, HttpsPolicy, HttpsUpgradeChain,
    MainFrameLoad, PagePermission, PageRequestReply, PermissionDecision, SecurityIndicator,
    TabCommand, TabId, main_frame_load, security_indicator, typed_https_upgrade,
};
use url::Url;

fn url(text: &str) -> Url {
    Url::parse(text).expect("a valid URL")
}

fn ready_controller() -> (BrowserController, TabId) {
    let mut controller = BrowserController::default();
    controller.set_search_url_builder(test_search());
    let tab_id = controller.active_tab_id();
    controller.handle_event(tab_id, BrowserEvent::AddressChanged { url: String::new() });
    (controller, tab_id)
}

fn commands(controller: &mut BrowserController) -> Vec<BrowserCommand> {
    std::iter::from_fn(|| controller.next_command())
        .map(TabCommand::into_command)
        .collect()
}

fn navigate(url: &str) -> BrowserCommand {
    BrowserCommand::Navigate {
        url: url.to_owned(),
    }
}

/// The engine shows the page at `url`: its address, a new document, and Mercury's error page
/// if `error` says so.
fn engine_shows(
    controller: &mut BrowserController,
    tab_id: TabId,
    url: &str,
    error: Option<ErrorPageKind>,
) {
    controller.handle_event(tab_id, BrowserEvent::PageLoadStarted);
    controller.handle_event(
        tab_id,
        BrowserEvent::AddressChanged {
            url: url.to_owned(),
        },
    );
    if let Some(kind) = error {
        controller.handle_event(
            tab_id,
            BrowserEvent::ErrorPageShown {
                kind,
                url: url.to_owned(),
            },
        );
    }
}

fn strict() -> HttpsPolicy {
    let mut policy = HttpsPolicy::default();
    policy.https_only = true;
    policy
}

// The indicator

#[test]
fn the_indicator_follows_the_scheme() {
    let cases = [
        ("", SecurityIndicator::None),
        ("https://example.com/", SecurityIndicator::Secure),
        ("http://example.com/", SecurityIndicator::NotSecure),
        ("http://localhost:8000/", SecurityIndicator::LocalHost),
        ("http://127.0.0.1:8000/", SecurityIndicator::LocalHost),
        ("http://[::1]/", SecurityIndicator::LocalHost),
        ("http://192.168.1.2/", SecurityIndicator::NotSecure),
        ("file:///tmp/a.html", SecurityIndicator::File),
        // A page can write an about:blank pop-up or open a data: page: never labelled as Mercury's.
        ("about:blank", SecurityIndicator::None),
        ("about:memory", SecurityIndicator::None),
        ("data:text/html,hi", SecurityIndicator::NotSecure),
        ("blob:https://example.com/0d3e", SecurityIndicator::Secure),
        ("blob:http://example.com/0d3e", SecurityIndicator::NotSecure),
        ("ftp://example.com/", SecurityIndicator::NotSecure),
    ];
    for (address, expected) in cases {
        assert_eq!(
            security_indicator(address, None, false),
            expected,
            "{address}"
        );
    }
}

#[test]
fn error_pages_and_certificate_exceptions_change_the_indicator() {
    let address = "https://example.com/";
    assert_eq!(
        security_indicator(address, Some(ErrorPageKind::Certificate), false),
        SecurityIndicator::CertificateError
    );
    assert_eq!(
        security_indicator(address, Some(ErrorPageKind::Network), false),
        SecurityIndicator::Internal
    );
    assert_eq!(
        security_indicator(address, None, true),
        SecurityIndicator::CertificateException
    );
    assert!(SecurityIndicator::CertificateException.is_warning());
    assert!(SecurityIndicator::NotSecure.is_warning());
    assert!(!SecurityIndicator::Secure.is_warning());
}

#[test]
fn the_controller_reports_the_indicator_of_each_tab() {
    let (mut controller, tab_id) = ready_controller();
    engine_shows(&mut controller, tab_id, "https://bad.example/", None);
    assert_eq!(
        controller.security_indicator(tab_id),
        SecurityIndicator::Secure
    );

    controller.handle_event(
        tab_id,
        BrowserEvent::ErrorPageShown {
            kind: ErrorPageKind::Certificate,
            url: "https://bad.example/".to_owned(),
        },
    );
    assert_eq!(
        controller.security_indicator(tab_id),
        SecurityIndicator::CertificateError
    );
    assert_eq!(
        controller.error_page(tab_id),
        Some(ErrorPageKind::Certificate)
    );

    // The user proceeds: the page loads again, this time with the exception.
    controller.add_certificate_exception("BAD.example");
    engine_shows(&mut controller, tab_id, "https://bad.example/", None);
    assert_eq!(controller.error_page(tab_id), None);
    assert_eq!(
        controller.security_indicator(tab_id),
        SecurityIndicator::CertificateException
    );
    assert!(controller.has_certificate_exception("bad.example"));
}

#[test]
fn an_error_page_for_another_address_does_not_count() {
    let (mut controller, tab_id) = ready_controller();
    engine_shows(&mut controller, tab_id, "https://example.com/", None);
    controller.handle_event(
        tab_id,
        BrowserEvent::ErrorPageShown {
            kind: ErrorPageKind::Certificate,
            url: "https://other.example/".to_owned(),
        },
    );
    assert_eq!(
        controller.security_indicator(tab_id),
        SecurityIndicator::Secure
    );
}

// Typed addresses

#[test]
fn typed_http_addresses_on_the_default_port_are_upgraded() {
    let policy = HttpsPolicy::default();
    assert_eq!(
        typed_https_upgrade(&policy, &url("http://example.com/a?b#c")),
        Some(url("https://example.com/a?b#c"))
    );
    assert_eq!(
        typed_https_upgrade(&policy, &url("http://example.com:80/")),
        Some(url("https://example.com/"))
    );
    assert_eq!(
        typed_https_upgrade(&policy, &url("http://example.com:8080/")),
        None
    );
    assert_eq!(
        typed_https_upgrade(&policy, &url("https://example.com/")),
        None
    );
}

#[test]
fn local_names_are_not_upgraded() {
    let policy = HttpsPolicy::default();
    for address in [
        "http://localhost/",
        "http://app.localhost/",
        "http://127.0.0.1/",
        "http://10.0.0.1/",
        "http://172.16.4.4/",
        "http://192.168.0.1/",
        "http://169.254.0.1/",
        "http://100.64.0.1/",
        "http://[::1]/",
        "http://[fd00::1]/",
        "http://[fe80::1]/",
        "http://router/",
        "http://printer.local/",
        "http://nas.home.arpa/",
        "http://host.internal/",
        "http://box.lan/",
    ] {
        assert_eq!(
            typed_https_upgrade(&policy, &url(address)),
            None,
            "{address}"
        );
        assert_eq!(
            main_frame_load(&strict(), &url(address), true),
            MainFrameLoad::Allow,
            "{address}"
        );
    }
    let mut local = HttpsPolicy::default();
    local.upgrade_local_names = true;
    assert_eq!(
        typed_https_upgrade(&local, &url("http://127.0.0.1:8471/x")),
        Some(url("https://127.0.0.1:8471/x"))
    );
}

#[test]
fn a_typed_http_address_falls_back_when_https_fails() {
    let (mut controller, tab_id) = ready_controller();
    controller
        .queue_navigation("http://example.com/page")
        .expect("navigates");
    assert_eq!(
        commands(&mut controller),
        vec![navigate("https://example.com/page")]
    );

    engine_shows(
        &mut controller,
        tab_id,
        "https://example.com/page",
        Some(ErrorPageKind::Network),
    );
    assert_eq!(
        commands(&mut controller),
        vec![navigate("http://example.com/page")]
    );
    assert_eq!(controller.https_only_interstitial(tab_id), None);

    // Once it has fallen back, a later error page is just an error page.
    engine_shows(&mut controller, tab_id, "http://example.com/page", None);
    engine_shows(
        &mut controller,
        tab_id,
        "https://example.com/page",
        Some(ErrorPageKind::Network),
    );
    assert_eq!(commands(&mut controller), vec![]);
}

#[test]
fn a_certificate_error_on_an_upgrade_also_falls_back() {
    let (mut controller, tab_id) = ready_controller();
    controller
        .queue_navigation("http://example.com/")
        .expect("navigates");
    commands(&mut controller);
    engine_shows(
        &mut controller,
        tab_id,
        "https://example.com/",
        Some(ErrorPageKind::Certificate),
    );
    assert_eq!(
        commands(&mut controller),
        vec![navigate("http://example.com/")]
    );
}

#[test]
fn a_successful_upgrade_stays_on_https() {
    let (mut controller, tab_id) = ready_controller();
    controller
        .queue_navigation("http://example.com/")
        .expect("navigates");
    commands(&mut controller);
    engine_shows(&mut controller, tab_id, "https://example.com/", None);
    engine_shows(&mut controller, tab_id, "https://example.com/next", None);
    engine_shows(
        &mut controller,
        tab_id,
        "https://example.com/",
        Some(ErrorPageKind::Network),
    );
    assert_eq!(commands(&mut controller), vec![]);
}

#[test]
fn addresses_without_a_scheme_and_https_addresses_are_left_alone() {
    let (mut controller, _) = ready_controller();
    controller
        .queue_navigation("example.com")
        .expect("navigates");
    controller
        .queue_navigation("https://example.com/")
        .expect("navigates");
    controller
        .queue_navigation("http://localhost:8000/")
        .expect("navigates");
    assert_eq!(
        commands(&mut controller),
        vec![
            navigate("https://example.com/"),
            navigate("https://example.com/"),
            navigate("http://localhost:8000/"),
        ]
    );
}

// HTTPS-only mode

#[test]
fn https_only_mode_upgrades_top_level_http_loads() {
    assert_eq!(
        main_frame_load(&HttpsPolicy::default(), &url("http://example.com/"), true,),
        MainFrameLoad::Allow
    );
    assert_eq!(
        main_frame_load(&strict(), &url("http://example.com:8080/a"), true),
        MainFrameLoad::Upgrade(url("https://example.com:8080/a"))
    );
    assert_eq!(
        main_frame_load(&strict(), &url("https://example.com/"), true),
        MainFrameLoad::Allow
    );
    // A form submission cannot be resent over HTTPS.
    assert_eq!(
        main_frame_load(&strict(), &url("http://example.com/"), false),
        MainFrameLoad::Block
    );
}

#[test]
fn exceptions_let_a_site_load_over_http() {
    let mut policy = strict();
    assert!(policy.add_exception("Example.com"));
    assert_eq!(
        main_frame_load(&policy, &url("http://example.com/"), false),
        MainFrameLoad::Allow
    );
    assert_eq!(
        typed_https_upgrade(&policy, &url("http://example.com/")),
        None
    );
    assert_eq!(policy.exceptions().collect::<Vec<_>>(), vec!["example.com"]);
    assert!(policy.remove_exception("example.com"));
    assert!(!policy.has_exception("example.com"));
}

#[test]
fn a_failed_upgrade_in_https_only_mode_shows_the_interstitial() {
    let (mut controller, tab_id) = ready_controller();
    controller.set_https_only(true);
    engine_shows(&mut controller, tab_id, "https://start.example/", None);
    controller.handle_event(
        tab_id,
        BrowserEvent::HttpsUpgraded {
            from: "http://plain.example/".to_owned(),
            to: "https://plain.example/".to_owned(),
        },
    );
    assert_eq!(
        commands(&mut controller),
        vec![navigate("https://plain.example/")]
    );
    engine_shows(
        &mut controller,
        tab_id,
        "https://plain.example/",
        Some(ErrorPageKind::Network),
    );
    let interstitial = controller
        .https_only_interstitial(tab_id)
        .expect("an interstitial");
    assert_eq!(interstitial.http_url, "http://plain.example/");
    assert_eq!(
        interstitial.failed_https_url.as_deref(),
        Some("https://plain.example/")
    );
    assert_eq!(interstitial.host(), "plain.example");
    assert_eq!(commands(&mut controller), vec![]);

    assert_eq!(
        controller.continue_to_http_site(tab_id).as_deref(),
        Some("plain.example")
    );
    assert!(controller.https_policy().has_exception("plain.example"));
    assert_eq!(controller.https_only_interstitial(tab_id), None);
    assert_eq!(
        commands(&mut controller),
        vec![navigate("http://plain.example/")]
    );
    assert_eq!(controller.continue_to_http_site(tab_id), None);
}

#[test]
fn an_upgrade_by_the_engine_makes_the_tab_load_the_https_address() {
    let (mut controller, tab_id) = ready_controller();
    controller.handle_event(
        tab_id,
        BrowserEvent::HttpsUpgraded {
            from: "http://plain.example/".to_owned(),
            to: "https://plain.example/".to_owned(),
        },
    );
    assert_eq!(
        commands(&mut controller),
        vec![navigate("https://plain.example/")]
    );

    // A tab whose first page never committed starts again at the HTTPS address.
    let mut controller = BrowserController::default();
    let tab_id = controller
        .open_tab_with(None, talaria_core::TabActivation::Foreground)
        .expect("a tab");
    controller.handle_event(
        tab_id,
        BrowserEvent::HttpsUpgraded {
            from: "http://plain.example/".to_owned(),
            to: "https://plain.example/".to_owned(),
        },
    );
    assert_eq!(
        controller.take_initial_url(tab_id).as_deref(),
        Some("https://plain.example/")
    );
    assert_eq!(controller.active_state().url(), "https://plain.example/");
}

#[test]
fn a_typed_http_address_in_https_only_mode_keeps_its_port_and_shows_the_interstitial() {
    let (mut controller, tab_id) = ready_controller();
    controller.set_https_only(true);
    controller
        .queue_navigation("http://plain.example:8080/")
        .expect("navigates");
    assert_eq!(
        commands(&mut controller),
        vec![navigate("https://plain.example:8080/")]
    );
    engine_shows(
        &mut controller,
        tab_id,
        "https://plain.example:8080/",
        Some(ErrorPageKind::Network),
    );
    assert!(controller.https_only_interstitial(tab_id).is_some());
    assert_eq!(commands(&mut controller), vec![]);
}

#[test]
fn a_blocked_load_shows_the_interstitial_and_going_back_leaves_it() {
    let (mut controller, tab_id) = ready_controller();
    controller.set_https_only(true);
    engine_shows(&mut controller, tab_id, "https://start.example/", None);
    controller.handle_event(
        tab_id,
        BrowserEvent::HistoryChanged {
            can_go_back: true,
            can_go_forward: false,
        },
    );
    controller.handle_event(
        tab_id,
        BrowserEvent::HttpsOnlyBlocked {
            url: "http://form.example/submit".to_owned(),
        },
    );
    assert_eq!(
        controller
            .https_only_interstitial(tab_id)
            .map(|interstitial| interstitial.failed_https_url.clone()),
        Some(None)
    );
    assert!(controller.leave_https_only_interstitial(tab_id));
    assert_eq!(commands(&mut controller), vec![BrowserCommand::GoBack]);
    assert!(!controller.leave_https_only_interstitial(tab_id));
}

#[test]
fn going_back_without_history_opens_a_blank_page() {
    let (mut controller, tab_id) = ready_controller();
    controller.handle_event(
        tab_id,
        BrowserEvent::HttpsOnlyBlocked {
            url: "http://form.example/".to_owned(),
        },
    );
    assert!(controller.leave_https_only_interstitial(tab_id));
    assert_eq!(commands(&mut controller), vec![navigate("about:blank")]);
}

#[test]
fn the_interstitial_goes_when_the_tab_moves_on_or_the_mode_is_turned_off() {
    let (mut controller, tab_id) = ready_controller();
    controller.handle_event(
        tab_id,
        BrowserEvent::HttpsOnlyBlocked {
            url: "http://form.example/".to_owned(),
        },
    );
    engine_shows(&mut controller, tab_id, "https://elsewhere.example/", None);
    assert_eq!(controller.https_only_interstitial(tab_id), None);

    controller.handle_event(
        tab_id,
        BrowserEvent::HttpsOnlyBlocked {
            url: "http://form.example/".to_owned(),
        },
    );
    controller.set_https_only(false);
    assert_eq!(controller.https_only_interstitial(tab_id), None);

    controller.handle_event(
        tab_id,
        BrowserEvent::HttpsOnlyBlocked {
            url: "http://form.example/".to_owned(),
        },
    );
    controller
        .queue_navigation("https://typed.example/")
        .expect("navigates");
    assert_eq!(controller.https_only_interstitial(tab_id), None);
}

// Permissions in the site-information popover

#[test]
fn remembered_permissions_can_be_listed_and_forgotten() {
    use talaria_core::{PageRequest, PageRequestAnswer, PageRequestId, PageRequestKind};

    let (mut controller, tab_id) = ready_controller();
    engine_shows(&mut controller, tab_id, "https://example.com/", None);
    controller.handle_event(
        tab_id,
        BrowserEvent::PageRequested(PageRequest {
            id: PageRequestId::new(1),
            url: "https://example.com/".to_owned(),
            kind: PageRequestKind::Permission(PagePermission::PersistentStorage),
            received_at: std::time::Duration::ZERO,
        }),
    );
    let pending = controller
        .pending_page_request(tab_id)
        .expect("a permission prompt");
    assert!(controller.answer_page_request(
        tab_id,
        pending.id(),
        PageRequestReply::new(PageRequestAnswer::Accept).remembering_for_site(),
        std::time::Duration::from_secs(1),
    ));
    assert_eq!(
        controller.site_permissions("https://example.com"),
        vec![(PagePermission::PersistentStorage, PermissionDecision::Allow)]
    );
    assert_eq!(controller.site_permissions("https://other.example"), vec![]);
    controller.forget_site_permission("https://example.com", PagePermission::PersistentStorage);
    assert_eq!(controller.site_permissions("https://example.com"), vec![]);
}

// The HTTPS-only upgrade chain of one tab (applied by the engine's load interception)

fn chain_decides(
    chain: &mut HttpsUpgradeChain,
    policy: &HttpsPolicy,
    address: &str,
) -> MainFrameLoad {
    chain.decide(policy, &url(address), true, true)
}

// A finished load starts a new chain, so the same `http://` address is upgraded again. This also
// pins the accepted gap that a page may loop through script or a meta refresh to its own `http`
// address (see the `HttpsUpgradeChain` docs); do not "fix" it by remembering across loads.
#[test]
fn the_chain_upgrades_the_same_link_again_after_a_finished_load() {
    let policy = strict();
    let mut chain = HttpsUpgradeChain::default();
    assert_eq!(
        chain_decides(&mut chain, &policy, "http://example.com/a"),
        MainFrameLoad::Upgrade(url("https://example.com/a"))
    );
    chain.load_finished();
    // The user clicks the same http:// link again later.
    assert_eq!(
        chain_decides(&mut chain, &policy, "http://example.com/a"),
        MainFrameLoad::Upgrade(url("https://example.com/a"))
    );
}

#[test]
fn the_chain_blocks_a_redirect_ping_pong() {
    let policy = strict();
    let mut chain = HttpsUpgradeChain::default();
    // https://a/1 sends to http://a/2, https://a/2 sends to http://a/1, and no load finishes.
    assert!(matches!(
        chain_decides(&mut chain, &policy, "http://a.example/1"),
        MainFrameLoad::Upgrade(_)
    ));
    assert!(matches!(
        chain_decides(&mut chain, &policy, "http://a.example/2"),
        MainFrameLoad::Upgrade(_)
    ));
    assert_eq!(
        chain_decides(&mut chain, &policy, "http://a.example/1"),
        MainFrameLoad::Block
    );
}

#[test]
fn the_chain_blocks_after_too_many_upgrades_without_a_finished_load() {
    let policy = strict();
    let mut chain = HttpsUpgradeChain::default();
    for step in 0..HttpsUpgradeChain::MAX_UPGRADES {
        assert!(
            matches!(
                chain_decides(&mut chain, &policy, &format!("http://a.example/{step}")),
                MainFrameLoad::Upgrade(_)
            ),
            "{step}"
        );
    }
    assert_eq!(
        chain_decides(&mut chain, &policy, "http://a.example/last"),
        MainFrameLoad::Block
    );
    chain.load_finished();
    assert!(matches!(
        chain_decides(&mut chain, &policy, "http://a.example/last"),
        MainFrameLoad::Upgrade(_)
    ));
}

#[test]
fn the_chain_leaves_frames_and_allowed_loads_alone() {
    let policy = strict();
    let mut chain = HttpsUpgradeChain::default();
    // A frame's load is never upgraded or counted.
    assert_eq!(
        chain.decide(&policy, &url("http://example.com/"), true, false),
        MainFrameLoad::Allow
    );
    assert_eq!(
        chain.decide(
            &HttpsPolicy::default(),
            &url("http://example.com/"),
            true,
            true
        ),
        MainFrameLoad::Allow
    );
    // A form submission is blocked.
    assert_eq!(
        chain.decide(&policy, &url("http://example.com/"), false, true),
        MainFrameLoad::Block
    );
}

#[test]
fn the_chain_says_what_to_tell_the_browser() {
    let address = url("http://example.com/");
    assert_eq!(
        HttpsUpgradeChain::event_for(&address, &MainFrameLoad::Allow),
        None
    );
    assert_eq!(
        HttpsUpgradeChain::event_for(
            &address,
            &MainFrameLoad::Upgrade(url("https://example.com/"))
        ),
        Some(BrowserEvent::HttpsUpgraded {
            from: "http://example.com/".to_owned(),
            to: "https://example.com/".to_owned(),
        })
    );
    assert_eq!(
        HttpsUpgradeChain::event_for(&address, &MainFrameLoad::Block),
        Some(BrowserEvent::HttpsOnlyBlocked {
            url: "http://example.com/".to_owned(),
        })
    );
}
