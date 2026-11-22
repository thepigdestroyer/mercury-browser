// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! Deterministic fuzzing (xorshift, no `rand`): nothing may panic, the index never changes an answer.

mod common;

use common::*;
use mercury_dnr::evaluator::{Evaluator, FrameState};
use mercury_dnr::limits::Limits;
use mercury_dnr::request::{RequestMetadata, ResourceType};
use mercury_dnr::rule::{RulesetKind, parse_ruleset};
use mercury_dnr::url_filter::{RegexFilter, UrlFilter, UrlTarget};
use mercury_dnr::verdict::{AllowScope, FrameGrant, FrameKind, RuleRef, RulesetId, Verdict};

const HOSTS: &[&str] = &[
    "example.com",
    "ads.example.com",
    "cdn.tracker.test",
    "a.test",
    "127.0.0.1",
    "xn--p1ai.test",
];
const PATHS: &[&str] = &[
    "",
    "/",
    "/ads/x.js",
    "/a/b/c",
    "/track?id=1",
    "/x.png?a=1&b=2",
    "/%41%20",
];
const SCHEMES: &[&str] = &["http", "https", "ws", "wss", "ftp"];
const FILTERS: &[&str] = &[
    "||example.com^",
    "||ads.example.com",
    "|https://",
    "/ads/",
    "*/track*",
    "x.js|",
    "||cdn.tracker.test/",
    "a*b*c",
    "^id=",
    "*",
    "|http:",
    ".png",
];
const REGEXES: &[&str] = &[
    r"/ads/",
    r"^https?://[a-z.]+/a/",
    r"\.(png|js)(\?|$)",
    r"(a|b)+c",
];
const TYPES: &[ResourceType] = &ResourceType::ALL;

fn gen_url(rng: &mut XorShift) -> String {
    format!(
        "{}://{}{}",
        rng.pick(SCHEMES),
        rng.pick(HOSTS),
        rng.pick(PATHS)
    )
}

fn gen_request(rng: &mut XorShift) -> RequestMetadata {
    let url = url(&gen_url(rng));
    let ty = TYPES[rng.below(TYPES.len())];
    let method = rng.pick(&["GET", "post", "PUT", "HEAD", "weird"]);
    let mut r = RequestMetadata::new(url, method, ty);
    if rng.chance(2) {
        r.initiator_origin = Some(
            rng.pick(&["https://example.com", "null", "https://a.test", "x"])
                .to_owned(),
        );
    }
    if rng.chance(3) {
        r.top_level_site = Some("https://a.test".into());
    }
    r.is_third_party = match rng.below(3) {
        0 => None,
        1 => Some(true),
        _ => Some(false),
    };
    if rng.chance(2) {
        r.tab_id = Some(i64::try_from(rng.below(4)).unwrap_or(0) - 1);
    }
    r
}

