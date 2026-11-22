// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! The permission data: names, placement, install warnings and their combination rules, sets, and
//! what a manifest asks for.

use mercury_extensions::id::ExtensionKind;
use mercury_extensions::manifest::{Manifest, ManifestLimits, ManifestVersion};
use mercury_extensions::match_pattern::{MatchPattern, MatchPatternSet};
use mercury_extensions::permissions::{
    ApiPermission, ManifestPermissions, PermissionList, PermissionSet, Placement, install_warnings,
};

fn set(api: &[&str], hosts: &[&str]) -> PermissionSet {
    PermissionSet {
        api: api.iter().map(|n| ApiPermission::parse(n)).collect(),
        hosts: MatchPatternSet::parse_all(hosts).unwrap(),
    }
}

fn texts(set: &PermissionSet, kind: ExtensionKind) -> Vec<String> {
    install_warnings(set, kind)
        .into_iter()
        .map(|w| w.text)
        .collect()
}

fn chrome(api: &[&str], hosts: &[&str]) -> Vec<String> {
    texts(&set(api, hosts), ExtensionKind::Chrome)
}

fn manifest(json: &str) -> Manifest {
    Manifest::parse(json.as_bytes(), &ManifestLimits::default()).unwrap()
}

// ---- names and metadata ------------------------------------------------------------------

#[test]
fn every_known_name_round_trips_and_is_not_unknown() {
    assert!(ApiPermission::KNOWN_NAMES.len() >= 60);
    for name in ApiPermission::KNOWN_NAMES {
        let p = ApiPermission::parse(name);
        assert!(!matches!(p, ApiPermission::Unknown(_)), "{name}");
        assert_eq!(p.name(), *name);
    }
}

#[test]
fn known_names_are_unique() {
    let mut names: Vec<&&str> = ApiPermission::KNOWN_NAMES.iter().collect();
    names.sort();
    let before = names.len();
    names.dedup();
    assert_eq!(before, names.len());
}

#[test]
fn the_brief_s_permissions_are_all_known() {
    for name in [
        "activeTab",
        "alarms",
        "bookmarks",
        "contextMenus",
        "cookies",
        "declarativeNetRequest",
        "declarativeNetRequestWithHostAccess",
        "declarativeNetRequestFeedback",
        "downloads",
        "history",
        "identity",
        "idle",
        "management",
        "nativeMessaging",
        "notifications",
        "offscreen",
        "privacy",
        "scripting",
        "sidePanel",
        "storage",
        "tabs",
        "topSites",
        "unlimitedStorage",
        "webNavigation",
        "webRequest",
        "webRequestBlocking",
        "webRequestAuthProvider",
        "clipboardRead",
        "clipboardWrite",
        "geolocation",
        "userScripts",
    ] {
        assert!(ApiPermission::KNOWN_NAMES.contains(&name), "{name}");
    }
}

#[test]
fn unknown_names_are_kept_verbatim() {
    let p = ApiPermission::parse("somethingNew");
    assert_eq!(p, ApiPermission::Unknown("somethingNew".to_owned()));
    assert_eq!(p.name(), "somethingNew");
    assert!(p.placement_ok(PermissionList::Required) && p.placement_ok(PermissionList::Optional));
    assert!(p.available_in(ManifestVersion::V3, ExtensionKind::Chrome));
}

#[test]
fn names_are_case_sensitive() {
    assert!(matches!(
        ApiPermission::parse("ACTIVETAB"),
        ApiPermission::Unknown(_)
    ));
    assert!(matches!(
        ApiPermission::parse("activetab"),
        ApiPermission::Unknown(_)
    ));
    assert!(matches!(
        ApiPermission::parse(""),
        ApiPermission::Unknown(_)
    ));
}

#[test]
fn serde_uses_the_manifest_name() {
    let p = ApiPermission::parse("downloads.open");
    assert_eq!(serde_json::to_string(&p).unwrap(), "\"downloads.open\"");
    let back: ApiPermission = serde_json::from_str("\"downloads.open\"").unwrap();
    assert_eq!(back, p);
    let unknown: ApiPermission = serde_json::from_str("\"zzz\"").unwrap();
    assert_eq!(unknown, ApiPermission::Unknown("zzz".to_owned()));
}

