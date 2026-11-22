// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! Version comparison (Chrome's numeric rule and Firefox's toolkit rule, with the MDN examples),
//! `judge_update`, and the identity check.

use std::cmp::Ordering;

use mercury_extensions::id::{ExtensionId, ExtensionKind};
use mercury_extensions::update::{
    InstalledExtension, RefuseReason, UpdateDecision, UpdateError, UpdateHash, UpdateOffer,
    UpdatePolicy, check_update_identity, compare_chrome_versions, compare_toolkit_versions,
    compare_versions, judge_update,
};
use url::Url;

const CHROME_ID: &str = "jpignaibiiemhngfjkcpokkamffknabf";
const OTHER_ID: &str = "abcdefghijklmnopabcdefghijklmnop";
const FIREFOX_ID: &str = "tool@example.org";

// ---- versions ----------------------------------------------------------------------------

#[test]
fn chrome_versions_compare_component_wise_and_numerically() {
    let cmp = |a: &str, b: &str| compare_chrome_versions(a, b).unwrap();
    assert_eq!(cmp("1", "1.0.0.0"), Ordering::Equal);
    assert_eq!(cmp("1.0", "1"), Ordering::Equal);
    assert_eq!(cmp("1.2", "1.10"), Ordering::Less);
    assert_eq!(cmp("1.10", "1.2"), Ordering::Greater);
    assert_eq!(cmp("2", "1.99.99.99"), Ordering::Greater);
    assert_eq!(cmp("1.0.0.1", "1"), Ordering::Greater);
    assert_eq!(cmp("0.0.1", "0.0.2"), Ordering::Less);
    assert_eq!(cmp("65535", "65534.65535"), Ordering::Greater);
}

#[test]
fn malformed_chrome_versions_are_errors() {
    for bad in [
        "",
        "a",
        "1.2.3.4.5",
        "1..2",
        "1.",
        ".1",
        "-1",
        "01",
        "1.0-beta",
        "99999999999",
    ] {
        assert_eq!(
            compare_chrome_versions(bad, "1"),
            Err(UpdateError::InvalidVersion),
            "{bad:?}"
        );
        assert_eq!(
            compare_chrome_versions("1", bad),
            Err(UpdateError::InvalidVersion),
            "{bad:?}"
        );
    }
}

/// MDN "Toolkit version format": a sorted list, `<` between neighbours and `==` where written.
#[test]
fn toolkit_versions_follow_the_mdn_ordering() {
    let chain: &[&[&str]] = &[
        &["1.-1"],
        &["1", "1.", "1.0", "1.0.0"],
        &["1.1pre0", "1.0+"],
        &["1.1pre1a"],
        &["1.1pre1aa"],
        &["1.1pre1"],
        &["1.1pre10a"],
        &["1.1pre10"],
        &["1.1", "1.1.0", "1.1.00"],
        &["1.1.0.1"],
        &["1.10"],
        &["1.*"],
        &["1.*.1"],
        &["100"],
    ];
    for (i, group) in chain.iter().enumerate() {
        for a in *group {
            for b in *group {
                assert_eq!(
                    compare_toolkit_versions(a, b),
                    Ordering::Equal,
                    "{a} == {b}"
                );
            }
            for later in &chain[i + 1..] {
                for b in *later {
                    assert_eq!(compare_toolkit_versions(a, b), Ordering::Less, "{a} < {b}");
                    assert_eq!(
                        compare_toolkit_versions(b, a),
                        Ordering::Greater,
                        "{b} > {a}"
                    );
                }
            }
        }
    }
}

#[test]
fn toolkit_string_parts_sort_before_the_bare_version() {
    let chain = [
        "1.1a",
        "1.1aa",
        "1.1ab",
        "1.1b",
        "1.1c",
        "1.1whatever",
        "1.1",
    ];
    for w in chain.windows(2) {
        assert_eq!(
            compare_toolkit_versions(w[0], w[1]),
            Ordering::Less,
            "{} < {}",
            w[0],
            w[1]
        );
    }
}

