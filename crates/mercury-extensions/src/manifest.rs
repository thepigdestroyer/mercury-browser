// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! `manifest.json` parsing and validation, manifest versions 2 and 3 (Chrome and Firefox keys).
//!
//! Behaviour per Chrome's manifest reference and MDN's `manifest.json` pages; written fresh.
//!
//! What is checked, and what is a warning:
//!
//! * Errors (the manifest is refused): not JSON, not an object, over [`ManifestLimits::max_bytes`],
//!   nesting over [`ManifestLimits::max_depth`] (checked with a byte scan before parsing, so the
//!   parser's stack is safe), a missing or odd `manifest_version`, `name` or `version`, a content
//!   script without `matches`, a `background` naming more than one of `service_worker`, `scripts`
//!   and `page`, a bad match pattern anywhere, a bad `key`, a bad id, a path that leaves the
//!   package, duplicate DNR ruleset ids, an MV3 content security policy that allows remote code.
//! * Warnings ([`ManifestWarning`]): keys of the other manifest version, `background.persistent:
//!   true` in MV3, blocking `webRequest` in a Chrome MV3 extension, unknown permission names,
//!   permissions that cannot be listed where they are.
//! * Unknown top-level keys are only counted in `unknown_keys` (names, never values).
//!
//! Decisions worth knowing:
//!
//! * File-valued keys: a single leading `/` means "the package root" and is stripped (Chrome
//!   resolves it that way); a leading `./` or a `.` component is dropped; `..`, a second leading
//!   `/`, a backslash, `:`, a NUL or other control character, an empty component and percent-encoded
//!   `.`, `/` or `\` are refused. Paths in the model are the normalised ones.
//! * A leading `//`-comment in `manifest.json` is not accepted (Chrome accepts none in published
//!   packages); a UTF-8 byte order mark is.
//! * MV3 policy per kind: for a Chrome extension `script-src`, `script-src-elem`, `worker-src`
//!   and `default-src` may only hold `'self'`, `'none'`, `'wasm-unsafe-eval'` and
//!   `'inline-speculation-rules'` (plus `localhost` sources when
//!   [`ManifestLimits::allow_localhost_csp`] is set, which is for unpacked extensions), and
//!   `object-src` only `'self'`, `'none'` and `'wasm-unsafe-eval'`. For a Firefox add-on (one with a
//!   `gecko.id`) the same list applies, and `localhost` sources and hash sources (`'sha256-...'`)
//!   are accepted as well. This is Mercury's reading of the two vendors' documentation, not a copy
//!   of either validator. MV2 policies are stored, not judged.

use std::collections::{BTreeMap, BTreeSet};

use serde_json::{Map, Value};
use url::Url;

use crate::id::{ExtensionId, ExtensionKind};
use crate::match_pattern::{MatchPattern, MatchPatternSet};
use crate::permissions::{ApiPermission, PermissionList};

/// Size and shape caps for [`Manifest::parse`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManifestLimits {
    /// Largest manifest, in bytes (default 1 MiB).
    pub max_bytes: usize,
    /// Deepest JSON nesting (default 64).
    pub max_depth: usize,
    /// Longest string value, in bytes (default 4096).
    pub max_string_len: usize,
    /// Longest file path, in bytes (default 512).
    pub max_path_len: usize,
    /// Most components in a file path (default 32).
    pub max_path_depth: usize,
    /// Most entries in any one list (default 10,000).
    pub max_list_len: usize,
    /// Most content script entries (default 1,000).
    pub max_content_scripts: usize,
    /// Most commands (default 1,000).
    pub max_commands: usize,
    /// Most unknown top-level key names kept (default 1,000).
    pub max_unknown_keys: usize,
    /// Accept `localhost` sources in an MV3 Chrome policy (Chrome allows them for unpacked
    /// extensions only). Default `false`.
    pub allow_localhost_csp: bool,
}

impl Default for ManifestLimits {
    fn default() -> Self {
        ManifestLimits {
            max_bytes: 1 << 20,
            max_depth: 64,
            max_string_len: 4096,
            max_path_len: 512,
            max_path_depth: 32,
            max_list_len: 10_000,
            max_content_scripts: 1_000,
            max_commands: 1_000,
            max_unknown_keys: 1_000,
            allow_localhost_csp: false,
        }
    }
}

/// Why a manifest was refused. The messages name the key, never its value.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ManifestError {
    /// The manifest is larger than the cap.
    #[error("the manifest is larger than {limit} bytes")]
    TooLarge {
        /// The cap.
        limit: usize,
    },
    /// The JSON nests deeper than the cap.
    #[error("the manifest nests deeper than {limit} levels")]
    NestingTooDeep {
        /// The cap.
        limit: usize,
    },
    /// The bytes are not valid JSON (or not valid UTF-8).
    #[error("the manifest is not valid JSON")]
    NotJson,
    /// The JSON is not an object.
    #[error("the manifest is not a JSON object")]
    NotAnObject,
    /// A required key is missing.
    #[error("the manifest key `{0}` is missing")]
    MissingField(&'static str),
    /// A key has the wrong type, an odd value or a control character.
    #[error("the manifest key `{0}` is invalid")]
    InvalidField(&'static str),
    /// `manifest_version` is present but is not 2 or 3.
    #[error("the manifest version is not supported")]
    UnsupportedManifestVersion,
    /// `version` is not a valid version for this kind of extension.
    #[error("the extension version is invalid")]
    InvalidVersion,
    /// A content script has no `matches`.
    #[error("content script {index} has no matches")]
    ContentScriptWithoutMatches {
        /// Position in `content_scripts`.
        index: usize,
    },
    /// `background` names more than one of `service_worker`, `scripts` and `page`.
    #[error("the background names more than one of service_worker, scripts and page")]
    BackgroundConflict,
    /// A path in the named key leaves the package or is malformed.
    #[error("a path in `{field}` is not allowed")]
    UnsafePath {
        /// The key.
        field: &'static str,
    },
    /// A match pattern in the named key is invalid.
    #[error("a match pattern in `{field}` is invalid")]
    BadMatchPattern {
        /// The key.
        field: &'static str,
    },
    /// `key` is not a base64 DER public key.
    #[error("the manifest key `key` is invalid")]
    BadKey,
    /// An extension id in the manifest is invalid.
    #[error("an extension id in the manifest is invalid")]
    BadId,
    /// Two `declarative_net_request.rule_resources` entries share an id.
    #[error("two declarative_net_request rulesets share an id")]
    DuplicateRulesetId,
    /// An MV3 content security policy allows remote code; `directive` names where.
    #[error("the content security policy allows remote code in `{directive}`")]
    InsecureCsp {
        /// The directive (a fixed name).
        directive: &'static str,
    },
    /// A list or table has more entries than the cap.
    #[error("`{field}` has too many entries")]
    TooMany {
        /// The key.
        field: &'static str,
    },
}

/// Something suspicious that does not stop the manifest from loading.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ManifestWarning {
    /// A manifest version 2 key in a version 3 manifest (ignored).
    #[error("`{key}` is a manifest version 2 key and is ignored in version 3")]
    Mv2KeyInMv3 {
        /// The key.
        key: &'static str,
    },
    /// A manifest version 3 key in a version 2 manifest (ignored).
    #[error("`{key}` is a manifest version 3 key and is ignored in version 2")]
    Mv3KeyInMv2 {
        /// The key.
        key: &'static str,
    },
    /// `background.persistent: true` in manifest version 3.
    #[error("a persistent background page is not available in manifest version 3")]
    PersistentBackgroundInMv3,
    /// `webRequestBlocking` in a Chrome manifest version 3 extension.
    #[error("blocking webRequest is not available in manifest version 3")]
    BlockingWebRequestInMv3,
    /// A host pattern listed in `permissions` of a version 3 manifest (ignored; use
    /// `host_permissions`).
    #[error("a host permission in `permissions` is ignored in manifest version 3")]
    HostPermissionInPermissionsMv3,
    /// A permission name Mercury does not know.
    #[error("an unknown permission is listed")]
    UnknownPermission,
    /// A permission listed where it is not allowed (for example an always-required one under
    /// `optional_permissions`); it is ignored.
    #[error("a permission is listed where it is not allowed")]
    PermissionMisplaced,
    /// A permission that manifest version 3 does not offer; it is ignored.
    #[error("a permission is not available in manifest version 3")]
    PermissionNotAvailableInMv3,
    /// `applications` is the old name of `browser_specific_settings`.
    #[error("`applications` is deprecated; use `browser_specific_settings`")]
    DeprecatedApplicationsKey,
}

