// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! The rule model and its JSON form (Chrome's `declarativeNetRequest.Rule`): static, dynamic and session
//! rulesets.
//!
//! Behaviour per <https://developer.chrome.com/docs/extensions/reference/api/declarativeNetRequest> and
//! the validation rules of Chromium's `indexed_rule.cc` (BSD-3-Clause; read, not copied).
//!
//! # Static, dynamic and session rulesets
//!
//! [`parse_ruleset`] turns the JSON of a ruleset into a [`ParsedRuleset`]. An invalid **static** rule is
//! skipped and reported in [`ParsedRuleset::warnings`] (Chrome ignores invalid static rules); an invalid
//! **dynamic or session** rule rejects the whole update. Unknown JSON fields are ignored; an unknown value
//! of a known enum makes the rule invalid. Conditions Mercury cannot evaluate (`responseHeaders`,
//! `excludedResponseHeaders`, and `regexFilter`/`regexSubstitution`/`regexOptions` inside a header
//! operation) make the rule invalid instead of being ignored, because ignoring them would widen the rule.
//!
//! # Where Mercury differs from Chrome
//!
//! * More than one of `url`, `extensionPath`, `transform` and `regexSubstitution` in one `redirect` is an
//!   error (Chrome silently takes the first).
//! * A `redirect.url` or `transform.scheme` is accepted for the schemes Chrome accepts, but the evaluator
//!   only ever emits `http`, `https` or the extension's own scheme; a rule whose static target is none of
//!   those is kept with a warning and never fires.
//! * Domains are lowercased, not rejected, when they have capitals (Chrome lowercases); non-ASCII domains,
//!   schemes, ports and paths in a domain are rejected.
//! * Header values with control characters other than a tab are rejected.
//! * `allowAllRequests` is counted as a *safe* rule, as in Chrome's documentation.

use std::collections::HashSet;
use std::io;

use serde_json::{Map, Value};
use thiserror::Error;
use url::Url;

use crate::limits::{CountedItem, LimitError, Limits, RulesetStats, check_count};
use crate::request::{MethodSet, RequestMethod, ResourceType, ResourceTypeSet};
use crate::url_filter::{FilterError, RegexFilter, UrlFilter, UrlPattern};

/// Which kind of ruleset a JSON document is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum RulesetKind {
    /// Shipped in the extension package and listed in the manifest.
    Static,
    /// Persisted by the extension at run time.
    Dynamic,
    /// Kept in memory until the browser quits; the only kind that may use `tabIds`.
    Session,
}

/// Whether a request is first or third party to its initiator.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DomainType {
    /// Same site as the initiator.
    FirstParty,
    /// Different site from the initiator (or unknown).
    ThirdParty,
}

/// What a rule matches. Empty lists mean "not specified" (an empty list in JSON is invalid).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Condition {
    /// The `urlFilter` or `regexFilter`, if any.
    pub pattern: Option<UrlPattern>,
    /// `initiatorDomains` (or the deprecated `domains`), lowercased.
    pub initiator_domains: Vec<String>,
    /// `excludedInitiatorDomains` (or `excludedDomains`).
    pub excluded_initiator_domains: Vec<String>,
    /// `requestDomains`.
    pub request_domains: Vec<String>,
    /// `excludedRequestDomains`.
    pub excluded_request_domains: Vec<String>,
    /// `topDomains`, matched against the top-level site (or the initiator when there is none).
    pub top_domains: Vec<String>,
    /// `excludedTopDomains`.
    pub excluded_top_domains: Vec<String>,
    /// The resource types the rule applies to, with the default and the exclusions already applied.
    pub resource_types: ResourceTypeSet,
    /// The request methods the rule applies to, with the exclusions already applied.
    pub request_methods: MethodSet,
    /// `domainType`.
    pub domain_type: Option<DomainType>,
    /// `tabIds` (session rules only); `-1` is "no tab".
    pub tab_ids: Vec<i32>,
    /// `excludedTabIds` (session rules only).
    pub excluded_tab_ids: Vec<i32>,
}

/// The operation of a header modification.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HeaderOperation {
    /// Add a value to the header.
    Append,
    /// Replace the header.
    Set,
    /// Delete the header.
    Remove,
}

/// One header modification of a `modifyHeaders` rule.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HeaderOp {
    /// The header name as written in the rule.
    pub header: String,
    /// What to do.
    pub operation: HeaderOperation,
    /// The value, for `append` and `set`.
    pub value: Option<String>,
}

/// The header modifications of a `modifyHeaders` rule.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ModifyHeaders {
    /// Changes to the request headers.
    pub request: Vec<HeaderOp>,
    /// Changes to the response headers.
    pub response: Vec<HeaderOp>,
}

/// One `addOrReplaceParams` entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueryParam {
    /// The parameter name.
    pub key: String,
    /// The value.
    pub value: String,
    /// Replace only when the key is already present.
    pub replace_only: bool,
}

/// `queryTransform`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct QueryTransform {
    /// Keys to remove (done first).
    pub remove_params: Vec<String>,
    /// Keys to add or replace.
    pub add_or_replace_params: Vec<QueryParam>,
}

/// `transform.query` or `transform.queryTransform` (exclusive).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QueryChange {
    /// A new query string (`?a=b`, or empty to clear).
    Replace(String),
    /// Edits to the existing query.
    Transform(QueryTransform),
}

/// `redirect.transform`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct UrlTransform {
    /// New scheme.
    pub scheme: Option<String>,
    /// New host.
    pub host: Option<String>,
    /// New port (empty clears it).
    pub port: Option<String>,
    /// New path.
    pub path: Option<String>,
    /// Query change.
    pub query: Option<QueryChange>,
    /// New fragment (`#x`, or empty to clear).
    pub fragment: Option<String>,
    /// New username (empty clears it).
    pub username: Option<String>,
    /// New password (empty clears it).
    pub password: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum SubPart {
    Text(String),
    Group(usize),
}

/// A parsed `regexSubstitution`: text with `\0` to `\9` capture references.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Substitution {
    parts: Vec<SubPart>,
}

impl Substitution {
    fn parse(text: &str) -> Result<Substitution, &'static str> {
        if text.is_empty() {
            return Err("is empty");
        }
        let mut parts = Vec::new();
        let mut current = String::new();
        let mut chars = text.chars();
        while let Some(c) = chars.next() {
            if c != '\\' {
                current.push(c);
                continue;
            }
            match chars.next() {
                Some('\\') => current.push('\\'),
                Some(d) if d.is_ascii_digit() => {
                    if !current.is_empty() {
                        parts.push(SubPart::Text(std::mem::take(&mut current)));
                    }
                    let group = d.to_digit(10).and_then(|v| usize::try_from(v).ok());
                    parts.push(SubPart::Group(group.unwrap_or(0)));
                }
                _ => return Err("has an escape that is not \\0 to \\9 or \\\\"),
            }
        }
        if !current.is_empty() {
            parts.push(SubPart::Text(current));
        }
        Ok(Substitution { parts })
    }

    /// The highest capture group referenced (0 when only `\0` or none).
    pub fn max_group(&self) -> usize {
        self.parts
            .iter()
            .filter_map(|p| match p {
                SubPart::Group(g) => Some(*g),
                SubPart::Text(_) => None,
            })
            .max()
            .unwrap_or(0)
    }

    /// The text with the groups of `captures` inserted (an unmatched group is empty).
    pub fn expand(&self, captures: &regex::Captures<'_>) -> String {
        let mut out = String::new();
        for part in &self.parts {
            match part {
                SubPart::Text(t) => out.push_str(t),
                SubPart::Group(g) => {
                    if let Some(m) = captures.get(*g) {
                        out.push_str(m.as_str());
                    }
                }
            }
        }
        out
    }
}

/// `action.redirect`: exactly one of the four forms.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Redirect {
    /// A path inside the extension, starting with `/`.
    ExtensionPath(String),
    /// An absolute URL.
    Url(Url),
    /// Edits to the request URL.
    Transform(UrlTransform),
    /// A substitution applied to the first match of the rule's `regexFilter`.
    RegexSubstitution(Substitution),
}

/// What a rule does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// Cancel the request.
    Block,
    /// Let the request through and mask lower-priority rules.
    Allow,
    /// Like `Allow`, for the request's whole frame tree (document-scoped).
    AllowAllRequests,
    /// Change `http` to `https` (and `ws` to `wss`).
    UpgradeScheme,
    /// Send the request elsewhere.
    Redirect(Redirect),
    /// Change headers.
    ModifyHeaders(ModifyHeaders),
}

impl Action {
    /// The tie-break order of actions at one priority (Chromium's `GetActionTypePriority`): `allow` 5,
    /// `allowAllRequests` 4, `block` 3, `upgradeScheme` 2, `redirect` 1, `modifyHeaders` 0.
    pub fn rank(&self) -> u8 {
        match self {
            Action::Allow => 5,
            Action::AllowAllRequests => 4,
            Action::Block => 3,
            Action::UpgradeScheme => 2,
            Action::Redirect(_) => 1,
            Action::ModifyHeaders(_) => 0,
        }
    }

    /// Whether the action is "unsafe" for the dynamic-rule quota: `redirect` and `modifyHeaders`.
    pub fn is_unsafe(&self) -> bool {
        matches!(self, Action::Redirect(_) | Action::ModifyHeaders(_))
    }
}

/// One declarativeNetRequest rule.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rule {
    /// The id, at least 1, unique in its ruleset.
    pub id: u32,
    /// The priority, at least 1; default 1.
    pub priority: u32,
    /// What the rule matches.
    pub condition: Condition,
    /// What the rule does.
    pub action: Action,
}

impl Rule {
    /// Whether the rule has a `regexFilter`.
    pub fn is_regex(&self) -> bool {
        matches!(self.condition.pattern, Some(UrlPattern::Regex(_)))
    }
}

/// A rule that was skipped (static rulesets) or accepted with a remark.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuleWarning {
    /// The position of the rule in the JSON array.
    pub index: usize,
    /// The rule id, when it could be read.
    pub rule_id: Option<u32>,
    /// What is wrong; never contains rule text.
    pub message: String,
}

/// A parsed ruleset.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ParsedRuleset {
    /// The valid rules, in file order.
    pub rules: Vec<Rule>,
    /// Rules skipped or remarks, in file order.
    pub warnings: Vec<RuleWarning>,
}

impl ParsedRuleset {
    /// Counts for the budget checks.
    pub fn stats(&self) -> RulesetStats {
        RulesetStats {
            rules: self.rules.len(),
            unsafe_rules: self.rules.iter().filter(|r| r.action.is_unsafe()).count(),
            regex_rules: self.rules.iter().filter(|r| r.is_regex()).count(),
        }
    }
}

