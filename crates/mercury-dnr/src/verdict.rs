// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! What the evaluator answers for a request, and how answers of several extensions combine.
//!
//! [`Rewrite`] has the same shape as `WebResourceRewrite` in `servo-embedder-traits/mercury/net.rs`
//! (EC37): a new URL and request-header changes. This crate does not depend on that crate (decision D1);
//! the engine lane converts. The net layer owns what a rewrite may change (the scheme stays http(s),
//! the origin stays unless top-level, no credential or cookie headers, bounded sizes) and refuses a
//! rewrite that breaks those rules; the evaluator only builds the URL.
//!
//! # Several extensions
//!
//! [`merge_verdicts`] follows Chromium's `RulesetManager::GetAction` and `GetModifyHeadersActions`
//! (`extensions/browser/api/declarative_net_request/ruleset_manager.cc`, read at Chromium `main`,
//! BSD-3-Clause; nothing copied) and Chrome's documentation: extensions are visited from the most
//! recently installed to the oldest; each one's action has a cross-extension class, `block` 3, `redirect`
//! or `upgradeScheme` 2, `allow` or `allowAllRequests` 1, and an action replaces the current one only if
//! its class is **strictly greater**, so a tie goes to the more recently installed extension. An `allow`
//! therefore protects a request from nothing outside its own extension: an older extension's `block` still
//! wins over a newer extension's `allow`. When the winner is a `block`, redirect or upgrade, nothing else
//! applies (no header changes). Otherwise the `modifyHeaders` changes of every extension are applied, most
//! recent extension first, with the conflict rules of `web_request_api_helpers.cc`
//! (`DNRHeaderAction::ConflictsWithSubsequentAction`): per header, the first accepted operation decides.
//! After an `append`, only appends follow; after a `set`, only appends **of the same extension** follow;
//! after a `remove`, nothing follows.

use std::collections::HashMap;

use url::Url;

use crate::rule::{HeaderOp, HeaderOperation};

/// Which ruleset of an extension a rule is in.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum RulesetId {
    /// A static ruleset, by the id the manifest gives it.
    Static(String),
    /// The dynamic ruleset.
    Dynamic,
    /// The session ruleset.
    Session,
}

/// A rule of one extension: its ruleset and its id in it.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct RuleRef {
    /// The ruleset.
    pub ruleset: RulesetId,
    /// The rule id in that ruleset.
    pub rule: u32,
}

/// A URL and request-header change, the shape of EC37's `WebResourceRewrite`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Rewrite {
    /// The new URL, if the URL changes. Its scheme is `http`, `https` or the extension's own.
    pub url: Option<Url>,
    /// Request headers to set (replace) as `(name, value)`.
    pub set_headers: Vec<(String, String)>,
    /// Request headers to remove.
    pub remove_headers: Vec<String>,
}

/// Which kind of document load an `allowAllRequests` rule matched.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameKind {
    /// A top-level navigation: the grant covers the whole page.
    MainFrame,
    /// A subframe navigation: the grant covers that frame and its descendants.
    SubFrame,
}

/// The scope of an `allowAllRequests` grant.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AllowScope {
    /// The priority of the rule: only rules with a strictly higher priority can still act in the frame.
    pub priority: u32,
    /// What kind of document the rule matched.
    pub frame: FrameKind,
}

/// An `allowAllRequests` grant the shell recorded for a frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FrameGrant {
    /// The rule that granted it.
    pub rule: RuleRef,
    /// Its scope.
    pub scope: AllowScope,
}

