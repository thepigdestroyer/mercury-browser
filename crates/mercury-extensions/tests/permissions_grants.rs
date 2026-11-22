// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! Site access, the `activeTab` lifecycle, optional permission requests, private windows, the kill
//! switch and update consent.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use mercury_extensions::id::ExtensionId;
use mercury_extensions::manifest::{Manifest, ManifestLimits};
use mercury_extensions::match_pattern::{MatchPattern, MatchPatternSet};
use mercury_extensions::permissions::{
    ActiveTabError, ActiveTabGrants, ApiPermission, BlockedSurface, CrashOutcome, Defaults,
    GrantReason, HostGrants, IncognitoPolicy, KillState, KillSwitch, KillSwitchConfig,
    PermissionError, PermissionRequest, PermissionSet, SiteAccess, TabId, UserGesture,
    may_run_in_private_window, permissions_added, plan_permission_removal, plan_permission_request,
    update_needs_consent,
};
use url::Url;

fn manifest(extra: &str) -> Manifest {
    let json = format!(r#"{{"manifest_version":3,"name":"T","version":"1",{extra}}}"#);
    Manifest::parse(json.as_bytes(), &ManifestLimits::default()).unwrap()
}

fn pat(s: &str) -> MatchPattern {
    MatchPattern::parse(s).unwrap()
}

fn pats(items: &[&str]) -> MatchPatternSet {
    MatchPatternSet::parse_all(items).unwrap()
}

fn u(s: &str) -> Url {
    Url::parse(s).unwrap()
}

fn t(secs: u64) -> SystemTime {
    UNIX_EPOCH + Duration::from_secs(secs)
}

fn ext(n: u8) -> ExtensionId {
    let letter = char::from(b'a' + n);
    ExtensionId::parse(&letter.to_string().repeat(32)).unwrap()
}

// ---- host grants and site access ---------------------------------------------------------

#[test]
fn defaults_withhold_broad_hosts() {
    assert!(Defaults::default().withhold_broad_hosts);
}

#[test]
fn broad_hosts_start_withheld_and_specific_ones_do_not() {
    let g = HostGrants::new(
        &pats(&["<all_urls>", "https://a.com/*"]),
        &MatchPatternSet::new(),
        &Defaults::default(),
    );
    assert!(g.allows(&u("https://a.com/x")));
    assert!(!g.allows(&u("https://other.org/")));
    assert!(matches!(g.site_access(), SiteAccess::OnSpecificSites(s) if s.len() == 1));
}

#[test]
fn only_broad_hosts_means_on_click() {
    let g = HostGrants::new(
        &pats(&["*://*/*"]),
        &MatchPatternSet::new(),
        &Defaults::default(),
    );
    assert_eq!(g.site_access(), SiteAccess::OnClick);
}

#[test]
fn without_withholding_everything_requested_is_granted() {
    let g = HostGrants::new(
        &pats(&["<all_urls>"]),
        &MatchPatternSet::new(),
        &Defaults {
            withhold_broad_hosts: false,
        },
    );
    assert!(g.allows(&u("https://anything.example/")));
    assert_eq!(g.site_access(), SiteAccess::OnAllSites);
}

#[test]
fn on_all_sites_grants_what_was_requested_and_nothing_more() {
    let mut g = HostGrants::new(
        &pats(&["https://*/*"]),
        &MatchPatternSet::new(),
        &Defaults::default(),
    );
    g.set_site_access(SiteAccess::OnAllSites);
    assert!(g.allows(&u("https://a.com/")));
    assert!(!g.allows(&u("http://a.com/")));
    assert_eq!(g.site_access(), SiteAccess::OnAllSites);
}

#[test]
fn on_click_removes_everything() {
    let mut g = HostGrants::new(
        &pats(&["https://a.com/*"]),
        &MatchPatternSet::new(),
        &Defaults::default(),
    );
    g.set_site_access(SiteAccess::OnClick);
    assert!(!g.allows(&u("https://a.com/")));
    assert_eq!(g.site_access(), SiteAccess::OnClick);
}

#[test]
fn on_specific_sites_intersects_with_what_was_requested() {
    let mut g = HostGrants::new(
        &pats(&["https://*/*"]),
        &MatchPatternSet::new(),
        &Defaults::default(),
    );
    g.set_site_access(SiteAccess::OnSpecificSites(pats(&[
        "https://a.com/*",
        "http://b.com/*",
    ])));
    assert!(g.allows(&u("https://a.com/x")));
    assert!(!g.allows(&u("https://b.com/")));
    assert!(!g.allows(&u("http://b.com/")), "http was never requested");
    assert!(matches!(g.site_access(), SiteAccess::OnSpecificSites(_)));
}

#[test]
fn on_specific_sites_keeps_requested_hosts_inside_the_chosen_ones() {
    let mut g = HostGrants::new(
        &pats(&["https://x.a.com/*", "https://y.b.com/*"]),
        &MatchPatternSet::new(),
        &Defaults::default(),
    );
    g.set_site_access(SiteAccess::OnSpecificSites(pats(&["https://*.a.com/*"])));
    assert!(g.allows(&u("https://x.a.com/")));
    assert!(!g.allows(&u("https://y.b.com/")));
}

#[test]
fn granting_outside_the_request_is_refused() {
    let mut g = HostGrants::new(
        &pats(&["https://a.com/*"]),
        &pats(&["https://b.com/*"]),
        &Defaults::default(),
    );
    assert_eq!(
        g.grant(pat("https://c.com/*")),
        Err(PermissionError::NotRequested)
    );
    assert_eq!(
        g.grant(pat("https://*/*")),
        Err(PermissionError::NotRequested)
    );
    assert!(g.grant(pat("https://b.com/anything")).is_ok());
    assert!(g.allows(&u("https://b.com/other")));
}

#[test]
fn a_narrower_grant_inside_a_broad_request_works() {
    let mut g = HostGrants::new(
        &pats(&["<all_urls>"]),
        &MatchPatternSet::new(),
        &Defaults::default(),
    );
    g.grant(pat("https://a.com/*")).unwrap();
    assert!(g.allows(&u("https://a.com/x")));
    assert!(!g.allows(&u("https://b.com/")));
}

#[test]
fn the_latest_decision_wins() {
    let mut g = HostGrants::new(
        &pats(&["https://*.a.com/*"]),
        &MatchPatternSet::new(),
        &Defaults::default(),
    );
    assert!(g.allows(&u("https://x.a.com/")));
    g.withhold(pat("https://*.a.com/*"));
    assert!(!g.allows(&u("https://x.a.com/")));
    g.grant(pat("https://x.a.com/*")).unwrap();
    assert!(g.allows(&u("https://x.a.com/")));
    assert!(!g.allows(&u("https://y.a.com/")));
    g.withhold(pat("https://x.a.com/*"));
    assert!(!g.allows(&u("https://x.a.com/")));
}

#[test]
fn effective_hosts_reflect_decisions() {
    let mut g = HostGrants::new(
        &pats(&["https://a.com/*", "https://b.com/*"]),
        &MatchPatternSet::new(),
        &Defaults::default(),
    );
    g.withhold(pat("https://a.com/*"));
    let eff = g.effective_hosts();
    assert_eq!(eff.len(), 1);
    assert!(eff.matches(&u("https://b.com/")));
    g.grant(pat("https://a.com/*")).unwrap();
    assert_eq!(g.effective_hosts().len(), 2);
}

#[test]
fn decisions_stay_bounded() {
    let mut g = HostGrants::new(
        &MatchPatternSet::new(),
        &pats(&["https://*/*"]),
        &Defaults::default(),
    );
    for i in 0..1500 {
        g.grant(pat(&format!("https://h{i}.example/*"))).unwrap();
    }
    assert!(g.allows(&u("https://h1499.example/")));
    assert!(g.effective_hosts().len() <= 1100);
}

// ---- activeTab lifecycle -----------------------------------------------------------------

fn gesture() -> UserGesture {
    UserGesture::from_user_input()
}

#[test]
fn an_active_tab_grant_covers_one_tab_and_one_origin() {
    let mut a = ActiveTabGrants::new();
    a.grant(
        TabId(1),
        &u("https://example.com/page"),
        GrantReason::ActionClick,
        &gesture(),
    )
    .unwrap();
    assert!(a.covers(TabId(1), &u("https://example.com/anything?q=1")));
    assert!(!a.covers(TabId(1), &u("https://other.com/")));
    assert!(
        !a.covers(TabId(1), &u("http://example.com/")),
        "another scheme is another origin"
    );
    assert!(
        !a.covers(TabId(1), &u("https://example.com:8443/")),
        "another port is another origin"
    );
    assert!(!a.covers(TabId(1), &u("https://sub.example.com/")));
    assert!(!a.covers(TabId(2), &u("https://example.com/")));
    assert_eq!(a.get(TabId(1)).unwrap().reason, GrantReason::ActionClick);
    assert_eq!(a.len(), 1);
}

#[test]
fn every_gesture_source_can_grant() {
    for (i, reason) in [
        GrantReason::ActionClick,
        GrantReason::ContextMenu,
        GrantReason::KeyboardCommand,
        GrantReason::Omnibox,
    ]
    .into_iter()
    .enumerate()
    {
        let mut a = ActiveTabGrants::new();
        let tab = TabId(i64::try_from(i).unwrap());
        a.grant(tab, &u("https://example.com/"), reason, &gesture())
            .unwrap();
        assert_eq!(a.get(tab).unwrap().reason, reason);
    }
}

#[test]
fn a_cross_document_navigation_ends_the_grant_even_for_the_same_origin() {
    let mut a = ActiveTabGrants::new();
    a.grant(
        TabId(1),
        &u("https://example.com/a"),
        GrantReason::ActionClick,
        &gesture(),
    )
    .unwrap();
    a.on_navigation(TabId(1), &u("https://example.com/b"), false);
    assert!(a.is_empty());
    assert!(!a.covers(TabId(1), &u("https://example.com/b")));
}

#[test]
fn a_cross_origin_navigation_ends_the_grant() {
    let mut a = ActiveTabGrants::new();
    a.grant(
        TabId(1),
        &u("https://example.com/a"),
        GrantReason::ActionClick,
        &gesture(),
    )
    .unwrap();
    a.on_navigation(TabId(1), &u("https://evil.example/"), false);
    assert!(!a.covers(TabId(1), &u("https://evil.example/")));
    assert!(!a.covers(TabId(1), &u("https://example.com/")));
}

#[test]
fn a_same_document_navigation_keeps_the_grant() {
    let mut a = ActiveTabGrants::new();
    a.grant(
        TabId(1),
        &u("https://example.com/a"),
        GrantReason::ActionClick,
        &gesture(),
    )
    .unwrap();
    a.on_navigation(TabId(1), &u("https://example.com/a#section"), true);
    a.on_navigation(TabId(1), &u("https://example.com/spa/route"), true);
    assert!(a.covers(TabId(1), &u("https://example.com/spa/route")));
}

#[test]
fn closing_the_tab_ends_the_grant() {
    let mut a = ActiveTabGrants::new();
    a.grant(
        TabId(1),
        &u("https://example.com/"),
        GrantReason::ActionClick,
        &gesture(),
    )
    .unwrap();
    a.grant(
        TabId(2),
        &u("https://example.com/"),
        GrantReason::ActionClick,
        &gesture(),
    )
    .unwrap();
    a.on_tab_closed(TabId(1));
    assert!(a.get(TabId(1)).is_none());
    assert!(a.get(TabId(2)).is_some());
    a.on_tab_closed(TabId(99));
    a.clear();
    assert!(a.is_empty());
}

#[test]
fn a_navigation_of_another_tab_leaves_the_grant_alone() {
    let mut a = ActiveTabGrants::new();
    a.grant(
        TabId(1),
        &u("https://example.com/"),
        GrantReason::ActionClick,
        &gesture(),
    )
    .unwrap();
    a.on_navigation(TabId(2), &u("https://x.com/"), false);
    assert!(a.covers(TabId(1), &u("https://example.com/")));
}

#[test]
fn a_new_grant_replaces_the_old_one_for_the_tab() {
    let mut a = ActiveTabGrants::new();
    a.grant(
        TabId(1),
        &u("https://one.example/"),
        GrantReason::ActionClick,
        &gesture(),
    )
    .unwrap();
    a.grant(
        TabId(1),
        &u("https://two.example/"),
        GrantReason::Omnibox,
        &gesture(),
    )
    .unwrap();
    assert!(!a.covers(TabId(1), &u("https://one.example/")));
    assert!(a.covers(TabId(1), &u("https://two.example/")));
    a.revoke(TabId(1));
    assert!(a.is_empty());
}

#[test]
fn blocked_surfaces_cannot_be_granted() {
    let mut a = ActiveTabGrants::new();
    for (url, surface) in [
        ("about:blank", BlockedSurface::InternalPage),
        ("mercury://settings", BlockedSurface::InternalPage),
        (
            "https://chromewebstore.google.com/",
            BlockedSurface::StorePage,
        ),
        ("https://addons.mozilla.org/", BlockedSurface::StorePage),
        ("view-source:https://a.com/", BlockedSurface::ViewSource),
        ("data:text/html,x", BlockedSurface::DataDocument),
        ("chrome://x/", BlockedSurface::UnsupportedScheme),
    ] {
        assert_eq!(
            a.grant(TabId(1), &u(url), GrantReason::ActionClick, &gesture()),
            Err(ActiveTabError::Blocked(surface)),
            "{url}"
        );
    }
    assert_eq!(
        a.grant(
            TabId(1),
            &u("file:///x"),
            GrantReason::ActionClick,
            &gesture()
        ),
        Err(ActiveTabError::NoOrigin)
    );
    assert!(a.is_empty());
}

#[test]
fn a_grant_never_covers_a_blocked_page_of_the_same_host() {
    let mut a = ActiveTabGrants::new();
    a.grant(
        TabId(1),
        &u("https://chrome.google.com/other"),
        GrantReason::ActionClick,
        &gesture(),
    )
    .unwrap();
    assert!(a.covers(TabId(1), &u("https://chrome.google.com/foo")));
    assert!(!a.covers(TabId(1), &u("https://chrome.google.com/webstore/detail/x")));
}

// ---- optional permission requests --------------------------------------------------------

const OPTIONAL_MANIFEST: &str = r#""permissions":["storage","tabs"],
    "optional_permissions":["bookmarks","history","notifications"],
    "host_permissions":["https://req.example/*"],
    "optional_host_permissions":["https://*.opt.example/*"]"#;

fn request(api: &[&str], hosts: &[&str]) -> PermissionRequest {
    PermissionRequest {
        api: api.iter().map(|n| ApiPermission::parse(n)).collect(),
        hosts: hosts.iter().map(|h| pat(h)).collect(),
    }
}

#[test]
fn a_request_without_a_gesture_is_refused() {
    let m = manifest(OPTIONAL_MANIFEST);
    assert_eq!(
        plan_permission_request(
            &m,
            &PermissionSet::new(),
            &request(&["bookmarks"], &[]),
            None
        ),
        Err(PermissionError::NoUserGesture)
    );
}

#[test]
fn an_empty_or_huge_request_is_refused() {
    let m = manifest(OPTIONAL_MANIFEST);
    let g = gesture();
    assert_eq!(
        plan_permission_request(
            &m,
            &PermissionSet::new(),
            &PermissionRequest::default(),
            Some(&g)
        ),
        Err(PermissionError::EmptyRequest)
    );
    let huge = PermissionRequest {
        api: Vec::new(),
        hosts: (0..10_001)
            .map(|i| pat(&format!("https://h{i}.opt.example/*")))
            .collect(),
    };
    assert_eq!(
        plan_permission_request(&m, &PermissionSet::new(), &huge, Some(&g)),
        Err(PermissionError::TooManyItems)
    );
}

#[test]
fn only_optional_permissions_can_be_requested() {
    let m = manifest(OPTIONAL_MANIFEST);
    let g = gesture();
    for name in ["cookies", "downloads", "madeUp"] {
        assert_eq!(
            plan_permission_request(&m, &PermissionSet::new(), &request(&[name], &[]), Some(&g)),
            Err(PermissionError::NotOptional),
            "{name}"
        );
    }
}

#[test]
fn an_optional_permission_goes_to_the_prompt_with_its_warning() {
    let m = manifest(OPTIONAL_MANIFEST);
    let plan = plan_permission_request(
        &m,
        &PermissionSet::new(),
        &request(&["bookmarks"], &[]),
        Some(&gesture()),
    )
    .unwrap();
    assert!(plan.to_prompt.api.contains(&ApiPermission::Bookmarks));
    assert!(plan.already_granted.is_empty());
    assert_eq!(plan.warnings.len(), 1);
    assert_eq!(plan.warnings[0].text, "Read and change your bookmarks");
}

#[test]
fn something_already_granted_is_not_prompted_again() {
    let m = manifest(OPTIONAL_MANIFEST);
    let mut have = PermissionSet::new();
    have.api.insert(ApiPermission::Bookmarks);
    let plan = plan_permission_request(
        &m,
        &have,
        &request(&["bookmarks", "history"], &[]),
        Some(&gesture()),
    )
    .unwrap();
    assert_eq!(plan.to_prompt.api.len(), 1);
    assert!(plan.to_prompt.api.contains(&ApiPermission::History));
    assert!(plan.already_granted.api.contains(&ApiPermission::Bookmarks));
    // The warnings are for exactly what is prompted.
    assert!(plan.warnings.iter().all(|w| !w.text.contains("bookmarks")));
    assert_eq!(plan.warnings.len(), 1);
}

#[test]
fn a_required_permission_is_already_part_of_the_install() {
    let m = manifest(OPTIONAL_MANIFEST);
    let plan = plan_permission_request(
        &m,
        &PermissionSet::new(),
        &request(&["tabs"], &[]),
        Some(&gesture()),
    )
    .unwrap();
    assert!(plan.to_prompt.is_empty());
    assert!(plan.warnings.is_empty());
    assert!(plan.already_granted.api.contains(&ApiPermission::Tabs));
}

#[test]
fn hosts_inside_an_optional_pattern_can_be_requested() {
    let m = manifest(OPTIONAL_MANIFEST);
    let g = gesture();
    let plan = plan_permission_request(
        &m,
        &PermissionSet::new(),
        &request(&[], &["https://shop.opt.example/cart"]),
        Some(&g),
    )
    .unwrap();
    assert_eq!(plan.to_prompt.hosts.len(), 1);
    assert!(
        plan.to_prompt
            .hosts
            .iter()
            .any(|h| h.to_string() == "https://shop.opt.example/*")
    );
    assert_eq!(
        plan.warnings[0].text,
        "Read and change your data on shop.opt.example"
    );
}

#[test]
fn hosts_outside_the_optional_patterns_are_refused() {
    let m = manifest(OPTIONAL_MANIFEST);
    let g = gesture();
    for host in [
        "https://evil.example/*",
        "https://*/*",
        "https://*.example/*",
        "http://shop.opt.example/*",
    ] {
        assert_eq!(
            plan_permission_request(&m, &PermissionSet::new(), &request(&[], &[host]), Some(&g)),
            Err(PermissionError::NotOptional),
            "{host}"
        );
    }
}

#[test]
fn a_required_host_asked_for_again_is_not_a_new_prompt_when_granted() {
    let m = manifest(OPTIONAL_MANIFEST);
    let mut have = PermissionSet::new();
    have.hosts.insert(pat("https://req.example/*"));
    let plan = plan_permission_request(
        &m,
        &have,
        &request(&[], &["https://req.example/*"]),
        Some(&gesture()),
    )
    .unwrap();
    assert!(plan.to_prompt.is_empty());
    assert_eq!(plan.already_granted.hosts.len(), 1);
}

#[test]
fn a_withheld_required_host_can_be_requested_again() {
    let m = manifest(OPTIONAL_MANIFEST);
    let plan = plan_permission_request(
        &m,
        &PermissionSet::new(),
        &request(&[], &["https://req.example/*"]),
        Some(&gesture()),
    )
    .unwrap();
    assert_eq!(plan.to_prompt.hosts.len(), 1);
}

#[test]
fn required_permissions_cannot_be_removed() {
    let m = manifest(OPTIONAL_MANIFEST);
    assert_eq!(
        plan_permission_removal(&m, &request(&["tabs"], &[])),
        Err(PermissionError::RemoveRequired)
    );
    assert_eq!(
        plan_permission_removal(&m, &request(&["storage", "bookmarks"], &[])),
        Err(PermissionError::RemoveRequired)
    );
    assert_eq!(
        plan_permission_removal(&m, &request(&[], &["https://req.example/*"])),
        Err(PermissionError::RemoveRequired)
    );
    assert_eq!(
        plan_permission_removal(&m, &request(&[], &["https://*/*"])),
        Err(PermissionError::RemoveRequired)
    );
}

#[test]
fn optional_permissions_can_be_removed() {
    let m = manifest(OPTIONAL_MANIFEST);
    let out = plan_permission_removal(&m, &request(&["bookmarks"], &["https://a.opt.example/*"]))
        .unwrap();
    assert!(out.api.contains(&ApiPermission::Bookmarks));
    assert_eq!(out.hosts.len(), 1);
}

#[test]
fn removing_nothing_is_an_empty_request() {
    let m = manifest(OPTIONAL_MANIFEST);
    assert_eq!(
        plan_permission_removal(&m, &PermissionRequest::default()),
        Err(PermissionError::EmptyRequest)
    );
}

#[test]
fn content_script_hosts_cannot_be_removed() {
    let m = manifest(r#""content_scripts":[{"matches":["https://cs.example/*"],"js":["a.js"]}]"#);
    assert_eq!(
        plan_permission_removal(&m, &request(&[], &["https://cs.example/*"])),
        Err(PermissionError::RemoveRequired)
    );
}

// ---- private windows ---------------------------------------------------------------------

#[test]
fn private_window_policy_table() {
    let normal = manifest(r#""description":"d""#);
    let split = manifest(r#""incognito":"split""#);
    let never = manifest(r#""incognito":"not_allowed""#);
    assert!(!may_run_in_private_window(
        &normal,
        IncognitoPolicy::NotAllowed
    ));
    assert!(may_run_in_private_window(
        &normal,
        IncognitoPolicy::AllowedByUser
    ));
    assert!(may_run_in_private_window(
        &split,
        IncognitoPolicy::AllowedByUser
    ));
    assert!(!may_run_in_private_window(
        &never,
        IncognitoPolicy::AllowedByUser
    ));
    assert!(!may_run_in_private_window(
        &never,
        IncognitoPolicy::NotAllowed
    ));
    assert_eq!(IncognitoPolicy::default(), IncognitoPolicy::NotAllowed);
}

// ---- the kill switch ---------------------------------------------------------------------

#[test]
fn three_crashes_in_ten_minutes_disable_an_extension() {
    let mut k = KillSwitch::default();
    let e = ext(0);
    assert_eq!(
        k.record_crash(&e, t(0)),
        CrashOutcome::Recorded { count: 1 }
    );
    assert_eq!(
        k.record_crash(&e, t(60)),
        CrashOutcome::Recorded { count: 2 }
    );
    assert!(k.is_enabled(&e));
    assert_eq!(k.record_crash(&e, t(120)), CrashOutcome::Disabled);
    assert!(!k.is_enabled(&e));
    assert_eq!(k.state(&e), KillState::CrashLoop);
}

#[test]
fn slow_crashes_never_trip_the_breaker() {
    let mut k = KillSwitch::default();
    let e = ext(0);
    for i in 0..20 {
        assert!(matches!(
            k.record_crash(&e, t(i * 700)),
            CrashOutcome::Recorded { count: 1 }
        ));
    }
    assert!(k.is_enabled(&e));
}

#[test]
fn old_crashes_age_out_of_the_window() {
    let mut k = KillSwitch::default();
    let e = ext(0);
    k.record_crash(&e, t(0));
    k.record_crash(&e, t(100));
    // 601 seconds after the first: it no longer counts.
    assert_eq!(
        k.record_crash(&e, t(601)),
        CrashOutcome::Recorded { count: 2 }
    );
}

#[test]
fn a_crash_exactly_at_the_window_edge_still_counts() {
    let mut k = KillSwitch::default();
    let e = ext(0);
    k.record_crash(&e, t(0));
    k.record_crash(&e, t(300));
    assert_eq!(k.record_crash(&e, t(600)), CrashOutcome::Disabled);
}

#[test]
fn reset_re_enables_and_forgets_the_crashes() {
    let mut k = KillSwitch::default();
    let e = ext(0);
    for i in 0..3 {
        k.record_crash(&e, t(i));
    }
    assert!(!k.is_enabled(&e));
    k.reset(&e);
    assert!(k.is_enabled(&e));
    assert_eq!(
        k.record_crash(&e, t(10)),
        CrashOutcome::Recorded { count: 1 }
    );
}

#[test]
fn a_disabled_extension_stays_disabled_on_more_crashes() {
    let mut k = KillSwitch::default();
    let e = ext(0);
    for i in 0..3 {
        k.record_crash(&e, t(i));
    }
    assert_eq!(k.record_crash(&e, t(5)), CrashOutcome::AlreadyDisabled);
    assert!(!k.is_enabled(&e));
}

#[test]
fn crashes_are_counted_per_extension() {
    let mut k = KillSwitch::default();
    for i in 0..3 {
        k.record_crash(&ext(0), t(i));
        k.record_crash(&ext(1), t(i * 1000));
    }
    assert!(!k.is_enabled(&ext(0)));
    assert!(k.is_enabled(&ext(1)));
}

#[test]
fn the_global_switch_disables_everything_and_reverts() {
    let mut k = KillSwitch::default();
    assert!(!k.all_disabled());
    k.set_all_disabled(true);
    assert!(k.all_disabled());
    assert_eq!(k.state(&ext(0)), KillState::GloballyDisabled);
    assert!(!k.is_enabled(&ext(5)));
    k.set_all_disabled(false);
    assert!(k.is_enabled(&ext(5)));
}

#[test]
fn the_global_switch_wins_over_the_crash_state_in_reporting() {
    let mut k = KillSwitch::default();
    for i in 0..3 {
        k.record_crash(&ext(0), t(i));
    }
    k.set_all_disabled(true);
    assert_eq!(k.state(&ext(0)), KillState::GloballyDisabled);
    k.set_all_disabled(false);
    assert_eq!(k.state(&ext(0)), KillState::CrashLoop);
}

#[test]
fn the_breaker_is_configurable() {
    let mut k = KillSwitch::new(KillSwitchConfig {
        max_crashes: 2,
        window: Duration::from_secs(30),
    });
    assert_eq!(
        k.record_crash(&ext(0), t(0)),
        CrashOutcome::Recorded { count: 1 }
    );
    assert_eq!(k.record_crash(&ext(0), t(10)), CrashOutcome::Disabled);
    assert_eq!(
        k.record_crash(&ext(1), t(0)),
        CrashOutcome::Recorded { count: 1 }
    );
    assert_eq!(
        k.record_crash(&ext(1), t(31)),
        CrashOutcome::Recorded { count: 1 }
    );
    // Zero would trip on the first crash.
    let mut z = KillSwitch::new(KillSwitchConfig {
        max_crashes: 0,
        window: Duration::from_secs(30),
    });
    assert_eq!(z.record_crash(&ext(0), t(0)), CrashOutcome::Disabled);
}

#[test]
fn time_going_backwards_does_not_panic() {
    let mut k = KillSwitch::default();
    k.record_crash(&ext(0), t(1000));
    assert!(matches!(
        k.record_crash(&ext(0), t(10)),
        CrashOutcome::Recorded { count: 2 }
    ));
}

#[test]
fn forgetting_an_extension_clears_its_state() {
    let mut k = KillSwitch::default();
    for i in 0..3 {
        k.record_crash(&ext(0), t(i));
    }
    k.forget(&ext(0));
    assert!(k.is_enabled(&ext(0)));
}

// ---- update consent ----------------------------------------------------------------------

#[test]
fn an_identical_manifest_needs_no_consent() {
    let m = manifest(OPTIONAL_MANIFEST);
    let d = permissions_added(&m, &m);
    assert!(d.is_empty());
    assert!(!update_needs_consent(&d));
}

#[test]
fn a_new_api_permission_needs_consent() {
    let old = manifest(r#""permissions":["storage"]"#);
    let new = manifest(r#""permissions":["storage","alarms"]"#);
    let d = permissions_added(&old, &new);
    assert_eq!(d.added_api.len(), 1);
    assert!(d.added_api.contains(&ApiPermission::Alarms));
    assert!(
        update_needs_consent(&d),
        "even a permission without a warning waits for the user"
    );
}

#[test]
fn a_new_host_needs_consent_but_a_subset_does_not() {
    let old = manifest(r#""host_permissions":["https://*.a.com/*"]"#);
    let subset = manifest(r#""host_permissions":["https://x.a.com/*"]"#);
    assert!(!update_needs_consent(&permissions_added(&old, &subset)));
    let new_host = manifest(r#""host_permissions":["https://*.a.com/*","https://b.com/*"]"#);
    let d = permissions_added(&old, &new_host);
    assert_eq!(d.added_hosts.len(), 1);
    assert!(update_needs_consent(&d));
}

#[test]
fn a_broader_host_needs_consent() {
    let old = manifest(r#""host_permissions":["https://a.com/*"]"#);
    let new = manifest(r#""host_permissions":["https://*.a.com/*"]"#);
    assert!(update_needs_consent(&permissions_added(&old, &new)));
    let all = manifest(r#""host_permissions":["<all_urls>"]"#);
    assert!(update_needs_consent(&permissions_added(&old, &all)));
}

#[test]
fn removing_permissions_needs_no_consent() {
    let old =
        manifest(r#""permissions":["storage","tabs"],"host_permissions":["https://a.com/*"]"#);
    let new = manifest(r#""permissions":["storage"]"#);
    assert!(!update_needs_consent(&permissions_added(&old, &new)));
}

#[test]
fn moving_a_permission_from_optional_to_required_needs_consent() {
    let old = manifest(r#""optional_permissions":["bookmarks"]"#);
    let new = manifest(r#""permissions":["bookmarks"]"#);
    assert!(update_needs_consent(&permissions_added(&old, &new)));
}

#[test]
fn new_optional_permissions_alone_need_no_consent() {
    let old = manifest(r#""permissions":["storage"]"#);
    let new = manifest(
        r#""permissions":["storage"],"optional_permissions":["history"],"optional_host_permissions":["https://*/*"]"#,
    );
    assert!(!update_needs_consent(&permissions_added(&old, &new)));
}

#[test]
fn new_content_script_hosts_need_consent() {
    let old = manifest(r#""host_permissions":["https://a.com/*"]"#);
    let inside = manifest(
        r#""host_permissions":["https://a.com/*"],"content_scripts":[{"matches":["https://a.com/deep/*"],"js":["a.js"]}]"#,
    );
    assert!(!update_needs_consent(&permissions_added(&old, &inside)));
    let outside = manifest(
        r#""host_permissions":["https://a.com/*"],"content_scripts":[{"matches":["https://z.com/*"],"js":["a.js"]}]"#,
    );
    assert!(update_needs_consent(&permissions_added(&old, &outside)));
}

#[test]
fn the_delta_knows_its_warnings() {
    let old = manifest(r#""permissions":["storage"]"#);
    let new = manifest(
        r#""permissions":["storage","bookmarks","downloads"],"host_permissions":["https://new.example/*"]"#,
    );
    let d = permissions_added(&old, &new);
    let texts: Vec<String> = d
        .warnings(mercury_extensions::id::ExtensionKind::Chrome)
        .into_iter()
        .map(|w| w.text)
        .collect();
    assert_eq!(
        texts,
        vec![
            "Read and change your data on new.example".to_owned(),
            "Read and change your bookmarks".to_owned(),
            "Manage your downloads".to_owned()
        ]
    );
}

#[test]
fn a_migration_from_mv2_to_mv3_compares_hosts_not_syntax() {
    let old = manifest_v2(r#""permissions":["tabs","https://a.com/*"]"#);
    let new = manifest(r#""permissions":["tabs"],"host_permissions":["https://a.com/*"]"#);
    assert!(!update_needs_consent(&permissions_added(&old, &new)));
}

fn manifest_v2(extra: &str) -> Manifest {
    let json = format!(r#"{{"manifest_version":2,"name":"T","version":"1",{extra}}}"#);
    Manifest::parse(json.as_bytes(), &ManifestLimits::default()).unwrap()
}