/// Why one rule is invalid. No variant carries rule text.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum RuleError {
    /// The rule (or a part that must be an object) is not a JSON object.
    #[error("{field} must be an object")]
    NotAnObject {
        /// The field.
        field: &'static str,
    },
    /// A required field is missing.
    #[error("{field} is required")]
    Missing {
        /// The field.
        field: &'static str,
    },
    /// A field has the wrong JSON type.
    #[error("{field} must be {expected}")]
    WrongType {
        /// The field.
        field: &'static str,
        /// The type wanted.
        expected: &'static str,
    },
    /// A number is out of range.
    #[error("{field} is out of range")]
    OutOfRange {
        /// The field.
        field: &'static str,
    },
    /// A value is not one of the known values.
    #[error("{field} has an unknown value")]
    UnknownValue {
        /// The field.
        field: &'static str,
    },
    /// A list is empty.
    #[error("{field} must not be an empty list")]
    Empty {
        /// The field.
        field: &'static str,
    },
    /// Two fields exclude each other.
    #[error("{a} and {b} cannot both be specified")]
    Conflict {
        /// One field.
        a: &'static str,
        /// The other.
        b: &'static str,
    },
    /// The field is known but Mercury cannot evaluate it.
    #[error("{field} is not supported")]
    Unsupported {
        /// The field.
        field: &'static str,
    },
    /// A value is not valid.
    #[error("{field} {reason}")]
    Invalid {
        /// The field.
        field: &'static str,
        /// What is wrong.
        reason: &'static str,
    },
    /// A limit was exceeded.
    #[error(transparent)]
    Limit(#[from] LimitError),
}

/// What kind of JSON error, without its message (which can quote input).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JsonErrorKind {
    /// Not valid JSON.
    Syntax,
    /// Valid JSON of the wrong shape.
    Data,
    /// The input ended early.
    Eof,
    /// Anything else.
    Other,
}

impl std::fmt::Display for JsonErrorKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            JsonErrorKind::Syntax => "syntax error",
            JsonErrorKind::Data => "unexpected data",
            JsonErrorKind::Eof => "unexpected end",
            JsonErrorKind::Other => "error",
        })
    }
}

/// Why a ruleset was refused.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum RulesetError {
    /// A limit was exceeded.
    #[error(transparent)]
    Limit(#[from] LimitError),
    /// The input is not valid JSON.
    #[error("the ruleset is not valid JSON ({kind} at line {line}, column {column})")]
    Json {
        /// The kind of error.
        kind: JsonErrorKind,
        /// The line.
        line: usize,
        /// The column.
        column: usize,
    },
    /// The top level is not an array.
    #[error("a ruleset must be a JSON array of rules")]
    NotAnArray,
    /// A rule is invalid (dynamic and session rulesets only).
    #[error("rule #{index} (id {id:?}) is invalid: {error}")]
    InvalidRule {
        /// The position in the array.
        index: usize,
        /// The rule id, when it could be read.
        id: Option<u32>,
        /// What is wrong.
        error: RuleError,
    },
    /// Two rules share an id (dynamic and session rulesets only; a static duplicate is skipped).
    #[error("rule id {id} is used more than once")]
    DuplicateId {
        /// The id.
        id: u32,
    },
}

type Obj = Map<String, Value>;

/// Parses the JSON of a ruleset. See the module docs for what is skipped and what is rejected.
pub fn parse_ruleset(
    json: &[u8],
    kind: RulesetKind,
    limits: &Limits,
) -> Result<ParsedRuleset, RulesetError> {
    if json.len() > limits.max_ruleset_bytes {
        return Err(LimitError::RulesetTooLarge {
            limit: limits.max_ruleset_bytes,
            found: json.len(),
        }
        .into());
    }
    if json_depth_exceeds(json, limits.max_json_depth) {
        return Err(LimitError::NestingTooDeep {
            limit: limits.max_json_depth,
        }
        .into());
    }
    let value: Value = serde_json::from_slice(json).map_err(|e| RulesetError::Json {
        kind: match e.classify() {
            serde_json::error::Category::Syntax => JsonErrorKind::Syntax,
            serde_json::error::Category::Data => JsonErrorKind::Data,
            serde_json::error::Category::Eof => JsonErrorKind::Eof,
            serde_json::error::Category::Io => JsonErrorKind::Other,
        },
        line: e.line(),
        column: e.column(),
    })?;
    let Value::Array(items) = value else {
        return Err(RulesetError::NotAnArray);
    };
    let (count_item, count_limit, regex_item) = match kind {
        RulesetKind::Static => (
            CountedItem::StaticRules,
            limits.global_static_rules,
            CountedItem::StaticRegexRules,
        ),
        RulesetKind::Dynamic => (
            CountedItem::DynamicRules,
            limits.max_dynamic_rules,
            CountedItem::DynamicRegexRules,
        ),
        RulesetKind::Session => (
            CountedItem::SessionRules,
            limits.max_session_rules,
            CountedItem::SessionRegexRules,
        ),
    };
    check_count(count_item, count_limit, items.len())?;

    let mut out = ParsedRuleset::default();
    let mut seen_ids: HashSet<u32> = HashSet::new();
    let mut regex_rules = 0usize;
    let mut unsafe_rules = 0usize;
    for (index, item) in items.iter().enumerate() {
        let id_hint = peek_id(item);
        let regex_exhausted = (regex_rules >= limits.max_regex_rules).then_some(regex_item);
        let parsed = serialised_len(item)
            .and_then(|len| {
                if len > limits.max_rule_bytes {
                    Err(RuleError::Limit(LimitError::RuleTooLarge {
                        limit: limits.max_rule_bytes,
                        found: len,
                    }))
                } else {
                    Ok(())
                }
            })
            .and_then(|()| parse_rule(item, kind, limits, regex_exhausted));
        match parsed {
            Ok((rule, remark)) => {
                if !seen_ids.insert(rule.id) {
                    if kind == RulesetKind::Static {
                        out.warnings.push(RuleWarning {
                            index,
                            rule_id: Some(rule.id),
                            message: "rule ignored: its id is already used by an earlier rule"
                                .to_owned(),
                        });
                        continue;
                    }
                    return Err(RulesetError::DuplicateId { id: rule.id });
                }
                if let Some(message) = remark {
                    out.warnings.push(RuleWarning {
                        index,
                        rule_id: Some(rule.id),
                        message: message.to_owned(),
                    });
                }
                regex_rules += usize::from(rule.is_regex());
                unsafe_rules += usize::from(rule.action.is_unsafe());
                out.rules.push(rule);
            }
            Err(error) => match (kind, error) {
                (RulesetKind::Static, error) => out.warnings.push(RuleWarning {
                    index,
                    rule_id: id_hint,
                    message: format!("rule ignored: {error}"),
                }),
                (_, RuleError::Limit(limit)) => return Err(RulesetError::Limit(limit)),
                (_, error) => {
                    return Err(RulesetError::InvalidRule {
                        index,
                        id: id_hint,
                        error,
                    });
                }
            },
        }
    }
    if kind == RulesetKind::Dynamic {
        check_count(
            CountedItem::UnsafeDynamicRules,
            limits.max_unsafe_dynamic_rules,
            unsafe_rules,
        )?;
    }
    Ok(out)
}

fn json_depth_exceeds(bytes: &[u8], limit: usize) -> bool {
    let mut depth = 0usize;
    let mut in_string = false;
    let mut escaped = false;
    for &b in bytes {
        if in_string {
            if escaped {
                escaped = false;
            } else if b == b'\\' {
                escaped = true;
            } else if b == b'"' {
                in_string = false;
            }
            continue;
        }
        match b {
            b'"' => in_string = true,
            b'[' | b'{' => {
                depth += 1;
                if depth > limit {
                    return true;
                }
            }
            b']' | b'}' => depth = depth.saturating_sub(1),
            _ => {}
        }
    }
    false
}

struct Counter(usize);

impl io::Write for Counter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0 = self.0.saturating_add(buf.len());
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn serialised_len(value: &Value) -> Result<usize, RuleError> {
    let mut counter = Counter(0);
    serde_json::to_writer(&mut counter, value).map_err(|_| RuleError::Invalid {
        field: "rule",
        reason: "cannot be serialised",
    })?;
    Ok(counter.0)
}

fn peek_id(value: &Value) -> Option<u32> {
    value
        .get("id")
        .and_then(Value::as_u64)
        .and_then(|id| u32::try_from(id).ok())
}

// --- field readers --------------------------------------------------------------------------------

fn key_of(field: &'static str) -> &'static str {
    field.rsplit('.').next().unwrap_or(field)
}

fn get_obj<'a>(m: &'a Obj, field: &'static str) -> Result<Option<&'a Obj>, RuleError> {
    match m.get(key_of(field)) {
        None => Ok(None),
        Some(Value::Object(o)) => Ok(Some(o)),
        Some(_) => Err(RuleError::NotAnObject { field }),
    }
}

fn get_str<'a>(m: &'a Obj, field: &'static str) -> Result<Option<&'a str>, RuleError> {
    match m.get(key_of(field)) {
        None => Ok(None),
        Some(Value::String(s)) => Ok(Some(s.as_str())),
        Some(_) => Err(RuleError::WrongType {
            field,
            expected: "a string",
        }),
    }
}

fn get_bool(m: &Obj, field: &'static str) -> Result<Option<bool>, RuleError> {
    match m.get(key_of(field)) {
        None => Ok(None),
        Some(Value::Bool(b)) => Ok(Some(*b)),
        Some(_) => Err(RuleError::WrongType {
            field,
            expected: "a boolean",
        }),
    }
}

fn number_as_i64(v: &Value, field: &'static str) -> Result<i64, RuleError> {
    match v {
        Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                Ok(i)
            } else if n.as_u64().is_some() {
                Err(RuleError::OutOfRange { field })
            } else {
                Err(RuleError::WrongType {
                    field,
                    expected: "an integer",
                })
            }
        }
        _ => Err(RuleError::WrongType {
            field,
            expected: "an integer",
        }),
    }
}

fn get_u32(m: &Obj, field: &'static str) -> Result<Option<u32>, RuleError> {
    match m.get(key_of(field)) {
        None => Ok(None),
        Some(v) => {
            let n = number_as_i64(v, field)?;
            u32::try_from(n)
                .map(Some)
                .map_err(|_| RuleError::OutOfRange { field })
        }
    }
}

fn get_list<'a>(
    m: &'a Obj,
    field: &'static str,
    limits: &Limits,
) -> Result<Option<&'a Vec<Value>>, RuleError> {
    match m.get(key_of(field)) {
        None => Ok(None),
        Some(Value::Array(items)) => {
            if items.is_empty() {
                return Err(RuleError::Empty { field });
            }
            if items.len() > limits.max_list_entries {
                return Err(LimitError::ListTooLong {
                    field,
                    limit: limits.max_list_entries,
                    found: items.len(),
                }
                .into());
            }
            Ok(Some(items))
        }
        Some(_) => Err(RuleError::WrongType {
            field,
            expected: "a list",
        }),
    }
}