#[test]
fn placement_table() {
    for name in [
        "activeTab",
        "alarms",
        "storage",
        "unlimitedStorage",
        "offscreen",
        "sidePanel",
        "declarativeNetRequest",
    ] {
        let p = ApiPermission::parse(name);
        assert!(p.placement_ok(PermissionList::Required), "{name}");
        assert!(!p.placement_ok(PermissionList::Optional), "{name}");
    }
    for name in [
        "bookmarks",
        "history",
        "tabs",
        "cookies",
        "downloads",
        "notifications",
        "topSites",
        "webRequest",
    ] {
        let p = ApiPermission::parse(name);
        assert!(p.placement_ok(PermissionList::Required), "{name}");
        assert!(p.placement_ok(PermissionList::Optional), "{name}");
    }
}

#[test]
fn placement_allows_covers_the_optional_only_case() {
    assert!(Placement::OptionalOnly.allows(PermissionList::Optional));
    assert!(!Placement::OptionalOnly.allows(PermissionList::Required));
    assert!(Placement::RequiredOnly.allows(PermissionList::Required));
    assert!(!Placement::RequiredOnly.allows(PermissionList::Optional));
    assert!(
        Placement::Both.allows(PermissionList::Required)
            && Placement::Both.allows(PermissionList::Optional)
    );
}

#[test]
fn only_webrequestblocking_is_missing_from_chrome_mv3() {
    for name in ApiPermission::KNOWN_NAMES {
        let p = ApiPermission::parse(name);
        let in_chrome_v3 = p.available_in(ManifestVersion::V3, ExtensionKind::Chrome);
        assert_eq!(in_chrome_v3, *name != "webRequestBlocking", "{name}");
        assert!(
            p.available_in(ManifestVersion::V3, ExtensionKind::Firefox),
            "{name}"
        );
        assert!(
            p.available_in(ManifestVersion::V2, ExtensionKind::Chrome),
            "{name}"
        );
    }
}

// ---- warning texts -----------------------------------------------------------------------

#[test]
fn chrome_warning_texts_match_the_documentation() {
    let table = [
        (
            "accessibilityFeatures.modify",
            "Change your accessibility settings",
        ),
        (
            "accessibilityFeatures.read",
            "Read your accessibility settings",
        ),
        ("bookmarks", "Read and change your bookmarks"),
        ("clipboardRead", "Read data you copy and paste"),
        ("clipboardWrite", "Modify data you copy and paste"),
        ("declarativeNetRequest", "Block content on any page"),
        (
            "declarativeNetRequestFeedback",
            "Read your browsing history",
        ),
        ("desktopCapture", "Capture content of your screen"),
        ("downloads", "Manage your downloads"),
        ("downloads.open", "Manage your downloads"),
        ("downloads.ui", "Manage your downloads"),
        ("favicon", "Read the icons of the websites you visit"),
        ("geolocation", "Detect your physical location"),
        (
            "history",
            "Read and change your browsing history on all signed-in devices",
        ),
        ("identity.email", "Know your email address"),
        ("management", "Manage your apps, extensions, and themes"),
        (
            "nativeMessaging",
            "Communicate with cooperating native applications",
        ),
        ("notifications", "Display notifications"),
        (
            "pageCapture",
            "Read and change all your data on all websites",
        ),
        ("privacy", "Change your privacy-related settings"),
        ("proxy", "Read and change all your data on all websites"),
        ("readingList", "Read and change entries in the reading list"),
        ("system.storage", "Identify and eject storage devices"),
        (
            "tabCapture",
            "Read and change all your data on all websites",
        ),
        ("tabGroups", "View and manage your tab groups"),
        ("tabs", "Read your browsing history"),
        (
            "topSites",
            "Read a list of your most frequently visited websites",
        ),
        ("ttsEngine", "Read all text spoken using synthesized speech"),
        (
            "webAuthenticationProxy",
            "Read and change all your data on all websites",
        ),
        ("webNavigation", "Read your browsing history"),
    ];
    for (name, text) in table {
        assert_eq!(chrome(&[name], &[]), vec![text.to_owned()], "{name}");
    }
}

#[test]
fn content_settings_warning_is_the_long_one() {
    let t = chrome(&["contentSettings"], &[]);
    assert_eq!(t.len(), 1);
    assert!(t[0].starts_with("Change your settings that control websites' access to features"));
}

#[test]
fn permissions_without_a_warning_show_nothing() {
    for name in [
        "activeTab",
        "alarms",
        "storage",
        "unlimitedStorage",
        "cookies",
        "scripting",
        "idle",
        "contextMenus",
        "declarativeNetRequestWithHostAccess",
        "offscreen",
        "sidePanel",
        "webRequest",
        "webRequestBlocking",
        "power",
        "search",
        "sessions",
        "userScripts",
        "background",
        "identity",
        "tts",
    ] {
        assert!(chrome(&[name], &[]).is_empty(), "{name}");
    }
}