/// The manifest version.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ManifestVersion {
    /// Manifest version 2.
    V2,
    /// Manifest version 3.
    V3,
}

/// `incognito`: how the extension behaves in private windows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Incognito {
    /// One shared instance (the default).
    #[default]
    Spanning,
    /// A separate instance per profile.
    Split,
    /// The extension never runs in a private window.
    NotAllowed,
}

/// When a content script runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum RunAt {
    /// `document_start`
    DocumentStart,
    /// `document_end`
    DocumentEnd,
    /// `document_idle` (the default)
    #[default]
    DocumentIdle,
}

/// The JavaScript world a content script runs in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ContentScriptWorld {
    /// `ISOLATED` (the default)
    #[default]
    Isolated,
    /// `MAIN`
    Main,
}

/// `default_icon` of an action: one path or a table of size to path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DefaultIcon {
    /// A single path.
    Single(String),
    /// Pixel size to path.
    Sized(BTreeMap<u32, String>),
}

/// `action`, `browser_action` or `page_action`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ActionSpec {
    /// `default_title` (may be a `__MSG_x__` placeholder; not resolved).
    pub default_title: Option<String>,
    /// `default_icon`.
    pub default_icon: Option<DefaultIcon>,
    /// `default_popup`, a package path.
    pub default_popup: Option<String>,
}

/// `background`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Background {
    /// `service_worker` (Chrome MV3), a package path.
    pub service_worker: Option<String>,
    /// `scripts` (Firefox MV3 event page, MV2 background scripts), package paths.
    pub scripts: Vec<String>,
    /// `page` (MV2), a package path.
    pub page: Option<String>,
    /// `persistent`.
    pub persistent: Option<bool>,
    /// `type: "module"`.
    pub module: bool,
}

/// One entry of `content_scripts`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ContentScript {
    /// `matches`.
    pub matches: MatchPatternSet,
    /// `exclude_matches`.
    pub exclude_matches: MatchPatternSet,
    /// `include_globs`.
    pub include_globs: Vec<String>,
    /// `exclude_globs`.
    pub exclude_globs: Vec<String>,
    /// `js`, package paths.
    pub js: Vec<String>,
    /// `css`, package paths.
    pub css: Vec<String>,
    /// `run_at`.
    pub run_at: RunAt,
    /// `all_frames`.
    pub all_frames: bool,
    /// `match_about_blank`.
    pub match_about_blank: bool,
    /// `match_origin_as_fallback`.
    pub match_origin_as_fallback: bool,
    /// `world`.
    pub world: ContentScriptWorld,
}

/// One entry of an MV3 `web_accessible_resources`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct WebAccessibleEntry {
    /// `resources`: package path globs.
    pub resources: Vec<String>,
    /// `matches`: pages that may load them.
    pub matches: MatchPatternSet,
    /// `extension_ids`: extensions that may load them.
    pub extension_ids: Vec<String>,
    /// `use_dynamic_url`.
    pub use_dynamic_url: bool,
}

/// `web_accessible_resources`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WebAccessibleResources {
    /// MV2: a list of path globs, open to every page.
    Legacy(Vec<String>),
    /// MV3: entries with their own audience.
    Entries(Vec<WebAccessibleEntry>),
}

impl Default for WebAccessibleResources {
    fn default() -> Self {
        WebAccessibleResources::Legacy(Vec::new())
    }
}

/// `content_security_policy`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ContentSecurityPolicy {
    /// The policy of extension pages (the whole string in MV2).
    pub extension_pages: Option<String>,
    /// The policy of sandboxed pages.
    pub sandbox: Option<String>,
    /// The policy of content script worlds.
    pub isolated_world: Option<String>,
}

/// One `declarative_net_request.rule_resources` entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuleResource {
    /// `id`.
    pub id: String,
    /// `enabled`.
    pub enabled: bool,
    /// `path`, a package path.
    pub path: String,
}

/// `options_ui` (or `options_page`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OptionsUi {
    /// The page, a package path.
    pub page: String,
    /// `open_in_tab`.
    pub open_in_tab: Option<bool>,
}

/// One entry of `commands`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Command {
    /// `suggested_key`: platform (`default`, `mac`, ...) to key.
    pub suggested_key: BTreeMap<String, String>,
    /// `description`.
    pub description: Option<String>,
    /// `global`.
    pub global: bool,
}

/// `browser_specific_settings.gecko` (or `applications.gecko`).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Gecko {
    /// `id`.
    pub id: Option<ExtensionId>,
    /// `strict_min_version`.
    pub strict_min_version: Option<String>,
    /// `strict_max_version`.
    pub strict_max_version: Option<String>,
    /// `update_url` (http or https; the update client demands https).
    pub update_url: Option<Url>,
}

/// `externally_connectable`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ExternallyConnectable {
    /// `ids`: extension ids, or `*`.
    pub ids: Vec<String>,
    /// `matches`: web pages that may connect.
    pub matches: MatchPatternSet,
}

/// `sandbox`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Sandbox {
    /// `pages`: package path globs.
    pub pages: Vec<String>,
    /// `content_security_policy`.
    pub content_security_policy: Option<String>,
}