#[test]
fn toolkit_numbers_are_numeric_not_textual() {
    assert_eq!(compare_toolkit_versions("1.9", "1.10"), Ordering::Less);
    assert_eq!(compare_toolkit_versions("2.0", "10.0"), Ordering::Less);
    assert_eq!(
        compare_toolkit_versions("1.0pre2", "1.0pre10"),
        Ordering::Less
    );
}

#[test]
fn toolkit_comparison_is_total_and_antisymmetric_on_a_grid() {
    let samples = [
        "", "0", "1", "1.0", "1.0a", "1.0pre1", "1.1", "1.*", "2", "1.-1", "1.0+", "1.0.0.1", "a",
        "pre", "1pre", "+", "-", "*",
    ];
    for a in samples {
        assert_eq!(compare_toolkit_versions(a, a), Ordering::Equal, "{a:?}");
        for b in samples {
            assert_eq!(
                compare_toolkit_versions(a, b),
                compare_toolkit_versions(b, a).reverse(),
                "{a:?} {b:?}"
            );
        }
    }
}

#[test]
fn compare_versions_picks_the_rule_by_kind() {
    assert_eq!(
        compare_versions(ExtensionKind::Chrome, "1.2", "1.10").unwrap(),
        Ordering::Less
    );
    assert_eq!(
        compare_versions(ExtensionKind::Firefox, "1.1pre1", "1.1").unwrap(),
        Ordering::Less
    );
    assert!(compare_versions(ExtensionKind::Chrome, "1.1pre1", "1.1").is_err());
    assert!(compare_versions(ExtensionKind::Firefox, "1 1", "1.1").is_err());
    assert!(compare_versions(ExtensionKind::Firefox, "", "1.1").is_err());
}

// ---- judging -----------------------------------------------------------------------------

fn installed(id: &str, version: &str) -> InstalledExtension {
    InstalledExtension {
        id: ExtensionId::parse(id).unwrap(),
        version: version.to_owned(),
        update_url: None,
    }
}

fn offer(id: &str, version: &str, url: &str, hash: bool) -> UpdateOffer {
    UpdateOffer {
        id: ExtensionId::parse(id).unwrap(),
        version: version.to_owned(),
        url: Url::parse(url).unwrap(),
        hash: hash.then_some(UpdateHash::Sha256([7; 32])),
        size: None,
        strict_min_version: None,
        strict_max_version: None,
    }
}

fn judge(inst: &InstalledExtension, o: &UpdateOffer) -> UpdateDecision {
    judge_update(inst, o, &UpdatePolicy::default())
}

fn refused(reason: RefuseReason) -> UpdateDecision {
    UpdateDecision::Refuse(reason)
}

#[test]
fn a_newer_https_offer_is_accepted() {
    let inst = installed(CHROME_ID, "1.0.0");
    assert_eq!(
        judge(
            &inst,
            &offer(CHROME_ID, "1.0.1", "https://dl.example/a.crx", true)
        ),
        UpdateDecision::Accept
    );
    assert_eq!(
        judge(
            &inst,
            &offer(CHROME_ID, "2", "https://dl.example/a.crx?x=1", true)
        ),
        UpdateDecision::Accept
    );
}

#[test]
fn downgrades_and_the_same_version_are_refused() {
    let inst = installed(CHROME_ID, "1.2.3");
    assert_eq!(
        judge(
            &inst,
            &offer(CHROME_ID, "1.2.2", "https://d.example/a", true)
        ),
        refused(RefuseReason::Downgrade)
    );
    assert_eq!(
        judge(&inst, &offer(CHROME_ID, "1.2", "https://d.example/a", true)),
        refused(RefuseReason::Downgrade)
    );
    assert_eq!(
        judge(
            &inst,
            &offer(CHROME_ID, "1.2.3", "https://d.example/a", true)
        ),
        refused(RefuseReason::SameVersion)
    );
    assert_eq!(
        judge(
            &inst,
            &offer(CHROME_ID, "1.2.3.0", "https://d.example/a", true)
        ),
        refused(RefuseReason::SameVersion)
    );
}