#[test]
fn unknown_permissions_show_nothing() {
    assert!(chrome(&["totallyMadeUp"], &[]).is_empty());
}

#[test]
fn debugger_shows_two_warnings() {
    let t = chrome(&["debugger"], &[]);
    assert_eq!(
        t,
        vec![
            "Read and change all your data on all websites".to_owned(),
            "Access the page debugger backend".to_owned()
        ]
    );
}

#[test]
fn duplicate_messages_are_shown_once() {
    assert_eq!(
        chrome(&["downloads", "downloads.open", "downloads.ui"], &[]).len(),
        1
    );
    assert_eq!(
        chrome(
            &["tabs", "webNavigation", "declarativeNetRequestFeedback"],
            &[]
        )
        .len(),
        1
    );
    assert_eq!(
        chrome(&["proxy", "pageCapture", "tabCapture"], &[]).len(),
        1
    );
}

#[test]
fn warning_ids_are_stable_and_unique_in_one_result() {
    let w = install_warnings(
        &set(
            &[
                "bookmarks",
                "history",
                "downloads",
                "privacy",
                "geolocation",
                "notifications",
            ],
            &["https://a.com/*"],
        ),
        ExtensionKind::Chrome,
    );
    let mut ids: Vec<&str> = w.iter().map(|w| w.id).collect();
    let before = ids.len();
    ids.sort();
    ids.dedup();
    assert_eq!(before, ids.len());
    assert!(w.iter().all(|w| !w.id.is_empty()));
}

// ---- host warnings -----------------------------------------------------------------------

#[test]
fn all_urls_gives_the_all_websites_warning() {
    for pattern in [
        "<all_urls>",
        "*://*/*",
        "https://*/*",
        "http://*/*",
        "https://*/path",
    ] {
        assert_eq!(
            chrome(&[], &[pattern]),
            vec!["Read and change all your data on all websites".to_owned()],
            "{pattern}"
        );
    }
}

#[test]
fn all_hosts_hides_the_specific_host_warnings() {
    assert_eq!(
        chrome(&[], &["<all_urls>", "https://a.com/*", "https://b.com/*"]),
        vec!["Read and change all your data on all websites".to_owned()]
    );
}

#[test]
fn host_warning_wording_by_count() {
    assert_eq!(
        chrome(&[], &["https://a.com/*"]),
        vec!["Read and change your data on a.com"]
    );
    assert_eq!(
        chrome(&[], &["https://a.com/*", "https://b.com/*"]),
        vec!["Read and change your data on a.com and b.com"]
    );
    assert_eq!(
        chrome(
            &[],
            &["https://a.com/*", "https://b.com/*", "https://c.com/*"]
        ),
        vec!["Read and change your data on a.com, b.com, and c.com"]
    );
    let w = install_warnings(
        &set(
            &[],
            &[
                "https://a.com/*",
                "https://b.com/*",
                "https://c.com/*",
                "https://d.com/*",
            ],
        ),
        ExtensionKind::Chrome,
    );
    assert_eq!(w.len(), 1);
    assert_eq!(
        w[0].text,
        "Read and change your data on a number of websites"
    );
    assert_eq!(w[0].details, vec!["a.com", "b.com", "c.com", "d.com"]);
}

#[test]
fn hosts_are_deduplicated_across_schemes_and_paths() {
    assert_eq!(
        chrome(
            &[],
            &["https://a.com/x", "http://a.com/*", "https://a.com/y*"]
        ),
        vec!["Read and change your data on a.com"]
    );
}

#[test]
fn subdomain_wildcards_show_the_domain() {
    assert_eq!(
        chrome(&[], &["https://*.a.com/*"]),
        vec!["Read and change your data on a.com"]
    );
}

#[test]
fn local_files_get_their_own_warning() {
    let w = install_warnings(
        &set(&[], &["file:///*", "https://a.com/*"]),
        ExtensionKind::Chrome,
    );
    let ids: Vec<&str> = w.iter().map(|w| w.id).collect();
    assert_eq!(ids, vec!["hosts", "file_hosts"]);
}