/// A parsed `manifest.json`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Manifest {
    /// `manifest_version`.
    pub manifest_version: ManifestVersion,
    /// Firefox when `gecko.id` is present, Chrome otherwise.
    pub kind: ExtensionKind,
    /// `name`.
    pub name: String,
    /// `version`, as written.
    pub version: String,
    /// `version_name`.
    pub version_name: Option<String>,
    /// `description`.
    pub description: Option<String>,
    /// `default_locale`.
    pub default_locale: Option<String>,
    /// `icons`: pixel size to path.
    pub icons: BTreeMap<u32, String>,
    /// `action` (MV3).
    pub action: Option<ActionSpec>,
    /// `browser_action` (MV2).
    pub browser_action: Option<ActionSpec>,
    /// `page_action` (MV2).
    pub page_action: Option<ActionSpec>,
    /// `background`.
    pub background: Option<Background>,
    /// `content_scripts`.
    pub content_scripts: Vec<ContentScript>,
    /// API permissions of `permissions`, in order, without duplicates.
    pub permissions: Vec<ApiPermission>,
    /// API permissions of `optional_permissions`.
    pub optional_permissions: Vec<ApiPermission>,
    /// Hosts requested at install: `host_permissions` (MV3) or the hosts in `permissions` (MV2).
    /// Paths are normalised to `/*`, as Chrome ignores them.
    pub host_permissions: MatchPatternSet,
    /// Hosts that can be requested later: `optional_host_permissions` (MV3) or the hosts in
    /// `optional_permissions` (MV2).
    pub optional_host_permissions: MatchPatternSet,
    /// `web_accessible_resources`.
    pub web_accessible_resources: WebAccessibleResources,
    /// `content_security_policy`.
    pub content_security_policy: ContentSecurityPolicy,
    /// `declarative_net_request.rule_resources`.
    pub dnr_rule_resources: Vec<RuleResource>,
    /// `options_ui` or `options_page`.
    pub options: Option<OptionsUi>,
    /// `side_panel.default_path`.
    pub side_panel: Option<String>,
    /// `commands`.
    pub commands: BTreeMap<String, Command>,
    /// `key`, as written.
    pub key: Option<String>,
    /// `update_url`.
    pub update_url: Option<Url>,
    /// `minimum_chrome_version`.
    pub minimum_chrome_version: Option<String>,
    /// `browser_specific_settings.gecko`.
    pub gecko: Option<Gecko>,
    /// `incognito`.
    pub incognito: Incognito,
    /// `externally_connectable`.
    pub externally_connectable: Option<ExternallyConnectable>,
    /// `devtools_page`, a package path.
    pub devtools_page: Option<String>,
    /// `chrome_url_overrides`: page name to package path.
    pub chrome_url_overrides: BTreeMap<String, String>,
    /// `storage.managed_schema`, a package path.
    pub managed_schema: Option<String>,
    /// `sandbox`.
    pub sandbox: Option<Sandbox>,
    /// `omnibox.keyword`.
    pub omnibox_keyword: Option<String>,
    /// Non-fatal findings.
    pub warnings: Vec<ManifestWarning>,
    /// Names of top-level keys Mercury does not read (never their values).
    pub unknown_keys: Vec<String>,
}

const KNOWN_KEYS: &[&str] = &[
    "manifest_version",
    "name",
    "version",
    "version_name",
    "description",
    "default_locale",
    "icons",
    "action",
    "browser_action",
    "page_action",
    "background",
    "content_scripts",
    "permissions",
    "optional_permissions",
    "host_permissions",
    "optional_host_permissions",
    "web_accessible_resources",
    "content_security_policy",
    "declarative_net_request",
    "options_ui",
    "options_page",
    "side_panel",
    "commands",
    "key",
    "update_url",
    "minimum_chrome_version",
    "browser_specific_settings",
    "applications",
    "incognito",
    "externally_connectable",
    "devtools_page",
    "chrome_url_overrides",
    "storage",
    "sandbox",
    "omnibox",
];

impl Manifest {
    /// Parses and validates a manifest. Never panics.
    pub fn parse(bytes: &[u8], limits: &ManifestLimits) -> Result<Manifest, ManifestError> {
        if bytes.len() > limits.max_bytes {
            return Err(ManifestError::TooLarge {
                limit: limits.max_bytes,
            });
        }
        let bytes = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF][..]).unwrap_or(bytes);
        if exceeds_nesting(bytes, limits.max_depth) {
            return Err(ManifestError::NestingTooDeep {
                limit: limits.max_depth,
            });
        }
        let value: Value = serde_json::from_slice(bytes).map_err(|_| ManifestError::NotJson)?;
        let Value::Object(root) = value else {
            return Err(ManifestError::NotAnObject);
        };
        Parser::new(limits).parse(&root)
    }

    /// The id the manifest declares: `gecko.id` if present, else the id of `key`. A Chrome
    /// package without `key` gets its id from the CRX or from the install path, not from here.
    pub fn declared_id(&self) -> Option<ExtensionId> {
        self.gecko_id().cloned().or_else(|| self.key_id())
    }

    /// The Firefox id (`gecko.id`), if any.
    pub fn gecko_id(&self) -> Option<&ExtensionId> {
        self.gecko.as_ref().and_then(|g| g.id.as_ref())
    }

    /// The id derived from `key`, if there is one (it was validated at parse time).
    pub fn key_id(&self) -> Option<ExtensionId> {
        self.key
            .as_deref()
            .and_then(|k| ExtensionId::from_manifest_key(k).ok())
    }

    /// Every package path the manifest points at and expects to exist (sorted, without
    /// duplicates, normalised). Globs (`web_accessible_resources`, `sandbox.pages`, anything with
    /// a `*`) are not listed.
    pub fn all_file_references(&self) -> Vec<String> {
        let mut set: BTreeSet<String> = BTreeSet::new();
        let mut add = |p: &str| {
            if !p.contains('*') {
                set.insert(p.to_owned());
            }
        };
        for p in self.icons.values() {
            add(p);
        }
        for action in [&self.action, &self.browser_action, &self.page_action]
            .into_iter()
            .flatten()
        {
            if let Some(p) = &action.default_popup {
                add(p);
            }
            match &action.default_icon {
                Some(DefaultIcon::Single(p)) => add(p),
                Some(DefaultIcon::Sized(m)) => m.values().for_each(|p| add(p)),
                None => {}
            }
        }
        if let Some(bg) = &self.background {
            if let Some(p) = &bg.service_worker {
                add(p);
            }
            if let Some(p) = &bg.page {
                add(p);
            }
            bg.scripts.iter().for_each(|p| add(p));
        }
        for cs in &self.content_scripts {
            cs.js.iter().for_each(|p| add(p));
            cs.css.iter().for_each(|p| add(p));
        }
        if let Some(o) = &self.options {
            add(&o.page);
        }
        if let Some(p) = &self.side_panel {
            add(p);
        }
        if let Some(p) = &self.devtools_page {
            add(p);
        }
        self.chrome_url_overrides.values().for_each(|p| add(p));
        self.dnr_rule_resources.iter().for_each(|r| add(&r.path));
        if let Some(p) = &self.managed_schema {
            add(p);
        }
        if let Some(s) = &self.sandbox {
            s.pages.iter().for_each(|p| add(p));
        }
        set.into_iter().collect()
    }
}