fn get_str_list<'a>(
    m: &'a Obj,
    field: &'static str,
    limits: &Limits,
) -> Result<Option<Vec<&'a str>>, RuleError> {
    let Some(items) = get_list(m, field, limits)? else {
        return Ok(None);
    };
    items
        .iter()
        .map(|v| {
            v.as_str().ok_or(RuleError::WrongType {
                field,
                expected: "a list of strings",
            })
        })
        .collect::<Result<Vec<_>, _>>()
        .map(Some)
}

// --- validation helpers ---------------------------------------------------------------------------

/// Lowercases a domain and checks it: ASCII, labels of letters, digits, `-` and `_`, or a bracketed
/// IPv6 literal. No scheme, port, path or wildcard.
pub(crate) fn normalise_domain(domain: &str) -> Result<String, &'static str> {
    if domain.is_empty() {
        return Err("has an empty domain");
    }
    if !domain.is_ascii() {
        return Err("has a domain that is not ASCII (use punycode)");
    }
    let lower = domain.to_ascii_lowercase();
    if let Some(inner) = lower
        .strip_prefix('[')
        .and_then(|rest| rest.strip_suffix(']'))
    {
        return if inner.parse::<std::net::Ipv6Addr>().is_ok() {
            Ok(lower)
        } else {
            Err("has an invalid IPv6 domain")
        };
    }
    if lower.len() > 253 {
        return Err("has a domain that is too long");
    }
    let ok = lower.split('.').all(|label| {
        !label.is_empty()
            && label.len() <= 63
            && label
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
    });
    if ok {
        Ok(lower)
    } else {
        Err("has a domain that is not a plain host name")
    }
}

fn is_header_name(name: &str) -> bool {
    !name.is_empty()
        && name.bytes().all(|b| {
            b.is_ascii_alphanumeric()
                || matches!(
                    b,
                    b'!' | b'#'
                        | b'$'
                        | b'%'
                        | b'&'
                        | b'\''
                        | b'*'
                        | b'+'
                        | b'-'
                        | b'.'
                        | b'^'
                        | b'_'
                        | b'`'
                        | b'|'
                        | b'~'
                )
        })
}

fn is_header_value(value: &str) -> bool {
    !value.bytes().any(|b| (b < 0x20 && b != b'\t') || b == 0x7f)
}

/// The request headers an `append` may target in Chrome (lowercase) and the separator used.
const REQUEST_APPEND_ALLOW_LIST: &[(&str, &str)] = &[
    ("accept", ", "),
    ("accept-encoding", ", "),
    ("accept-language", ", "),
    ("access-control-request-headers", ", "),
    ("cache-control", ", "),
    ("connection", ", "),
    ("content-language", ", "),
    ("cookie", "; "),
    ("forwarded", ", "),
    ("if-match", ", "),
    ("if-none-match", ", "),
    ("keep-alive", ", "),
    ("range", ", "),
    ("te", ", "),
    ("trailer", ""),
    ("transfer-encoding", ", "),
    ("upgrade", ", "),
    ("user-agent", " "),
    ("via", ", "),
    ("want-digest", ", "),
    ("x-forwarded-for", ", "),
];

/// The separator Chrome puts between the existing value and an appended one for a request header, or
/// `None` when appending to that header is not allowed (case-insensitive).
pub fn request_header_append_separator(header: &str) -> Option<&'static str> {
    let lower = header.to_ascii_lowercase();
    REQUEST_APPEND_ALLOW_LIST
        .iter()
        .find(|(name, _)| *name == lower)
        .map(|(_, sep)| *sep)
}

// --- the rule parser ------------------------------------------------------------------------------

fn parse_rule(
    value: &Value,
    kind: RulesetKind,
    limits: &Limits,
    regex_exhausted: Option<CountedItem>,
) -> Result<(Rule, Option<&'static str>), RuleError> {
    let Value::Object(obj) = value else {
        return Err(RuleError::NotAnObject { field: "rule" });
    };
    let id = get_u32(obj, "id")?.ok_or(RuleError::Missing { field: "id" })?;
    if id < 1 {
        return Err(RuleError::OutOfRange { field: "id" });
    }
    let priority = get_u32(obj, "priority")?.unwrap_or(1);
    if priority < 1 {
        return Err(RuleError::OutOfRange { field: "priority" });
    }
    let action_obj = get_obj(obj, "action")?.ok_or(RuleError::Missing { field: "action" })?;
    let condition_obj =
        get_obj(obj, "condition")?.ok_or(RuleError::Missing { field: "condition" })?;
    let (condition, explicit_types) =
        parse_condition(condition_obj, kind, limits, regex_exhausted)?;
    let (action, remark) = parse_action(action_obj, limits)?;
    if action == Action::AllowAllRequests {
        let frames: ResourceTypeSet = [ResourceType::MainFrame, ResourceType::SubFrame]
            .into_iter()
            .collect();
        match explicit_types {
            Some(types) if !types.is_empty() && types.is_subset_of(frames) => {}
            _ => {
                return Err(RuleError::Invalid {
                    field: "condition.resourceTypes",
                    reason: "must be given for allowAllRequests and may only list main_frame and sub_frame",
                });
            }
        }
    }
    if let Action::Redirect(Redirect::RegexSubstitution(sub)) = &action {
        match &condition.pattern {
            Some(UrlPattern::Regex(regex)) => {
                if sub.max_group() >= regex.regex().captures_len() {
                    return Err(RuleError::Invalid {
                        field: "action.redirect.regexSubstitution",
                        reason: "refers to a capture group the regexFilter does not have",
                    });
                }
            }
            _ => {
                return Err(RuleError::Invalid {
                    field: "action.redirect.regexSubstitution",
                    reason: "needs a regexFilter",
                });
            }
        }
    }
    Ok((
        Rule {
            id,
            priority,
            condition,
            action,
        },
        remark,
    ))
}

fn filter_error(field: &'static str, e: FilterError, limits: &Limits) -> RuleError {
    match e {
        FilterError::Empty => RuleError::Invalid {
            field,
            reason: "must not be empty",
        },
        FilterError::TooLong { found, .. } => RuleError::Limit(LimitError::FilterTooLong {
            field,
            limit: limits.max_filter_bytes,
            found,
        }),
        FilterError::NonAscii => RuleError::Invalid {
            field,
            reason: "must be ASCII",
        },
        FilterError::ControlCharacter => RuleError::Invalid {
            field,
            reason: "must not contain control characters",
        },
        FilterError::DomainAnchorEmpty => RuleError::Invalid {
            field,
            reason: "has a domain anchor with nothing after it",
        },
        FilterError::DomainAnchorWildcard => RuleError::Invalid {
            field,
            reason: "must not start with ||*",
        },
        FilterError::RegexTooLarge => RuleError::Invalid {
            field,
            reason: "is a more complex regex than allowed",
        },
        FilterError::RegexUnsupported => RuleError::Invalid {
            field,
            reason: "is not a supported regular expression",
        },
    }
}

fn parse_domains(
    m: &Obj,
    field: &'static str,
    limits: &Limits,
) -> Result<Option<Vec<String>>, RuleError> {
    let Some(list) = get_str_list(m, field, limits)? else {
        return Ok(None);
    };
    list.into_iter()
        .map(|d| normalise_domain(d).map_err(|reason| RuleError::Invalid { field, reason }))
        .collect::<Result<Vec<_>, _>>()
        .map(Some)
}

fn parse_types(
    m: &Obj,
    field: &'static str,
    limits: &Limits,
) -> Result<Option<ResourceTypeSet>, RuleError> {
    let Some(list) = get_str_list(m, field, limits)? else {
        return Ok(None);
    };
    let mut set = ResourceTypeSet::empty();
    for name in list {
        set.insert(ResourceType::from_name(name).ok_or(RuleError::UnknownValue { field })?);
    }
    Ok(Some(set))
}

fn parse_methods(
    m: &Obj,
    field: &'static str,
    limits: &Limits,
) -> Result<Option<MethodSet>, RuleError> {
    let Some(list) = get_str_list(m, field, limits)? else {
        return Ok(None);
    };
    let mut set = MethodSet::default();
    for name in list {
        set.insert(RequestMethod::from_name(name).ok_or(RuleError::UnknownValue { field })?);
    }
    Ok(Some(set))
}

fn parse_tab_ids(
    m: &Obj,
    field: &'static str,
    limits: &Limits,
) -> Result<Option<Vec<i32>>, RuleError> {
    let Some(list) = get_list(m, field, limits)? else {
        return Ok(None);
    };
    list.iter()
        .map(|v| {
            let n = number_as_i64(v, field)?;
            i32::try_from(n).map_err(|_| RuleError::OutOfRange { field })
        })
        .collect::<Result<Vec<_>, _>>()
        .map(Some)
}

type ConditionParse = (Condition, Option<ResourceTypeSet>);

