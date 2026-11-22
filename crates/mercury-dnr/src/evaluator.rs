// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! The evaluator: request metadata in, one [`Verdict`] out, for the rulesets of one extension.
//!
//! Semantics per <https://developer.chrome.com/docs/extensions/reference/api/declarativeNetRequest>
//! ("Rule evaluation") and Chromium's `CompositeMatcher::GetAction`/`GetModifyHeadersActions` and
//! `RulesetManager` (BSD-3-Clause; read, not copied). Several extensions combine with
//! [`merge_verdicts`](crate::verdict::merge_verdicts).
//!
//! # Order inside one extension
//!
//! All the extension's rulesets (static ones that are enabled, dynamic, session) are one pool. Among the
//! matching rules the highest `priority` wins; at one priority the order of actions is `allow`, then
//! `allowAllRequests`, then `block`, then `upgradeScheme`, then `redirect` (Chromium's
//! `GetActionTypePriority`). A rule that cannot act (an `upgradeScheme` on an `https` request, a redirect
//! whose URL cannot be built or would not change the URL, or one that would produce a scheme other than
//! `http`, `https` or the extension's own) is **skipped** and the next best rule is used. Ties between
//! rules with the same priority and action go to the earlier ruleset, then the lower rule id (browsers
//! have agreed not to standardise this order).
//!
//! * `allow` and `allowAllRequests` at priority P stop every rule of priority P or lower.
//! * A winning `block`, `upgradeScheme` or `redirect` is the whole answer: no header changes apply.
//! * Otherwise the matching `modifyHeaders` rules with a priority strictly above the winning `allow`'s
//!   (and above the frame's grant) are collected, highest priority first, and their operations are
//!   filtered by the conflict rules in [`crate::verdict`]. Response-header operations come back in
//!   [`Verdict::ModifyHeaders`] but are not part of a [`Rewrite`](crate::verdict::Rewrite).
//!
//! # `allowAllRequests`
//!
//! A matching `allowAllRequests` rule on a `main_frame` or `sub_frame` request returns
//! [`Verdict::AllowAllRequests`]. The shell keeps the grant with the frame ([`FrameState::with_grant`])
//! and passes the frame's state with every later request of that frame and its descendants; only rules
//! with a priority strictly above the grant's can still act. A grant that masks everything answers
//! [`Verdict::Allow`] with the granting rule.
//!
//! # What is evaluated
//!
//! Requests whose scheme is `http`, `https`, `ws`, `wss` or `ftp`. Everything else (the extension's own
//! scheme, `file`, `data`, `blob`, ...) answers [`Verdict::None`]: the extension may not touch it, and
//! `file` access is a permission decision for the shell.
//!
//! # Not done here
//!
//! The built-in blocker runs first and an extension's `allow` cannot override it (decision D5): that
//! ordering is the engine lane's. `webRequest`, `getMatchedRules` and the action badge are follow-ups.
//! Rules cannot match on response headers (they are refused at parse time).

use url::Url;

use crate::index::RuleIndex;
use crate::request::{
    RequestMetadata, RequestMethod, ResourceType, host_matches_domain, host_of_origin,
    normalise_host,
};
use crate::rule::{
    Action, Condition, DomainType, HeaderOp, ParsedRuleset, QueryChange, QueryTransform, Redirect,
    Rule, UrlTransform,
};
use crate::url_filter::{UrlPattern, UrlTarget};
use crate::verdict::{
    AllowScope, FrameGrant, FrameKind, Rewrite, RuleRef, RulesetId, Verdict, resolve_header_ops,
};

/// The `allowAllRequests` grants the shell recorded for the frame a request belongs to (its own grant
/// and its ancestors', merged: only the strongest one matters).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FrameState {
    grant: Option<FrameGrant>,
}

impl FrameState {
    /// A frame with no grant.
    pub fn new() -> Self {
        FrameState::default()
    }

    /// The state after recording a grant. The grant with the higher priority is kept; on a tie the one
    /// already recorded stays.
    #[must_use]
    pub fn with_grant(mut self, grant: FrameGrant) -> Self {
        let replace = match &self.grant {
            None => true,
            Some(existing) => grant.scope.priority > existing.scope.priority,
        };
        if replace {
            self.grant = Some(grant);
        }
        self
    }