/// What one extension's rules do to a request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    /// No rule applies.
    None,
    /// An `allow` rule matched (or a grant covered the request): the extension leaves the request alone.
    Allow {
        /// The rule.
        rule: RuleRef,
    },
    /// An `allowAllRequests` rule matched a document load: the shell stores the grant in the frame's
    /// [`FrameState`](crate::evaluator::FrameState).
    AllowAllRequests {
        /// The rule.
        rule: RuleRef,
        /// Its scope.
        scope: AllowScope,
    },
    /// Cancel the request.
    Block {
        /// The rule.
        rule: RuleRef,
    },
    /// Send the request to the `https` (or `wss`) form of its URL.
    Upgrade {
        /// The rule.
        rule: RuleRef,
        /// The new URL.
        rewrite: Rewrite,
    },
    /// Send the request elsewhere.
    Redirect {
        /// The rule.
        rule: RuleRef,
        /// The new URL.
        rewrite: Rewrite,
    },
    /// Change headers. Response-header operations are not part of [`Rewrite`]: the net layer has no
    /// response hook yet (the engine lane decides). `append` request operations are also not
    /// representable in a [`Rewrite`]; they are listed here for the engine.
    ModifyHeaders {
        /// The rules that contributed.
        rules: Vec<RuleRef>,
        /// Accepted request-header operations, highest priority first.
        request_ops: Vec<HeaderOp>,
        /// Accepted response-header operations, highest priority first.
        response_ops: Vec<HeaderOp>,
    },
}

impl Verdict {
    /// The rewrite for the net layer: the URL change of an upgrade or redirect, or the request-header
    /// `set` and `remove` operations of a header verdict (`append` and all response operations are
    /// dropped: the shape has no place for them). `None` for every other verdict, and for a header verdict
    /// whose operations are all appends or response changes.
    pub fn rewrite(&self) -> Option<Rewrite> {
        match self {
            Verdict::Upgrade { rewrite, .. } | Verdict::Redirect { rewrite, .. } => {
                Some(rewrite.clone())
            }
            Verdict::ModifyHeaders { request_ops, .. } => {
                let mut rewrite = Rewrite::default();
                for op in request_ops {
                    match (op.operation, &op.value) {
                        (HeaderOperation::Set, Some(value)) => {
                            rewrite.set_headers.push((op.header.clone(), value.clone()))
                        }
                        (HeaderOperation::Remove, _) => {
                            rewrite.remove_headers.push(op.header.clone());
                        }
                        _ => {}
                    }
                }
                (!rewrite.set_headers.is_empty() || !rewrite.remove_headers.is_empty())
                    .then_some(rewrite)
            }
            _ => None,
        }
    }

    /// Whether the request must be cancelled.
    pub fn is_block(&self) -> bool {
        matches!(self, Verdict::Block { .. })
    }

    /// The grant to store in the frame's state, for an `AllowAllRequests` verdict.
    pub fn grant(&self) -> Option<FrameGrant> {
        match self {
            Verdict::AllowAllRequests { rule, scope } => Some(FrameGrant {
                rule: rule.clone(),
                scope: *scope,
            }),
            _ => None,
        }
    }

    /// The cross-extension class of Chromium's `GetAction`: block 3, redirect and upgrade 2, allow 1,
    /// nothing 0.
    fn class(&self) -> u8 {
        match self {
            Verdict::Block { .. } => 3,
            Verdict::Redirect { .. } | Verdict::Upgrade { .. } => 2,
            Verdict::Allow { .. } | Verdict::AllowAllRequests { .. } => 1,
            Verdict::None | Verdict::ModifyHeaders { .. } => 0,
        }
    }
}

/// Applies the conflict rules of Chromium's `ModifyRequestHeadersForAction` to a list of operations in
/// priority order (highest first). `owner` identifies the extension; `T` is carried through (a rule).
/// Returns the accepted operations in the same order. Header names compare ASCII case-insensitively.
pub(crate) fn resolve_header_ops<T>(items: Vec<(u64, T, HeaderOp)>) -> Vec<(T, HeaderOp)> {
    let mut first: HashMap<String, (HeaderOperation, u64)> = HashMap::new();
    let mut accepted = Vec::new();
    for (owner, tag, op) in items {
        let key = op.header.to_ascii_lowercase();
        if let Some((first_op, first_owner)) = first.get(&key) {
            let conflicts = match first_op {
                HeaderOperation::Append => op.operation != HeaderOperation::Append,
                HeaderOperation::Set => {
                    *first_owner != owner || op.operation != HeaderOperation::Append
                }
                HeaderOperation::Remove => true,
            };
            if conflicts {
                continue;
            }
        } else {
            first.insert(key, (op.operation, owner));
        }
        accepted.push((tag, op));
    }
    accepted
}