fn parse_condition(
    m: &Obj,
    kind: RulesetKind,
    limits: &Limits,
    regex_exhausted: Option<CountedItem>,
) -> Result<ConditionParse, RuleError> {
    for field in ["responseHeaders", "excludedResponseHeaders"] {
        if m.contains_key(field) {
            return Err(RuleError::Unsupported {
                field: if field == "responseHeaders" {
                    "condition.responseHeaders"
                } else {
                    "condition.excludedResponseHeaders"
                },
            });
        }
    }

    let case_sensitive = get_bool(m, "condition.isUrlFilterCaseSensitive")?.unwrap_or(false);
    let url_filter = get_str(m, "condition.urlFilter")?;
    let regex_filter = get_str(m, "condition.regexFilter")?;
    let pattern = match (url_filter, regex_filter) {
        (Some(_), Some(_)) => {
            return Err(RuleError::Conflict {
                a: "condition.urlFilter",
                b: "condition.regexFilter",
            });
        }
        (Some(f), None) => Some(UrlPattern::Filter(
            UrlFilter::parse(f, case_sensitive, limits.max_filter_bytes)
                .map_err(|e| filter_error("condition.urlFilter", e, limits))?,
        )),
        (None, Some(r)) => {
            if let Some(item) = regex_exhausted {
                return Err(RuleError::Limit(LimitError::TooManyRules {
                    item,
                    limit: limits.max_regex_rules,
                    found: limits.max_regex_rules.saturating_add(1),
                }));
            }
            Some(UrlPattern::Regex(
                RegexFilter::compile(r, case_sensitive, limits)
                    .map_err(|e| filter_error("condition.regexFilter", e, limits))?,
            ))
        }
        (None, None) => None,
    };

    let domains = parse_domains(m, "condition.domains", limits)?;
    let initiator = parse_domains(m, "condition.initiatorDomains", limits)?;
    if domains.is_some() && initiator.is_some() {
        return Err(RuleError::Conflict {
            a: "condition.domains",
            b: "condition.initiatorDomains",
        });
    }
    let excluded_domains = parse_domains(m, "condition.excludedDomains", limits)?;
    let excluded_initiator = parse_domains(m, "condition.excludedInitiatorDomains", limits)?;
    if excluded_domains.is_some() && excluded_initiator.is_some() {
        return Err(RuleError::Conflict {
            a: "condition.excludedDomains",
            b: "condition.excludedInitiatorDomains",
        });
    }
    let request_domains = parse_domains(m, "condition.requestDomains", limits)?;
    let excluded_request = parse_domains(m, "condition.excludedRequestDomains", limits)?;
    let top_domains = parse_domains(m, "condition.topDomains", limits)?;
    let excluded_top = parse_domains(m, "condition.excludedTopDomains", limits)?;

    let included_types = parse_types(m, "condition.resourceTypes", limits)?;
    let explicit_types = included_types;
    let excluded_types = parse_types(m, "condition.excludedResourceTypes", limits)?;
    let resource_types = match (included_types, excluded_types) {
        (Some(inc), Some(exc)) => {
            if inc.intersects(exc) {
                return Err(RuleError::Invalid {
                    field: "condition.resourceTypes",
                    reason: "includes a type that excludedResourceTypes excludes",
                });
            }
            inc
        }
        (Some(inc), None) => inc,
        (None, Some(exc)) => {
            let rest = ResourceTypeSet::all().minus(exc);
            if rest.is_empty() {
                return Err(RuleError::Invalid {
                    field: "condition.excludedResourceTypes",
                    reason: "excludes every resource type",
                });
            }
            rest
        }
        (None, None) => ResourceTypeSet::rule_default(),
    };

    let included_methods = parse_methods(m, "condition.requestMethods", limits)?;
    let excluded_methods = parse_methods(m, "condition.excludedRequestMethods", limits)?;
    let request_methods = match (included_methods, excluded_methods) {
        (Some(inc), Some(exc)) => {
            if inc.intersects(exc) {
                return Err(RuleError::Invalid {
                    field: "condition.requestMethods",
                    reason: "includes a method that excludedRequestMethods excludes",
                });
            }
            inc
        }
        (Some(inc), None) => inc,
        (None, Some(exc)) => {
            let rest = MethodSet::all().minus(exc);
            if rest.is_empty() {
                return Err(RuleError::Invalid {
                    field: "condition.excludedRequestMethods",
                    reason: "excludes every method",
                });
            }
            rest
        }
        (None, None) => MethodSet::all(),
    };

    let domain_type = match get_str(m, "condition.domainType")? {
        None => None,
        Some("firstParty") => Some(DomainType::FirstParty),
        Some("thirdParty") => Some(DomainType::ThirdParty),
        Some(_) => {
            return Err(RuleError::UnknownValue {
                field: "condition.domainType",
            });
        }
    };

    let tab_ids = parse_tab_ids(m, "condition.tabIds", limits)?;
    let excluded_tab_ids = parse_tab_ids(m, "condition.excludedTabIds", limits)?;
    if kind != RulesetKind::Session && (tab_ids.is_some() || excluded_tab_ids.is_some()) {
        return Err(RuleError::Invalid {
            field: "condition.tabIds",
            reason: "is only supported for session rules",
        });
    }
    let tab_ids = tab_ids.unwrap_or_default();
    let mut excluded_tab_ids = excluded_tab_ids.unwrap_or_default();
    if tab_ids.iter().any(|t| excluded_tab_ids.contains(t)) {
        return Err(RuleError::Invalid {
            field: "condition.tabIds",
            reason: "includes a tab that excludedTabIds excludes",
        });
    }
    if !tab_ids.is_empty() {
        excluded_tab_ids.clear();
    }

    Ok((
        Condition {
            pattern,
            initiator_domains: domains.or(initiator).unwrap_or_default(),
            excluded_initiator_domains: excluded_domains.or(excluded_initiator).unwrap_or_default(),
            request_domains: request_domains.unwrap_or_default(),
            excluded_request_domains: excluded_request.unwrap_or_default(),
            top_domains: top_domains.unwrap_or_default(),
            excluded_top_domains: excluded_top.unwrap_or_default(),
            resource_types,
            request_methods,
            domain_type,
            tab_ids,
            excluded_tab_ids,
        },
        explicit_types,
    ))
}

