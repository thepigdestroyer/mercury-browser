// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! The examples of Chrome's `declarativeNetRequest` reference page and MDN, as JSON rulesets with the
//! verdicts the documentation gives (facts, not code). Source:
//! <https://developer.chrome.com/docs/extensions/reference/api/declarativeNetRequest>.

mod common;

use common::*;
use mercury_dnr::evaluator::FrameState;
use mercury_dnr::request::ResourceType::*;
use mercury_dnr::verdict::{FrameKind, Verdict};

/// The "priority" example of the reference page, rules 1 to 4.
fn priority_example() -> Vec<String> {
    vec![
        rule(
            1,
            1,
            r#""urlFilter":"||google.com/","resourceTypes":["main_frame"]"#,
            r#""type":"block""#,
        ),
        rule(
            2,
            1,
            r#""urlFilter":"||google.com/123","resourceTypes":["main_frame"]"#,
            r#""type":"allow""#,
        ),
        rule(
            3,
            2,
            r#""urlFilter":"||google.com/12345","resourceTypes":["main_frame"]"#,
            r#""type":"block""#,
        ),
        rule(
            4,
            1,
            r#""urlFilter":"||google.com/","resourceTypes":["main_frame"]"#,
            r#""type":"redirect","redirect":{"url":"https://example.com"}"#,
        ),
    ]
}

#[test]
fn priority_example_navigation_to_google_com_is_blocked_by_rule_1() {
    let e = evaluator(&priority_example());
    let v = eval(&e, &req("https://google.com", MainFrame));
    assert_eq!(rule_id(&v), Some(1));
    assert!(v.is_block());
}

#[test]
fn priority_example_navigation_to_google_com_1234_is_allowed_by_rule_2() {
    let e = evaluator(&priority_example());
    let v = eval(&e, &req("https://google.com/1234", MainFrame));
    assert!(matches!(v, Verdict::Allow { .. }));
    assert_eq!(rule_id(&v), Some(2));
}

#[test]
fn priority_example_navigation_to_google_com_12345_is_blocked_by_rule_3() {
    let e = evaluator(&priority_example());
    let v = eval(&e, &req("https://google.com/12345", MainFrame));
    assert_eq!(rule_id(&v), Some(3));
    assert!(v.is_block());
}

#[test]
fn priority_example_subresources_are_not_matched_because_the_rules_name_main_frame() {
    let e = evaluator(&priority_example());
    assert_eq!(
        eval(&e, &req("https://google.com/x.js", Script)),
        Verdict::None
    );
}

#[test]
fn block_scripts_with_abc_from_foo_com() {
    let e = evaluator(&[rule(
        1,
        1,
        r#""urlFilter":"abc","initiatorDomains":["foo.com"],"resourceTypes":["script"]"#,
        r#""type":"block""#,
    )]);
    let script = |initiator: Option<&str>, ty| {
        let mut r = req("https://cdn.test/abc.js", ty);
        r.initiator_origin = initiator.map(str::to_owned);
        eval(&e, &r)
    };
    assert!(script(Some("https://foo.com"), Script).is_block());
    assert!(script(Some("https://sub.foo.com"), Script).is_block());
    assert_eq!(script(Some("https://bar.com"), Script), Verdict::None);
    assert_eq!(script(Some("https://notfoo.com"), Script), Verdict::None);
    assert_eq!(script(Some("https://foo.com"), Image), Verdict::None);
    assert_eq!(script(None, Script), Verdict::None);
}

#[test]
fn redirect_to_an_extension_page() {
    let e = evaluator(&[rule(
        1,
        1,
        r#""urlFilter":"||www.example.com/","resourceTypes":["main_frame"]"#,
        r#""type":"redirect","redirect":{"extensionPath":"/a.jpg"}"#,
    )]);
    let v = eval(&e, &req("https://www.example.com/", MainFrame));
    assert_eq!(
        redirect_url(&v).as_deref(),
        Some("mercury-extension://abcdefghijklmnopabcdefghijklmnop/a.jpg")
    );
}

#[test]
fn redirect_with_transform_scheme_and_host() {
    let e = evaluator(&[rule(
        1,
        1,
        r#""urlFilter":"||example.com/","resourceTypes":["main_frame"]"#,
        r#""type":"redirect","redirect":{"transform":{"scheme":"https","host":"new.example.com"}}"#,
    )]);
    assert_eq!(
        redirect_url(&eval(&e, &req("http://example.com/x?y=1", MainFrame))).as_deref(),
        Some("https://new.example.com/x?y=1")
    );
    assert_eq!(
        redirect_url(&eval(&e, &req("https://www.example.com/p", MainFrame))).as_deref(),
        Some("https://new.example.com/p")
    );
    assert_eq!(
        eval(&e, &req("https://other.test/", MainFrame)),
        Verdict::None
    );
}