/// Whether the JSON text nests deeper than `limit` (counted on `{` and `[` outside strings).
pub(crate) fn exceeds_nesting(bytes: &[u8], limit: usize) -> bool {
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
            b'{' | b'[' => {
                depth += 1;
                if depth > limit {
                    return true;
                }
            }
            b'}' | b']' => depth = depth.saturating_sub(1),
            _ => {}
        }
    }
    false
}

/// Normalises a package path (see the module documentation). `None` if it is not allowed.
pub(crate) fn normalize_package_path(
    raw: &str,
    max_len: usize,
    max_depth: usize,
) -> Option<String> {
    if raw.is_empty() || raw.len() > max_len {
        return None;
    }
    if raw.chars().any(|c| c.is_control() || c == '\\' || c == ':') {
        return None;
    }
    let lower = raw.to_ascii_lowercase();
    if lower.contains("%2e") || lower.contains("%2f") || lower.contains("%5c") {
        return None;
    }
    let body = raw.strip_prefix('/').unwrap_or(raw);
    if body.starts_with('/') {
        return None;
    }
    let mut parts: Vec<&str> = Vec::new();
    for part in body.split('/') {
        match part {
            "." => {}
            ".." | "" => return None,
            p => parts.push(p),
        }
    }
    if parts.is_empty() || parts.len() > max_depth {
        return None;
    }
    Some(parts.join("/"))
}

/// Chrome's version rule: one to four dot-separated integers, each 0 to 65535 without leading
/// zeros.
pub(crate) fn is_chrome_version(v: &str) -> bool {
    let parts: Vec<&str> = v.split('.').collect();
    if parts.is_empty() || parts.len() > 4 {
        return false;
    }
    parts.iter().all(|p| {
        !p.is_empty()
            && p.len() <= 5
            && p.bytes().all(|b| b.is_ascii_digit())
            && (*p == "0" || !p.starts_with('0'))
            && p.parse::<u32>().is_ok_and(|n| n <= 65535)
    })
}

/// Firefox's toolkit version format: ASCII letters, digits and `. + - * _`.
pub(crate) fn is_toolkit_version(v: &str) -> bool {
    !v.is_empty()
        && v.len() <= 100
        && v.bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'+' | b'-' | b'*' | b'_'))
}

struct Parser<'a> {
    limits: &'a ManifestLimits,
    version: ManifestVersion,
    kind: ExtensionKind,
    warnings: Vec<ManifestWarning>,
}

type Obj = Map<String, Value>;

impl<'a> Parser<'a> {
    fn new(limits: &'a ManifestLimits) -> Self {
        Parser {
            limits,
            version: ManifestVersion::V3,
            kind: ExtensionKind::Chrome,
            warnings: Vec::new(),
        }
    }