    /// The strongest grant, if any.
    pub fn grant(&self) -> Option<&FrameGrant> {
        self.grant.as_ref()
    }

    /// The state a child frame starts with: the same grant (a grant covers descendants).
    pub fn for_child(&self) -> FrameState {
        self.clone()
    }

    fn priority(&self) -> u32 {
        self.grant.as_ref().map_or(0, |g| g.scope.priority)
    }
}

#[derive(Debug)]
struct IndexedRuleset {
    id: RulesetId,
    rules: Vec<Rule>,
    index: RuleIndex,
}

/// The evaluator for the rulesets of one extension.
#[derive(Debug)]
pub struct Evaluator {
    rulesets: Vec<IndexedRuleset>,
    extension_base: Option<Url>,
}

struct Hit<'a> {
    ruleset_order: usize,
    ruleset: &'a RulesetId,
    rule: &'a Rule,
}

impl Hit<'_> {
    fn rule_ref(&self) -> RuleRef {
        RuleRef {
            ruleset: self.ruleset.clone(),
            rule: self.rule.id,
        }
    }
}

/// Everything about a request that conditions look at, computed once.
struct Ctx<'a> {
    req: &'a RequestMetadata,
    target: UrlTarget<'a>,
    method: RequestMethod,
    host: String,
    initiator_host: Option<String>,
    top_host: Option<String>,
    third_party: bool,
    tab: i64,
}

impl<'a> Ctx<'a> {
    fn new(req: &'a RequestMetadata) -> Self {
        let initiator_host = req.initiator_origin.as_deref().and_then(host_of_origin);
        let top_host = req
            .top_level_site
            .as_deref()
            .and_then(host_of_origin)
            .or_else(|| initiator_host.clone());
        Ctx {
            req,
            target: UrlTarget::new(&req.url),
            method: req.request_method(),
            host: req.url.host_str().map(normalise_host).unwrap_or_default(),
            initiator_host,
            top_host,
            third_party: req.is_third_party != Some(false),
            tab: req.effective_tab_id(),
        }
    }
}

impl Evaluator {
    /// Builds an evaluator over the rulesets of one extension. `extension_base` is the extension's own
    /// origin URL (`mercury-extension://<id>/`), used to turn `extensionPath` into a URL; without it
    /// those redirects are skipped. A base that is not hierarchical, has no host, or uses `http(s)` is
    /// ignored.
    pub fn new(
        rulesets: impl IntoIterator<Item = (RulesetId, ParsedRuleset)>,
        extension_base: Option<Url>,
    ) -> Self {
        let rulesets = rulesets
            .into_iter()
            .map(|(id, parsed)| {
                let index = RuleIndex::build(
                    parsed
                        .rules
                        .iter()
                        .map(|rule| rule.condition.pattern.as_ref()),
                );
                IndexedRuleset {
                    id,
                    rules: parsed.rules,
                    index,
                }
            })
            .collect();
        let extension_base = extension_base.filter(|base| {
            !base.cannot_be_a_base()
                && base.host_str().is_some()
                && !matches!(base.scheme(), "http" | "https")
        });
        Evaluator {
            rulesets,
            extension_base,
        }
    }

    /// How many rules are loaded.
    pub fn rule_count(&self) -> usize {
        self.rulesets.iter().map(|r| r.rules.len()).sum()
    }

    /// What the extension's rules do to `request`, given the grants recorded for its frame.
    pub fn evaluate(&self, request: &RequestMetadata, frame: &FrameState) -> Verdict {
        self.evaluate_with(request, frame, true)
    }

    /// The same answer as [`evaluate`](Self::evaluate), computed by testing every rule instead of using
    /// the token index. This is the reference the differential tests compare against.
    pub fn evaluate_unindexed(&self, request: &RequestMetadata, frame: &FrameState) -> Verdict {
        self.evaluate_with(request, frame, false)
    }