#[test]
fn many_hosts_cap_the_detail_lines() {
    let hosts: Vec<String> = (0..500)
        .map(|i| format!("https://h{i:03}.example/*"))
        .collect();
    let refs: Vec<&str> = hosts.iter().map(String::as_str).collect();
    let w = install_warnings(&set(&[], &refs), ExtensionKind::Chrome);
    assert_eq!(w[0].details.len(), 100);
}

#[test]
fn host_warnings_come_first_then_api_warnings_in_order() {
    let w = install_warnings(
        &set(&["bookmarks", "downloads"], &["https://a.com/*"]),
        ExtensionKind::Chrome,
    );
    let ids: Vec<&str> = w.iter().map(|w| w.id).collect();
    assert_eq!(ids, vec!["hosts", "bookmarks", "downloads"]);
}

// ---- combination rules -------------------------------------------------------------------

#[test]
fn all_urls_hides_the_tabs_warning() {
    assert_eq!(
        chrome(&["tabs"], &["<all_urls>"]),
        vec!["Read and change all your data on all websites"]
    );
    assert_eq!(chrome(&["webNavigation"], &["<all_urls>"]).len(), 1);
}

#[test]
fn specific_hosts_do_not_hide_the_tabs_warning() {
    assert_eq!(chrome(&["tabs"], &["https://a.com/*"]).len(), 2);
}

#[test]
fn history_hides_the_plain_history_read_warning() {
    let t = chrome(&["history", "tabs", "webNavigation"], &[]);
    assert_eq!(
        t,
        vec!["Read and change your browsing history on all signed-in devices"]
    );
}

#[test]
fn sessions_with_tabs_gives_the_signed_in_variant() {
    let t = chrome(&["sessions", "tabs"], &[]);
    assert_eq!(
        t,
        vec!["Read your browsing history on all your signed-in devices"]
    );
}

#[test]
fn sessions_with_history_adds_nothing_beyond_history() {
    let t = chrome(&["sessions", "history"], &[]);
    assert_eq!(
        t,
        vec!["Read and change your browsing history on all signed-in devices"]
    );
}

#[test]
fn sessions_with_tabs_and_all_urls_is_covered_by_all_urls() {
    assert_eq!(
        chrome(&["sessions", "tabs"], &["<all_urls>"]),
        vec!["Read and change all your data on all websites"]
    );
}

#[test]
fn sessions_alone_shows_nothing() {
    assert!(chrome(&["sessions"], &[]).is_empty());
}

#[test]
fn declarative_net_request_is_not_hidden_by_all_hosts() {
    // Chromium's exact rule could not be verified; the restrictive reading shows the warning.
    let t = chrome(&["declarativeNetRequest"], &["<all_urls>"]);
    assert!(t.contains(&"Block content on any page".to_owned()));
    assert!(t.contains(&"Read and change all your data on all websites".to_owned()));
}

#[test]
fn an_empty_set_has_no_warnings() {
    assert!(install_warnings(&PermissionSet::new(), ExtensionKind::Chrome).is_empty());
    assert!(install_warnings(&PermissionSet::new(), ExtensionKind::Firefox).is_empty());
}

// ---- Firefox wording ---------------------------------------------------------------------

#[test]
fn firefox_wording_is_its_own() {
    let t = |api: &[&str], hosts: &[&str]| texts(&set(api, hosts), ExtensionKind::Firefox);
    assert_eq!(
        t(&[], &["<all_urls>"]),
        vec!["Access your data for all websites"]
    );
    assert_eq!(
        t(&[], &["https://a.com/*"]),
        vec!["Access your data for a.com"]
    );
    assert_eq!(t(&["tabs"], &[]), vec!["Access browser tabs"]);
    assert_eq!(t(&["bookmarks"], &[]), vec!["Read and modify bookmarks"]);
    assert_eq!(
        t(&["history", "topSites"], &[]),
        vec!["Access browsing history"]
    );
    assert_eq!(t(&["tabHide"], &[]), vec!["Hide and show browser tabs"]);
    assert_eq!(t(&["storage", "ttsEngine"], &[]), Vec::<String>::new());
}

// ---- sets --------------------------------------------------------------------------------

#[test]
fn set_union_and_difference() {
    let a = set(&["tabs", "storage"], &["https://a.com/*"]);
    let b = set(
        &["storage", "history"],
        &["https://*.a.com/*", "https://b.com/*"],
    );
    let u = a.union(&b);
    assert_eq!(u.api.len(), 3);
    assert_eq!(u.hosts.len(), 3);
    let d = b.difference(&a);
    assert_eq!(d.api.len(), 1);
    assert!(d.api.contains(&ApiPermission::History));
    // `https://*.a.com/*` is not covered by `https://a.com/*`; `https://b.com/*` is new too.
    assert_eq!(d.hosts.len(), 2);
    assert!(a.difference(&a).is_empty());
}