#[test]
fn only_https_is_accepted_by_default() {
    let inst = installed(CHROME_ID, "1.0");
    for url in [
        "http://dl.example/a.crx",
        "http://localhost/a.crx",
        "http://127.0.0.1/a.crx",
        "ftp://dl.example/a.crx",
        "file:///tmp/a.crx",
        "data:application/octet-stream,x",
        "javascript:alert(1)",
        "ws://dl.example/a",
    ] {
        assert_eq!(
            judge(&inst, &offer(CHROME_ID, "2.0", url, true)),
            refused(RefuseReason::NotHttps),
            "{url}"
        );
    }
}

#[test]
fn loopback_http_needs_the_policy_flag_and_a_real_loopback_host() {
    let inst = installed(CHROME_ID, "1.0");
    let policy = UpdatePolicy {
        allow_insecure_loopback: true,
    };
    for url in [
        "http://localhost:8080/a.crx",
        "http://127.0.0.1/a.crx",
        "http://127.1.2.3:9/a",
        "http://[::1]/a.crx",
    ] {
        assert_eq!(
            judge_update(&inst, &offer(CHROME_ID, "2.0", url, true), &policy),
            UpdateDecision::Accept,
            "{url}"
        );
        assert!(policy.url_allowed(&Url::parse(url).unwrap()), "{url}");
        assert!(
            !UpdatePolicy::default().url_allowed(&Url::parse(url).unwrap()),
            "{url}"
        );
    }
    for url in [
        "http://localhost.evil.example/a",
        "http://127.0.0.1.evil.example/a",
        "http://example.com/a",
        "http://10.0.0.1/a",
        "http://[::2]/a",
    ] {
        assert_eq!(
            judge_update(&inst, &offer(CHROME_ID, "2.0", url, true), &policy),
            refused(RefuseReason::NotHttps),
            "{url}"
        );
    }
    // The flag never allows other schemes.
    assert_eq!(
        judge_update(
            &inst,
            &offer(CHROME_ID, "2.0", "ftp://localhost/a", true),
            &policy
        ),
        refused(RefuseReason::NotHttps)
    );
}

#[test]
fn credentials_in_the_url_are_refused() {
    let inst = installed(CHROME_ID, "1.0");
    for url in [
        "https://user:pass@dl.example/a.crx",
        "https://user@dl.example/a.crx",
        "https://:pass@dl.example/a.crx",
    ] {
        assert_eq!(
            judge(&inst, &offer(CHROME_ID, "2.0", url, true)),
            refused(RefuseReason::CredentialsInUrl),
            "{url}"
        );
    }
}

#[test]
fn a_fragment_in_the_url_is_refused() {
    let inst = installed(CHROME_ID, "1.0");
    for url in ["https://dl.example/a.crx#frag", "https://dl.example/a.crx#"] {
        assert_eq!(
            judge(&inst, &offer(CHROME_ID, "2.0", url, true)),
            refused(RefuseReason::FragmentInUrl),
            "{url}"
        );
    }
}

#[test]
fn an_offer_for_another_id_is_refused() {
    let inst = installed(CHROME_ID, "1.0");
    assert_eq!(
        judge(&inst, &offer(OTHER_ID, "2.0", "https://dl.example/a", true)),
        refused(RefuseReason::IdMismatch)
    );
}

#[test]
fn id_mismatch_is_reported_before_anything_else() {
    let inst = installed(CHROME_ID, "1.0");
    assert_eq!(
        judge(&inst, &offer(OTHER_ID, "0.1", "http://dl.example/a", false)),
        refused(RefuseReason::IdMismatch)
    );
}