    fn evaluate_with(
        &self,
        request: &RequestMetadata,
        frame: &FrameState,
        indexed: bool,
    ) -> Verdict {
        if !matches!(
            request.url.scheme(),
            "http" | "https" | "ws" | "wss" | "ftp"
        ) {
            return Verdict::None;
        }
        let ctx = Ctx::new(request);
        let grant_priority = frame.priority();

        let mut terminal: Vec<Hit<'_>> = Vec::new();
        let mut modifying: Vec<Hit<'_>> = Vec::new();
        for (ruleset_order, ruleset) in self.rulesets.iter().enumerate() {
            let candidates: Vec<usize> = if indexed {
                ruleset
                    .index
                    .candidates(ctx.target.text())
                    .into_iter()
                    .filter_map(|p| usize::try_from(p).ok())
                    .collect()
            } else {
                (0..ruleset.rules.len()).collect()
            };
            for position in candidates {
                let Some(rule) = ruleset.rules.get(position) else {
                    continue;
                };
                if rule.priority <= grant_priority || !condition_matches(&rule.condition, &ctx) {
                    continue;
                }
                let hit = Hit {
                    ruleset_order,
                    ruleset: &ruleset.id,
                    rule,
                };
                if matches!(rule.action, Action::ModifyHeaders(_)) {
                    modifying.push(hit);
                } else {
                    terminal.push(hit);
                }
            }
        }
        terminal.sort_by(|a, b| {
            b.rule
                .priority
                .cmp(&a.rule.priority)
                .then(b.rule.action.rank().cmp(&a.rule.action.rank()))
                .then(a.ruleset_order.cmp(&b.ruleset_order))
                .then(a.rule.id.cmp(&b.rule.id))
        });
        modifying.sort_by(|a, b| {
            b.rule
                .priority
                .cmp(&a.rule.priority)
                .then(a.ruleset_order.cmp(&b.ruleset_order))
                .then(a.rule.id.cmp(&b.rule.id))
        });

        let mut allowed: Option<(Verdict, u32)> = None;
        for hit in &terminal {
            match &hit.rule.action {
                Action::Allow => {
                    allowed = Some((
                        Verdict::Allow {
                            rule: hit.rule_ref(),
                        },
                        hit.rule.priority,
                    ));
                    break;
                }
                Action::AllowAllRequests => {
                    let frame_kind = match request.resource_type {
                        ResourceType::MainFrame => FrameKind::MainFrame,
                        ResourceType::SubFrame => FrameKind::SubFrame,
                        _ => continue,
                    };
                    allowed = Some((
                        Verdict::AllowAllRequests {
                            rule: hit.rule_ref(),
                            scope: AllowScope {
                                priority: hit.rule.priority,
                                frame: frame_kind,
                            },
                        },
                        hit.rule.priority,
                    ));
                    break;
                }
                Action::Block => {
                    return Verdict::Block {
                        rule: hit.rule_ref(),
                    };
                }
                Action::UpgradeScheme => {
                    if let Some(rewrite) = upgrade_rewrite(&request.url) {
                        return Verdict::Upgrade {
                            rule: hit.rule_ref(),
                            rewrite,
                        };
                    }
                }
                Action::Redirect(redirect) => {
                    if let Some(rewrite) = self.redirect_rewrite(hit.rule, redirect, &ctx) {
                        return Verdict::Redirect {
                            rule: hit.rule_ref(),
                            rewrite,
                        };
                    }
                }
                Action::ModifyHeaders(_) => {}
            }
        }

        let threshold = allowed
            .as_ref()
            .map_or(grant_priority, |(_, p)| (*p).max(grant_priority));
        let mut request_items: Vec<(u64, RuleRef, HeaderOp)> = Vec::new();
        let mut response_items: Vec<(u64, RuleRef, HeaderOp)> = Vec::new();
        for hit in modifying.iter().filter(|h| h.rule.priority > threshold) {
            if let Action::ModifyHeaders(ops) = &hit.rule.action {
                let rule_ref = hit.rule_ref();
                request_items.extend(
                    ops.request
                        .iter()
                        .map(|op| (0, rule_ref.clone(), op.clone())),
                );
                response_items.extend(
                    ops.response
                        .iter()
                        .map(|op| (0, rule_ref.clone(), op.clone())),
                );
            }
        }
        let request_accepted = resolve_header_ops(request_items);
        let response_accepted = resolve_header_ops(response_items);
        if !request_accepted.is_empty() || !response_accepted.is_empty() {
            let mut rules: Vec<RuleRef> = Vec::new();
            for (rule_ref, _) in request_accepted.iter().chain(response_accepted.iter()) {
                if !rules.contains(rule_ref) {
                    rules.push(rule_ref.clone());
                }
            }
            return Verdict::ModifyHeaders {
                rules,
                request_ops: request_accepted.into_iter().map(|(_, op)| op).collect(),
                response_ops: response_accepted.into_iter().map(|(_, op)| op).collect(),
            };
        }
        if let Some((verdict, _)) = allowed {
            return verdict;
        }
        match frame.grant() {
            Some(grant) => Verdict::Allow {
                rule: grant.rule.clone(),
            },
            None => Verdict::None,
        }
    }