    fn parse(mut self, root: &Obj) -> Result<Manifest, ManifestError> {
        self.version = match root.get("manifest_version") {
            None => return Err(ManifestError::MissingField("manifest_version")),
            Some(v) => match v.as_u64() {
                Some(2) => ManifestVersion::V2,
                Some(3) => ManifestVersion::V3,
                _ => return Err(ManifestError::UnsupportedManifestVersion),
            },
        };
        let gecko = self.gecko(root)?;
        if gecko.as_ref().is_some_and(|g| g.id.is_some()) {
            self.kind = ExtensionKind::Firefox;
        }
        let name = self.req_string(root, "name")?.trim().to_owned();
        if name.is_empty() {
            return Err(ManifestError::InvalidField("name"));
        }
        let version = self.req_string(root, "version")?;
        let version_ok = is_chrome_version(&version)
            || (self.kind == ExtensionKind::Firefox && is_toolkit_version(&version));
        if !version_ok {
            return Err(ManifestError::InvalidVersion);
        }
        let key = self.opt_string(root, "key")?;
        if let Some(k) = &key {
            ExtensionId::from_manifest_key(k).map_err(|_| ManifestError::BadKey)?;
        }
        let default_locale = self.opt_string(root, "default_locale")?;
        if let Some(l) = &default_locale
            && (l.len() > 32
                || l.is_empty()
                || !l
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-'))
        {
            return Err(ManifestError::InvalidField("default_locale"));
        }
        let minimum_chrome_version = self.opt_string(root, "minimum_chrome_version")?;
        if let Some(v) = &minimum_chrome_version
            && (v.len() > 32 || !v.bytes().all(|b| b.is_ascii_digit() || b == b'.') || v.is_empty())
        {
            return Err(ManifestError::InvalidField("minimum_chrome_version"));
        }
        let update_url = self.opt_url(root, "update_url")?;
        let incognito = match self.opt_string(root, "incognito")?.as_deref() {
            None | Some("spanning") => Incognito::Spanning,
            Some("split") => Incognito::Split,
            Some("not_allowed") => Incognito::NotAllowed,
            Some(_) => return Err(ManifestError::InvalidField("incognito")),
        };

        let icons = self.icons(root.get("icons"), "icons")?;
        let action = self.action(root, "action")?;
        let browser_action = self.action(root, "browser_action")?;
        let page_action = self.action(root, "page_action")?;
        let background = self.background(root)?;
        let content_scripts = self.content_scripts(root)?;
        let (permissions, optional_permissions, host_permissions, optional_host_permissions) =
            self.permissions(root)?;
        let web_accessible_resources = self.web_accessible_resources(root)?;
        let content_security_policy = self.csp(root)?;
        let dnr_rule_resources = self.dnr(root)?;
        let options = self.options(root)?;
        let side_panel = match self.opt_obj(root, "side_panel")? {
            Some(o) => self.opt_path(o, "default_path", "side_panel")?,
            None => None,
        };
        let commands = self.commands(root)?;
        let externally_connectable = self.externally_connectable(root)?;
        let devtools_page = self.opt_path(root, "devtools_page", "devtools_page")?;
        let chrome_url_overrides = self.url_overrides(root)?;
        let managed_schema = match self.opt_obj(root, "storage")? {
            Some(o) => self.opt_path(o, "managed_schema", "storage")?,
            None => None,
        };
        let sandbox = self.sandbox(root)?;
        let omnibox_keyword = match self.opt_obj(root, "omnibox")? {
            Some(o) => self.opt_string(o, "keyword")?,
            None => None,
        };

        let mut unknown_keys = Vec::new();
        for k in root.keys() {
            if !KNOWN_KEYS.contains(&k.as_str()) {
                if unknown_keys.len() >= self.limits.max_unknown_keys {
                    break;
                }
                unknown_keys.push(k.chars().take(128).collect::<String>());
            }
        }
        unknown_keys.sort();

        if self.version == ManifestVersion::V3 {
            for key in ["browser_action", "page_action"] {
                if root.contains_key(key) {
                    self.warnings.push(ManifestWarning::Mv2KeyInMv3 { key });
                }
            }
            if background.as_ref().and_then(|b| b.persistent) == Some(true) {
                self.warnings
                    .push(ManifestWarning::PersistentBackgroundInMv3);
            }
            if self.kind == ExtensionKind::Chrome
                && permissions.contains(&ApiPermission::WebRequestBlocking)
            {
                self.warnings.push(ManifestWarning::BlockingWebRequestInMv3);
            }
        } else {
            for key in [
                "action",
                "host_permissions",
                "optional_host_permissions",
                "side_panel",
            ] {
                if root.contains_key(key) {
                    self.warnings.push(ManifestWarning::Mv3KeyInMv2 { key });
                }
            }
            if background
                .as_ref()
                .is_some_and(|b| b.service_worker.is_some())
            {
                self.warnings.push(ManifestWarning::Mv3KeyInMv2 {
                    key: "background.service_worker",
                });
            }
        }

        for (list, which) in [
            (&permissions, PermissionList::Required),
            (&optional_permissions, PermissionList::Optional),
        ] {
            for p in list {
                if matches!(p, ApiPermission::Unknown(_)) {
                    self.warnings.push(ManifestWarning::UnknownPermission);
                    continue;
                }
                if !p.placement_ok(which) {
                    self.warnings.push(ManifestWarning::PermissionMisplaced);
                }
                if !p.available_in(self.version, self.kind) {
                    self.warnings
                        .push(ManifestWarning::PermissionNotAvailableInMv3);
                }
            }
        }

        Ok(Manifest {
            manifest_version: self.version,
            kind: self.kind,
            name,
            version,
            version_name: self.opt_string(root, "version_name")?,
            description: self.opt_string(root, "description")?,
            default_locale,
            icons,
            action,
            browser_action,
            page_action,
            background,
            content_scripts,
            permissions,
            optional_permissions,
            host_permissions,
            optional_host_permissions,
            web_accessible_resources,
            content_security_policy,
            dnr_rule_resources,
            options,
            side_panel,
            commands,
            key,
            update_url,
            minimum_chrome_version,
            gecko,
            incognito,
            externally_connectable,
            devtools_page,
            chrome_url_overrides,
            managed_schema,
            sandbox,
            omnibox_keyword,
            warnings: self.warnings,
            unknown_keys,
        })
    }

    // ---- typed access ------------------------------------------------------------------

    fn string(&self, v: &Value, field: &'static str) -> Result<String, ManifestError> {
        let s = v.as_str().ok_or(ManifestError::InvalidField(field))?;
        if s.len() > self.limits.max_string_len || s.chars().any(char::is_control) {
            return Err(ManifestError::InvalidField(field));
        }
        Ok(s.to_owned())
    }

    fn req_string(&self, o: &Obj, key: &'static str) -> Result<String, ManifestError> {
        match o.get(key) {
            None => Err(ManifestError::MissingField(key)),
            Some(v) => self.string(v, key),
        }
    }

    fn opt_string(&self, o: &Obj, key: &'static str) -> Result<Option<String>, ManifestError> {
        o.get(key).map(|v| self.string(v, key)).transpose()
    }

    fn opt_bool(
        &self,
        o: &Obj,
        key: &'static str,
        field: &'static str,
    ) -> Result<Option<bool>, ManifestError> {
        match o.get(key) {
            None => Ok(None),
            Some(Value::Bool(b)) => Ok(Some(*b)),
            Some(_) => Err(ManifestError::InvalidField(field)),
        }
    }

    fn opt_obj<'o>(&self, o: &'o Obj, key: &'static str) -> Result<Option<&'o Obj>, ManifestError> {
        match o.get(key) {
            None => Ok(None),
            Some(Value::Object(m)) => Ok(Some(m)),
            Some(_) => Err(ManifestError::InvalidField(key)),
        }
    }

    fn array<'o>(
        &self,
        v: &'o Value,
        field: &'static str,
    ) -> Result<&'o Vec<Value>, ManifestError> {
        match v {
            Value::Array(a) if a.len() <= self.limits.max_list_len => Ok(a),
            Value::Array(_) => Err(ManifestError::TooMany { field }),
            _ => Err(ManifestError::InvalidField(field)),
        }
    }

    fn string_list(
        &self,
        v: Option<&Value>,
        field: &'static str,
    ) -> Result<Vec<String>, ManifestError> {
        match v {
            None => Ok(Vec::new()),
            Some(v) => self
                .array(v, field)?
                .iter()
                .map(|s| self.string(s, field))
                .collect(),
        }
    }

    fn path(&self, raw: &str, field: &'static str) -> Result<String, ManifestError> {
        normalize_package_path(raw, self.limits.max_path_len, self.limits.max_path_depth)
            .ok_or(ManifestError::UnsafePath { field })
    }

    /// A file-valued JSON value: the type is checked, and control characters (a NUL included)
    /// make it an unsafe path rather than an invalid string.
    fn path_value(&self, v: &Value, field: &'static str) -> Result<String, ManifestError> {
        let s = v.as_str().ok_or(ManifestError::InvalidField(field))?;
        self.path(s, field)
    }

    fn opt_path(
        &self,
        o: &Obj,
        key: &'static str,
        field: &'static str,
    ) -> Result<Option<String>, ManifestError> {
        o.get(key).map(|v| self.path_value(v, field)).transpose()
    }

    fn path_list(
        &self,
        v: Option<&Value>,
        field: &'static str,
    ) -> Result<Vec<String>, ManifestError> {
        match v {
            None => Ok(Vec::new()),
            Some(v) => self
                .array(v, field)?
                .iter()
                .map(|s| self.path_value(s, field))
                .collect(),
        }
    }

    fn opt_url(&self, o: &Obj, key: &'static str) -> Result<Option<Url>, ManifestError> {
        match self.opt_string(o, key)? {
            None => Ok(None),
            Some(s) => match Url::parse(&s) {
                Ok(u) if matches!(u.scheme(), "http" | "https") && u.host().is_some() => {
                    Ok(Some(u))
                }
                _ => Err(ManifestError::InvalidField(key)),
            },
        }
    }

    fn patterns(
        &self,
        v: Option<&Value>,
        field: &'static str,
        any_path: bool,
    ) -> Result<MatchPatternSet, ManifestError> {
        let items = self.string_list(v, field)?;
        let mut parsed = Vec::with_capacity(items.len());
        for item in &items {
            let p =
                MatchPattern::parse(item).map_err(|_| ManifestError::BadMatchPattern { field })?;
            parsed.push(if any_path { p.with_any_path() } else { p });
        }
        MatchPatternSet::from_patterns(parsed).map_err(|_| ManifestError::TooMany { field })
    }

    // ---- sections ----------------------------------------------------------------------

    fn gecko(&mut self, root: &Obj) -> Result<Option<Gecko>, ManifestError> {
        let settings = match (
            root.get("browser_specific_settings"),
            root.get("applications"),
        ) {
            (Some(v), _) => Some(v),
            (None, Some(v)) => {
                self.warnings
                    .push(ManifestWarning::DeprecatedApplicationsKey);
                Some(v)
            }
            (None, None) => None,
        };
        let Some(settings) = settings else {
            return Ok(None);
        };
        let Value::Object(settings) = settings else {
            return Err(ManifestError::InvalidField("browser_specific_settings"));
        };
        let Some(gecko) = self.opt_obj(settings, "gecko")? else {
            return Ok(None);
        };
        let id = match self.opt_string(gecko, "id")? {
            None => None,
            Some(s) => Some(ExtensionId::parse(&s).map_err(|_| ManifestError::BadId)?),
        };
        let version_field =
            |this: &Self, key: &'static str| -> Result<Option<String>, ManifestError> {
                match this.opt_string(gecko, key)? {
                    Some(s) if !is_toolkit_version(&s) => {
                        Err(ManifestError::InvalidField("browser_specific_settings"))
                    }
                    other => Ok(other),
                }
            };
        Ok(Some(Gecko {
            id,
            strict_min_version: version_field(self, "strict_min_version")?,
            strict_max_version: version_field(self, "strict_max_version")?,
            update_url: self.opt_url(gecko, "update_url")?,
        }))
    }

    fn icons(
        &self,
        v: Option<&Value>,
        field: &'static str,
    ) -> Result<BTreeMap<u32, String>, ManifestError> {
        let mut out = BTreeMap::new();
        let Some(v) = v else {
            return Ok(out);
        };
        let Value::Object(m) = v else {
            return Err(ManifestError::InvalidField(field));
        };
        if m.len() > self.limits.max_list_len {
            return Err(ManifestError::TooMany { field });
        }
        for (size, path) in m {
            let size: u32 = size
                .parse()
                .map_err(|_| ManifestError::InvalidField(field))?;
            out.insert(size, self.path_value(path, field)?);
        }
        Ok(out)
    }

    fn action(&self, root: &Obj, key: &'static str) -> Result<Option<ActionSpec>, ManifestError> {
        let Some(o) = self.opt_obj(root, key)? else {
            return Ok(None);
        };
        let default_icon = match o.get("default_icon") {
            None => None,
            Some(Value::String(s)) => Some(DefaultIcon::Single(self.path(s, key)?)),
            Some(v @ Value::Object(_)) => Some(DefaultIcon::Sized(self.icons(Some(v), key)?)),
            Some(_) => return Err(ManifestError::InvalidField(key)),
        };
        Ok(Some(ActionSpec {
            default_title: self.opt_string(o, "default_title")?,
            default_icon,
            default_popup: self.opt_path(o, "default_popup", key)?,
        }))
    }

    fn background(&mut self, root: &Obj) -> Result<Option<Background>, ManifestError> {
        let Some(o) = self.opt_obj(root, "background")? else {
            return Ok(None);
        };
        let present = ["service_worker", "scripts", "page"]
            .iter()
            .filter(|k| o.contains_key(**k))
            .count();
        if present > 1 {
            return Err(ManifestError::BackgroundConflict);
        }
        let module = match self.opt_string(o, "type")?.as_deref() {
            None | Some("classic") => false,
            Some("module") => true,
            Some(_) => return Err(ManifestError::InvalidField("background")),
        };
        Ok(Some(Background {
            service_worker: self.opt_path(o, "service_worker", "background")?,
            scripts: self.path_list(o.get("scripts"), "background")?,
            page: self.opt_path(o, "page", "background")?,
            persistent: self.opt_bool(o, "persistent", "background")?,
            module,
        }))
    }

    fn content_scripts(&self, root: &Obj) -> Result<Vec<ContentScript>, ManifestError> {
        let Some(v) = root.get("content_scripts") else {
            return Ok(Vec::new());
        };
        let items = self.array(v, "content_scripts")?;
        if items.len() > self.limits.max_content_scripts {
            return Err(ManifestError::TooMany {
                field: "content_scripts",
            });
        }
        let mut out = Vec::with_capacity(items.len());
        for (index, item) in items.iter().enumerate() {
            let Value::Object(o) = item else {
                return Err(ManifestError::InvalidField("content_scripts"));
            };
            let matches = self.patterns(o.get("matches"), "content_scripts", false)?;
            if matches.is_empty() {
                return Err(ManifestError::ContentScriptWithoutMatches { index });
            }
            let run_at = match self.opt_string(o, "run_at")?.as_deref() {
                None | Some("document_idle") => RunAt::DocumentIdle,
                Some("document_start") => RunAt::DocumentStart,
                Some("document_end") => RunAt::DocumentEnd,
                Some(_) => return Err(ManifestError::InvalidField("content_scripts")),
            };
            let world = match self.opt_string(o, "world")?.as_deref() {
                None | Some("ISOLATED") => ContentScriptWorld::Isolated,
                Some("MAIN") => ContentScriptWorld::Main,
                Some(_) => return Err(ManifestError::InvalidField("content_scripts")),
            };
            out.push(ContentScript {
                matches,
                exclude_matches: self.patterns(
                    o.get("exclude_matches"),
                    "content_scripts",
                    false,
                )?,
                include_globs: self.string_list(o.get("include_globs"), "content_scripts")?,
                exclude_globs: self.string_list(o.get("exclude_globs"), "content_scripts")?,
                js: self.path_list(o.get("js"), "content_scripts")?,
                css: self.path_list(o.get("css"), "content_scripts")?,
                run_at,
                all_frames: self
                    .opt_bool(o, "all_frames", "content_scripts")?
                    .unwrap_or(false),
                match_about_blank: self
                    .opt_bool(o, "match_about_blank", "content_scripts")?
                    .unwrap_or(false),
                match_origin_as_fallback: self
                    .opt_bool(o, "match_origin_as_fallback", "content_scripts")?
                    .unwrap_or(false),
                world,
            });
        }
        Ok(out)
    }

    #[allow(clippy::type_complexity)]
    fn permissions(
        &mut self,
        root: &Obj,
    ) -> Result<
        (
            Vec<ApiPermission>,
            Vec<ApiPermission>,
            MatchPatternSet,
            MatchPatternSet,
        ),
        ManifestError,
    > {
        let mut host_required: Vec<MatchPattern> = Vec::new();
        let mut host_optional: Vec<MatchPattern> = Vec::new();
        let required =
            self.permission_list(root.get("permissions"), "permissions", &mut host_required)?;
        let optional = self.permission_list(
            root.get("optional_permissions"),
            "optional_permissions",
            &mut host_optional,
        )?;
        let mut required_hosts =
            MatchPatternSet::from_patterns(host_required).map_err(|_| ManifestError::TooMany {
                field: "permissions",
            })?;
        let mut optional_hosts =
            MatchPatternSet::from_patterns(host_optional).map_err(|_| ManifestError::TooMany {
                field: "optional_permissions",
            })?;
        if self.version == ManifestVersion::V3 {
            required_hosts = required_hosts.union(&self.patterns(
                root.get("host_permissions"),
                "host_permissions",
                true,
            )?);
            optional_hosts = optional_hosts.union(&self.patterns(
                root.get("optional_host_permissions"),
                "optional_host_permissions",
                true,
            )?);
        } else {
            // The MV3 keys still have to be valid if present, but they are ignored.
            self.patterns(root.get("host_permissions"), "host_permissions", true)?;
            self.patterns(
                root.get("optional_host_permissions"),
                "optional_host_permissions",
                true,
            )?;
        }
        Ok((required, optional, required_hosts, optional_hosts))
    }

    fn permission_list(
        &mut self,
        v: Option<&Value>,
        field: &'static str,
        hosts: &mut Vec<MatchPattern>,
    ) -> Result<Vec<ApiPermission>, ManifestError> {
        let items = self.string_list(v, field)?;
        let mut out: Vec<ApiPermission> = Vec::new();
        for item in &items {
            if item.len() > 256 {
                return Err(ManifestError::InvalidField(field));
            }
            if item == "<all_urls>" || item.contains("://") {
                let p = MatchPattern::parse(item)
                    .map_err(|_| ManifestError::BadMatchPattern { field })?;
                if self.version == ManifestVersion::V2 {
                    hosts.push(p.with_any_path());
                } else {
                    self.warnings
                        .push(ManifestWarning::HostPermissionInPermissionsMv3);
                }
                continue;
            }
            let perm = ApiPermission::parse(item);
            if !out.contains(&perm) {
                out.push(perm);
            }
        }
        Ok(out)
    }

    fn web_accessible_resources(
        &self,
        root: &Obj,
    ) -> Result<WebAccessibleResources, ManifestError> {
        let field = "web_accessible_resources";
        let Some(v) = root.get(field) else {
            return Ok(WebAccessibleResources::default());
        };
        let items = self.array(v, field)?;
        if self.version == ManifestVersion::V2 {
            return Ok(WebAccessibleResources::Legacy(
                self.path_list(Some(v), field)?,
            ));
        }
        let mut out = Vec::with_capacity(items.len());
        for item in items {
            let Value::Object(o) = item else {
                return Err(ManifestError::InvalidField(field));
            };
            let extension_ids = self.string_list(o.get("extension_ids"), field)?;
            for id in &extension_ids {
                if id != "*" && ExtensionId::parse(id).is_err() {
                    return Err(ManifestError::BadId);
                }
            }
            out.push(WebAccessibleEntry {
                resources: self.path_list(o.get("resources"), field)?,
                matches: self.patterns(o.get("matches"), field, false)?,
                extension_ids,
                use_dynamic_url: self.opt_bool(o, "use_dynamic_url", field)?.unwrap_or(false),
            });
        }
        Ok(WebAccessibleResources::Entries(out))
    }

    fn csp(&self, root: &Obj) -> Result<ContentSecurityPolicy, ManifestError> {
        let field = "content_security_policy";
        let policy = |v: &Value| -> Result<String, ManifestError> {
            let s = self.string(v, field)?;
            Ok(s)
        };
        let mut out = ContentSecurityPolicy::default();
        match (root.get(field), self.version) {
            (None, _) => {}
            (Some(v @ Value::String(_)), ManifestVersion::V2) => {
                out.extension_pages = Some(policy(v)?)
            }
            (Some(Value::Object(o)), ManifestVersion::V3) => {
                for (key, slot) in [
                    ("extension_pages", &mut out.extension_pages),
                    ("sandbox", &mut out.sandbox),
                    ("isolated_world", &mut out.isolated_world),
                ] {
                    if let Some(v) = o.get(key) {
                        *slot = Some(policy(v)?);
                    }
                }
                for text in [&out.extension_pages, &out.isolated_world]
                    .into_iter()
                    .flatten()
                {
                    check_mv3_csp(text, self.kind, self.limits)
                        .map_err(|directive| ManifestError::InsecureCsp { directive })?;
                }
            }
            _ => return Err(ManifestError::InvalidField(field)),
        }
        Ok(out)
    }

    fn dnr(&self, root: &Obj) -> Result<Vec<RuleResource>, ManifestError> {
        let field = "declarative_net_request";
        let Some(o) = self.opt_obj(root, field)? else {
            return Ok(Vec::new());
        };
        let Some(list) = o.get("rule_resources") else {
            return Ok(Vec::new());
        };
        let items = self.array(list, field)?;
        if items.len() > 100 {
            return Err(ManifestError::TooMany { field });
        }
        let mut seen = BTreeSet::new();
        let mut out = Vec::with_capacity(items.len());
        for item in items {
            let Value::Object(r) = item else {
                return Err(ManifestError::InvalidField(field));
            };
            let id = self
                .req_string(r, "id")
                .map_err(|_| ManifestError::InvalidField(field))?;
            if id.is_empty() {
                return Err(ManifestError::InvalidField(field));
            }
            if !seen.insert(id.clone()) {
                return Err(ManifestError::DuplicateRulesetId);
            }
            let path = r.get("path").ok_or(ManifestError::InvalidField(field))?;
            out.push(RuleResource {
                id,
                enabled: self.opt_bool(r, "enabled", field)?.unwrap_or(true),
                path: self.path_value(path, field)?,
            });
        }
        Ok(out)
    }

    fn options(&self, root: &Obj) -> Result<Option<OptionsUi>, ManifestError> {
        if let Some(o) = self.opt_obj(root, "options_ui")? {
            let page = o
                .get("page")
                .ok_or(ManifestError::MissingField("options_ui"))?;
            return Ok(Some(OptionsUi {
                page: self.path_value(page, "options_ui")?,
                open_in_tab: self.opt_bool(o, "open_in_tab", "options_ui")?,
            }));
        }
        match root.get("options_page") {
            Some(p) => Ok(Some(OptionsUi {
                page: self.path_value(p, "options_page")?,
                open_in_tab: Some(true),
            })),
            None => Ok(None),
        }
    }

    fn commands(&self, root: &Obj) -> Result<BTreeMap<String, Command>, ManifestError> {
        let field = "commands";
        let mut out = BTreeMap::new();
        let Some(o) = self.opt_obj(root, field)? else {
            return Ok(out);
        };
        if o.len() > self.limits.max_commands {
            return Err(ManifestError::TooMany { field });
        }
        for (name, spec) in o {
            if name.len() > 256 || name.chars().any(char::is_control) {
                return Err(ManifestError::InvalidField(field));
            }
            let Value::Object(c) = spec else {
                return Err(ManifestError::InvalidField(field));
            };
            let mut suggested = BTreeMap::new();
            match c.get("suggested_key") {
                None => {}
                Some(v @ Value::String(_)) => {
                    suggested.insert("default".to_owned(), self.string(v, field)?);
                }
                Some(Value::Object(m)) => {
                    for (platform, key) in m {
                        if platform.len() > 32 {
                            return Err(ManifestError::InvalidField(field));
                        }
                        suggested.insert(platform.clone(), self.string(key, field)?);
                    }
                }
                Some(_) => return Err(ManifestError::InvalidField(field)),
            }
            out.insert(
                name.clone(),
                Command {
                    suggested_key: suggested,
                    description: self.opt_string(c, "description")?,
                    global: self.opt_bool(c, "global", field)?.unwrap_or(false),
                },
            );
        }
        Ok(out)
    }

    fn externally_connectable(
        &self,
        root: &Obj,
    ) -> Result<Option<ExternallyConnectable>, ManifestError> {
        let field = "externally_connectable";
        let Some(o) = self.opt_obj(root, field)? else {
            return Ok(None);
        };
        let ids = self.string_list(o.get("ids"), field)?;
        for id in &ids {
            if id != "*" && ExtensionId::parse(id).is_err() {
                return Err(ManifestError::BadId);
            }
        }
        Ok(Some(ExternallyConnectable {
            ids,
            matches: self.patterns(o.get("matches"), field, false)?,
        }))
    }

    fn url_overrides(&self, root: &Obj) -> Result<BTreeMap<String, String>, ManifestError> {
        let field = "chrome_url_overrides";
        let mut out = BTreeMap::new();
        let Some(o) = self.opt_obj(root, field)? else {
            return Ok(out);
        };
        if o.len() > 64 {
            return Err(ManifestError::TooMany { field });
        }
        for (page, path) in o {
            if page.len() > 64 {
                return Err(ManifestError::InvalidField(field));
            }
            out.insert(page.clone(), self.path_value(path, field)?);
        }
        Ok(out)
    }

    fn sandbox(&self, root: &Obj) -> Result<Option<Sandbox>, ManifestError> {
        let field = "sandbox";
        let Some(o) = self.opt_obj(root, field)? else {
            return Ok(None);
        };
        Ok(Some(Sandbox {
            pages: self.path_list(o.get("pages"), field)?,
            content_security_policy: self.opt_string(o, "content_security_policy")?,
        }))
    }
}