type ActionParse = (Action, Option<&'static str>);

fn parse_action(m: &Obj, limits: &Limits) -> Result<ActionParse, RuleError> {
    let kind = get_str(m, "action.type")?.ok_or(RuleError::Missing {
        field: "action.type",
    })?;
    match kind {
        "block" => Ok((Action::Block, None)),
        "allow" => Ok((Action::Allow, None)),
        "allowAllRequests" => Ok((Action::AllowAllRequests, None)),
        "upgradeScheme" => Ok((Action::UpgradeScheme, None)),
        "redirect" => {
            let redirect = get_obj(m, "action.redirect")?.ok_or(RuleError::Missing {
                field: "action.redirect",
            })?;
            parse_redirect(redirect, limits)
        }
        "modifyHeaders" => Ok((
            Action::ModifyHeaders(parse_modify_headers(m, limits)?),
            None,
        )),
        _ => Err(RuleError::UnknownValue {
            field: "action.type",
        }),
    }
}

const REDIRECT_NEVER_FIRES: &str =
    "redirect target scheme is never applied (only http, https and the extension's own scheme are)";

fn parse_redirect(m: &Obj, limits: &Limits) -> Result<ActionParse, RuleError> {
    let url = get_str(m, "action.redirect.url")?;
    let path = get_str(m, "action.redirect.extensionPath")?;
    let transform = get_obj(m, "action.redirect.transform")?;
    let substitution = get_str(m, "action.redirect.regexSubstitution")?;
    let given = [
        url.is_some(),
        path.is_some(),
        transform.is_some(),
        substitution.is_some(),
    ]
    .into_iter()
    .filter(|b| *b)
    .count();
    if given == 0 {
        return Err(RuleError::Invalid {
            field: "action.redirect",
            reason: "needs one of url, extensionPath, transform or regexSubstitution",
        });
    }
    if given > 1 {
        return Err(RuleError::Invalid {
            field: "action.redirect",
            reason: "must have only one of url, extensionPath, transform and regexSubstitution",
        });
    }
    if let Some(url) = url {
        let parsed = Url::parse(url).map_err(|_| RuleError::Invalid {
            field: "action.redirect.url",
            reason: "is not a valid absolute URL",
        })?;
        if parsed.scheme() == "javascript" {
            return Err(RuleError::Invalid {
                field: "action.redirect.url",
                reason: "must not be a javascript: URL",
            });
        }
        let remark = (!matches!(parsed.scheme(), "http" | "https" | "mercury-extension"))
            .then_some(REDIRECT_NEVER_FIRES);
        return Ok((Action::Redirect(Redirect::Url(parsed)), remark));
    }
    if let Some(path) = path {
        let bad = !path.starts_with('/')
            || path.starts_with("//")
            || path.contains('\\')
            || path.bytes().any(|b| b < 0x20 || b == 0x7f)
            || path.len() > limits.max_filter_bytes;
        if bad {
            return Err(RuleError::Invalid {
                field: "action.redirect.extensionPath",
                reason: "must be a path that starts with a single /",
            });
        }
        return Ok((
            Action::Redirect(Redirect::ExtensionPath(path.to_owned())),
            None,
        ));
    }
    if let Some(t) = transform {
        let (transform, remark) = parse_transform(t, limits)?;
        return Ok((Action::Redirect(Redirect::Transform(transform)), remark));
    }
    if let Some(s) = substitution {
        let sub = Substitution::parse(s).map_err(|reason| RuleError::Invalid {
            field: "action.redirect.regexSubstitution",
            reason,
        })?;
        return Ok((Action::Redirect(Redirect::RegexSubstitution(sub)), None));
    }
    Err(RuleError::Invalid {
        field: "action.redirect",
        reason: "needs one of url, extensionPath, transform or regexSubstitution",
    })
}

fn parse_transform(
    m: &Obj,
    limits: &Limits,
) -> Result<(UrlTransform, Option<&'static str>), RuleError> {
    let mut t = UrlTransform::default();
    let mut remark = None;
    if let Some(scheme) = get_str(m, "action.redirect.transform.scheme")? {
        match scheme {
            "http" | "https" | "mercury-extension" => {}
            "ftp" => remark = Some(REDIRECT_NEVER_FIRES),
            _ => {
                return Err(RuleError::Invalid {
                    field: "action.redirect.transform.scheme",
                    reason: "must be http, https, ftp or the extension scheme",
                });
            }
        }
        t.scheme = Some(scheme.to_owned());
    }
    if let Some(host) = get_str(m, "action.redirect.transform.host")? {
        if host.is_empty()
            || host
                .bytes()
                .any(|b| b <= b' ' || matches!(b, b'/' | b'\\' | b'?' | b'#' | b'@') || b == 0x7f)
        {
            return Err(RuleError::Invalid {
                field: "action.redirect.transform.host",
                reason: "is not a host name",
            });
        }
        t.host = Some(host.to_owned());
    }
    if let Some(port) = get_str(m, "action.redirect.transform.port")? {
        let ok = port.is_empty()
            || (port.len() <= 5
                && port.bytes().all(|b| b.is_ascii_digit())
                && port.parse::<u32>().is_ok_and(|p| p <= 65_535));
        if !ok {
            return Err(RuleError::Invalid {
                field: "action.redirect.transform.port",
                reason: "must be empty or a number up to 65535",
            });
        }
        t.port = Some(port.to_owned());
    }
    if let Some(path) = get_str(m, "action.redirect.transform.path")? {
        t.path = Some(path.to_owned());
    }
    let query = get_str(m, "action.redirect.transform.query")?;
    let query_transform = get_obj(m, "action.redirect.transform.queryTransform")?;
    match (query, query_transform) {
        (Some(_), Some(_)) => {
            return Err(RuleError::Conflict {
                a: "action.redirect.transform.query",
                b: "action.redirect.transform.queryTransform",
            });
        }
        (Some(q), None) => {
            if !(q.is_empty() || q.starts_with('?')) {
                return Err(RuleError::Invalid {
                    field: "action.redirect.transform.query",
                    reason: "must be empty or start with ?",
                });
            }
            t.query = Some(QueryChange::Replace(q.to_owned()));
        }
        (None, Some(qt)) => {
            t.query = Some(QueryChange::Transform(parse_query_transform(qt, limits)?))
        }
        (None, None) => {}
    }
    if let Some(fragment) = get_str(m, "action.redirect.transform.fragment")? {
        if !(fragment.is_empty() || fragment.starts_with('#')) {
            return Err(RuleError::Invalid {
                field: "action.redirect.transform.fragment",
                reason: "must be empty or start with #",
            });
        }
        t.fragment = Some(fragment.to_owned());
    }
    if let Some(u) = get_str(m, "action.redirect.transform.username")? {
        t.username = Some(u.to_owned());
    }
    if let Some(p) = get_str(m, "action.redirect.transform.password")? {
        t.password = Some(p.to_owned());
    }
    Ok((t, remark))
}

fn parse_query_transform(m: &Obj, limits: &Limits) -> Result<QueryTransform, RuleError> {
    let mut qt = QueryTransform::default();
    if let Some(remove) = get_str_list(
        m,
        "action.redirect.transform.queryTransform.removeParams",
        limits,
    )? {
        qt.remove_params = remove.into_iter().map(str::to_owned).collect();
    }
    if let Some(items) = get_list(
        m,
        "action.redirect.transform.queryTransform.addOrReplaceParams",
        limits,
    )? {
        for item in items {
            let Value::Object(o) = item else {
                return Err(RuleError::NotAnObject {
                    field: "action.redirect.transform.queryTransform.addOrReplaceParams",
                });
            };
            let key = get_str(
                o,
                "action.redirect.transform.queryTransform.addOrReplaceParams.key",
            )?
            .ok_or(RuleError::Missing {
                field: "action.redirect.transform.queryTransform.addOrReplaceParams.key",
            })?;
            let value = get_str(
                o,
                "action.redirect.transform.queryTransform.addOrReplaceParams.value",
            )?
            .ok_or(RuleError::Missing {
                field: "action.redirect.transform.queryTransform.addOrReplaceParams.value",
            })?;
            let replace_only = get_bool(
                o,
                "action.redirect.transform.queryTransform.addOrReplaceParams.replaceOnly",
            )?
            .unwrap_or(false);
            qt.add_or_replace_params.push(QueryParam {
                key: key.to_owned(),
                value: value.to_owned(),
                replace_only,
            });
        }
    }
    Ok(qt)
}

fn parse_modify_headers(m: &Obj, limits: &Limits) -> Result<ModifyHeaders, RuleError> {
    let request = parse_header_ops(m, "action.requestHeaders", true, limits)?;
    let response = parse_header_ops(m, "action.responseHeaders", false, limits)?;
    if request.is_none() && response.is_none() {
        return Err(RuleError::Invalid {
            field: "action.requestHeaders",
            reason: "or responseHeaders is needed for modifyHeaders",
        });
    }
    let request = request.unwrap_or_default();
    let response = response.unwrap_or_default();
    let total = request.len() + response.len();
    if total > limits.max_header_operations {
        return Err(LimitError::TooManyHeaderOperations {
            limit: limits.max_header_operations,
            found: total,
        }
        .into());
    }
    Ok(ModifyHeaders { request, response })
}

fn parse_header_ops(
    m: &Obj,
    field: &'static str,
    is_request: bool,
    limits: &Limits,
) -> Result<Option<Vec<HeaderOp>>, RuleError> {
    let Some(items) = get_list(m, field, limits)? else {
        return Ok(None);
    };
    if items.len() > limits.max_header_operations {
        return Err(LimitError::TooManyHeaderOperations {
            limit: limits.max_header_operations,
            found: items.len(),
        }
        .into());
    }
    let mut ops = Vec::with_capacity(items.len());
    for item in items {
        let Value::Object(o) = item else {
            return Err(RuleError::NotAnObject { field });
        };
        for unsupported in ["regexFilter", "regexSubstitution", "regexOptions"] {
            if o.contains_key(unsupported) {
                return Err(RuleError::Unsupported {
                    field: "action.requestHeaders.regexFilter",
                });
            }
        }
        let header = get_str(o, "action.headers.header")?.ok_or(RuleError::Missing {
            field: "action.headers.header",
        })?;
        if !is_header_name(header) {
            return Err(RuleError::Invalid {
                field: "action.headers.header",
                reason: "is not a valid header name",
            });
        }
        let operation = match get_str(o, "action.headers.operation")?.ok_or(RuleError::Missing {
            field: "action.headers.operation",
        })? {
            "append" => HeaderOperation::Append,
            "set" => HeaderOperation::Set,
            "remove" => HeaderOperation::Remove,
            _ => {
                return Err(RuleError::UnknownValue {
                    field: "action.headers.operation",
                });
            }
        };
        if is_request
            && operation == HeaderOperation::Append
            && request_header_append_separator(header).is_none()
        {
            return Err(RuleError::Invalid {
                field: "action.requestHeaders",
                reason: "cannot append to this request header",
            });
        }
        let value = get_str(o, "action.headers.value")?;
        if let Some(v) = value {
            if !is_header_value(v) {
                return Err(RuleError::Invalid {
                    field: "action.headers.value",
                    reason: "is not a valid header value",
                });
            }
            if operation == HeaderOperation::Remove {
                return Err(RuleError::Invalid {
                    field: "action.headers.value",
                    reason: "must not be given for remove",
                });
            }
        } else if operation != HeaderOperation::Remove {
            return Err(RuleError::Missing {
                field: "action.headers.value",
            });
        }
        ops.push(HeaderOp {
            header: header.to_owned(),
            operation,
            value: value.map(str::to_owned),
        });
    }
    Ok(Some(ops))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn limits() -> Limits {
        Limits::default()
    }

    fn rule_json(condition: &str, action: &str) -> String {
        format!(r#"{{"id":1,"condition":{{{condition}}},"action":{{{action}}}}}"#)
    }

    fn one_with(kind: RulesetKind, json: &str) -> Result<Rule, RuleError> {
        let value: Value = serde_json::from_str(json).expect("test json");
        parse_rule(&value, kind, &limits(), None).map(|(rule, _)| rule)
    }

    fn one(json: &str) -> Result<Rule, RuleError> {
        one_with(RulesetKind::Dynamic, json)
    }

    fn block(condition: &str) -> Result<Rule, RuleError> {
        one(&rule_json(condition, r#""type":"block""#))
    }

    fn ruleset(kind: RulesetKind, json: &str) -> Result<ParsedRuleset, RulesetError> {
        parse_ruleset(json.as_bytes(), kind, &limits())
    }

    // --- defaults and basics -----------------------------------------------------------------------

    #[test]
    fn minimal_block_rule_gets_chromes_defaults() {
        let r = block(r#""urlFilter":"abc""#).expect("valid");
        assert_eq!(r.id, 1);
        assert_eq!(r.priority, 1);
        assert_eq!(r.action, Action::Block);
        assert!(!r.condition.resource_types.contains(ResourceType::MainFrame));
        assert!(r.condition.resource_types.contains(ResourceType::Script));
        assert_eq!(r.condition.request_methods, MethodSet::all());
        assert_eq!(r.condition.domain_type, None);
        match r.condition.pattern {
            Some(UrlPattern::Filter(f)) => assert!(!f.is_case_sensitive()),
            other => panic!("unexpected pattern {other:?}"),
        }
    }

    #[test]
    fn explicit_priority_is_kept() {
        let json = r#"{"id":7,"priority":42,"condition":{},"action":{"type":"allow"}}"#;
        let r = one(json).expect("valid");
        assert_eq!((r.id, r.priority), (7, 42));
        assert_eq!(r.action, Action::Allow);
    }

    #[test]
    fn id_zero_negative_fractional_missing_and_huge_are_refused() {
        for id in ["0", "-1", "1.5", "\"1\"", "null", "1099511627776"] {
            let json = format!(r#"{{"id":{id},"condition":{{}},"action":{{"type":"block"}}}}"#);
            assert!(one(&json).is_err(), "id {id}");
        }
        let json = r#"{"condition":{},"action":{"type":"block"}}"#;
        assert_eq!(one(json), Err(RuleError::Missing { field: "id" }));
    }

    #[test]
    fn priority_zero_and_negative_are_refused() {
        for p in ["0", "-3", "2.5", "\"2\""] {
            let json = format!(
                r#"{{"id":1,"priority":{p},"condition":{{}},"action":{{"type":"block"}}}}"#
            );
            assert!(one(&json).is_err(), "priority {p}");
        }
    }

    #[test]
    fn missing_condition_or_action_is_refused() {
        assert_eq!(
            one(r#"{"id":1,"action":{"type":"block"}}"#),
            Err(RuleError::Missing { field: "condition" })
        );
        assert_eq!(
            one(r#"{"id":1,"condition":{}}"#),
            Err(RuleError::Missing { field: "action" })
        );
        assert_eq!(
            one(r#"{"id":1,"condition":[],"action":{"type":"block"}}"#),
            Err(RuleError::NotAnObject { field: "condition" })
        );
        assert_eq!(one("[]"), Err(RuleError::NotAnObject { field: "rule" }));
    }

    #[test]
    fn unknown_action_type_is_refused() {
        assert_eq!(
            one(&rule_json("", r#""type":"explode""#)),
            Err(RuleError::UnknownValue {
                field: "action.type"
            })
        );
        assert_eq!(
            one(&rule_json("", r#""type":"Block""#)),
            Err(RuleError::UnknownValue {
                field: "action.type"
            })
        );
    }

    #[test]
    fn unknown_json_fields_are_ignored() {
        let json = r#"{"id":1,"future":true,"condition":{"newCondition":1},"action":{"type":"block","x":[]}}"#;
        assert!(one(json).is_ok());
    }

    // --- urlFilter and regexFilter -----------------------------------------------------------------

    #[test]
    fn url_filter_and_regex_filter_are_exclusive() {
        assert_eq!(
            block(r#""urlFilter":"a","regexFilter":"b""#),
            Err(RuleError::Conflict {
                a: "condition.urlFilter",
                b: "condition.regexFilter"
            })
        );
    }

    #[test]
    fn bad_url_filters_are_refused() {
        for filter in ["", "||", "||*abc", "пример"] {
            let cond = format!(r#""urlFilter":"{filter}""#);
            assert!(block(&cond).is_err(), "{filter:?}");
        }
    }

    #[test]
    fn url_filter_must_be_a_string() {
        assert!(matches!(
            block(r#""urlFilter":5"#),
            Err(RuleError::WrongType { .. })
        ));
        assert!(matches!(
            block(r#""urlFilter":null"#),
            Err(RuleError::WrongType { .. })
        ));
    }

    #[test]
    fn case_sensitivity_flag_reaches_the_filter() {
        let r = block(r#""urlFilter":"A","isUrlFilterCaseSensitive":true"#).expect("valid");
        match r.condition.pattern {
            Some(UrlPattern::Filter(f)) => assert!(f.is_case_sensitive()),
            other => panic!("unexpected {other:?}"),
        }
        let r = block(r#""regexFilter":"A","isUrlFilterCaseSensitive":true"#).expect("valid");
        match r.condition.pattern {
            Some(UrlPattern::Regex(f)) => assert!(f.is_case_sensitive()),
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn unsupported_or_huge_regexes_are_refused() {
        assert!(block(r#""regexFilter":"(a)\\1""#).is_err());
        assert!(block(r#""regexFilter":"(?=a)b""#).is_err());
        assert!(block(r#""regexFilter":"((a{100}){100}){100}""#).is_err());
        assert!(block(r#""regexFilter":"""#).is_err());
    }

    #[test]
    fn long_filters_hit_the_limit_error() {
        let long = "a".repeat(2049);
        assert!(matches!(
            block(&format!(r#""urlFilter":"{long}""#)),
            Err(RuleError::Limit(LimitError::FilterTooLong { .. }))
        ));
        let at = "a".repeat(2048);
        assert!(block(&format!(r#""urlFilter":"{at}""#)).is_ok());
    }

    // --- domains -----------------------------------------------------------------------------------

    #[test]
    fn deprecated_domains_are_aliases_of_the_initiator_lists() {
        let r = block(r#""domains":["A.com"],"excludedDomains":["b.com"]"#).expect("valid");
        assert_eq!(r.condition.initiator_domains, vec!["a.com".to_owned()]);
        assert_eq!(
            r.condition.excluded_initiator_domains,
            vec!["b.com".to_owned()]
        );
    }

    #[test]
    fn domains_and_initiator_domains_conflict() {
        assert!(matches!(
            block(r#""domains":["a.com"],"initiatorDomains":["b.com"]"#),
            Err(RuleError::Conflict { .. })
        ));
        assert!(matches!(
            block(r#""excludedDomains":["a.com"],"excludedInitiatorDomains":["b.com"]"#),
            Err(RuleError::Conflict { .. })
        ));
    }

    #[test]
    fn empty_lists_are_refused() {
        for key in [
            "initiatorDomains",
            "excludedInitiatorDomains",
            "requestDomains",
            "excludedRequestDomains",
            "resourceTypes",
            "excludedResourceTypes",
            "requestMethods",
            "excludedRequestMethods",
            "topDomains",
        ] {
            assert!(
                matches!(
                    block(&format!(r#""{key}":[]"#)),
                    Err(RuleError::Empty { .. })
                ),
                "{key}"
            );
        }
    }

    #[test]
    fn bad_domains_are_refused() {
        for d in [
            "https://a.com",
            "a.com:80",
            "a.com/path",
            "*.a.com",
            "a..com",
            ".a.com",
            "a.com.",
            "пример.рф",
            "a b.com",
            "",
            "[zzz]",
        ] {
            let cond = format!(r#""initiatorDomains":["{d}"]"#);
            assert!(block(&cond).is_err(), "{d:?}");
        }
    }

    #[test]
    fn good_domains_are_kept_lowercased() {
        let r =
            block(r#""requestDomains":["Example.COM","xn--p1ai","127.0.0.1","[::1]","a_b.test"]"#)
                .expect("valid");
        assert_eq!(
            r.condition.request_domains,
            vec!["example.com", "xn--p1ai", "127.0.0.1", "[::1]", "a_b.test"]
        );
    }

    #[test]
    fn list_entry_limit_edge() {
        let at: Vec<String> = (0..10_000).map(|i| format!("\"a{i}.test\"")).collect();
        let cond = format!(r#""requestDomains":[{}]"#, at.join(","));
        assert!(block(&cond).is_ok());
        let over: Vec<String> = (0..10_001).map(|i| format!("\"a{i}.test\"")).collect();
        let cond = format!(r#""requestDomains":[{}]"#, over.join(","));
        assert!(matches!(
            block(&cond),
            Err(RuleError::Limit(LimitError::ListTooLong {
                limit: 10_000,
                found: 10_001,
                ..
            }))
        ));
    }

    // --- resource types, methods, domain type, tabs -------------------------------------------------

    #[test]
    fn explicit_resource_types_are_exact() {
        let r = block(r#""resourceTypes":["main_frame","script"]"#).expect("valid");
        let got: Vec<_> = r.condition.resource_types.iter().collect();
        assert_eq!(got, vec![ResourceType::MainFrame, ResourceType::Script]);
    }

    #[test]
    fn excluded_types_alone_start_from_every_type_including_main_frame() {
        let r = block(r#""excludedResourceTypes":["image"]"#).expect("valid");
        assert!(r.condition.resource_types.contains(ResourceType::MainFrame));
        assert!(!r.condition.resource_types.contains(ResourceType::Image));
    }

    #[test]
    fn unknown_resource_type_and_overlap_and_exclude_all_are_refused() {
        assert!(matches!(
            block(r#""resourceTypes":["video"]"#),
            Err(RuleError::UnknownValue { .. })
        ));
        assert!(matches!(
            block(r#""resourceTypes":["script"],"excludedResourceTypes":["script"]"#),
            Err(RuleError::Invalid { .. })
        ));
        let all: Vec<String> = ResourceType::ALL
            .iter()
            .map(|t| format!("\"{}\"", t.name()))
            .collect();
        assert!(matches!(
            block(&format!(r#""excludedResourceTypes":[{}]"#, all.join(","))),
            Err(RuleError::Invalid { .. })
        ));
    }

    #[test]
    fn request_methods_are_lowercase_and_exact() {
        let r = block(r#""requestMethods":["get","post"]"#).expect("valid");
        assert!(r.condition.request_methods.contains(RequestMethod::Get));
        assert!(!r.condition.request_methods.contains(RequestMethod::Put));
        assert!(matches!(
            block(r#""requestMethods":["GET"]"#),
            Err(RuleError::UnknownValue { .. })
        ));
        let r = block(r#""excludedRequestMethods":["get"]"#).expect("valid");
        assert!(!r.condition.request_methods.contains(RequestMethod::Get));
        assert!(r.condition.request_methods.contains(RequestMethod::Other));
        assert!(matches!(
            block(r#""requestMethods":["get"],"excludedRequestMethods":["get"]"#),
            Err(RuleError::Invalid { .. })
        ));
    }

    #[test]
    fn domain_type_values() {
        assert_eq!(
            block(r#""domainType":"firstParty""#)
                .expect("valid")
                .condition
                .domain_type,
            Some(DomainType::FirstParty)
        );
        assert_eq!(
            block(r#""domainType":"thirdParty""#)
                .expect("valid")
                .condition
                .domain_type,
            Some(DomainType::ThirdParty)
        );
        assert!(matches!(
            block(r#""domainType":"secondParty""#),
            Err(RuleError::UnknownValue { .. })
        ));
    }

    #[test]
    fn tab_ids_are_for_session_rules_only() {
        let json = rule_json(r#""tabIds":[1,-1]"#, r#""type":"block""#);
        assert!(matches!(
            one_with(RulesetKind::Dynamic, &json),
            Err(RuleError::Invalid { .. })
        ));
        assert!(matches!(
            one_with(RulesetKind::Static, &json),
            Err(RuleError::Invalid { .. })
        ));
        let r = one_with(RulesetKind::Session, &json).expect("valid");
        assert_eq!(r.condition.tab_ids, vec![1, -1]);
        let json = rule_json(r#""excludedTabIds":[3]"#, r#""type":"block""#);
        assert!(one_with(RulesetKind::Dynamic, &json).is_err());
    }

    #[test]
    fn tab_id_overlap_is_refused_and_included_ids_win_over_excluded() {
        let overlap = rule_json(r#""tabIds":[1],"excludedTabIds":[1]"#, r#""type":"block""#);
        assert!(one_with(RulesetKind::Session, &overlap).is_err());
        let both = rule_json(r#""tabIds":[1],"excludedTabIds":[2]"#, r#""type":"block""#);
        let r = one_with(RulesetKind::Session, &both).expect("valid");
        assert!(r.condition.excluded_tab_ids.is_empty());
    }

    #[test]
    fn tab_ids_must_be_integers_in_range() {
        for ids in ["[1.5]", "[\"1\"]", "[2147483648]", "[-2147483649]"] {
            let json = rule_json(&format!(r#""tabIds":{ids}"#), r#""type":"block""#);
            assert!(one_with(RulesetKind::Session, &json).is_err(), "{ids}");
        }
    }

    #[test]
    fn response_header_conditions_are_refused_not_ignored() {
        assert_eq!(
            block(r#""responseHeaders":[{"header":"x"}]"#),
            Err(RuleError::Unsupported {
                field: "condition.responseHeaders"
            })
        );
        assert_eq!(
            block(r#""excludedResponseHeaders":[{"header":"x"}]"#),
            Err(RuleError::Unsupported {
                field: "condition.excludedResponseHeaders"
            })
        );
    }

    // --- allowAllRequests --------------------------------------------------------------------------

    #[test]
    fn allow_all_requests_needs_frame_resource_types() {
        let action = r#""type":"allowAllRequests""#;
        assert!(one(&rule_json(r#""resourceTypes":["main_frame"]"#, action)).is_ok());
        assert!(
            one(&rule_json(
                r#""resourceTypes":["sub_frame","main_frame"]"#,
                action
            ))
            .is_ok()
        );
        assert!(one(&rule_json("", action)).is_err());
        assert!(one(&rule_json(r#""resourceTypes":["script"]"#, action)).is_err());
        assert!(
            one(&rule_json(
                r#""resourceTypes":["main_frame","script"]"#,
                action
            ))
            .is_err()
        );
        assert!(one(&rule_json(r#""excludedResourceTypes":["script"]"#, action)).is_err());
    }

    // --- redirect ----------------------------------------------------------------------------------

    fn redirect(body: &str) -> Result<Rule, RuleError> {
        one(&rule_json(
            r#""urlFilter":"a""#,
            &format!(r#""type":"redirect","redirect":{{{body}}}"#),
        ))
    }

    #[test]
    fn redirect_needs_exactly_one_target() {
        assert!(matches!(
            one(&rule_json("", r#""type":"redirect""#)),
            Err(RuleError::Missing { .. })
        ));
        assert!(matches!(redirect(""), Err(RuleError::Invalid { .. })));
        assert!(matches!(
            redirect(r#""url":"https://a.test/","extensionPath":"/x""#),
            Err(RuleError::Invalid { .. })
        ));
    }

    #[test]
    fn redirect_url_is_validated() {
        assert!(redirect(r#""url":"https://a.test/x""#).is_ok());
        assert!(redirect(r#""url":"not a url""#).is_err());
        assert!(redirect(r#""url":"/relative""#).is_err());
        assert!(redirect(r#""url":"javascript:alert(1)""#).is_err());
        assert!(redirect(r#""url":"JavaScript:alert(1)""#).is_err());
    }

    #[test]
    fn redirect_to_a_scheme_that_never_fires_is_kept_with_a_warning() {
        let json = format!(
            "[{}]",
            rule_json(
                r#""urlFilter":"a""#,
                r#""type":"redirect","redirect":{"url":"data:text/plain,x"}"#
            )
        );
        let parsed = ruleset(RulesetKind::Dynamic, &json).expect("valid");
        assert_eq!(parsed.rules.len(), 1);
        assert_eq!(parsed.warnings.len(), 1);
        assert_eq!(parsed.warnings[0].rule_id, Some(1));
    }

    #[test]
    fn extension_path_must_be_a_single_slash_path() {
        assert!(redirect(r#""extensionPath":"/a.jpg""#).is_ok());
        for bad in ["a.jpg", "//evil.test/x", "/a\\\\b", "", "/a\\u0000b"] {
            assert!(
                redirect(&format!(r#""extensionPath":"{bad}""#)).is_err(),
                "{bad:?}"
            );
        }
    }

    #[test]
    fn transform_fields_are_validated() {
        assert!(redirect(r#""transform":{"scheme":"https","host":"new.example.com"}"#).is_ok());
        assert!(redirect(r#""transform":{"scheme":"gopher"}"#).is_err());
        assert!(redirect(r#""transform":{"host":""}"#).is_err());
        assert!(redirect(r#""transform":{"host":"a/b"}"#).is_err());
        assert!(redirect(r#""transform":{"port":"8080"}"#).is_ok());
        assert!(redirect(r#""transform":{"port":""}"#).is_ok());
        assert!(redirect(r#""transform":{"port":"65535"}"#).is_ok());
        assert!(redirect(r#""transform":{"port":"65536"}"#).is_err());
        assert!(redirect(r#""transform":{"port":"abc"}"#).is_err());
        assert!(redirect(r#""transform":{"port":"-1"}"#).is_err());
        assert!(redirect(r#""transform":{"query":"?a=b"}"#).is_ok());
        assert!(redirect(r#""transform":{"query":""}"#).is_ok());
        assert!(redirect(r#""transform":{"query":"a=b"}"#).is_err());
        assert!(redirect(r##""transform":{"fragment":"#x"}"##).is_ok());
        assert!(redirect(r#""transform":{"fragment":"x"}"#).is_err());
    }

    #[test]
    fn transform_query_and_query_transform_conflict() {
        assert!(matches!(
            redirect(r#""transform":{"query":"?a","queryTransform":{"removeParams":["a"]}}"#),
            Err(RuleError::Conflict { .. })
        ));
    }

    #[test]
    fn query_transform_entries_need_key_and_value() {
        assert!(
            redirect(
                r#""transform":{"queryTransform":{"addOrReplaceParams":[{"key":"a","value":"b"}]}}"#
            )
            .is_ok()
        );
        assert!(
            redirect(r#""transform":{"queryTransform":{"addOrReplaceParams":[{"key":"a"}]}}"#)
                .is_err()
        );
        assert!(
            redirect(r#""transform":{"queryTransform":{"addOrReplaceParams":[{"value":"a"}]}}"#)
                .is_err()
        );
        assert!(redirect(r#""transform":{"queryTransform":{"removeParams":[1]}}"#).is_err());
    }

    #[test]
    fn ftp_transform_scheme_is_valid_but_warned() {
        let json = format!(
            "[{}]",
            rule_json(
                r#""urlFilter":"a""#,
                r#""type":"redirect","redirect":{"transform":{"scheme":"ftp"}}"#
            )
        );
        let parsed = ruleset(RulesetKind::Dynamic, &json).expect("valid");
        assert_eq!(parsed.warnings.len(), 1);
    }

    #[test]
    fn regex_substitution_needs_a_regex_filter_and_valid_groups() {
        let action = r#""type":"redirect","redirect":{"regexSubstitution":"https://\\1.x.test/"}"#;
        assert!(one(&rule_json(r#""urlFilter":"a""#, action)).is_err());
        assert!(one(&rule_json("", action)).is_err());
        assert!(one(&rule_json(r#""regexFilter":"^http://(a|b)\\.x""#, action)).is_ok());
        assert!(one(&rule_json(r#""regexFilter":"^http://a\\.x""#, action)).is_err());
    }

    #[test]
    fn regex_substitution_syntax_is_validated() {
        let cond = r#""regexFilter":"(a)(b)""#;
        let sub = |s: &str| {
            one(&rule_json(
                cond,
                &format!(r#""type":"redirect","redirect":{{"regexSubstitution":"{s}"}}"#),
            ))
        };
        assert!(sub("https://x.test/\\\\0\\\\1\\\\2").is_ok());
        assert!(sub("https://x.test/\\\\\\\\").is_ok());
        assert!(sub("https://x.test/\\\\3").is_err());
        assert!(sub("https://x.test/\\\\n").is_err());
        assert!(sub("https://x.test/\\\\").is_err());
        assert!(sub("").is_err());
    }

    // --- modifyHeaders -----------------------------------------------------------------------------

    fn headers(action_body: &str) -> Result<Rule, RuleError> {
        let sep = if action_body.is_empty() { "" } else { "," };
        one(&rule_json(
            "",
            &format!(r#""type":"modifyHeaders"{sep}{action_body}"#),
        ))
    }

    #[test]
    fn modify_headers_needs_a_header_list() {
        assert!(headers("").is_err());
        assert!(matches!(
            headers(r#""requestHeaders":[]"#),
            Err(RuleError::Empty { .. })
        ));
        let r = headers(r#""requestHeaders":[{"header":"cookie","operation":"remove"}]"#)
            .expect("valid");
        let Action::ModifyHeaders(m) = r.action else {
            panic!("not modifyHeaders");
        };
        assert_eq!(m.request.len(), 1);
        assert!(m.response.is_empty());
    }

    #[test]
    fn header_operations_are_validated() {
        let ok = |s: &str| headers(&format!(r#""responseHeaders":[{s}]"#));
        assert!(ok(r#"{"header":"X-A","operation":"set","value":"1"}"#).is_ok());
        assert!(ok(r#"{"header":"X-A","operation":"append","value":"1"}"#).is_ok());
        assert!(ok(r#"{"header":"X-A","operation":"remove"}"#).is_ok());
        assert!(ok(r#"{"header":"X-A","operation":"set"}"#).is_err());
        assert!(ok(r#"{"header":"X-A","operation":"remove","value":"1"}"#).is_err());
        assert!(ok(r#"{"header":"X A","operation":"set","value":"1"}"#).is_err());
        assert!(ok(r#"{"header":"","operation":"set","value":"1"}"#).is_err());
        assert!(ok(r#"{"header":"X-A","operation":"swap","value":"1"}"#).is_err());
        assert!(ok(r#"{"header":"X-A","operation":"set","value":"a\r\nX-B: 1"}"#).is_err());
        assert!(ok(r#"{"header":"X-A","operation":"set","value":"a\u0000"}"#).is_err());
        assert!(ok(r#"{"header":"X-A","operation":"set","value":"a\tb"}"#).is_ok());
        assert!(ok(r#"{"operation":"set","value":"1"}"#).is_err());
    }

    #[test]
    fn request_header_append_is_limited_to_chromes_list() {
        let append = |h: &str| {
            headers(&format!(
                r#""requestHeaders":[{{"header":"{h}","operation":"append","value":"v"}}]"#
            ))
        };
        assert!(append("Accept").is_ok());
        assert!(append("cookie").is_ok());
        assert!(append("X-Forwarded-For").is_ok());
        assert!(append("authorization").is_err());
        assert!(append("x-custom").is_err());
        assert_eq!(request_header_append_separator("Cookie"), Some("; "));
        assert_eq!(request_header_append_separator("host"), None);
    }

    #[test]
    fn header_regex_options_are_refused_not_ignored() {
        assert!(matches!(
            headers(
                r#""requestHeaders":[{"header":"cookie","operation":"remove","regexFilter":"a"}]"#
            ),
            Err(RuleError::Unsupported { .. })
        ));
    }

    #[test]
    fn header_operation_count_edge() {
        let op = r#"{"header":"x-a","operation":"set","value":"1"}"#;
        let at = vec![op; 100].join(",");
        assert!(headers(&format!(r#""responseHeaders":[{at}]"#)).is_ok());
        let over = vec![op; 101].join(",");
        assert!(matches!(
            headers(&format!(r#""responseHeaders":[{over}]"#)),
            Err(RuleError::Limit(LimitError::TooManyHeaderOperations {
                limit: 100,
                found: 101
            }))
        ));
        let split = format!(
            r#""requestHeaders":[{}],"responseHeaders":[{}]"#,
            vec![op; 60].join(","),
            vec![op; 41].join(",")
        );
        assert!(headers(&split).is_err());
    }

    // --- rulesets ----------------------------------------------------------------------------------

    fn block_rule(id: u32) -> String {
        format!(r#"{{"id":{id},"condition":{{"urlFilter":"a"}},"action":{{"type":"block"}}}}"#)
    }

    #[test]
    fn static_ruleset_skips_invalid_rules_with_warnings() {
        let json = format!(
            r#"[{}, {{"id":2,"condition":{{"urlFilter":""}},"action":{{"type":"block"}}}}, {}]"#,
            block_rule(1),
            block_rule(3)
        );
        let parsed = ruleset(RulesetKind::Static, &json).expect("static skips");
        let ids: Vec<u32> = parsed.rules.iter().map(|r| r.id).collect();
        assert_eq!(ids, vec![1, 3]);
        assert_eq!(parsed.warnings.len(), 1);
        assert_eq!(parsed.warnings[0].index, 1);
        assert_eq!(parsed.warnings[0].rule_id, Some(2));
    }

    #[test]
    fn dynamic_and_session_rulesets_reject_the_whole_update() {
        let json = format!(
            r#"[{}, {{"id":2,"condition":{{"urlFilter":""}},"action":{{"type":"block"}}}}]"#,
            block_rule(1)
        );
        for kind in [RulesetKind::Dynamic, RulesetKind::Session] {
            match ruleset(kind, &json) {
                Err(RulesetError::InvalidRule { index, id, .. }) => {
                    assert_eq!((index, id), (1, Some(2)));
                }
                other => panic!("unexpected {other:?}"),
            }
        }
    }

    #[test]
    fn duplicate_ids_skip_in_static_and_reject_in_dynamic() {
        let json = format!("[{}, {}]", block_rule(5), block_rule(5));
        let parsed = ruleset(RulesetKind::Static, &json).expect("static");
        assert_eq!(parsed.rules.len(), 1);
        assert_eq!(parsed.warnings.len(), 1);
        assert_eq!(
            ruleset(RulesetKind::Dynamic, &json),
            Err(RulesetError::DuplicateId { id: 5 })
        );
    }

    #[test]
    fn a_rule_over_16_kib_is_skipped_or_rejected() {
        let pad = "a".repeat(16 * 1024);
        let big =
            format!(r#"{{"id":1,"padding":"{pad}","condition":{{}},"action":{{"type":"block"}}}}"#);
        let json = format!("[{big}]");
        let parsed = ruleset(RulesetKind::Static, &json).expect("static");
        assert!(parsed.rules.is_empty());
        assert!(matches!(
            ruleset(RulesetKind::Dynamic, &json),
            Err(RulesetError::Limit(LimitError::RuleTooLarge { .. }))
        ));
    }

    #[test]
    fn rule_size_edge_is_exact() {
        let base = r#"{"id":1,"p":"","condition":{},"action":{"type":"block"}}"#;
        let slack = 16 * 1024 - base.len();
        let at = format!(
            r#"{{"id":1,"p":"{}","condition":{{}},"action":{{"type":"block"}}}}"#,
            "a".repeat(slack)
        );
        assert!(ruleset(RulesetKind::Dynamic, &format!("[{at}]")).is_ok());
        let over = format!(
            r#"{{"id":1,"p":"{}","condition":{{}},"action":{{"type":"block"}}}}"#,
            "a".repeat(slack + 1)
        );
        assert!(ruleset(RulesetKind::Dynamic, &format!("[{over}]")).is_err());
    }

    #[test]
    fn ruleset_byte_limit_edge() {
        let limits = Limits {
            max_ruleset_bytes: 64,
            ..Limits::default()
        };
        let at = format!("[{}]", " ".repeat(62));
        assert_eq!(at.len(), 64);
        assert!(parse_ruleset(at.as_bytes(), RulesetKind::Dynamic, &limits).is_ok());
        let over = format!("[{}]", " ".repeat(63));
        assert_eq!(
            parse_ruleset(over.as_bytes(), RulesetKind::Dynamic, &limits),
            Err(RulesetError::Limit(LimitError::RulesetTooLarge {
                limit: 64,
                found: 65
            }))
        );
    }

    #[test]
    fn dynamic_rule_count_edge() {
        let at: Vec<String> = (1..=30_000).map(block_rule).collect();
        let parsed = ruleset(RulesetKind::Dynamic, &format!("[{}]", at.join(","))).expect("at");
        assert_eq!(parsed.rules.len(), 30_000);
        let over: Vec<String> = (1..=30_001).map(block_rule).collect();
        assert_eq!(
            ruleset(RulesetKind::Dynamic, &format!("[{}]", over.join(","))),
            Err(RulesetError::Limit(LimitError::TooManyRules {
                item: CountedItem::DynamicRules,
                limit: 30_000,
                found: 30_001
            }))
        );
    }

    #[test]
    fn session_rule_count_edge() {
        let at: Vec<String> = (1..=5_000).map(block_rule).collect();
        assert!(ruleset(RulesetKind::Session, &format!("[{}]", at.join(","))).is_ok());
        let over: Vec<String> = (1..=5_001).map(block_rule).collect();
        assert!(matches!(
            ruleset(RulesetKind::Session, &format!("[{}]", over.join(","))),
            Err(RulesetError::Limit(LimitError::TooManyRules {
                item: CountedItem::SessionRules,
                ..
            }))
        ));
    }

    fn redirect_rule(id: u32) -> String {
        format!(
            r#"{{"id":{id},"condition":{{"urlFilter":"a"}},"action":{{"type":"redirect","redirect":{{"url":"https://r.test/"}}}}}}"#
        )
    }

    #[test]
    fn unsafe_dynamic_rule_edge() {
        let at: Vec<String> = (1..=5_000).map(redirect_rule).collect();
        let parsed = ruleset(RulesetKind::Dynamic, &format!("[{}]", at.join(","))).expect("at");
        assert_eq!(parsed.stats().unsafe_rules, 5_000);
        let over: Vec<String> = (1..=5_001).map(redirect_rule).collect();
        assert_eq!(
            ruleset(RulesetKind::Dynamic, &format!("[{}]", over.join(","))),
            Err(RulesetError::Limit(LimitError::TooManyRules {
                item: CountedItem::UnsafeDynamicRules,
                limit: 5_000,
                found: 5_001
            }))
        );
    }

    #[test]
    fn safe_rules_do_not_count_as_unsafe() {
        let rules: Vec<String> = (1..=6_000).map(block_rule).collect();
        let parsed = ruleset(RulesetKind::Dynamic, &format!("[{}]", rules.join(","))).expect("ok");
        assert_eq!(parsed.stats().unsafe_rules, 0);
    }

    fn regex_rule(id: u32) -> String {
        format!(
            r#"{{"id":{id},"condition":{{"regexFilter":"a{id}b"}},"action":{{"type":"block"}}}}"#
        )
    }

    #[test]
    fn dynamic_regex_rule_edge() {
        let at: Vec<String> = (1..=1_000).map(regex_rule).collect();
        let parsed = ruleset(RulesetKind::Dynamic, &format!("[{}]", at.join(","))).expect("at");
        assert_eq!(parsed.stats().regex_rules, 1_000);
        let over: Vec<String> = (1..=1_001).map(regex_rule).collect();
        assert_eq!(
            ruleset(RulesetKind::Dynamic, &format!("[{}]", over.join(","))),
            Err(RulesetError::Limit(LimitError::TooManyRules {
                item: CountedItem::DynamicRegexRules,
                limit: 1_000,
                found: 1_001
            }))
        );
    }

    #[test]
    fn static_regex_rules_over_the_limit_are_skipped_with_a_warning() {
        let over: Vec<String> = (1..=1_001).map(regex_rule).collect();
        let parsed = ruleset(RulesetKind::Static, &format!("[{}]", over.join(","))).expect("ok");
        assert_eq!(parsed.rules.len(), 1_000);
        assert_eq!(parsed.warnings.len(), 1);
        assert_eq!(parsed.warnings[0].rule_id, Some(1_001));
    }

    #[test]
    fn static_ruleset_over_the_global_pool_is_refused() {
        let at = format!("[{}]", vec!["0"; 330_000].join(","));
        let parsed = ruleset(RulesetKind::Static, &at).expect("at the pool size");
        assert_eq!(parsed.warnings.len(), 330_000);
        let over = format!("[{}]", vec!["0"; 330_001].join(","));
        assert!(matches!(
            ruleset(RulesetKind::Static, &over),
            Err(RulesetError::Limit(LimitError::TooManyRules {
                item: CountedItem::StaticRules,
                ..
            }))
        ));
    }

    #[test]
    fn json_nesting_edge() {
        let nest = |depth: usize| format!("{}{}", "[".repeat(depth), "]".repeat(depth));
        assert!(ruleset(RulesetKind::Static, &nest(64)).is_ok());
        assert_eq!(
            ruleset(RulesetKind::Static, &nest(65)),
            Err(RulesetError::Limit(LimitError::NestingTooDeep {
                limit: 64
            }))
        );
    }

    #[test]
    fn brackets_inside_strings_do_not_count_as_nesting() {
        let pad = "[".repeat(1000);
        let json = format!(
            r#"[{{"id":1,"note":"{pad}\"[[[","condition":{{}},"action":{{"type":"block"}}}}]"#
        );
        assert!(ruleset(RulesetKind::Dynamic, &json).is_ok());
    }

    #[test]
    fn deep_nesting_does_not_overflow_the_stack() {
        let deep = format!("{}{}", "[".repeat(200_000), "]".repeat(200_000));
        assert!(matches!(
            ruleset(RulesetKind::Dynamic, &deep),
            Err(RulesetError::Limit(LimitError::NestingTooDeep { .. }))
        ));
    }

    #[test]
    fn top_level_must_be_an_array() {
        for json in ["{}", "\"x\"", "null", "1", "true"] {
            assert_eq!(
                ruleset(RulesetKind::Dynamic, json),
                Err(RulesetError::NotAnArray),
                "{json}"
            );
        }
    }

    #[test]
    fn malformed_json_is_an_error_not_a_panic() {
        let cases: Vec<Vec<u8>> = vec![
            b"".to_vec(),
            b"[".to_vec(),
            b"[{".to_vec(),
            b"[{\"id\":1,".to_vec(),
            br#"[{"id":1,"condition":{},"action":{"type":"block"}}"#.to_vec(),
            br#"[{"id":1,}]"#.to_vec(),
            b"[1 2]".to_vec(),
            b"\xEF\xBB\xBF[]".to_vec(),
            b"[] []".to_vec(),
            b"[\"a\0b\"]".to_vec(),
            b"[\"\\ud800\"]".to_vec(),
            b"[\"\\udc00\"]".to_vec(),
            b"[\xff\xfe]".to_vec(),
            b"[1e999]".to_vec(),
            b"[99999999999999999999999999999999999999999]".to_vec(),
        ];
        for case in cases {
            for kind in [
                RulesetKind::Static,
                RulesetKind::Dynamic,
                RulesetKind::Session,
            ] {
                let result = parse_ruleset(&case, kind, &limits());
                if kind != RulesetKind::Static || result.is_err() {
                    // dynamic and session must be errors for these inputs; static may only skip rules
                    // when the JSON itself is valid
                    assert!(result.is_err(), "{:?}", String::from_utf8_lossy(&case));
                }
            }
        }
    }

    #[test]
    fn huge_numbers_in_rule_fields_are_refused() {
        let json =
            r#"[{"id":99999999999999999999999999,"condition":{},"action":{"type":"block"}}]"#;
        assert!(ruleset(RulesetKind::Dynamic, json).is_err());
        let json = r#"[{"id":1,"priority":1e30,"condition":{},"action":{"type":"block"}}]"#;
        assert!(ruleset(RulesetKind::Dynamic, json).is_err());
    }

    #[test]
    fn errors_and_warnings_never_quote_the_rule() {
        let secret = "s3cr3t-пример-value";
        let json = format!(
            r#"[{{"id":1,"condition":{{"urlFilter":"{secret}"}},"action":{{"type":"block"}}}}]"#
        );
        let parsed = ruleset(RulesetKind::Static, &json).expect("static skips");
        assert!(!parsed.warnings[0].message.contains("s3cr3t"));
        let err = ruleset(RulesetKind::Dynamic, &json).expect_err("rejected");
        assert!(!err.to_string().contains("s3cr3t"));
        let json = format!(r#"[{{"id":"{secret}"}}]"#);
        let err = ruleset(RulesetKind::Dynamic, &json).expect_err("rejected");
        assert!(!err.to_string().contains("s3cr3t"));
        let json = format!(r#"[{{"id":1, "x": "{secret}" ]"#);
        let err = ruleset(RulesetKind::Dynamic, &json).expect_err("rejected");
        assert!(!err.to_string().contains("s3cr3t"));
    }

    #[test]
    fn stats_count_unsafe_and_regex_rules() {
        let json = format!("[{},{},{}]", block_rule(1), redirect_rule(2), regex_rule(3));
        let stats = ruleset(RulesetKind::Dynamic, &json).expect("ok").stats();
        assert_eq!(
            stats,
            RulesetStats {
                rules: 3,
                unsafe_rules: 1,
                regex_rules: 1
            }
        );
    }

    #[test]
    fn empty_ruleset_is_valid() {
        let parsed = ruleset(RulesetKind::Static, "[]").expect("ok");
        assert!(parsed.rules.is_empty() && parsed.warnings.is_empty());
    }

    #[test]
    fn action_ranks_follow_chromium_order() {
        let ranks = [
            Action::Allow.rank(),
            Action::AllowAllRequests.rank(),
            Action::Block.rank(),
            Action::UpgradeScheme.rank(),
            Action::Redirect(Redirect::ExtensionPath("/a".into())).rank(),
            Action::ModifyHeaders(ModifyHeaders::default()).rank(),
        ];
        assert_eq!(ranks, [5, 4, 3, 2, 1, 0]);
    }
}