    fn scheme_allowed(&self, url: &Url) -> bool {
        match url.scheme() {
            "http" | "https" => true,
            scheme => self
                .extension_base
                .as_ref()
                .is_some_and(|base| base.scheme() == scheme && base.host_str() == url.host_str()),
        }
    }

    fn redirect_rewrite(&self, rule: &Rule, redirect: &Redirect, ctx: &Ctx<'_>) -> Option<Rewrite> {
        let request_url = &ctx.req.url;
        let new_url = match redirect {
            Redirect::Url(url) => url.clone(),
            Redirect::ExtensionPath(path) => {
                let base = self.extension_base.as_ref()?;
                let joined = base.join(path).ok()?;
                if joined.scheme() != base.scheme() || joined.host_str() != base.host_str() {
                    return None;
                }
                joined
            }
            Redirect::Transform(transform) => apply_transform(request_url, transform)?,
            Redirect::RegexSubstitution(substitution) => {
                let Some(UrlPattern::Regex(filter)) = &rule.condition.pattern else {
                    return None;
                };
                let text = ctx.target.text();
                let captures = filter.regex().captures(text)?;
                let whole = captures.get(0)?;
                let replaced = format!(
                    "{}{}{}",
                    text.get(..whole.start())?,
                    substitution.expand(&captures),
                    text.get(whole.end()..)?
                );
                Url::parse(&replaced).ok()?
            }
        };
        if !self.scheme_allowed(&new_url) || new_url == *request_url {
            return None;
        }
        Some(Rewrite {
            url: Some(new_url),
            ..Rewrite::default()
        })
    }
}

fn upgrade_rewrite(url: &Url) -> Option<Rewrite> {
    let target = match url.scheme() {
        "http" | "ftp" => "https",
        "ws" => "wss",
        _ => return None,
    };
    let mut upgraded = url.clone();
    upgraded.set_scheme(target).ok()?;
    Some(Rewrite {
        url: Some(upgraded),
        ..Rewrite::default()
    })
}

// --- conditions -----------------------------------------------------------------------------------

fn any_domain_matches(domains: &[String], host: &str) -> bool {
    domains.iter().any(|d| host_matches_domain(host, d))
}

fn condition_matches(c: &Condition, ctx: &Ctx<'_>) -> bool {
    if !c.resource_types.contains(ctx.req.resource_type) {
        return false;
    }
    if !c.request_methods.contains(ctx.method) {
        return false;
    }
    match c.domain_type {
        Some(DomainType::FirstParty) if ctx.third_party => return false,
        Some(DomainType::ThirdParty) if !ctx.third_party => return false,
        _ => {}
    }
    if !c.tab_ids.is_empty() && !c.tab_ids.iter().any(|t| i64::from(*t) == ctx.tab) {
        return false;
    }
    if c.excluded_tab_ids.iter().any(|t| i64::from(*t) == ctx.tab) {
        return false;
    }
    if !domains_condition(
        &c.initiator_domains,
        &c.excluded_initiator_domains,
        ctx.initiator_host.as_deref(),
    ) {
        return false;
    }
    if !domains_condition(
        &c.request_domains,
        &c.excluded_request_domains,
        Some(ctx.host.as_str()),
    ) {
        return false;
    }
    if !domains_condition(
        &c.top_domains,
        &c.excluded_top_domains,
        ctx.top_host.as_deref(),
    ) {
        return false;
    }
    match &c.pattern {
        Some(pattern) => pattern.matches(&ctx.target),
        None => true,
    }
}