/// Checks an MV3 policy (see the module documentation). `Err` names the directive.
fn check_mv3_csp(
    policy: &str,
    kind: ExtensionKind,
    limits: &ManifestLimits,
) -> Result<(), &'static str> {
    let mut seen: BTreeSet<String> = BTreeSet::new();
    for directive in policy.split(';') {
        let mut tokens = directive.split_ascii_whitespace();
        let Some(name) = tokens.next() else {
            continue;
        };
        let name = name.to_ascii_lowercase();
        // The first occurrence of a directive wins; later ones are ignored by browsers.
        if !seen.insert(name.clone()) {
            continue;
        }
        let tokens: Vec<String> = tokens.map(str::to_ascii_lowercase).collect();
        match name.as_str() {
            "script-src" => {
                check_script_sources(&tokens, kind, limits).map_err(|()| "script-src")?
            }
            "script-src-elem" => {
                check_script_sources(&tokens, kind, limits).map_err(|()| "script-src-elem")?
            }
            "worker-src" => {
                check_script_sources(&tokens, kind, limits).map_err(|()| "worker-src")?
            }
            "default-src" => {
                check_script_sources(&tokens, kind, limits).map_err(|()| "default-src")?
            }
            "object-src"
                if !tokens
                    .iter()
                    .all(|t| matches!(t.as_str(), "'self'" | "'none'" | "'wasm-unsafe-eval'")) =>
            {
                return Err("object-src");
            }
            _ => {}
        }
    }
    Ok(())
}

