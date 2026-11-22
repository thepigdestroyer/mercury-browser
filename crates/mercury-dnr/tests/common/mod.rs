// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! Helpers shared by the integration tests.
#![allow(dead_code)]

use mercury_dnr::evaluator::{Evaluator, FrameState};
use mercury_dnr::limits::Limits;
use mercury_dnr::request::{RequestMetadata, ResourceType};
use mercury_dnr::rule::{ParsedRuleset, RulesetKind, parse_ruleset};
use mercury_dnr::verdict::{RulesetId, Verdict};
use url::Url;

pub const EXT_BASE: &str = "mercury-extension://abcdefghijklmnopabcdefghijklmnop/";

pub fn url(s: &str) -> Url {
    Url::parse(s).expect("test url")
}

pub fn parse(kind: RulesetKind, json: &str) -> ParsedRuleset {
    parse_ruleset(json.as_bytes(), kind, &Limits::default()).expect("test ruleset")
}

/// `{"id":..,"priority":..,"condition":{..},"action":{..}}`
pub fn rule(id: u32, priority: u32, condition: &str, action: &str) -> String {
    format!(
        r#"{{"id":{id},"priority":{priority},"condition":{{{condition}}},"action":{{{action}}}}}"#
    )
}

pub fn list(rules: &[String]) -> String {
    format!("[{}]", rules.join(","))
}

/// An evaluator over one dynamic ruleset with the extension base set.
pub fn evaluator(rules: &[String]) -> Evaluator {
    Evaluator::new(
        [(
            RulesetId::Dynamic,
            parse(RulesetKind::Dynamic, &list(rules)),
        )],
        Some(url(EXT_BASE)),
    )
}

pub fn req(u: &str, resource_type: ResourceType) -> RequestMetadata {
    RequestMetadata::new(url(u), "GET", resource_type)
}

pub fn eval(e: &Evaluator, r: &RequestMetadata) -> Verdict {
    let indexed = e.evaluate(r, &FrameState::new());
    let linear = e.evaluate_unindexed(r, &FrameState::new());
    assert_eq!(
        indexed, linear,
        "the index changed the answer for {}",
        r.url
    );
    indexed
}

pub fn rule_id(v: &Verdict) -> Option<u32> {
    match v {
        Verdict::Allow { rule }
        | Verdict::AllowAllRequests { rule, .. }
        | Verdict::Block { rule }
        | Verdict::Upgrade { rule, .. }
        | Verdict::Redirect { rule, .. } => Some(rule.rule),
        Verdict::ModifyHeaders { rules, .. } => rules.first().map(|r| r.rule),
        Verdict::None => None,
    }
}

pub fn redirect_url(v: &Verdict) -> Option<String> {
    match v {
        Verdict::Redirect { rewrite, .. } | Verdict::Upgrade { rewrite, .. } => {
            rewrite.url.as_ref().map(|u| u.as_str().to_owned())
        }
        _ => None,
    }
}

/// A small deterministic generator (no `rand` in the tree for this crate).
pub struct XorShift(pub u64);

impl XorShift {
    pub fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }

    pub fn below(&mut self, n: usize) -> usize {
        usize::try_from(self.next() % u64::try_from(n.max(1)).unwrap_or(1)).unwrap_or(0)
    }

    pub fn chance(&mut self, one_in: usize) -> bool {
        self.below(one_in) == 0
    }

    pub fn pick<'a, T: ?Sized>(&mut self, items: &[&'a T]) -> &'a T {
        items[self.below(items.len())]
    }
}