/// An include list and an exclude list against a host. With no host (no initiator, opaque origin) an
/// include list cannot match and an exclude list excludes nothing.
fn domains_condition(include: &[String], exclude: &[String], host: Option<&str>) -> bool {
    match host {
        None => include.is_empty(),
        Some(host) => {
            (include.is_empty() || any_domain_matches(include, host))
                && !any_domain_matches(exclude, host)
        }
    }
}

// --- URL transform --------------------------------------------------------------------------------

fn apply_transform(url: &Url, t: &UrlTransform) -> Option<Url> {
    let mut u = url.clone();
    if let Some(scheme) = &t.scheme {
        u.set_scheme(scheme).ok()?;
    }
    if let Some(host) = &t.host {
        u.set_host(Some(host)).ok()?;
    }
    if let Some(port) = &t.port {
        if port.is_empty() {
            u.set_port(None).ok()?;
        } else {
            u.set_port(Some(port.parse::<u16>().ok()?)).ok()?;
        }
    }
    if let Some(path) = &t.path {
        u.set_path(path);
    }
    match &t.query {
        Some(QueryChange::Replace(query)) => {
            let query = query.strip_prefix('?').unwrap_or(query);
            u.set_query((!query.is_empty()).then_some(query));
        }
        Some(QueryChange::Transform(qt)) => apply_query_transform(&mut u, qt),
        None => {}
    }
    if let Some(fragment) = &t.fragment {
        let fragment = fragment.strip_prefix('#').unwrap_or(fragment);
        u.set_fragment((!fragment.is_empty()).then_some(fragment));
    }
    if let Some(name) = &t.username {
        u.set_username(name).ok()?;
    }
    if let Some(password) = &t.password {
        u.set_password((!password.is_empty()).then_some(password.as_str()))
            .ok()?;
    }
    Some(u)
}

/// Removes keys, then replaces the first occurrence of each key to add (and drops its other
/// occurrences), appending keys that are missing unless `replaceOnly`. Untouched parameters keep their
/// exact bytes. Behaviour per the documentation of `QueryTransform` and `QueryKeyValue`.
fn apply_query_transform(url: &mut Url, qt: &QueryTransform) {
    let existing = url.query().unwrap_or("").to_owned();
    let mut pairs: Vec<String> = if existing.is_empty() {
        Vec::new()
    } else {
        existing.split('&').map(str::to_owned).collect()
    };
    for key in &qt.remove_params {
        pairs.retain(|pair| query_key(pair) != *key);
    }
    for param in &qt.add_or_replace_params {
        let encoded = format!(
            "{}={}",
            percent_encode(&param.key),
            percent_encode(&param.value)
        );
        let mut found = false;
        let mut next = Vec::with_capacity(pairs.len() + 1);
        for pair in pairs {
            if query_key(&pair) == param.key {
                if !found {
                    found = true;
                    next.push(encoded.clone());
                }
            } else {
                next.push(pair);
            }
        }
        if !found && !param.replace_only {
            next.push(encoded);
        }
        pairs = next;
    }
    let joined = pairs.join("&");
    url.set_query((!joined.is_empty()).then_some(joined.as_str()));
}

fn query_key(pair: &str) -> String {
    let key = pair.split_once('=').map_or(pair, |(k, _)| k);
    percent_decode(key)
}

fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0usize;
    while let Some(&b) = bytes.get(i) {
        let hex = |offset: usize| {
            bytes
                .get(i + offset)
                .and_then(|h| char::from(*h).to_digit(16))
                .and_then(|d| u8::try_from(d).ok())
        };
        if b == b'%'
            && let (Some(hi), Some(lo)) = (hex(1), hex(2))
        {
            out.push(hi * 16 + lo);
            i += 3;
            continue;
        }
        out.push(if b == b'+' { b' ' } else { b });
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn percent_encode(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for &b in text.as_bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~') {
            out.push(char::from(b));
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}