fn check_script_sources(
    tokens: &[String],
    kind: ExtensionKind,
    limits: &ManifestLimits,
) -> Result<(), ()> {
    let localhost_ok = kind == ExtensionKind::Firefox || limits.allow_localhost_csp;
    for t in tokens {
        let ok = match t.as_str() {
            "'self'" | "'none'" | "'wasm-unsafe-eval'" | "'inline-speculation-rules'" => true,
            other => {
                (localhost_ok && is_localhost_source(other))
                    || (kind == ExtensionKind::Firefox && is_hash_source(other))
            }
        };
        if !ok {
            return Err(());
        }
    }
    Ok(())
}

fn is_localhost_source(token: &str) -> bool {
    let Some(rest) = token
        .strip_prefix("http://")
        .or_else(|| token.strip_prefix("https://"))
    else {
        return false;
    };
    let host_port = rest.split('/').next().unwrap_or("");
    let (host, port) = match host_port.rsplit_once(':') {
        Some((h, p)) => (h, Some(p)),
        None => (host_port, None),
    };
    matches!(host, "localhost" | "127.0.0.1")
        && port.is_none_or(|p| p == "*" || (!p.is_empty() && p.bytes().all(|b| b.is_ascii_digit())))
        && rest.len() == host_port.len()
}