#[test]
fn set_contains_and_subset() {
    let big = set(&["tabs", "storage"], &["https://*.a.com/*"]);
    let small = set(&["tabs"], &["https://x.a.com/*"]);
    assert!(big.contains(&small));
    assert!(small.is_subset(&big));
    assert!(!small.contains(&big));
    assert!(!big.is_subset(&small));
    assert!(big.contains(&PermissionSet::new()));
    assert!(PermissionSet::new().is_subset(&PermissionSet::new()));
}

#[test]
fn set_host_queries_ignore_paths() {
    let s = set(&[], &["https://a.com/*"]);
    assert!(s.covers_host(&MatchPattern::parse("https://a.com/private/page").unwrap()));
    assert!(!s.covers_host(&MatchPattern::parse("https://b.com/*").unwrap()));
    assert!(set(&["tabs"], &[]).contains_api(&ApiPermission::Tabs));
    assert!(!set(&["tabs"], &[]).contains_api(&ApiPermission::History));
}

// ---- what a manifest asks for ------------------------------------------------------------

#[test]
fn manifest_permissions_split_required_and_optional() {
    let m = manifest(
        r#"{"manifest_version":3,"name":"T","version":"1",
            "permissions":["storage","tabs"],"optional_permissions":["bookmarks"],
            "host_permissions":["https://a.com/x"],"optional_host_permissions":["https://*/*"],
            "content_scripts":[{"matches":["https://c.com/page/*"],"js":["a.js"]}]}"#,
    );
    let p = ManifestPermissions::from(&m);
    assert_eq!(p.required.api.len(), 2);
    assert_eq!(p.optional.api.len(), 1);
    assert_eq!(p.required.hosts.len(), 1);
    // The path of a host permission is ignored.
    assert!(
        p.required
            .hosts
            .iter()
            .any(|h| h.to_string() == "https://a.com/*")
    );
    assert_eq!(p.optional.hosts.len(), 1);
    // Content script hosts count for warnings, with the path dropped.
    assert!(
        p.scriptable_hosts
            .iter()
            .any(|h| h.to_string() == "https://c.com/*")
    );
    let w = p.warning_set();
    assert_eq!(w.hosts.len(), 2);
    assert!(!w.contains_api(&ApiPermission::Unknown("x".into())));
}

#[test]
fn mv2_manifest_hosts_come_out_of_permissions() {
    let m = manifest(
        r#"{"manifest_version":2,"name":"T","version":"1",
            "permissions":["tabs","https://a.com/*"],"optional_permissions":["https://b.com/*","history"]}"#,
    );
    let p = ManifestPermissions::from(&m);
    assert_eq!(p.required.hosts.len(), 1);
    assert_eq!(p.optional.hosts.len(), 1);
    assert_eq!(p.optional.api.len(), 1);
}

#[test]
fn misplaced_and_unavailable_permissions_are_left_out() {
    let m = manifest(
        r#"{"manifest_version":3,"name":"T","version":"1",
            "permissions":["storage","webRequestBlocking"],"optional_permissions":["activeTab","tabs"]}"#,
    );
    let p = ManifestPermissions::from(&m);
    assert!(p.required.api.contains(&ApiPermission::Storage));
    assert!(!p.required.api.contains(&ApiPermission::WebRequestBlocking));
    assert!(!p.optional.api.contains(&ApiPermission::ActiveTab));
    assert!(p.optional.api.contains(&ApiPermission::Tabs));
}

#[test]
fn firefox_keeps_webrequestblocking_in_mv3() {
    let m = manifest(
        r#"{"manifest_version":3,"name":"T","version":"1",
            "browser_specific_settings":{"gecko":{"id":"a@b.c"}},"permissions":["webRequest","webRequestBlocking"]}"#,
    );
    assert!(
        ManifestPermissions::from(&m)
            .required
            .api
            .contains(&ApiPermission::WebRequestBlocking)
    );
}

#[test]
fn unknown_permissions_stay_in_the_sets() {
    let m =
        manifest(r#"{"manifest_version":3,"name":"T","version":"1","permissions":["newThing"]}"#);
    assert!(
        ManifestPermissions::from(&m)
            .required
            .api
            .contains(&ApiPermission::Unknown("newThing".into()))
    );
}
