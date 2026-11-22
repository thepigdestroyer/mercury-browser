// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! Evaluator behaviour: priorities, action order, redirects and transforms, headers, conditions.

mod common;

use common::*;
use mercury_dnr::evaluator::{Evaluator, FrameState};
use mercury_dnr::request::{RequestMetadata, ResourceType, ResourceType::*};
use mercury_dnr::rule::{HeaderOperation, RulesetKind};
use mercury_dnr::verdict::{AllowScope, FrameGrant, FrameKind, RuleRef, RulesetId, Verdict};

const BLOCK: &str = r#""type":"block""#;
const ALLOW: &str = r#""type":"allow""#;
const UPGRADE: &str = r#""type":"upgradeScheme""#;
const REDIRECT_R: &str = r#""type":"redirect","redirect":{"url":"https://r.test/"}"#;
const ALL_TYPES: &str =
    r#""resourceTypes":["main_frame","sub_frame","script","image","xmlhttprequest"]"#;

fn cond(filter: &str) -> String {
    format!(r#""urlFilter":"{filter}",{ALL_TYPES}"#)
}

fn script(u: &str) -> RequestMetadata {
    req(u, Script)
}

// --- priority and action order ---------------------------------------------------------------------

#[test]
fn no_rules_means_no_verdict() {
    let e = evaluator(&[]);
    assert_eq!(eval(&e, &script("https://a.test/")), Verdict::None);
    assert_eq!(e.rule_count(), 0);
}

#[test]
fn a_non_matching_rule_means_no_verdict() {
    let e = evaluator(&[rule(1, 1, &cond("zzz"), BLOCK)]);
    assert_eq!(eval(&e, &script("https://a.test/")), Verdict::None);
}

#[test]
fn higher_priority_wins_over_action_order() {
    let e = evaluator(&[
        rule(1, 1, &cond("a.test"), BLOCK),
        rule(2, 2, &cond("a.test"), REDIRECT_R),
    ]);
    assert_eq!(rule_id(&eval(&e, &script("https://a.test/"))), Some(2));
}

#[test]
fn at_one_priority_allow_beats_block() {
    let e = evaluator(&[
        rule(1, 1, &cond("a.test"), BLOCK),
        rule(2, 1, &cond("a.test"), ALLOW),
    ]);
    let v = eval(&e, &script("https://a.test/"));
    assert!(matches!(v, Verdict::Allow { .. }));
}

#[test]
fn at_one_priority_block_beats_upgrade_and_redirect() {
    let e = evaluator(&[
        rule(1, 1, &cond("a.test"), REDIRECT_R),
        rule(2, 1, &cond("a.test"), UPGRADE),
        rule(3, 1, &cond("a.test"), BLOCK),
    ]);
    assert!(eval(&e, &script("http://a.test/")).is_block());
}

#[test]
fn at_one_priority_upgrade_beats_redirect() {
    let e = evaluator(&[
        rule(1, 1, &cond("a.test"), REDIRECT_R),
        rule(2, 1, &cond("a.test"), UPGRADE),
    ]);
    let v = eval(&e, &script("http://a.test/"));
    assert!(matches!(v, Verdict::Upgrade { .. }));
}

#[test]
fn allow_all_requests_beats_block_at_one_priority_and_allow_beats_it() {
    let frames = r#""resourceTypes":["main_frame"],"urlFilter":"a.test""#;
    let e = evaluator(&[
        rule(1, 1, frames, BLOCK),
        rule(2, 1, frames, r#""type":"allowAllRequests""#),
    ]);
    let v = eval(&e, &req("https://a.test/", MainFrame));
    assert!(matches!(v, Verdict::AllowAllRequests { .. }));
    let e = evaluator(&[
        rule(1, 1, frames, r#""type":"allowAllRequests""#),
        rule(2, 1, frames, ALLOW),
    ]);
    let v = eval(&e, &req("https://a.test/", MainFrame));
    assert!(matches!(v, Verdict::Allow { .. }));
}

#[test]
fn allow_masks_lower_and_equal_priority_rules() {
    let e = evaluator(&[
        rule(1, 3, &cond("a.test"), ALLOW),
        rule(2, 3, &cond("a.test"), BLOCK),
        rule(3, 2, &cond("a.test"), BLOCK),
    ]);
    assert!(matches!(
        eval(&e, &script("https://a.test/")),
        Verdict::Allow { .. }
    ));
}

#[test]
fn a_higher_priority_block_beats_an_allow() {
    let e = evaluator(&[
        rule(1, 1, &cond("a.test"), ALLOW),
        rule(2, 2, &cond("a.test"), BLOCK),
    ]);
    assert!(eval(&e, &script("https://a.test/")).is_block());
}

#[test]
fn ties_go_to_the_earlier_ruleset_then_the_lower_id() {
    let parse_one = |id: u32| {
        parse(
            RulesetKind::Static,
            &list(&[rule(id, 1, &cond("a.test"), BLOCK)]),
        )
    };
    let e = Evaluator::new(
        [
            (RulesetId::Static("second".into()), parse_one(1)),
            (RulesetId::Static("first".into()), parse_one(9)),
        ],
        None,
    );
    let v = e.evaluate(&script("https://a.test/"), &FrameState::new());
    assert_eq!(
        v,
        Verdict::Block {
            rule: RuleRef {
                ruleset: RulesetId::Static("second".into()),
                rule: 1
            }
        }
    );
    let e = evaluator(&[
        rule(5, 1, &cond("a.test"), BLOCK),
        rule(2, 1, &cond("a.test"), BLOCK),
    ]);
    assert_eq!(rule_id(&eval(&e, &script("https://a.test/"))), Some(2));
}

#[test]
fn static_dynamic_and_session_rulesets_are_one_pool() {
    let e = Evaluator::new(
        [
            (
                RulesetId::Static("s".into()),
                parse(
                    RulesetKind::Static,
                    &list(&[rule(1, 1, &cond("a.test"), BLOCK)]),
                ),
            ),
            (
                RulesetId::Session,
                parse(
                    RulesetKind::Session,
                    &list(&[rule(1, 5, &cond("a.test"), ALLOW)]),
                ),
            ),
        ],
        None,
    );
    let v = e.evaluate(&script("https://a.test/"), &FrameState::new());
    assert_eq!(
        v,
        Verdict::Allow {
            rule: RuleRef {
                ruleset: RulesetId::Session,
                rule: 1
            }
        }
    );
}

// --- redirects -------------------------------------------------------------------------------------

#[test]
fn a_redirect_that_cannot_be_built_is_skipped_for_the_next_best_rule() {
    let e = Evaluator::new(
        [(
            RulesetId::Dynamic,
            parse(
                RulesetKind::Dynamic,
                &list(&[
                    rule(
                        1,
                        5,
                        &cond("a.test"),
                        r#""type":"redirect","redirect":{"extensionPath":"/x.png"}"#,
                    ),
                    rule(2, 1, &cond("a.test"), BLOCK),
                ]),
            ),
        )],
        None,
    );
    let v = e.evaluate(&script("https://a.test/"), &FrameState::new());
    assert_eq!(rule_id(&v), Some(2));
    assert!(v.is_block());
}

#[test]
fn a_redirect_to_a_non_http_scheme_is_skipped() {
    let e = evaluator(&[
        rule(
            1,
            5,
            &cond("a.test"),
            r#""type":"redirect","redirect":{"url":"data:text/plain,x"}"#,
        ),
        rule(2, 1, &cond("a.test"), UPGRADE),
    ]);
    assert_eq!(
        rule_id(&eval(&e, &script("http://a.test/"))),
        Some(2),
        "the data: redirect must not be emitted"
    );
}

#[test]
fn a_redirect_to_the_same_url_is_skipped() {
    let e = evaluator(&[rule(
        1,
        1,
        &cond("a.test"),
        r#""type":"redirect","redirect":{"url":"https://a.test/"}"#,
    )]);
    assert_eq!(eval(&e, &script("https://a.test/")), Verdict::None);
}

#[test]
fn a_redirect_to_another_extension_is_not_emitted() {
    let e = evaluator(&[rule(
        1,
        1,
        &cond("a.test"),
        r#""type":"redirect","redirect":{"url":"mercury-extension://ponmlkjihgfedcbaponmlkjihgfedcba/x"}"#,
    )]);
    assert_eq!(eval(&e, &script("https://a.test/")), Verdict::None);
}

#[test]
fn a_redirect_to_the_own_extension_url_is_emitted() {
    let e = evaluator(&[rule(
        1,
        1,
        &cond("a.test"),
        r#""type":"redirect","redirect":{"url":"mercury-extension://abcdefghijklmnopabcdefghijklmnop/x"}"#,
    )]);
    let v = eval(&e, &script("https://a.test/"));
    assert_eq!(
        redirect_url(&v).as_deref(),
        Some("mercury-extension://abcdefghijklmnopabcdefghijklmnop/x")
    );
}

#[test]
fn extension_path_cannot_escape_to_another_host() {
    let e = evaluator(&[rule(
        1,
        1,
        &cond("a.test"),
        r#""type":"redirect","redirect":{"extensionPath":"/../../x"}"#,
    )]);
    let v = eval(&e, &script("https://a.test/"));
    assert_eq!(
        redirect_url(&v).as_deref(),
        Some("mercury-extension://abcdefghijklmnopabcdefghijklmnop/x")
    );
}

#[test]
fn an_http_base_is_ignored_so_extension_paths_never_become_web_urls() {
    let parsed = parse(
        RulesetKind::Dynamic,
        &list(&[rule(
            1,
            1,
            &cond("a.test"),
            r#""type":"redirect","redirect":{"extensionPath":"/x"}"#,
        )]),
    );
    let e = Evaluator::new(
        [(RulesetId::Dynamic, parsed)],
        Some(url("https://evil.test/")),
    );
    assert_eq!(
        e.evaluate(&script("https://a.test/"), &FrameState::new()),
        Verdict::None
    );
}

fn transform(t: &str) -> Evaluator {
    evaluator(&[rule(
        1,
        1,
        &cond("a.test"),
        &format!(r#""type":"redirect","redirect":{{"transform":{{{t}}}}}"#),
    )])
}

fn transformed(t: &str, from: &str) -> Option<String> {
    redirect_url(&eval(&transform(t), &script(from)))
}

#[test]
fn transform_port_path_fragment_and_credentials() {
    assert_eq!(
        transformed(r#""port":"8443""#, "https://a.test/p").as_deref(),
        Some("https://a.test:8443/p")
    );
    assert_eq!(
        transformed(r#""port":"""#, "https://a.test:8443/p").as_deref(),
        Some("https://a.test/p")
    );
    assert_eq!(
        transformed(r#""path":"/new""#, "https://a.test/old?q=1").as_deref(),
        Some("https://a.test/new?q=1")
    );
    assert_eq!(
        transformed(r##""fragment":"#frag""##, "https://a.test/p").as_deref(),
        Some("https://a.test/p#frag")
    );
    assert_eq!(
        transformed(r#""username":"u","password":"p""#, "https://a.test/p").as_deref(),
        Some("https://u:p@a.test/p")
    );
    assert_eq!(
        transformed(r#""query":"?x=1""#, "https://a.test/p?old=1").as_deref(),
        Some("https://a.test/p?x=1")
    );
    assert_eq!(
        transformed(r#""query":"""#, "https://a.test/p?old=1").as_deref(),
        Some("https://a.test/p")
    );
}

#[test]
fn transform_that_changes_nothing_is_skipped() {
    assert_eq!(transformed(r#""scheme":"https""#, "https://a.test/p"), None);
}

#[test]
fn transform_that_makes_an_invalid_url_is_skipped_without_panic() {
    assert_eq!(
        transformed(r#""host":"a<b.test""#, "https://a.test/p"),
        None
    );
    assert_eq!(
        transformed(r#""scheme":"mercury-extension""#, "https://a.test/p"),
        None
    );
}

#[test]
fn query_transform_removes_adds_and_replaces() {
    let t = r#""queryTransform":{"removeParams":["utm_source","fbclid"],"addOrReplaceParams":[{"key":"ref","value":"mercury"}]}"#;
    assert_eq!(
        transformed(t, "https://a.test/p?utm_source=x&keep=1&fbclid=2").as_deref(),
        Some("https://a.test/p?keep=1&ref=mercury")
    );
    assert_eq!(
        transformed(t, "https://a.test/p?ref=old&keep=1").as_deref(),
        Some("https://a.test/p?ref=mercury&keep=1")
    );
}

#[test]
fn query_transform_replace_only_does_not_add() {
    let t =
        r#""queryTransform":{"addOrReplaceParams":[{"key":"ref","value":"m","replaceOnly":true}]}"#;
    assert_eq!(transformed(t, "https://a.test/p?keep=1"), None);
    assert_eq!(
        transformed(t, "https://a.test/p?ref=old").as_deref(),
        Some("https://a.test/p?ref=m")
    );
}

#[test]
fn query_transform_keeps_untouched_bytes_and_encodes_new_ones() {
    let t = r#""queryTransform":{"addOrReplaceParams":[{"key":"a b","value":"c&d"}]}"#;
    assert_eq!(
        transformed(t, "https://a.test/p?x=%7E+y").as_deref(),
        Some("https://a.test/p?x=%7E+y&a%20b=c%26d")
    );
}

#[test]
fn query_transform_replaces_the_first_duplicate_and_drops_the_rest() {
    let t = r#""queryTransform":{"addOrReplaceParams":[{"key":"k","value":"new"}]}"#;
    assert_eq!(
        transformed(t, "https://a.test/p?k=1&x=2&k=3").as_deref(),
        Some("https://a.test/p?k=new&x=2")
    );
}

#[test]
fn query_transform_removing_everything_clears_the_query() {
    let t = r#""queryTransform":{"removeParams":["a"]}"#;
    assert_eq!(
        transformed(t, "https://a.test/p?a=1").as_deref(),
        Some("https://a.test/p")
    );
}

#[test]
fn regex_substitution_replaces_only_the_first_match_and_keeps_the_rest() {
    let e = evaluator(&[rule(
        1,
        1,
        r#""regexFilter":"/old/","resourceTypes":["script"]"#,
        r#""type":"redirect","redirect":{"regexSubstitution":"/new/"}"#,
    )]);
    let v = eval(&e, &script("https://a.test/old/x/old/y"));
    assert_eq!(
        redirect_url(&v).as_deref(),
        Some("https://a.test/new/x/old/y")
    );
}

#[test]
fn regex_substitution_whole_match_and_unmatched_group() {
    let e = evaluator(&[rule(
        1,
        1,
        r#""regexFilter":"^https://(a)?b\\.test","resourceTypes":["script"]"#,
        r#""type":"redirect","redirect":{"regexSubstitution":"https://c.test[\\1]\\0"}"#,
    )]);
    let v = eval(&e, &script("https://b.test/x"));
    assert_eq!(redirect_url(&v), None, "the result is not a valid URL");
    let e = evaluator(&[rule(
        1,
        1,
        r#""regexFilter":"^https://(a)?b\\.test","resourceTypes":["script"]"#,
        r#""type":"redirect","redirect":{"regexSubstitution":"https://c.test/\\1/"}"#,
    )]);
    let v = eval(&e, &script("https://b.test/x"));
    assert_eq!(redirect_url(&v).as_deref(), Some("https://c.test///x"));
}

#[test]
fn regex_substitution_that_produces_a_javascript_url_is_not_emitted() {
    let e = evaluator(&[rule(
        1,
        1,
        r#""regexFilter":"^https://a\\.test/(.*)","resourceTypes":["script"]"#,
        r#""type":"redirect","redirect":{"regexSubstitution":"javascript:\\1"}"#,
    )]);
    assert_eq!(eval(&e, &script("https://a.test/alert(1)")), Verdict::None);
}

// --- upgradeScheme ---------------------------------------------------------------------------------

#[test]
fn upgrade_keeps_non_default_ports_and_upgrades_websockets() {
    let e = evaluator(&[rule(
        1,
        1,
        r#""resourceTypes":["script","websocket"]"#,
        UPGRADE,
    )]);
    assert_eq!(
        redirect_url(&eval(&e, &script("http://a.test:8080/x"))).as_deref(),
        Some("https://a.test:8080/x")
    );
    assert_eq!(
        redirect_url(&eval(&e, &script("http://a.test:80/x"))).as_deref(),
        Some("https://a.test/x")
    );
    assert_eq!(
        redirect_url(&eval(&e, &req("ws://a.test/sock", WebSocket))).as_deref(),
        Some("wss://a.test/sock")
    );
}

#[test]
fn upgrade_does_not_act_on_secure_requests_and_lets_lower_rules_act() {
    let e = evaluator(&[
        rule(1, 2, &cond("a.test"), UPGRADE),
        rule(2, 1, &cond("a.test"), BLOCK),
    ]);
    assert_eq!(rule_id(&eval(&e, &script("https://a.test/"))), Some(2));
    assert_eq!(rule_id(&eval(&e, &script("http://a.test/"))), Some(1));
}

// --- headers ---------------------------------------------------------------------------------------

fn set_header(h: &str, v: &str) -> String {
    format!(
        r#""type":"modifyHeaders","requestHeaders":[{{"header":"{h}","operation":"set","value":"{v}"}}]"#
    )
}

#[test]
fn header_rules_above_an_allow_apply_and_those_at_or_below_do_not() {
    let e = evaluator(&[
        rule(1, 2, &cond("a.test"), ALLOW),
        rule(2, 3, &cond("a.test"), &set_header("x-up", "1")),
        rule(3, 2, &cond("a.test"), &set_header("x-eq", "1")),
        rule(4, 1, &cond("a.test"), &set_header("x-low", "1")),
    ]);
    let v = eval(&e, &script("https://a.test/"));
    let Verdict::ModifyHeaders { request_ops, .. } = v else {
        panic!("expected header changes, got {v:?}");
    };
    let names: Vec<&str> = request_ops.iter().map(|o| o.header.as_str()).collect();
    assert_eq!(names, vec!["x-up"]);
}

#[test]
fn header_rules_do_not_apply_when_the_request_is_blocked_or_redirected() {
    let e = evaluator(&[
        rule(1, 1, &cond("a.test"), BLOCK),
        rule(2, 9, &cond("a.test"), &set_header("x", "1")),
    ]);
    assert!(eval(&e, &script("https://a.test/")).is_block());
    let e = evaluator(&[
        rule(1, 1, &cond("a.test"), REDIRECT_R),
        rule(2, 9, &cond("a.test"), &set_header("x", "1")),
    ]);
    assert!(matches!(
        eval(&e, &script("https://a.test/")),
        Verdict::Redirect { .. }
    ));
}

#[test]
fn header_conflicts_within_one_extension_follow_chromes_rules() {
    let append = |h: &str, v: &str| {
        format!(
            r#""type":"modifyHeaders","requestHeaders":[{{"header":"{h}","operation":"append","value":"{v}"}}]"#
        )
    };
    let remove = |h: &str| {
        format!(
            r#""type":"modifyHeaders","requestHeaders":[{{"header":"{h}","operation":"remove"}}]"#
        )
    };
    let e = evaluator(&[
        rule(1, 4, &cond("a.test"), &set_header("accept", "set")),
        rule(2, 3, &cond("a.test"), &append("accept", "extra")),
        rule(3, 2, &cond("a.test"), &remove("accept")),
        rule(4, 1, &cond("a.test"), &set_header("accept", "late")),
    ]);
    let Verdict::ModifyHeaders {
        request_ops, rules, ..
    } = eval(&e, &script("https://a.test/"))
    else {
        panic!("expected header changes");
    };
    let ops: Vec<(HeaderOperation, &str)> = request_ops
        .iter()
        .map(|o| (o.operation, o.value.as_deref().unwrap_or("")))
        .collect();
    assert_eq!(
        ops,
        vec![
            (HeaderOperation::Set, "set"),
            (HeaderOperation::Append, "extra")
        ]
    );
    assert_eq!(rules.iter().map(|r| r.rule).collect::<Vec<_>>(), vec![1, 2]);
}

#[test]
fn a_remove_by_a_higher_priority_rule_blocks_lower_ones() {
    let remove =
        r#""type":"modifyHeaders","requestHeaders":[{"header":"X-A","operation":"remove"}]"#;
    let e = evaluator(&[
        rule(1, 2, &cond("a.test"), remove),
        rule(2, 1, &cond("a.test"), &set_header("x-a", "1")),
    ]);
    let v = eval(&e, &script("https://a.test/"));
    let Verdict::ModifyHeaders { request_ops, .. } = &v else {
        panic!("expected header changes");
    };
    assert_eq!(request_ops.len(), 1);
    assert_eq!(request_ops[0].operation, HeaderOperation::Remove);
}

#[test]
fn response_header_operations_are_returned_apart_from_the_rewrite() {
    let e = evaluator(&[rule(
        1,
        1,
        &cond("a.test"),
        r#""type":"modifyHeaders","responseHeaders":[{"header":"x-r","operation":"set","value":"1"}]"#,
    )]);
    let v = eval(&e, &script("https://a.test/"));
    let Verdict::ModifyHeaders {
        request_ops,
        response_ops,
        ..
    } = &v
    else {
        panic!("expected header changes");
    };
    assert!(request_ops.is_empty());
    assert_eq!(response_ops.len(), 1);
    assert_eq!(v.rewrite(), None);
}

// --- allowAllRequests and frame state --------------------------------------------------------------

fn grant(rule: u32, priority: u32) -> FrameGrant {
    FrameGrant {
        rule: RuleRef {
            ruleset: RulesetId::Dynamic,
            rule,
        },
        scope: AllowScope {
            priority,
            frame: FrameKind::SubFrame,
        },
    }
}

#[test]
fn a_grant_answers_allow_even_when_no_rule_matches() {
    let e = evaluator(&[]);
    let frame = FrameState::new().with_grant(grant(7, 1));
    let v = e.evaluate(&script("https://a.test/"), &frame);
    assert!(matches!(v, Verdict::Allow { rule } if rule.rule == 7));
}

#[test]
fn a_grant_masks_rules_of_equal_priority_and_lets_higher_ones_act() {
    let e = evaluator(&[
        rule(1, 2, &cond("a.test"), BLOCK),
        rule(2, 3, &cond("b.test"), BLOCK),
    ]);
    let frame = FrameState::new().with_grant(grant(9, 2));
    assert!(matches!(
        e.evaluate(&script("https://a.test/"), &frame),
        Verdict::Allow { .. }
    ));
    assert!(e.evaluate(&script("https://b.test/"), &frame).is_block());
}

#[test]
fn a_grant_masks_header_rules_at_or_below_its_priority() {
    let e = evaluator(&[
        rule(1, 2, &cond("a.test"), &set_header("x-a", "1")),
        rule(2, 3, &cond("a.test"), &set_header("x-b", "1")),
    ]);
    let frame = FrameState::new().with_grant(grant(9, 2));
    let v = e.evaluate(&script("https://a.test/"), &frame);
    let Verdict::ModifyHeaders { request_ops, .. } = v else {
        panic!("expected header changes");
    };
    assert_eq!(request_ops.len(), 1);
    assert_eq!(request_ops[0].header, "x-b");
}

#[test]
fn frame_state_keeps_the_strongest_grant_and_children_inherit_it() {
    let frame = FrameState::new()
        .with_grant(grant(1, 2))
        .with_grant(grant(2, 5))
        .with_grant(grant(3, 4))
        .with_grant(grant(4, 5));
    assert_eq!(frame.grant().map(|g| g.rule.rule), Some(2));
    assert_eq!(frame.for_child(), frame);
}

#[test]
fn allow_all_requests_on_a_sub_frame_reports_the_sub_frame_scope() {
    let e = evaluator(&[rule(
        1,
        1,
        r#""resourceTypes":["sub_frame"],"urlFilter":"||ok.test""#,
        r#""type":"allowAllRequests""#,
    )]);
    let v = eval(&e, &req("https://ok.test/f", SubFrame));
    assert_eq!(v.grant().map(|g| g.scope.frame), Some(FrameKind::SubFrame));
    assert_eq!(
        eval(&e, &req("https://ok.test/f", MainFrame)),
        Verdict::None
    );
}

// --- conditions ------------------------------------------------------------------------------------

fn from(initiator: Option<&str>, u: &str) -> RequestMetadata {
    let mut r = script(u);
    r.initiator_origin = initiator.map(str::to_owned);
    r
}

#[test]
fn initiator_domains_match_subdomains_on_a_label_boundary() {
    let e = evaluator(&[rule(1, 1, r#""initiatorDomains":["example.com"]"#, BLOCK)]);
    let blocked = |o: Option<&str>| eval(&e, &from(o, "https://t.test/x")).is_block();
    assert!(blocked(Some("https://example.com")));
    assert!(blocked(Some("https://a.b.example.com:8443")));
    assert!(blocked(Some("https://EXAMPLE.com")));
    assert!(!blocked(Some("https://notexample.com")));
    assert!(!blocked(Some("https://example.com.evil.test")));
}

#[test]
fn a_request_without_an_initiator_matches_initiator_domains_only_when_empty() {
    let e = evaluator(&[rule(1, 1, r#""initiatorDomains":["example.com"]"#, BLOCK)]);
    assert_eq!(eval(&e, &from(None, "https://t.test/")), Verdict::None);
    assert_eq!(
        eval(&e, &from(Some("null"), "https://t.test/")),
        Verdict::None
    );
    assert_eq!(
        eval(&e, &from(Some("garbage"), "https://t.test/")),
        Verdict::None
    );
}

#[test]
fn excluded_initiator_domains_do_not_exclude_a_request_without_initiator() {
    let e = evaluator(&[rule(
        1,
        1,
        r#""excludedInitiatorDomains":["example.com"]"#,
        BLOCK,
    )]);
    assert!(eval(&e, &from(None, "https://t.test/")).is_block());
    assert!(eval(&e, &from(Some("https://other.test"), "https://t.test/")).is_block());
    assert_eq!(
        eval(&e, &from(Some("https://a.example.com"), "https://t.test/")),
        Verdict::None
    );
}

#[test]
fn exclusions_beat_inclusions() {
    let e = evaluator(&[rule(
        1,
        1,
        r#""initiatorDomains":["example.com"],"excludedInitiatorDomains":["sub.example.com"]"#,
        BLOCK,
    )]);
    assert!(eval(&e, &from(Some("https://example.com"), "https://t.test/")).is_block());
    assert_eq!(
        eval(
            &e,
            &from(Some("https://x.sub.example.com"), "https://t.test/")
        ),
        Verdict::None
    );
}

#[test]
fn request_domains_match_the_request_host() {
    let e = evaluator(&[rule(
        1,
        1,
        r#""requestDomains":["tracker.test"],"excludedRequestDomains":["ok.tracker.test"]"#,
        BLOCK,
    )]);
    assert!(eval(&e, &script("https://tracker.test/x")).is_block());
    assert!(eval(&e, &script("https://a.tracker.test/x")).is_block());
    assert_eq!(
        eval(&e, &script("https://nottracker.test/x")),
        Verdict::None
    );
    assert_eq!(
        eval(&e, &script("https://ok.tracker.test/x")),
        Verdict::None
    );
    assert_eq!(
        eval(&e, &script("https://other.test/?u=tracker.test")),
        Verdict::None
    );
}

#[test]
fn request_domains_with_a_trailing_dot_host_and_ip_literals() {
    let e = evaluator(&[rule(
        1,
        1,
        r#""requestDomains":["tracker.test","127.0.0.1"]"#,
        BLOCK,
    )]);
    assert!(eval(&e, &script("https://tracker.test./x")).is_block());
    assert!(eval(&e, &script("http://127.0.0.1:8080/x")).is_block());
    assert_eq!(eval(&e, &script("http://10.127.0.1/x")), Verdict::None);
}

#[test]
fn top_domains_use_the_top_level_site_and_fall_back_to_the_initiator() {
    let e = evaluator(&[rule(1, 1, r#""topDomains":["news.test"]"#, BLOCK)]);
    let mut r = script("https://t.test/x");
    r.top_level_site = Some("https://www.news.test".into());
    assert!(eval(&e, &r).is_block());
    let mut r = script("https://t.test/x");
    r.top_level_site = Some("https://other.test".into());
    r.initiator_origin = Some("https://news.test".into());
    assert_eq!(eval(&e, &r), Verdict::None, "the top-level site wins");
    let mut r = script("https://t.test/x");
    r.initiator_origin = Some("https://news.test".into());
    assert!(eval(&e, &r).is_block());
    assert_eq!(eval(&e, &script("https://t.test/x")), Verdict::None);
}

#[test]
fn domain_type_treats_unknown_as_third_party() {
    let first = evaluator(&[rule(1, 1, r#""domainType":"firstParty""#, BLOCK)]);
    let third = evaluator(&[rule(1, 1, r#""domainType":"thirdParty""#, BLOCK)]);
    let mut r = script("https://t.test/");
    assert_eq!(
        eval(&first, &r),
        Verdict::None,
        "unknown is not first party"
    );
    assert!(eval(&third, &r).is_block(), "unknown is third party");
    r.is_third_party = Some(false);
    assert!(eval(&first, &r).is_block());
    assert_eq!(eval(&third, &r), Verdict::None);
    r.is_third_party = Some(true);
    assert_eq!(eval(&first, &r), Verdict::None);
    assert!(eval(&third, &r).is_block());
}

#[test]
fn request_methods_match_case_insensitively_and_unknown_methods_are_other() {
    let post = evaluator(&[rule(1, 1, r#""requestMethods":["post"]"#, BLOCK)]);
    let mut r = script("https://t.test/");
    r.method = "POST".into();
    assert!(eval(&post, &r).is_block());
    r.method = "post".into();
    assert!(eval(&post, &r).is_block());
    r.method = "GET".into();
    assert_eq!(eval(&post, &r), Verdict::None);
    let other = evaluator(&[rule(1, 1, r#""requestMethods":["other"]"#, BLOCK)]);
    r.method = "PROPFIND".into();
    assert!(eval(&other, &r).is_block());
    let not_get = evaluator(&[rule(1, 1, r#""excludedRequestMethods":["get"]"#, BLOCK)]);
    r.method = "GET".into();
    assert_eq!(eval(&not_get, &r), Verdict::None);
    r.method = "head".into();
    assert!(eval(&not_get, &r).is_block());
}

#[test]
fn resource_types_limit_the_rule() {
    let e = evaluator(&[rule(1, 1, r#""resourceTypes":["image","font"]"#, BLOCK)]);
    assert!(eval(&e, &req("https://t.test/a.png", Image)).is_block());
    assert!(eval(&e, &req("https://t.test/a.woff", Font)).is_block());
    for ty in ResourceType::ALL {
        if !matches!(ty, Image | Font) {
            assert_eq!(
                eval(&e, &req("https://t.test/", ty)),
                Verdict::None,
                "{ty:?}"
            );
        }
    }
}

#[test]
fn rules_without_resource_types_skip_main_frame_only() {
    let e = evaluator(&[rule(1, 1, r#""urlFilter":"t.test""#, BLOCK)]);
    assert_eq!(eval(&e, &req("https://t.test/", MainFrame)), Verdict::None);
    for ty in ResourceType::ALL {
        if ty != MainFrame {
            assert!(eval(&e, &req("https://t.test/", ty)).is_block(), "{ty:?}");
        }
    }
}

#[test]
fn tab_ids_apply_to_session_rules() {
    let parsed = parse(
        RulesetKind::Session,
        &list(&[rule(1, 1, r#""tabIds":[5,-1]"#, BLOCK)]),
    );
    let e = Evaluator::new([(RulesetId::Session, parsed)], None);
    let frame = FrameState::new();
    let tab = |id: Option<i64>| {
        let mut r = script("https://t.test/");
        r.tab_id = id;
        e.evaluate(&r, &frame).is_block()
    };
    assert!(tab(Some(5)));
    assert!(tab(None), "no tab is -1");
    assert!(tab(Some(-1)));
    assert!(!tab(Some(6)));
    let parsed = parse(
        RulesetKind::Session,
        &list(&[rule(1, 1, r#""excludedTabIds":[5]"#, BLOCK)]),
    );
    let e = Evaluator::new([(RulesetId::Session, parsed)], None);
    let mut r = script("https://t.test/");
    r.tab_id = Some(5);
    assert!(!e.evaluate(&r, &frame).is_block());
    r.tab_id = Some(6);
    assert!(e.evaluate(&r, &frame).is_block());
}

#[test]
fn regex_filters_match_anywhere_and_respect_case() {
    let e = evaluator(&[rule(1, 1, r#""regexFilter":"/Ads/[0-9]+\\.js$""#, BLOCK)]);
    assert!(eval(&e, &script("https://t.test/x/ads/12.js")).is_block());
    assert_eq!(
        eval(&e, &script("https://t.test/x/ads/12.js?v=1")),
        Verdict::None
    );
    let e = evaluator(&[rule(
        1,
        1,
        r#""regexFilter":"/Ads/","isUrlFilterCaseSensitive":true"#,
        BLOCK,
    )]);
    assert!(eval(&e, &script("https://t.test/Ads/")).is_block());
    assert_eq!(eval(&e, &script("https://t.test/ads/")), Verdict::None);
}

#[test]
fn url_filters_are_case_insensitive_by_default_and_the_fragment_is_ignored() {
    let e = evaluator(&[rule(1, 1, r#""urlFilter":"/ADS/""#, BLOCK)]);
    assert!(eval(&e, &script("https://t.test/ads/x")).is_block());
    assert_eq!(eval(&e, &script("https://t.test/#/ads/")), Verdict::None);
}

#[test]
fn combined_conditions_all_have_to_hold() {
    let e = evaluator(&[rule(
        1,
        1,
        r#""urlFilter":"||cdn.test","initiatorDomains":["a.test"],"requestMethods":["get"],"resourceTypes":["script"],"domainType":"thirdParty""#,
        BLOCK,
    )]);
    let mut r = from(Some("https://a.test"), "https://cdn.test/x.js");
    r.is_third_party = Some(true);
    assert!(eval(&e, &r).is_block());
    r.is_third_party = Some(false);
    assert_eq!(eval(&e, &r), Verdict::None);
    r.is_third_party = Some(true);
    r.resource_type = Image;
    assert_eq!(eval(&e, &r), Verdict::None);
}

// --- what is evaluated -----------------------------------------------------------------------------

#[test]
fn only_web_schemes_are_evaluated() {
    let e = evaluator(&[rule(
        1,
        1,
        r#""resourceTypes":["script","main_frame"]"#,
        BLOCK,
    )]);
    for u in [
        "mercury-extension://abcdefghijklmnopabcdefghijklmnop/x.js",
        "file:///etc/hosts",
        "data:text/javascript,1",
        "blob:https://a.test/uuid",
        "about:blank",
        "mercury://settings",
    ] {
        assert_eq!(eval(&e, &req(u, Script)), Verdict::None, "{u}");
    }
    for u in [
        "http://a.test/",
        "https://a.test/",
        "ws://a.test/",
        "wss://a.test/",
        "ftp://a.test/",
    ] {
        assert!(eval(&e, &req(u, Script)).is_block(), "{u}");
    }
}

#[test]
fn rule_count_sums_all_rulesets() {
    let e = Evaluator::new(
        [
            (
                RulesetId::Static("a".into()),
                parse(RulesetKind::Static, &list(&[rule(1, 1, &cond("a"), BLOCK)])),
            ),
            (
                RulesetId::Dynamic,
                parse(
                    RulesetKind::Dynamic,
                    &list(&[rule(1, 1, &cond("a"), BLOCK), rule(2, 1, &cond("b"), BLOCK)]),
                ),
            ),
        ],
        None,
    );
    assert_eq!(e.rule_count(), 3);
}