#[test]
fn firefox_uses_the_toolkit_rule() {
    let inst = installed(FIREFOX_ID, "1.0");
    assert_eq!(
        judge(
            &inst,
            &offer(FIREFOX_ID, "1.0pre1", "https://dl.example/a.xpi", false)
        ),
        refused(RefuseReason::Downgrade)
    );
    assert_eq!(
        judge(
            &inst,
            &offer(FIREFOX_ID, "1.0.0", "https://dl.example/a.xpi", false)
        ),
        refused(RefuseReason::SameVersion)
    );
    assert_eq!(
        judge(
            &inst,
            &offer(FIREFOX_ID, "1.1pre1", "https://dl.example/a.xpi", false)
        ),
        UpdateDecision::Accept
    );
    assert_eq!(
        judge(
            &inst,
            &offer(FIREFOX_ID, "1.10", "https://dl.example/a.xpi", false)
        ),
        UpdateDecision::Accept
    );
}

#[test]
fn an_xpi_over_http_needs_a_hash_and_over_https_does_not() {
    let inst = installed(FIREFOX_ID, "1.0");
    let policy = UpdatePolicy {
        allow_insecure_loopback: true,
    };
    assert_eq!(
        judge_update(
            &inst,
            &offer(FIREFOX_ID, "2.0", "http://localhost/a.xpi", false),
            &policy
        ),
        refused(RefuseReason::MissingHash)
    );
    assert_eq!(
        judge_update(
            &inst,
            &offer(FIREFOX_ID, "2.0", "http://localhost/a.xpi", true),
            &policy
        ),
        UpdateDecision::Accept
    );
    assert_eq!(
        judge(
            &inst,
            &offer(FIREFOX_ID, "2.0", "https://dl.example/a.xpi", false)
        ),
        UpdateDecision::Accept
    );
}

#[test]
fn malformed_versions_are_refused_not_panicked_on() {
    let inst = installed(CHROME_ID, "1.0");
    for v in ["", "x", "1.2.3.4.5", "-1"] {
        assert_eq!(
            judge(&inst, &offer(CHROME_ID, v, "https://d.example/a", true)),
            refused(RefuseReason::InvalidVersion),
            "{v:?}"
        );
    }
    let broken = installed(CHROME_ID, "garbage");
    assert_eq!(
        judge(
            &broken,
            &offer(CHROME_ID, "2.0", "https://d.example/a", true)
        ),
        refused(RefuseReason::InvalidVersion)
    );
    let ff = installed(FIREFOX_ID, "1.0");
    assert_eq!(
        judge(&ff, &offer(FIREFOX_ID, "1 0", "https://d.example/a", true)),
        refused(RefuseReason::InvalidVersion)
    );
}

#[test]
fn the_url_checks_run_before_the_version_check() {
    let inst = installed(CHROME_ID, "5.0");
    // A downgrade over http reports the transport problem first.
    assert_eq!(
        judge(&inst, &offer(CHROME_ID, "1.0", "http://d.example/a", true)),
        refused(RefuseReason::NotHttps)
    );
}

// ---- identity ----------------------------------------------------------------------------

#[test]
fn the_downloaded_package_must_have_the_installed_id() {
    let a = ExtensionId::parse(CHROME_ID).unwrap();
    let b = ExtensionId::parse(OTHER_ID).unwrap();
    assert_eq!(check_update_identity(&a, &a), Ok(()));
    assert_eq!(
        check_update_identity(&a, &b),
        Err(UpdateError::IdentityMismatch)
    );
    let ff = ExtensionId::parse(FIREFOX_ID).unwrap();
    let ff_upper = ExtensionId::parse("Tool@example.org").unwrap();
    assert_eq!(check_update_identity(&ff, &ff), Ok(()));
    assert_eq!(
        check_update_identity(&ff, &ff_upper),
        Err(UpdateError::IdentityMismatch)
    );
    assert_eq!(
        check_update_identity(&ff, &a),
        Err(UpdateError::IdentityMismatch)
    );
}

#[test]
fn url_allowed_table() {
    let default = UpdatePolicy::default();
    assert!(default.url_allowed(&Url::parse("https://example.com/").unwrap()));
    assert!(!default.url_allowed(&Url::parse("http://example.com/").unwrap()));
    assert!(!default.url_allowed(&Url::parse("mailto:a@b.c").unwrap()));
}
