// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! `check_for_updates` against a fake transport. No test touches the network: the fake serves
//! canned bytes and records every URL it was asked for.

use std::cell::RefCell;

use mercury_extensions::id::ExtensionId;
use mercury_extensions::update::{
    BrowserInfo, InstalledExtension, RefuseReason, TransportError, UpdateError, UpdateHash,
    UpdateLimits, UpdateOutcome, UpdatePolicy, UpdateStatus, UpdateTransport, check_for_updates,
};
use url::Url;

const ID_A: &str = "jpignaibiiemhngfjkcpokkamffknabf";
const ID_B: &str = "abcdefghijklmnopabcdefghijklmnop";
const FF_ID: &str = "tool@example.org";
const HASH: &str = "b290f77cde930116b290f77cde930116b290f77cde930116b290f77cde930116";

/// Serves canned answers by URL prefix and records what it was asked.
struct Fake {
    routes: Vec<(String, Result<Vec<u8>, TransportError>)>,
    calls: RefCell<Vec<(String, u64)>>,
}

impl Fake {
    fn new(routes: Vec<(&str, Result<Vec<u8>, TransportError>)>) -> Self {
        Fake {
            routes: routes.into_iter().map(|(p, r)| (p.to_owned(), r)).collect(),
            calls: RefCell::new(Vec::new()),
        }
    }

    fn urls(&self) -> Vec<String> {
        self.calls.borrow().iter().map(|(u, _)| u.clone()).collect()
    }
}

impl UpdateTransport for Fake {
    fn fetch(&self, url: &Url, max_bytes: u64) -> Result<Vec<u8>, TransportError> {
        self.calls.borrow_mut().push((url.to_string(), max_bytes));
        for (prefix, answer) in &self.routes {
            if url.as_str().starts_with(prefix.as_str()) {
                return answer.clone();
            }
        }
        Err(TransportError::Refused)
    }
}

fn id(s: &str) -> ExtensionId {
    ExtensionId::parse(s).unwrap()
}

fn installed(id_text: &str, version: &str, update_url: Option<&str>) -> InstalledExtension {
    InstalledExtension {
        id: id(id_text),
        version: version.to_owned(),
        update_url: update_url.map(|u| Url::parse(u).unwrap()),
    }
}

fn browser() -> BrowserInfo {
    BrowserInfo {
        prod: "mercury".to_owned(),
        prodversion: "0.3.0".to_owned(),
        os: "mac".to_owned(),
        arch: "arm64".to_owned(),
        lang: None,
    }
}