/// Combines the verdicts of several extensions into one. `extension_install_order` grows with install
/// time: a larger number is a more recently installed extension. See the module docs for the rules and
/// where they come from. For a merged `ModifyHeaders`, `rules` lists every rule of the contributing
/// verdicts (a verdict does not say which rule gave which operation).
pub fn merge_verdicts(verdicts: impl IntoIterator<Item = (u64, Verdict)>) -> Verdict {
    let mut ordered: Vec<(u64, Verdict)> = verdicts.into_iter().collect();
    ordered.sort_by_key(|(order, _)| std::cmp::Reverse(*order));

    let mut winner: Option<&Verdict> = None;
    let mut winner_class = 0u8;
    for (_, verdict) in &ordered {
        if verdict.class() > winner_class {
            winner_class = verdict.class();
            winner = Some(verdict);
        }
    }
    if winner_class >= 2
        && let Some(verdict) = winner
    {
        return verdict.clone();
    }

    let mut rules = Vec::new();
    let mut request_items = Vec::new();
    let mut response_items = Vec::new();
    for (order, verdict) in &ordered {
        if let Verdict::ModifyHeaders {
            rules: r,
            request_ops,
            response_ops,
        } = verdict
        {
            rules.extend(r.iter().cloned());
            request_items.extend(request_ops.iter().map(|op| (*order, (), op.clone())));
            response_items.extend(response_ops.iter().map(|op| (*order, (), op.clone())));
        }
    }
    let request_ops: Vec<HeaderOp> = resolve_header_ops(request_items)
        .into_iter()
        .map(|(_, op)| op)
        .collect();
    let response_ops: Vec<HeaderOp> = resolve_header_ops(response_items)
        .into_iter()
        .map(|(_, op)| op)
        .collect();
    if !request_ops.is_empty() || !response_ops.is_empty() {
        return Verdict::ModifyHeaders {
            rules,
            request_ops,
            response_ops,
        };
    }
    winner.cloned().unwrap_or(Verdict::None)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rr(id: u32) -> RuleRef {
        RuleRef {
            ruleset: RulesetId::Dynamic,
            rule: id,
        }
    }

    fn url(s: &str) -> Url {
        Url::parse(s).expect("url")
    }

    fn op(header: &str, operation: HeaderOperation, value: Option<&str>) -> HeaderOp {
        HeaderOp {
            header: header.into(),
            operation,
            value: value.map(str::to_owned),
        }
    }

    fn modify(rule: u32, request: Vec<HeaderOp>) -> Verdict {
        Verdict::ModifyHeaders {
            rules: vec![rr(rule)],
            request_ops: request,
            response_ops: vec![],
        }
    }

    fn redirect(rule: u32) -> Verdict {
        Verdict::Redirect {
            rule: rr(rule),
            rewrite: Rewrite {
                url: Some(url("https://r.test/")),
                ..Rewrite::default()
            },
        }
    }

    #[test]
    fn rewrite_of_redirect_and_upgrade_is_the_stored_url() {
        let v = redirect(1);
        assert_eq!(
            v.rewrite().and_then(|r| r.url),
            Some(url("https://r.test/"))
        );
        let u = Verdict::Upgrade {
            rule: rr(2),
            rewrite: Rewrite {
                url: Some(url("https://u.test/")),
                ..Rewrite::default()
            },
        };
        assert!(u.rewrite().is_some());
    }

    #[test]
    fn rewrite_of_other_verdicts_is_none() {
        assert_eq!(Verdict::None.rewrite(), None);
        assert_eq!(Verdict::Block { rule: rr(1) }.rewrite(), None);
        assert_eq!(Verdict::Allow { rule: rr(1) }.rewrite(), None);
    }

    #[test]
    fn rewrite_of_headers_keeps_set_and_remove_and_drops_append_and_response() {
        let v = Verdict::ModifyHeaders {
            rules: vec![rr(1)],
            request_ops: vec![
                op("x-a", HeaderOperation::Set, Some("1")),
                op("x-b", HeaderOperation::Remove, None),
                op("accept", HeaderOperation::Append, Some("text/plain")),
            ],
            response_ops: vec![op("x-c", HeaderOperation::Set, Some("2"))],
        };
        let r = v.rewrite().expect("rewrite");
        assert_eq!(r.url, None);
        assert_eq!(r.set_headers, vec![("x-a".into(), "1".into())]);
        assert_eq!(r.remove_headers, vec!["x-b".to_owned()]);
    }

    #[test]
    fn rewrite_of_response_only_headers_is_none() {
        let v = Verdict::ModifyHeaders {
            rules: vec![rr(1)],
            request_ops: vec![],
            response_ops: vec![op("x-c", HeaderOperation::Set, Some("2"))],
        };
        assert_eq!(v.rewrite(), None);
    }

    #[test]
    fn grant_comes_only_from_allow_all_requests() {
        let scope = AllowScope {
            priority: 3,
            frame: FrameKind::SubFrame,
        };
        let v = Verdict::AllowAllRequests { rule: rr(4), scope };
        assert_eq!(v.grant().map(|g| g.scope), Some(scope));
        assert_eq!(Verdict::Allow { rule: rr(4) }.grant(), None);
    }

    #[test]
    fn merge_of_nothing_is_none() {
        assert_eq!(merge_verdicts(Vec::new()), Verdict::None);
        assert_eq!(
            merge_verdicts(vec![(1, Verdict::None), (2, Verdict::None)]),
            Verdict::None
        );
    }

    #[test]
    fn merge_block_beats_a_newer_redirect() {
        let merged = merge_verdicts(vec![(1, Verdict::Block { rule: rr(1) }), (2, redirect(2))]);
        assert_eq!(merged, Verdict::Block { rule: rr(1) });
    }

    #[test]
    fn merge_block_beats_a_newer_allow() {
        let merged = merge_verdicts(vec![
            (1, Verdict::Block { rule: rr(1) }),
            (9, Verdict::Allow { rule: rr(2) }),
        ]);
        assert_eq!(merged, Verdict::Block { rule: rr(1) });
    }

    #[test]
    fn merge_redirect_beats_a_newer_allow() {
        let merged = merge_verdicts(vec![(1, redirect(1)), (2, Verdict::Allow { rule: rr(2) })]);
        assert_eq!(merged, redirect(1));
    }

    #[test]
    fn merge_tie_goes_to_the_most_recent_extension() {
        let merged = merge_verdicts(vec![
            (5, Verdict::Block { rule: rr(5) }),
            (7, Verdict::Block { rule: rr(7) }),
            (6, Verdict::Block { rule: rr(6) }),
        ]);
        assert_eq!(merged, Verdict::Block { rule: rr(7) });
        let merged = merge_verdicts(vec![(1, redirect(1)), (2, redirect(2))]);
        assert_eq!(merged, redirect(2));
    }

    #[test]
    fn merge_allow_wins_over_nothing_and_ties_to_the_newest() {
        let merged = merge_verdicts(vec![
            (1, Verdict::Allow { rule: rr(1) }),
            (2, Verdict::Allow { rule: rr(2) }),
        ]);
        assert_eq!(merged, Verdict::Allow { rule: rr(2) });
    }

    #[test]
    fn merge_block_suppresses_header_changes_of_everyone() {
        let merged = merge_verdicts(vec![
            (1, Verdict::Block { rule: rr(1) }),
            (2, modify(2, vec![op("x", HeaderOperation::Set, Some("1"))])),
        ]);
        assert_eq!(merged, Verdict::Block { rule: rr(1) });
    }

    #[test]
    fn merge_allow_does_not_suppress_header_changes_of_other_extensions() {
        let merged = merge_verdicts(vec![
            (3, Verdict::Allow { rule: rr(1) }),
            (1, modify(2, vec![op("x", HeaderOperation::Set, Some("1"))])),
        ]);
        let Verdict::ModifyHeaders { request_ops, .. } = merged else {
            panic!("expected header changes");
        };
        assert_eq!(request_ops.len(), 1);
    }

    #[test]
    fn merge_header_ops_recent_extension_first() {
        let merged = merge_verdicts(vec![
            (
                1,
                modify(1, vec![op("x-a", HeaderOperation::Set, Some("old"))]),
            ),
            (
                2,
                modify(2, vec![op("x-b", HeaderOperation::Set, Some("new"))]),
            ),
        ]);
        let Verdict::ModifyHeaders {
            request_ops, rules, ..
        } = merged
        else {
            panic!("expected header changes");
        };
        assert_eq!(request_ops[0].header, "x-b");
        assert_eq!(request_ops[1].header, "x-a");
        assert_eq!(rules.len(), 2);
    }

    #[test]
    fn resolve_set_blocks_other_extensions_set_and_remove() {
        let got = resolve_header_ops(vec![
            (2, (), op("X", HeaderOperation::Set, Some("new"))),
            (1, (), op("x", HeaderOperation::Set, Some("old"))),
            (1, (), op("x", HeaderOperation::Remove, None)),
        ]);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].1.value.as_deref(), Some("new"));
    }

    #[test]
    fn resolve_set_allows_appends_of_the_same_extension_only() {
        let got = resolve_header_ops(vec![
            (2, 1, op("cookie", HeaderOperation::Set, Some("a=1"))),
            (2, 2, op("cookie", HeaderOperation::Append, Some("b=2"))),
            (1, 3, op("cookie", HeaderOperation::Append, Some("c=3"))),
        ]);
        let tags: Vec<i32> = got.iter().map(|(t, _)| *t).collect();
        assert_eq!(tags, vec![1, 2]);
    }

    #[test]
    fn resolve_append_allows_only_more_appends() {
        let got = resolve_header_ops(vec![
            (1, 1, op("accept", HeaderOperation::Append, Some("a"))),
            (1, 2, op("accept", HeaderOperation::Set, Some("b"))),
            (2, 3, op("accept", HeaderOperation::Append, Some("c"))),
            (2, 4, op("accept", HeaderOperation::Remove, None)),
        ]);
        let tags: Vec<i32> = got.iter().map(|(t, _)| *t).collect();
        assert_eq!(tags, vec![1, 3]);
    }

    #[test]
    fn resolve_remove_blocks_everything_after_it() {
        let got = resolve_header_ops(vec![
            (1, 1, op("x", HeaderOperation::Remove, None)),
            (1, 2, op("x", HeaderOperation::Append, Some("a"))),
            (1, 3, op("x", HeaderOperation::Set, Some("a"))),
            (1, 4, op("y", HeaderOperation::Set, Some("a"))),
        ]);
        let tags: Vec<i32> = got.iter().map(|(t, _)| *t).collect();
        assert_eq!(tags, vec![1, 4]);
    }

    #[test]
    fn rule_refs_order_by_ruleset_then_id() {
        let a = RuleRef {
            ruleset: RulesetId::Static("a".into()),
            rule: 9,
        };
        let b = RuleRef {
            ruleset: RulesetId::Dynamic,
            rule: 1,
        };
        assert!(a < b);
    }
}