#[test]
fn redirect_with_regex_substitution() {
    let e = evaluator(&[rule(
        1,
        1,
        r#""regexFilter":"^https://www\\.(abc|def)\\.xyz\\.com/","resourceTypes":["main_frame"]"#,
        r#""type":"redirect","redirect":{"regexSubstitution":"https://\\1.xyz.com/"}"#,
    )]);
    assert_eq!(
        redirect_url(&eval(&e, &req("https://www.abc.xyz.com/path", MainFrame))).as_deref(),
        Some("https://abc.xyz.com/path")
    );
    assert_eq!(
        redirect_url(&eval(&e, &req("https://www.def.xyz.com/a?b=1", MainFrame))).as_deref(),
        Some("https://def.xyz.com/a?b=1")
    );
    assert_eq!(
        eval(&e, &req("https://www.ghi.xyz.com/path", MainFrame)),
        Verdict::None
    );
}

#[test]
fn remove_cookies_from_main_frames_and_sub_frames() {
    let e = evaluator(&[rule(
        1,
        1,
        r#""resourceTypes":["main_frame","sub_frame"]"#,
        r#""type":"modifyHeaders","requestHeaders":[{"header":"cookie","operation":"remove"}]"#,
    )]);
    for ty in [MainFrame, SubFrame] {
        let v = eval(&e, &req("https://a.test/", ty));
        let rewrite = v.rewrite().expect("header rewrite");
        assert_eq!(rewrite.remove_headers, vec!["cookie".to_owned()]);
        assert!(rewrite.url.is_none());
    }
    assert_eq!(eval(&e, &req("https://a.test/x.js", Script)), Verdict::None);
}

#[test]
fn upgrade_scheme_example() {
    let e = evaluator(&[rule(
        1,
        1,
        r#""urlFilter":"|http://example.com","resourceTypes":["main_frame"]"#,
        r#""type":"upgradeScheme""#,
    )]);
    let v = eval(&e, &req("http://example.com/", MainFrame));
    assert_eq!(redirect_url(&v).as_deref(), Some("https://example.com/"));
    assert_eq!(
        eval(&e, &req("https://example.com/", MainFrame)),
        Verdict::None
    );
}

#[test]
fn set_request_header_example() {
    let e = evaluator(&[rule(
        1,
        1,
        r#""urlFilter":"||example.com","resourceTypes":["xmlhttprequest"]"#,
        r#""type":"modifyHeaders","requestHeaders":[{"header":"X-Mercury","operation":"set","value":"1"}]"#,
    )]);
    let v = eval(&e, &req("https://example.com/api", XmlHttpRequest));
    let rewrite = v.rewrite().expect("rewrite");
    assert_eq!(rewrite.set_headers, vec![("X-Mercury".into(), "1".into())]);
}

#[test]
fn allow_all_requests_for_a_frame_masks_lower_priority_blocks_but_not_higher_ones() {
    let e = evaluator(&[
        rule(
            1,
            2,
            r#""urlFilter":"||example.com/","resourceTypes":["main_frame"]"#,
            r#""type":"allowAllRequests""#,
        ),
        rule(2, 1, r#""urlFilter":"||ads.test""#, r#""type":"block""#),
        rule(3, 3, r#""urlFilter":"||evil.test""#, r#""type":"block""#),
    ]);
    let nav = eval(&e, &req("https://example.com/", MainFrame));
    let grant = nav.grant().expect("a grant");
    assert_eq!(grant.scope.priority, 2);
    assert_eq!(grant.scope.frame, FrameKind::MainFrame);

    let frame = FrameState::new().with_grant(grant);
    let ads = req("https://ads.test/x.js", Script);
    assert_eq!(rule_id(&e.evaluate(&ads, &frame)), Some(1));
    assert!(matches!(e.evaluate(&ads, &frame), Verdict::Allow { .. }));
    assert!(e.evaluate(&ads, &FrameState::new()).is_block());
    let evil = req("https://evil.test/x.js", Script);
    assert!(e.evaluate(&evil, &frame).is_block());
    assert_eq!(
        e.evaluate(&evil, &frame),
        e.evaluate_unindexed(&evil, &frame)
    );
}

#[test]
fn the_documented_url_filter_table_through_the_evaluator() {
    let cases: &[(&str, &[&str], &[&str])] = &[
        (
            "abc",
            &["https://abcd.com/", "https://example.com/abcd"],
            &["https://ab.com/"],
        ),
        (
            "abc*d",
            &["https://abcd.com/", "https://example.com/abcxyzd"],
            &["https://abc.com/"],
        ),
        (
            "||a.example.com",
            &[
                "https://a.example.com/",
                "https://b.a.example.com/xyz",
                "https://a.example.company/",
            ],
            &["https://example.com/"],
        ),
        (
            "|https*",
            &["https://example.com/"],
            &["http://example.com/", "http://https.com/"],
        ),
        (
            "example*^123|",
            &["https://example.com/123", "http://abc.com/example?123"],
            &["https://example.com/1234", "https://abc.com/example0123"],
        ),
    ];
    for (filter, hits, misses) in cases {
        let e = evaluator(&[rule(
            1,
            1,
            &format!(r#""urlFilter":"{filter}""#),
            r#""type":"block""#,
        )]);
        for u in *hits {
            assert!(eval(&e, &req(u, Script)).is_block(), "{filter} vs {u}");
        }
        for u in *misses {
            assert_eq!(eval(&e, &req(u, Script)), Verdict::None, "{filter} vs {u}");
        }
    }
}