fn gupdate(body: &str) -> Vec<u8> {
    format!(r#"<?xml version="1.0"?><gupdate xmlns="http://www.google.com/update2/response" protocol="2.0">{body}</gupdate>"#).into_bytes()
}

fn offer_app(id_text: &str, version: &str, codebase: &str) -> String {
    format!(
        r#"<app appid="{id_text}" status="ok"><updatecheck status="ok" codebase="{codebase}" version="{version}" hash_sha256="{HASH}" size="99"/></app>"#
    )
}

fn run(fake: &Fake, installed: &[InstalledExtension]) -> Vec<UpdateOutcome> {
    check_for_updates(
        fake,
        installed,
        &browser(),
        &UpdatePolicy::default(),
        &UpdateLimits::default(),
    )
}

fn status_of(outcomes: &[UpdateOutcome], id_text: &str) -> UpdateStatus {
    outcomes
        .iter()
        .find(|o| o.id.as_str() == id_text)
        .unwrap()
        .status
        .clone()
}

// ---- Chrome-style ------------------------------------------------------------------------

#[test]
fn an_available_update_comes_back_with_its_hash_and_nothing_else_is_fetched() {
    let fake = Fake::new(vec![(
        "https://u.example/crx",
        Ok(gupdate(&offer_app(ID_A, "1.5", "https://dl.example/a.crx"))),
    )]);
    let out = run(
        &fake,
        &[installed(ID_A, "1.0", Some("https://u.example/crx"))],
    );
    assert_eq!(out.len(), 1);
    let UpdateStatus::Available(offer) = &out[0].status else {
        panic!("{:?}", out[0].status);
    };
    assert_eq!(offer.version, "1.5");
    assert_eq!(offer.url.as_str(), "https://dl.example/a.crx");
    assert_eq!(offer.size, Some(99));
    let Some(UpdateHash::Sha256(h)) = offer.hash else {
        panic!("a sha256 hash is expected");
    };
    assert_eq!(h[0], 0xb2, "the hash is the server's hash_sha256, decoded");
    assert!(!UpdateHash::Sha256(h).verify(b"not the package"));
    assert_eq!(fake.urls().len(), 1, "one request, no download");
}

#[test]
fn every_requested_url_is_https_and_capped() {
    let fake = Fake::new(vec![("https://u.example/", Ok(gupdate("")))]);
    let limits = UpdateLimits::default();
    check_for_updates(
        &fake,
        &[
            installed(ID_A, "1.0", Some("https://u.example/crx")),
            installed(ID_B, "1.0", Some("http://plain.example/crx")),
            installed(
                "tool@example.org",
                "1.0",
                Some("http://plain.example/x.json"),
            ),
        ],
        &browser(),
        &UpdatePolicy::default(),
        &limits,
    );
    let calls = fake.calls.borrow();
    assert!(!calls.is_empty());
    for (url, max) in calls.iter() {
        assert!(url.starts_with("https://"), "{url}");
        assert_eq!(*max, u64::try_from(limits.max_response_bytes).unwrap());
    }
    assert!(
        !calls.iter().any(|(u, _)| u.contains("plain.example")),
        "http was never contacted"
    );
}

#[test]
fn an_up_to_date_extension_is_reported_as_such() {
    let fake = Fake::new(vec![(
        "https://u.example/",
        Ok(gupdate(&format!(
            r#"<app appid="{ID_A}" status="ok"><updatecheck status="noupdate"/></app>"#
        ))),
    )]);
    let out = run(
        &fake,
        &[installed(ID_A, "1.0", Some("https://u.example/crx"))],
    );
    assert_eq!(out[0].status, UpdateStatus::UpToDate);
}

#[test]
fn an_extension_without_an_update_url_costs_no_request() {
    let fake = Fake::new(vec![]);
    let out = run(&fake, &[installed(ID_A, "1.0", None)]);
    assert_eq!(out[0].status, UpdateStatus::NoUpdateUrl);
    assert!(fake.urls().is_empty());
}

#[test]
fn extensions_sharing_an_update_url_share_one_request() {
    let body = format!(
        "{}{}",
        offer_app(ID_A, "2.0", "https://dl.example/a.crx"),
        offer_app(ID_B, "3.0", "https://dl.example/b.crx")
    );
    let fake = Fake::new(vec![("https://u.example/", Ok(gupdate(&body)))]);
    let out = run(
        &fake,
        &[
            installed(ID_A, "1.0", Some("https://u.example/crx")),
            installed(ID_B, "1.0", Some("https://u.example/crx")),
        ],
    );
    assert_eq!(fake.urls().len(), 1);
    assert!(matches!(out[0].status, UpdateStatus::Available(_)));
    assert!(matches!(out[1].status, UpdateStatus::Available(_)));
}

#[test]
fn outcomes_follow_the_order_of_the_installed_list() {
    let fake = Fake::new(vec![("https://u.example/", Ok(gupdate("")))]);
    let list = [
        installed(ID_B, "1.0", None),
        installed(ID_A, "1.0", Some("https://u.example/crx")),
        installed(FF_ID, "1.0", None),
    ];
    let out = run(&fake, &list);
    let ids: Vec<&str> = out.iter().map(|o| o.id.as_str()).collect();
    assert_eq!(ids, vec![ID_B, ID_A, FF_ID]);
}

#[test]
fn a_transport_failure_is_reported_per_extension() {
    for e in [
        TransportError::Network,
        TransportError::Timeout,
        TransportError::HttpStatus(503),
        TransportError::TooLarge,
        TransportError::Refused,
    ] {
        let fake = Fake::new(vec![("https://u.example/", Err(e))]);
        let out = run(
            &fake,
            &[installed(ID_A, "1.0", Some("https://u.example/crx"))],
        );
        assert_eq!(
            out[0].status,
            UpdateStatus::Failed(UpdateError::Transport(e)),
            "{e:?}"
        );
    }
}

#[test]
fn a_malformed_or_oversized_response_is_a_failure_not_a_panic() {
    let big = vec![b' '; UpdateLimits::default().max_response_bytes + 1];
    for (bytes, expected) in [
        (b"not xml".to_vec(), UpdateError::NotXml),
        (
            b"<!DOCTYPE x><gupdate/>".to_vec(),
            UpdateError::DoctypeNotAllowed,
        ),
        (big, UpdateError::TooLarge),
        (vec![0xFF, 0xFE], UpdateError::NotXml),
    ] {
        let fake = Fake::new(vec![("https://u.example/", Ok(bytes))]);
        let out = run(
            &fake,
            &[installed(ID_A, "1.0", Some("https://u.example/crx"))],
        );
        assert_eq!(out[0].status, UpdateStatus::Failed(expected));
    }
}

#[test]
fn a_response_that_forgets_an_extension_is_a_failure_for_it_only() {
    let fake = Fake::new(vec![(
        "https://u.example/",
        Ok(gupdate(&offer_app(ID_A, "2.0", "https://dl.example/a.crx"))),
    )]);
    let out = run(
        &fake,
        &[
            installed(ID_A, "1.0", Some("https://u.example/crx")),
            installed(ID_B, "1.0", Some("https://u.example/crx")),
        ],
    );
    assert!(matches!(status_of(&out, ID_A), UpdateStatus::Available(_)));
    assert_eq!(
        status_of(&out, ID_B),
        UpdateStatus::Failed(UpdateError::MissingResult)
    );
}

#[test]
fn an_app_the_client_never_asked_about_is_ignored() {
    let fake = Fake::new(vec![(
        "https://u.example/",
        Ok(gupdate(&offer_app(
            ID_B,
            "9.0",
            "https://evil.example/b.crx",
        ))),
    )]);
    let out = run(
        &fake,
        &[installed(ID_A, "1.0", Some("https://u.example/crx"))],
    );
    assert_eq!(
        out[0].status,
        UpdateStatus::Failed(UpdateError::MissingResult)
    );
}

#[test]
fn a_server_error_is_a_failure() {
    let fake = Fake::new(vec![(
        "https://u.example/",
        Ok(gupdate(&format!(
            r#"<app appid="{ID_A}" status="error-unknownApplication"/>"#
        ))),
    )]);
    let out = run(
        &fake,
        &[installed(ID_A, "1.0", Some("https://u.example/crx"))],
    );
    assert_eq!(out[0].status, UpdateStatus::Failed(UpdateError::Server));
}

#[test]
fn offers_are_judged_before_they_are_reported() {
    let cases: [(&str, &str, RefuseReason); 5] = [
        ("0.9", "https://dl.example/a.crx", RefuseReason::Downgrade),
        (
            "1.0.0",
            "https://dl.example/a.crx",
            RefuseReason::SameVersion,
        ),
        ("2.0", "http://dl.example/a.crx", RefuseReason::NotHttps),
        (
            "2.0",
            "https://user:pw@dl.example/a.crx",
            RefuseReason::CredentialsInUrl,
        ),
        (
            "2.0",
            "https://dl.example/a.crx#x",
            RefuseReason::FragmentInUrl,
        ),
    ];
    for (version, codebase, reason) in cases {
        let fake = Fake::new(vec![(
            "https://u.example/",
            Ok(gupdate(&offer_app(ID_A, version, codebase))),
        )]);
        let out = run(
            &fake,
            &[installed(ID_A, "1.0", Some("https://u.example/crx"))],
        );
        match &out[0].status {
            UpdateStatus::Refused { reason: r, offer } => {
                assert_eq!(*r, reason, "{version} {codebase}");
                assert_eq!(offer.id, id(ID_A));
            }
            other => panic!("{version} {codebase}: {other:?}"),
        }
    }
}

#[test]
fn an_http_update_url_is_not_contacted_without_the_loopback_flag() {
    let fake = Fake::new(vec![("http://", Ok(gupdate("")))]);
    let out = run(
        &fake,
        &[installed(ID_A, "1.0", Some("http://localhost/crx"))],
    );
    assert_eq!(
        out[0].status,
        UpdateStatus::Failed(UpdateError::UrlNotAllowed)
    );
    assert!(fake.urls().is_empty());
}

#[test]
fn the_loopback_flag_allows_a_local_test_server() {
    let fake = Fake::new(vec![(
        "http://localhost:8080/",
        Ok(gupdate(&offer_app(
            ID_A,
            "2.0",
            "http://localhost:8080/a.crx",
        ))),
    )]);
    let policy = UpdatePolicy {
        allow_insecure_loopback: true,
    };
    let out = check_for_updates(
        &fake,
        &[installed(ID_A, "1.0", Some("http://localhost:8080/crx"))],
        &browser(),
        &policy,
        &UpdateLimits::default(),
    );
    assert!(
        matches!(out[0].status, UpdateStatus::Available(_)),
        "{:?}",
        out[0].status
    );
    assert!(
        fake.urls()
            .iter()
            .all(|u| u.starts_with("http://localhost"))
    );
}

// ---- Firefox-style -----------------------------------------------------------------------

fn manifest_json(updates: &str) -> Vec<u8> {
    format!(r#"{{"addons":{{"{FF_ID}":{{"updates":[{updates}]}}}}}}"#).into_bytes()
}

#[test]
fn the_newest_acceptable_firefox_update_wins() {
    let json = manifest_json(
        r#"{"version":"1.1","update_link":"https://dl.example/1.1.xpi"},
           {"version":"3.0","update_link":"https://dl.example/3.0.xpi"},
           {"version":"2.0","update_link":"https://dl.example/2.0.xpi"},
           {"version":"0.5","update_link":"https://dl.example/0.5.xpi"}"#,
    );
    let fake = Fake::new(vec![("https://u.example/", Ok(json))]);
    let out = run(
        &fake,
        &[installed(
            FF_ID,
            "1.0",
            Some("https://u.example/updates.json"),
        )],
    );
    let UpdateStatus::Available(offer) = &out[0].status else {
        panic!("{:?}", out[0].status);
    };
    assert_eq!(offer.version, "3.0");
    assert_eq!(offer.url.as_str(), "https://dl.example/3.0.xpi");
    assert_eq!(offer.hash, None);
    assert_eq!(
        fake.urls(),
        vec!["https://u.example/updates.json".to_owned()]
    );
}

#[test]
fn firefox_updates_that_are_not_newer_mean_up_to_date() {
    let json = manifest_json(
        r#"{"version":"1.0","update_link":"https://dl.example/1.0.xpi"},{"version":"0.9","update_link":"https://dl.example/0.9.xpi"}"#,
    );
    let fake = Fake::new(vec![("https://u.example/", Ok(json))]);
    let out = run(
        &fake,
        &[installed(
            FF_ID,
            "1.0",
            Some("https://u.example/updates.json"),
        )],
    );
    assert_eq!(out[0].status, UpdateStatus::UpToDate);
}

#[test]
fn a_newer_firefox_update_over_http_is_refused() {
    let json = manifest_json(r#"{"version":"2.0","update_link":"http://dl.example/2.0.xpi"}"#);
    let fake = Fake::new(vec![("https://u.example/", Ok(json))]);
    let out = run(
        &fake,
        &[installed(
            FF_ID,
            "1.0",
            Some("https://u.example/updates.json"),
        )],
    );
    assert!(
        matches!(
            out[0].status,
            UpdateStatus::Refused {
                reason: RefuseReason::NotHttps,
                ..
            }
        ),
        "{:?}",
        out[0].status
    );
}

#[test]
fn a_firefox_manifest_without_our_addon_is_up_to_date() {
    let json = br#"{"addons":{"someone-else@example.org":{"updates":[]}}}"#.to_vec();
    let fake = Fake::new(vec![("https://u.example/", Ok(json))]);
    let out = run(
        &fake,
        &[installed(
            FF_ID,
            "1.0",
            Some("https://u.example/updates.json"),
        )],
    );
    assert_eq!(out[0].status, UpdateStatus::UpToDate);
}

#[test]
fn firefox_failures_are_reported() {
    let fake = Fake::new(vec![("https://u.example/", Ok(b"{ nope".to_vec()))]);
    let out = run(
        &fake,
        &[installed(
            FF_ID,
            "1.0",
            Some("https://u.example/updates.json"),
        )],
    );
    assert_eq!(out[0].status, UpdateStatus::Failed(UpdateError::NotJson));
    let down = Fake::new(vec![("https://u.example/", Err(TransportError::Timeout))]);
    let out = run(
        &down,
        &[installed(
            FF_ID,
            "1.0",
            Some("https://u.example/updates.json"),
        )],
    );
    assert_eq!(
        out[0].status,
        UpdateStatus::Failed(UpdateError::Transport(TransportError::Timeout))
    );
    let none = run(&Fake::new(vec![]), &[installed(FF_ID, "1.0", None)]);
    assert_eq!(none[0].status, UpdateStatus::NoUpdateUrl);
}

#[test]
fn chrome_and_firefox_extensions_are_checked_in_one_call() {
    let fake = Fake::new(vec![
        (
            "https://c.example/",
            Ok(gupdate(&offer_app(ID_A, "2.0", "https://dl.example/a.crx"))),
        ),
        (
            "https://f.example/",
            Ok(manifest_json(
                r#"{"version":"2.0","update_link":"https://dl.example/f.xpi"}"#,
            )),
        ),
    ]);
    let out = run(
        &fake,
        &[
            installed(ID_A, "1.0", Some("https://c.example/crx")),
            installed(FF_ID, "1.0", Some("https://f.example/u.json")),
        ],
    );
    assert!(matches!(out[0].status, UpdateStatus::Available(_)));
    assert!(matches!(out[1].status, UpdateStatus::Available(_)));
    assert!(fake.urls().iter().all(|u| u.starts_with("https://")));
}

#[test]
fn a_hostile_transport_answer_for_the_wrong_size_is_still_checked() {
    // The transport promised to stop at max_bytes; if it does not, the parsers still refuse.
    let fake = Fake::new(vec![("https://f.example/", Ok(vec![b'{'; 2_000_000]))]);
    let out = run(
        &fake,
        &[installed(FF_ID, "1.0", Some("https://f.example/u.json"))],
    );
    assert_eq!(out[0].status, UpdateStatus::Failed(UpdateError::TooLarge));
}
