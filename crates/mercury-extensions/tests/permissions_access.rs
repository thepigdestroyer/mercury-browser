// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! `can_access`: the one check injection, privileged fetch and `webRequest` rely on. Every deny
//! reason is exercised, every blocked surface is refused even with everything granted, and a
//! generated URL loop checks that no blocked surface is ever allowed.

use mercury_extensions::id::ExtensionId;
use mercury_extensions::manifest::{Manifest, ManifestLimits};
use mercury_extensions::match_pattern::MatchPattern;
use mercury_extensions::permissions::{
    Access, AccessContext, AllowReason, BlockedSurface, Defaults, DenyReason,
    ExtensionPermissionState, FileAccess, GrantReason, IncognitoPolicy, SurfaceClass, TabId,
    UserGesture, can_access, classify_surface,
};
use url::Url;

const OWN: &str = "jpignaibiiemhngfjkcpokkamffknabf";
const OTHER: &str = "abcdefghijklmnopabcdefghijklmnop";

fn own() -> ExtensionId {
    ExtensionId::parse(OWN).unwrap()
}

fn manifest(extra: &str) -> Manifest {
    let json = format!(r#"{{"manifest_version":3,"name":"T","version":"1",{extra}}}"#);
    Manifest::parse(json.as_bytes(), &ManifestLimits::default()).unwrap()
}

fn state(extra: &str, withhold_broad: bool) -> ExtensionPermissionState {
    ExtensionPermissionState::new(
        own(),
        &manifest(extra),
        &Defaults {
            withhold_broad_hosts: withhold_broad,
        },
    )
}

/// Everything granted: all hosts, activeTab, file URLs, private windows.
fn everything() -> ExtensionPermissionState {
    let mut s = state(
        r#""permissions":["activeTab"],"host_permissions":["<all_urls>"]"#,
        false,
    );
    s.file_access = FileAccess::Allowed;
    s.incognito_choice = IncognitoPolicy::AllowedByUser;
    s
}

fn u(s: &str) -> Url {
    Url::parse(s).unwrap_or_else(|e| panic!("{s}: {e}"))
}

fn access(
    state: &ExtensionPermissionState,
    url: &str,
    tab: Option<i64>,
    private: bool,
    top: bool,
) -> Access {
    let id = own();
    let url = u(url);
    can_access(
        &AccessContext {
            extension: &id,
            url: &url,
            tab: tab.map(TabId),
            is_private_window: private,
            frame_is_top: top,
        },
        state,
    )
}

fn simple(state: &ExtensionPermissionState, url: &str) -> Access {
    access(state, url, None, false, true)
}

// ---- host permissions --------------------------------------------------------------------

#[test]
fn a_granted_host_is_allowed() {
    let s = state(r#""host_permissions":["https://example.com/*"]"#, true);
    assert_eq!(
        simple(&s, "https://example.com/page"),
        Access::Allowed(AllowReason::HostPermission)
    );
    assert!(simple(&s, "https://example.com/page").is_allowed());
}

#[test]
fn another_host_is_denied() {
    let s = state(r#""host_permissions":["https://example.com/*"]"#, true);
    assert_eq!(
        simple(&s, "https://other.org/"),
        Access::Denied(DenyReason::NoHostPermission)
    );
    assert_eq!(
        simple(&s, "http://example.com/"),
        Access::Denied(DenyReason::NoHostPermission)
    );
}

#[test]
fn no_permissions_at_all_is_denied() {
    let s = state(r#""description":"d""#, true);
    assert_eq!(
        simple(&s, "https://example.com/"),
        Access::Denied(DenyReason::NoHostPermission)
    );
}

#[test]
fn broad_hosts_are_withheld_by_default_and_granted_on_request() {
    let mut s = state(r#""host_permissions":["<all_urls>"]"#, true);
    assert_eq!(
        simple(&s, "https://example.com/"),
        Access::Denied(DenyReason::NoHostPermission)
    );
    s.host_grants
        .grant(MatchPattern::parse("<all_urls>").unwrap())
        .unwrap();
    assert_eq!(
        simple(&s, "https://example.com/"),
        Access::Allowed(AllowReason::HostPermission)
    );
}

#[test]
fn a_user_withheld_host_is_denied_again() {
    let mut s = state(
        r#""host_permissions":["https://a.com/*","https://b.com/*"]"#,
        true,
    );
    assert!(simple(&s, "https://a.com/").is_allowed());
    s.host_grants
        .withhold(MatchPattern::parse("https://a.com/*").unwrap());
    assert!(!simple(&s, "https://a.com/").is_allowed());
    assert!(simple(&s, "https://b.com/").is_allowed());
}

#[test]
fn content_script_hosts_count_as_scriptable_hosts() {
    let s = state(
        r#""content_scripts":[{"matches":["https://example.com/app/*"],"js":["a.js"]}]"#,
        true,
    );
    assert!(simple(&s, "https://example.com/other").is_allowed());
}

#[test]
fn the_check_is_for_one_extension() {
    let s = everything();
    let other = ExtensionId::parse(OTHER).unwrap();
    let url = u("https://example.com/");
    let a = can_access(
        &AccessContext {
            extension: &other,
            url: &url,
            tab: None,
            is_private_window: false,
            frame_is_top: true,
        },
        &s,
    );
    assert_eq!(a, Access::Denied(DenyReason::WrongExtension));
}

// ---- activeTab through the check ---------------------------------------------------------

#[test]
fn active_tab_allows_the_granted_origin_only() {
    let mut s = state(r#""permissions":["activeTab"]"#, true);
    assert_eq!(
        access(&s, "https://example.com/", Some(1), false, true),
        Access::Denied(DenyReason::NoHostPermission)
    );
    let gesture = UserGesture::from_user_input();
    s.grant_active_tab(
        TabId(1),
        &u("https://example.com/page"),
        GrantReason::ActionClick,
        &gesture,
    )
    .unwrap();
    assert_eq!(
        access(&s, "https://example.com/other", Some(1), false, true),
        Access::Allowed(AllowReason::ActiveTab)
    );
    // A subframe of another origin, another scheme, another port, another tab: all denied.
    for url in [
        "https://ads.example.net/",
        "http://example.com/",
        "https://example.com:8443/",
        "https://sub.example.com/",
    ] {
        assert_eq!(
            access(&s, url, Some(1), false, false),
            Access::Denied(DenyReason::NoHostPermission),
            "{url}"
        );
    }
    assert_eq!(
        access(&s, "https://example.com/", Some(2), false, true),
        Access::Denied(DenyReason::NoHostPermission)
    );
    assert_eq!(
        access(&s, "https://example.com/", None, false, true),
        Access::Denied(DenyReason::NoHostPermission)
    );
}

#[test]
fn a_same_origin_subframe_is_covered_by_active_tab() {
    let mut s = state(r#""permissions":["activeTab"]"#, true);
    s.grant_active_tab(
        TabId(1),
        &u("https://example.com/"),
        GrantReason::ContextMenu,
        &UserGesture::from_user_input(),
    )
    .unwrap();
    assert!(access(&s, "https://example.com/frame", Some(1), false, false).is_allowed());
}

#[test]
fn active_tab_needs_the_permission() {
    let mut s = state(r#""description":"d""#, true);
    let r = s.grant_active_tab(
        TabId(1),
        &u("https://example.com/"),
        GrantReason::ActionClick,
        &UserGesture::from_user_input(),
    );
    assert_eq!(
        r,
        Err(mercury_extensions::permissions::ActiveTabError::NotDeclared)
    );
}

#[test]
fn active_tab_never_reaches_a_file_page() {
    let mut s = state(r#""permissions":["activeTab"]"#, true);
    s.file_access = FileAccess::Allowed;
    let r = s.grant_active_tab(
        TabId(1),
        &u("file:///etc/hosts"),
        GrantReason::ActionClick,
        &UserGesture::from_user_input(),
    );
    assert_eq!(
        r,
        Err(mercury_extensions::permissions::ActiveTabError::NoOrigin)
    );
    assert_eq!(
        access(&s, "file:///etc/hosts", Some(1), false, true),
        Access::Denied(DenyReason::NoHostPermission)
    );
}

// ---- private windows ---------------------------------------------------------------------

#[test]
fn private_windows_are_off_by_default() {
    let s = state(r#""host_permissions":["https://example.com/*"]"#, true);
    assert_eq!(
        access(&s, "https://example.com/", None, true, true),
        Access::Denied(DenyReason::PrivateWindowNotAllowed)
    );
    assert!(access(&s, "https://example.com/", None, false, true).is_allowed());
}

#[test]
fn the_user_can_allow_private_windows() {
    let mut s = state(r#""host_permissions":["https://example.com/*"]"#, true);
    s.incognito_choice = IncognitoPolicy::AllowedByUser;
    assert!(access(&s, "https://example.com/", None, true, true).is_allowed());
}

#[test]
fn a_not_allowed_manifest_never_runs_in_private_windows() {
    let mut s = state(
        r#""incognito":"not_allowed","host_permissions":["https://example.com/*"]"#,
        true,
    );
    s.incognito_choice = IncognitoPolicy::AllowedByUser;
    assert_eq!(
        access(&s, "https://example.com/", None, true, true),
        Access::Denied(DenyReason::PrivateWindowNotAllowed)
    );
}

// ---- files -------------------------------------------------------------------------------

#[test]
fn file_pages_need_the_user_toggle_even_with_all_urls() {
    let mut s = state(r#""host_permissions":["<all_urls>"]"#, false);
    assert_eq!(
        simple(&s, "file:///Users/me/doc.html"),
        Access::Denied(DenyReason::FileAccessNotGranted)
    );
    s.file_access = FileAccess::Allowed;
    assert_eq!(
        simple(&s, "file:///Users/me/doc.html"),
        Access::Allowed(AllowReason::HostPermission)
    );
}

#[test]
fn the_file_toggle_alone_is_not_enough() {
    let mut s = state(r#""host_permissions":["https://example.com/*"]"#, false);
    s.file_access = FileAccess::Allowed;
    assert_eq!(
        simple(&s, "file:///Users/me/doc.html"),
        Access::Denied(DenyReason::NoHostPermission)
    );
}

#[test]
fn file_access_is_off_by_default() {
    assert_eq!(
        state(r#""description":"d""#, true).file_access,
        FileAccess::Denied
    );
    assert_eq!(FileAccess::default(), FileAccess::Denied);
}

// ---- blocked surfaces --------------------------------------------------------------------

#[test]
fn every_blocked_surface_is_refused_with_everything_granted() {
    let s = everything();
    let blocked: &[(&str, BlockedSurface)] = &[
        ("mercury://settings", BlockedSurface::InternalPage),
        ("mercury:newtab", BlockedSurface::InternalPage),
        ("about:blank", BlockedSurface::InternalPage),
        ("about:preferences", BlockedSurface::InternalPage),
        (
            "mercury-extension://abcdefghijklmnopabcdefghijklmnop/page.html",
            BlockedSurface::OtherExtensionPage,
        ),
        (
            "view-source:https://example.com/",
            BlockedSurface::ViewSource,
        ),
        ("data:text/html,<h1>hi</h1>", BlockedSurface::DataDocument),
        (
            "blob:https://example.com/0b4f1b9c-1c0a-4a37-8ea6-1f1e5a3a8d11",
            BlockedSurface::BlobDocument,
        ),
        (
            "https://chromewebstore.google.com/detail/x/abc",
            BlockedSurface::StorePage,
        ),
        (
            "https://chrome.google.com/webstore/detail/x/abc",
            BlockedSurface::StorePage,
        ),
        (
            "https://addons.mozilla.org/en-US/firefox/addon/x/",
            BlockedSurface::StorePage,
        ),
        (
            "https://microsoftedge.microsoft.com/addons/detail/x/abc",
            BlockedSurface::StorePage,
        ),
        ("chrome://settings/", BlockedSurface::UnsupportedScheme),
        (
            "moz-extension://1234/page.html",
            BlockedSurface::UnsupportedScheme,
        ),
        (
            "chrome-extension://abc/page.html",
            BlockedSurface::UnsupportedScheme,
        ),
        ("javascript:alert(1)", BlockedSurface::UnsupportedScheme),
        ("resource://gre/x", BlockedSurface::UnsupportedScheme),
        (
            "devtools://devtools/bundled/inspector.html",
            BlockedSurface::UnsupportedScheme,
        ),
    ];
    for (url, reason) in blocked {
        assert_eq!(
            access(&s, url, Some(1), false, true),
            Access::Denied(DenyReason::Blocked(*reason)),
            "{url}"
        );
    }
}

#[test]
fn blocked_surfaces_stay_blocked_with_an_active_tab_grant() {
    let mut s = everything();
    let g = UserGesture::from_user_input();
    for url in [
        "https://chromewebstore.google.com/",
        "mercury://settings",
        "about:blank",
        "view-source:https://a.com/",
    ] {
        let url = u(url);
        assert!(
            s.grant_active_tab(TabId(9), &url, GrantReason::ActionClick, &g)
                .is_err(),
            "{url}"
        );
    }
    s.grant_active_tab(
        TabId(9),
        &u("https://example.com/"),
        GrantReason::ActionClick,
        &g,
    )
    .unwrap();
    assert!(matches!(
        access(
            &s,
            "https://chromewebstore.google.com/",
            Some(9),
            false,
            true
        ),
        Access::Denied(DenyReason::Blocked(BlockedSurface::StorePage))
    ));
}

#[test]
fn store_page_variants_are_all_blocked() {
    let s = everything();
    for url in [
        "https://CHROMEWEBSTORE.google.com/",
        "https://chromewebstore.google.com./detail/x",
        "https://www.chromewebstore.google.com/",
        "https://chromewebstore.google.com:8443/x",
        "https://chrome.google.com/webstore",
        "https://chrome.google.com/webstore/",
        "https://chrome.google.com//webstore/x",
        "https://chrome.google.com/WebStore/x",
        "https://chrome.google.com/%77ebstore/x",
        "https://chrome.google.com/a/../webstore/x",
        "https://addons.mozilla.org",
        "https://addons.mozilla.org./",
        "https://www.addons.mozilla.org/x",
        "http://addons.mozilla.org/x",
        "https://microsoftedge.microsoft.com/addons",
        "https://microsoftedge.microsoft.com/%61ddons/x",
        "https://microsoftedge.microsoft.com//Addons/x",
    ] {
        assert!(
            matches!(
                simple(&s, url),
                Access::Denied(DenyReason::Blocked(BlockedSurface::StorePage))
            ),
            "{url}"
        );
    }
}

#[test]
fn lookalikes_of_store_pages_are_ordinary_pages() {
    let s = everything();
    for url in [
        "https://chrome.google.com/other",
        "https://chrome.google.com/",
        "https://microsoftedge.microsoft.com/home",
        "https://notaddons.mozilla.org/",
        "https://addons.mozilla.org.evil.example/",
        "https://example.com/webstore",
        "https://example.com/addons",
        "https://chromewebstore.google.com.evil.example/",
        "https://developer.mozilla.org/",
    ] {
        assert_eq!(
            simple(&s, url),
            Access::Allowed(AllowReason::HostPermission),
            "{url}"
        );
    }
}

#[test]
fn own_extension_pages_are_allowed_without_any_grant() {
    let s = state(r#""description":"d""#, true);
    assert_eq!(
        simple(&s, &format!("mercury-extension://{OWN}/popup.html")),
        Access::Allowed(AllowReason::OwnPage)
    );
    assert_eq!(
        simple(&s, &format!("mercury-extension://{OTHER}/popup.html")),
        Access::Denied(DenyReason::Blocked(BlockedSurface::OtherExtensionPage))
    );
}

#[test]
fn data_and_blob_are_blocked_only_at_the_top_level() {
    let mut s = state(r#""host_permissions":["data:*"]"#, false);
    s.file_access = FileAccess::Allowed;
    assert_eq!(
        access(&s, "data:text/html,x", None, false, true),
        Access::Denied(DenyReason::Blocked(BlockedSurface::DataDocument))
    );
    assert_eq!(
        access(&s, "data:text/html,x", None, false, false),
        Access::Allowed(AllowReason::HostPermission)
    );
    assert_eq!(
        access(&s, "blob:https://example.com/id", None, false, true),
        Access::Denied(DenyReason::Blocked(BlockedSurface::BlobDocument))
    );
    assert_eq!(
        access(&s, "blob:https://example.com/id", None, false, false),
        Access::Denied(DenyReason::NoHostPermission)
    );
}

#[test]
fn all_urls_does_not_include_data() {
    let s = everything();
    assert_eq!(
        access(&s, "data:text/plain,x", None, false, false),
        Access::Denied(DenyReason::NoHostPermission)
    );
}

#[test]
fn classify_surface_table() {
    let id = own();
    let table: &[(&str, bool, SurfaceClass)] = &[
        ("https://example.com/", true, SurfaceClass::Web),
        ("http://example.com/", false, SurfaceClass::Web),
        ("ftp://example.com/", true, SurfaceClass::Web),
        ("ws://example.com/", false, SurfaceClass::Web),
        ("file:///x", true, SurfaceClass::File),
        ("urn:uuid:1", true, SurfaceClass::Web),
        ("data:,x", false, SurfaceClass::Web),
        (
            "data:,x",
            true,
            SurfaceClass::Blocked(BlockedSurface::DataDocument),
        ),
        (
            "about:blank",
            false,
            SurfaceClass::Blocked(BlockedSurface::InternalPage),
        ),
        (
            "mercury://x",
            true,
            SurfaceClass::Blocked(BlockedSurface::InternalPage),
        ),
    ];
    for (url, top, expected) in table {
        assert_eq!(
            classify_surface(&u(url), Some(&id), *top),
            *expected,
            "{url} top={top}"
        );
    }
    assert_eq!(
        classify_surface(&u(&format!("mercury-extension://{OWN}/a")), Some(&id), true),
        SurfaceClass::OwnExtensionPage
    );
    assert_eq!(
        classify_surface(&u(&format!("mercury-extension://{OWN}/a")), None, true),
        SurfaceClass::Blocked(BlockedSurface::OtherExtensionPage)
    );
}

#[test]
fn firefox_ids_get_a_derived_host_label_for_their_own_pages() {
    let ff = ExtensionId::parse("tool@example.org").unwrap();
    let label = ff.host_label();
    assert_eq!(label.len(), 32);
    let url = u(&format!("mercury-extension://{label}/page.html"));
    assert_eq!(
        classify_surface(&url, Some(&ff), true),
        SurfaceClass::OwnExtensionPage
    );
    assert_eq!(
        classify_surface(&url, Some(&own()), true),
        SurfaceClass::Blocked(BlockedSurface::OtherExtensionPage)
    );
}

// ---- the property loop -------------------------------------------------------------------

fn decode(path: &str) -> String {
    let b = path.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%'
            && let Some(hex) = path.get(i + 1..i + 3)
            && hex.bytes().all(|c| c.is_ascii_hexdigit())
        {
            out.push(u8::from_str_radix(hex, 16).unwrap());
            i += 3;
            continue;
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).to_ascii_lowercase()
}

/// An independent statement of "this must never be allowed".
fn must_be_blocked(url: &Url, top: bool) -> bool {
    let scheme = url.scheme();
    if [
        "mercury",
        "about",
        "view-source",
        "chrome",
        "javascript",
        "moz-extension",
        "chrome-extension",
        "resource",
        "devtools",
    ]
    .contains(&scheme)
    {
        return true;
    }
    if scheme == "mercury-extension" {
        return url.host_str() != Some(OWN);
    }
    if top && (scheme == "data" || scheme == "blob") {
        return true;
    }
    if !["http", "https", "ws", "wss", "ftp"].contains(&scheme) {
        return false;
    }
    let Some(host) = url.host_str() else {
        return false;
    };
    let host = host.trim_end_matches('.').to_ascii_lowercase();
    let first = decode(url.path())
        .trim_start_matches('/')
        .split('/')
        .next()
        .unwrap_or("")
        .to_owned();
    let under = |base: &str| host == base || host.ends_with(&format!(".{base}"));
    under("chromewebstore.google.com")
        || under("addons.mozilla.org")
        || (host == "chrome.google.com" && first == "webstore")
        || (host == "microsoftedge.microsoft.com" && first == "addons")
}

fn xorshift(state: &mut u64) -> u64 {
    *state ^= *state << 13;
    *state ^= *state >> 7;
    *state ^= *state << 17;
    *state
}

#[test]
fn no_generated_blocked_surface_is_ever_allowed() {
    let s = everything();
    let schemes = [
        "http",
        "https",
        "ftp",
        "ws",
        "wss",
        "file",
        "mercury",
        "about",
        "view-source",
        "data",
        "blob",
        "chrome",
        "javascript",
        "mercury-extension",
        "moz-extension",
        "chrome-extension",
        "resource",
        "devtools",
    ];
    let hosts = [
        "example.com",
        "chromewebstore.google.com",
        "CHROME.GOOGLE.COM",
        "addons.mozilla.org.",
        "www.addons.mozilla.org",
        "microsoftedge.microsoft.com",
        "a.b.c",
        "127.0.0.1",
        "[::1]",
        "",
        OWN,
        OTHER,
        "chrome.google.com.evil.example",
    ];
    let ports = ["", ":443", ":8080"];
    let paths = [
        "/",
        "/webstore",
        "//webstore/x",
        "/%77ebstore",
        "/WebStore/detail",
        "/addons",
        "/addons/x",
        "/x/../webstore",
        "/a?b#c",
        "/%61ddons",
        "/index.html",
    ];
    let mut checked = 0usize;
    let mut blocked_seen = 0usize;
    let mut run = |text: String| {
        let Ok(url) = Url::parse(&text) else {
            return;
        };
        for top in [true, false] {
            let blocked = must_be_blocked(&url, top);
            let a = access(&s, url.as_str(), Some(1), false, top);
            if blocked {
                assert!(
                    matches!(a, Access::Denied(DenyReason::Blocked(_))),
                    "{url} top={top} -> {a:?}"
                );
                blocked_seen += 1;
            }
            checked += 1;
        }
    };
    for scheme in schemes {
        for host in hosts {
            for port in ports {
                for path in paths {
                    run(format!("{scheme}://{host}{port}{path}"));
                    run(format!("{scheme}:{host}{port}{path}"));
                }
            }
        }
    }
    let mut state = 0x1234_5678_9ABC_DEF0u64;
    for _ in 0..4000 {
        let scheme = schemes[(xorshift(&mut state) as usize) % schemes.len()];
        let host = hosts[(xorshift(&mut state) as usize) % hosts.len()];
        let path = paths[(xorshift(&mut state) as usize) % paths.len()];
        let junk = ["", "%", "%2", "\\", "@", "?x=1", "#f", ".", ".."]
            [(xorshift(&mut state) as usize) % 9];
        run(format!("{scheme}://{host}{path}{junk}"));
    }
    assert!(checked > 20_000, "checked {checked}");
    assert!(blocked_seen > 2_000, "blocked {blocked_seen}");
}

#[test]
fn private_windows_deny_everything_by_default_for_generated_urls() {
    let s = state(r#""host_permissions":["<all_urls>"]"#, false);
    for url in [
        "https://a.com/",
        "http://b.org/x",
        "ftp://c.net/",
        "file:///x",
        "ws://d/",
        "https://chromewebstore.google.com/",
    ] {
        let a = access(&s, url, Some(1), true, true);
        assert!(!a.is_allowed(), "{url}");
    }
}