fn is_hash_source(token: &str) -> bool {
    ["'sha256-", "'sha384-", "'sha512-"].iter().any(|p| {
        token.strip_prefix(p).is_some_and(|rest| {
            rest.strip_suffix('\'').is_some_and(|b64| {
                !b64.is_empty()
                    && b64.bytes().all(|b| {
                        b.is_ascii_alphanumeric() || matches!(b, b'+' | b'/' | b'=' | b'-' | b'_')
                    })
            })
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nesting_scan_ignores_brackets_in_strings() {
        assert!(!exceeds_nesting(br#"{"a":"[[[[[[[[[["}"#, 2));
        assert!(exceeds_nesting(b"[[[[", 3));
        assert!(!exceeds_nesting(b"[[[]]]", 3));
    }

    #[test]
    fn paths_are_normalised() {
        let n = |s: &str| normalize_package_path(s, 512, 32);
        assert_eq!(n("a/b.js").as_deref(), Some("a/b.js"));
        assert_eq!(n("/a/b.js").as_deref(), Some("a/b.js"));
        assert_eq!(n("./a/./b.js").as_deref(), Some("a/b.js"));
        for bad in [
            "", "..", "../a", "a/../b", "//a", "a//b", "a/", "a\\b", "C:\\x", "a\0b", "%2e%2e/a",
            "/",
        ] {
            assert_eq!(n(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn chrome_versions() {
        for ok in ["1", "0", "1.2", "1.2.3.4", "65535.0.0.1", "0.0.1"] {
            assert!(is_chrome_version(ok), "{ok}");
        }
        for bad in [
            "",
            "1.2.3.4.5",
            "01",
            "1.02",
            "65536",
            "1..2",
            "a",
            "-1",
            "1.",
            "1.0-beta",
        ] {
            assert!(!is_chrome_version(bad), "{bad}");
        }
    }

    #[test]
    fn localhost_sources() {
        assert!(is_localhost_source("http://localhost"));
        assert!(is_localhost_source("http://localhost:*"));
        assert!(is_localhost_source("http://127.0.0.1:8080"));
        assert!(!is_localhost_source("http://localhost.evil.com"));
        assert!(!is_localhost_source("http://localhost/x.js"));
        assert!(!is_localhost_source("http://localhost:80a"));
    }
}