fn gen_rule(rng: &mut XorShift, id: u32) -> String {
    let mut cond: Vec<String> = Vec::new();
    match rng.below(4) {
        0 | 1 => cond.push(format!(r#""urlFilter":"{}""#, rng.pick(FILTERS))),
        2 => cond.push(format!(
            r#""regexFilter":"{}""#,
            rng.pick(REGEXES).replace('\\', "\\\\")
        )),
        _ => {}
    }
    if rng.chance(3) {
        cond.push(format!(r#""initiatorDomains":["{}"]"#, rng.pick(HOSTS)));
    }
    if rng.chance(4) {
        cond.push(format!(r#""requestDomains":["{}"]"#, rng.pick(HOSTS)));
    }
    if rng.chance(3) {
        cond.push(format!(
            r#""resourceTypes":["{}","{}"]"#,
            TYPES[rng.below(TYPES.len())].name(),
            TYPES[rng.below(TYPES.len())].name()
        ));
    }
    if rng.chance(5) {
        cond.push(format!(
            r#""requestMethods":["{}"]"#,
            rng.pick(&["get", "post", "other"])
        ));
    }
    if rng.chance(5) {
        cond.push(format!(
            r#""domainType":"{}""#,
            rng.pick(&["firstParty", "thirdParty"])
        ));
    }
    let action = match rng.below(9) {
        0 | 1 => r#""type":"block""#.to_owned(),
        2 => r#""type":"allow""#.to_owned(),
        3 => r#""type":"upgradeScheme""#.to_owned(),
        4 => r#""type":"redirect","redirect":{"url":"https://r.test/x"}"#.to_owned(),
        5 => r#""type":"redirect","redirect":{"transform":{"scheme":"https","queryTransform":{"removeParams":["a"]}}}"#.to_owned(),
        6 => r#""type":"redirect","redirect":{"extensionPath":"/p.png"}"#.to_owned(),
        7 => {
            cond.retain(|c| !c.starts_with(r#""resourceTypes""#));
            cond.push(r#""resourceTypes":["main_frame","sub_frame"]"#.to_owned());
            r#""type":"allowAllRequests""#.to_owned()
        }
        _ => format!(
            r#""type":"modifyHeaders","requestHeaders":[{{"header":"{}","operation":"{}","value":"v"}}]"#,
            rng.pick(&["accept", "x-a", "cookie"]),
            rng.pick(&["set", "append"])
        ),
    };
    let action = if action.contains("append") && action.contains("x-a") {
        action.replace("append", "set")
    } else {
        action
    };
    rule(
        id,
        u32::try_from(1 + rng.below(4)).unwrap_or(1),
        &cond.join(","),
        &action,
    )
}

fn gen_ruleset(rng: &mut XorShift, n: usize) -> String {
    let rules: Vec<String> = (1..=n)
        .map(|i| gen_rule(rng, u32::try_from(i).unwrap_or(1)))
        .collect();
    list(&rules)
}

fn mutate(rng: &mut XorShift, input: &[u8]) -> Vec<u8> {
    let mut out = input.to_vec();
    for _ in 0..1 + rng.below(4) {
        if out.is_empty() {
            out.push(b'[');
            continue;
        }
        let at = rng.below(out.len());
        match rng.below(6) {
            0 => out[at] ^= 1 << rng.below(8),
            1 => {
                out.remove(at);
            }
            2 => {
                const JUNK: &[u8] = &[b'[', b']', b'{', b'}', b'"', b',', b':', b'\\', 0, 0xFF];
                out.insert(at, JUNK[rng.below(JUNK.len())]);
            }
            3 => out.truncate(at),
            4 => {
                let end = (at + rng.below(32)).min(out.len());
                let chunk = out[at..end].to_vec();
                out.splice(at..at, chunk);
            }
            _ => out[at] = rng.below(256) as u8,
        }
    }
    out
}

#[test]
fn mutated_rulesets_never_panic_and_the_index_never_changes_an_answer() {
    let mut rng = XorShift(0x1234_5678_9ABC_DEF1);
    let seeds: Vec<String> = (0..6).map(|_| gen_ruleset(&mut rng, 12)).collect();
    let mut parsed_ok = 0usize;
    let mut evaluations = 0usize;
    for iteration in 0..3000usize {
        let seed = &seeds[iteration % seeds.len()];
        let input = mutate(&mut rng, seed.as_bytes());
        for kind in [
            RulesetKind::Static,
            RulesetKind::Dynamic,
            RulesetKind::Session,
        ] {
            if let Ok(parsed) = parse_ruleset(&input, kind, &Limits::default()) {
                parsed_ok += 1;
                let e = Evaluator::new([(RulesetId::Dynamic, parsed)], Some(url(EXT_BASE)));
                for _ in 0..3 {
                    let r = gen_request(&mut rng);
                    let frame = FrameState::new();
                    assert_eq!(e.evaluate(&r, &frame), e.evaluate_unindexed(&r, &frame));
                    evaluations += 1;
                }
            }
        }
    }
    assert!(parsed_ok > 500, "only {parsed_ok} mutated rulesets parsed");
    assert!(evaluations > 1500);
}

#[test]
fn mutated_junk_never_panics_the_parser() {
    let mut rng = XorShift(0xDEAD_BEEF_CAFE_F00D);
    for _ in 0..5000 {
        let len = rng.below(200);
        let junk: Vec<u8> = (0..len).map(|_| rng.below(256) as u8).collect();
        for kind in [
            RulesetKind::Static,
            RulesetKind::Dynamic,
            RulesetKind::Session,
        ] {
            let _ = parse_ruleset(&junk, kind, &Limits::default());
        }
    }
}

#[test]
fn random_filter_text_never_panics_the_filter_compilers() {
    let mut rng = XorShift(0x0BAD_C0DE_0BAD_C0DE);
    let alphabet: &[u8] = b"ab|^*.-_%/?&=:()[]{}\\+$ \n\0\xC3\xA9";
    let limits = Limits::default();
    for _ in 0..4000 {
        let len = rng.below(40);
        let bytes: Vec<u8> = (0..len)
            .map(|_| alphabet[rng.below(alphabet.len())])
            .collect();
        if let Ok(text) = String::from_utf8(bytes) {
            if let Ok(f) = UrlFilter::parse(&text, rng.chance(2), 2048) {
                for u in ["https://a.test/ab", "", "|", "ab^ab", "https://x/\u{e9}"] {
                    let _ = f.matches(&UrlTarget::from_text(u));
                }
            }
            if let Ok(f) = RegexFilter::compile(&text, rng.chance(2), &limits) {
                let _ = f.is_match(&UrlTarget::from_text("https://a.test/ab"));
            }
        }
    }
}

#[test]
fn random_urls_never_panic_the_evaluator() {
    let mut rng = XorShift(0x5151_5151_A1A1_A1A1);
    let rules = gen_ruleset(&mut rng, 40);
    let e = Evaluator::new(
        [(RulesetId::Dynamic, parse(RulesetKind::Static, &rules))],
        Some(url(EXT_BASE)),
    );
    let alphabet: &[u8] = b"abc./:?#&=%@[]\\ \t~^|*";
    let mut parsed = 0usize;
    for _ in 0..6000 {
        let scheme = rng.pick(SCHEMES);
        let len = rng.below(60);
        let tail: String = (0..len)
            .map(|_| char::from(alphabet[rng.below(alphabet.len())]))
            .collect();
        if let Ok(u) = url::Url::parse(&format!("{scheme}://{tail}")) {
            parsed += 1;
            let mut r = RequestMetadata::new(u, "GET", TYPES[rng.below(TYPES.len())]);
            r.initiator_origin = Some(tail.clone());
            let frame = FrameState::new();
            assert_eq!(e.evaluate(&r, &frame), e.evaluate_unindexed(&r, &frame));
        }
    }
    assert!(parsed > 500, "only {parsed} URLs parsed");
}

#[test]
fn differential_over_random_rulesets_requests_and_grants() {
    let mut rng = XorShift(0xFEED_FACE_0123_4567);
    let mut kinds = [0usize; 7];
    let mut total = 0usize;
    for _ in 0..120 {
        let n = 5 + rng.below(40);
        let e = Evaluator::new(
            [(
                RulesetId::Dynamic,
                parse(RulesetKind::Static, &gen_ruleset(&mut rng, n)),
            )],
            Some(url(EXT_BASE)),
        );
        for _ in 0..200 {
            let r = gen_request(&mut rng);
            let mut frame = FrameState::new();
            if rng.chance(4) {
                frame = frame.with_grant(FrameGrant {
                    rule: RuleRef {
                        ruleset: RulesetId::Dynamic,
                        rule: 999,
                    },
                    scope: AllowScope {
                        priority: u32::try_from(rng.below(4)).unwrap_or(0),
                        frame: FrameKind::MainFrame,
                    },
                });
            }
            let v = e.evaluate(&r, &frame);
            assert_eq!(v, e.evaluate_unindexed(&r, &frame), "request {}", r.url);
            kinds[match v {
                Verdict::None => 0,
                Verdict::Allow { .. } => 1,
                Verdict::AllowAllRequests { .. } => 2,
                Verdict::Block { .. } => 3,
                Verdict::Upgrade { .. } => 4,
                Verdict::Redirect { .. } => 5,
                Verdict::ModifyHeaders { .. } => 6,
            }] += 1;
            total += 1;
        }
    }
    assert!(total >= 20_000);
    assert!(kinds.iter().all(|k| *k > 0), "verdict coverage {kinds:?}");
}

#[test]
fn a_million_byte_url_is_handled_without_blowing_up() {
    let rules = list(&[
        rule(1, 1, r#""urlFilter":"a*b*c*d*e*zzz""#, r#""type":"block""#),
        rule(2, 1, r#""urlFilter":"||a.test/*x|""#, r#""type":"allow""#),
        rule(3, 1, r#""regexFilter":"(a|b)*c$""#, r#""type":"block""#),
        rule(4, 1, r#""urlFilter":"zzz^""#, r#""type":"block""#),
    ]);
    let e = evaluator_from(&rules);
    let big = format!("https://a.test/{}", "ab".repeat(500_000));
    let r = RequestMetadata::new(url(&big), "GET", ResourceType::Script);
    let v = e.evaluate(&r, &FrameState::new());
    assert_eq!(v, e.evaluate_unindexed(&r, &FrameState::new()));
}

fn evaluator_from(json: &str) -> Evaluator {
    Evaluator::new(
        [(RulesetId::Dynamic, parse(RulesetKind::Dynamic, json))],
        None,
    )
}

#[test]
fn evaluating_a_ruleset_of_many_rules_is_consistent() {
    let mut rng = XorShift(0x7777_1111_3333_5555);
    let e = Evaluator::new(
        [(
            RulesetId::Dynamic,
            parse(RulesetKind::Static, &gen_ruleset(&mut rng, 3000)),
        )],
        Some(url(EXT_BASE)),
    );
    assert!(e.rule_count() > 1500);
    for _ in 0..300 {
        let r = gen_request(&mut rng);
        assert_eq!(
            e.evaluate(&r, &FrameState::new()),
            e.evaluate_unindexed(&r, &FrameState::new())
        );
    }
}
