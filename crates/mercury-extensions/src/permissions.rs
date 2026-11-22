// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! The permission and security model of extensions (EE33), as data plus checks. No clock, no
//! I/O: every function that depends on time takes `now`.
//!
//! * [`ApiPermission`] and its metadata (where it may be listed, whether MV3 offers it) and the
//!   install warnings ([`install_warnings`]), written from Chrome's public permissions list and
//!   permission-warning guidance (no Chromium source was copied; the Firefox wording is Mercury's
//!   own short phrasing of what MDN documents).
//! * [`PermissionSet`], [`ManifestPermissions`], [`HostGrants`] and [`SiteAccess`]: what an
//!   extension asked for and what the user allows. Mercury withholds broad host access until the
//!   user grants it ([`Defaults::withhold_broad_hosts`]).
//! * [`ActiveTabGrants`]: the `activeTab` lifecycle.
//! * [`plan_permission_request`] and [`plan_permission_removal`]: optional permissions.
//! * [`can_access`]: THE check that injection (EE21), privileged fetch (EE30) and `webRequest`
//!   (EE29) call before touching a page. Blocked surfaces are refused whatever was granted.
//! * [`KillSwitch`]: the global switch and the crash breaker.
//! * [`permissions_added`] and [`update_needs_consent`]: an update that adds anything waits for the
//!   user.
//!
//! Chrome's cross-permission rules as implemented in [`install_warnings`]: an all-hosts grant
//! hides the specific-host warnings and the "browsing history" warning of `tabs`, `webNavigation`
//! and `declarativeNetRequestFeedback`; `history` hides the same warning; `sessions` together with
//! `tabs` gives the "signed-in devices" variant. Where Chromium's behaviour could not be verified
//! (for example whether `declarativeNetRequest` is hidden by all-hosts) the more restrictive
//! reading is taken: the warning is shown.

use std::collections::{BTreeMap, BTreeSet};
use std::time::{Duration, SystemTime};

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use url::Url;

use crate::id::{ExtensionId, ExtensionKind};
use crate::manifest::{Incognito, Manifest, ManifestVersion};
use crate::match_pattern::{HostPattern, MAX_PATTERNS_PER_SET, MatchPattern, MatchPatternSet};

/// Which manifest list a permission is written in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PermissionList {
    /// `permissions`
    Required,
    /// `optional_permissions`
    Optional,
}

/// Where a permission may be listed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Placement {
    /// In `permissions` and in `optional_permissions`.
    Both,
    /// Only in `permissions`.
    RequiredOnly,
    /// Only in `optional_permissions`.
    OptionalOnly,
}

impl Placement {
    /// Whether a permission with this placement may be in `list`.
    pub fn allows(self, list: PermissionList) -> bool {
        matches!(
            (self, list),
            (Placement::Both, _)
                | (Placement::RequiredOnly, PermissionList::Required)
                | (Placement::OptionalOnly, PermissionList::Optional)
        )
    }
}

macro_rules! api_permissions {
    ($( $var:ident => $name:literal, $place:ident, chrome_mv3: $ch3:literal; )*) => {
        /// A named API permission. Names Mercury does not know are kept as
        /// [`ApiPermission::Unknown`] (Chrome and Firefox warn and ignore them).
        ///
        /// Where a permission may be listed is Mercury's reading of Chrome's documentation: it is
        /// `RequiredOnly` when Chrome documents it as not optional or when that could not be
        /// verified (the entry is then ignored under `optional_permissions` with a warning).
        #[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
        pub enum ApiPermission {
            $(
                #[doc = concat!("The `", $name, "` permission.")]
                $var,
            )*
            /// A permission name Mercury does not know.
            Unknown(String),
        }

        impl ApiPermission {
            /// The permission for a manifest name (unknown names are kept).
            pub fn parse(name: &str) -> ApiPermission {
                match name {
                    $( $name => ApiPermission::$var, )*
                    other => ApiPermission::Unknown(other.to_owned()),
                }
            }

            /// The manifest name.
            pub fn name(&self) -> &str {
                match self {
                    $( ApiPermission::$var => $name, )*
                    ApiPermission::Unknown(n) => n,
                }
            }

            /// Where this permission may be listed (`Both` for an unknown one).
            pub fn placement(&self) -> Placement {
                match self {
                    $( ApiPermission::$var => Placement::$place, )*
                    ApiPermission::Unknown(_) => Placement::Both,
                }
            }

            fn chrome_mv3(&self) -> bool {
                match self {
                    $( ApiPermission::$var => $ch3, )*
                    ApiPermission::Unknown(_) => true,
                }
            }

            /// Every known permission name.
            pub const KNOWN_NAMES: &'static [&'static str] = &[ $( $name, )* ];
        }
    };
}

api_permissions! {
    AccessibilityFeaturesModify => "accessibilityFeatures.modify", Both, chrome_mv3: true;
    AccessibilityFeaturesRead => "accessibilityFeatures.read", Both, chrome_mv3: true;
    ActiveTab => "activeTab", RequiredOnly, chrome_mv3: true;
    Alarms => "alarms", RequiredOnly, chrome_mv3: true;
    Background => "background", Both, chrome_mv3: true;
    Bookmarks => "bookmarks", Both, chrome_mv3: true;
    BrowserSettings => "browserSettings", Both, chrome_mv3: true;
    BrowsingData => "browsingData", Both, chrome_mv3: true;
    ClipboardRead => "clipboardRead", Both, chrome_mv3: true;
    ClipboardWrite => "clipboardWrite", Both, chrome_mv3: true;
    ContentSettings => "contentSettings", Both, chrome_mv3: true;
    ContextMenus => "contextMenus", Both, chrome_mv3: true;
    ContextualIdentities => "contextualIdentities", Both, chrome_mv3: true;
    Cookies => "cookies", Both, chrome_mv3: true;
    Debugger => "debugger", Both, chrome_mv3: true;
    DeclarativeContent => "declarativeContent", Both, chrome_mv3: true;
    DeclarativeNetRequest => "declarativeNetRequest", RequiredOnly, chrome_mv3: true;
    DeclarativeNetRequestFeedback => "declarativeNetRequestFeedback", Both, chrome_mv3: true;
    DeclarativeNetRequestWithHostAccess => "declarativeNetRequestWithHostAccess", RequiredOnly, chrome_mv3: true;
    DesktopCapture => "desktopCapture", Both, chrome_mv3: true;
    Dns => "dns", Both, chrome_mv3: true;
    Downloads => "downloads", Both, chrome_mv3: true;
    DownloadsOpen => "downloads.open", Both, chrome_mv3: true;
    DownloadsUi => "downloads.ui", Both, chrome_mv3: true;
    Favicon => "favicon", RequiredOnly, chrome_mv3: true;
    Find => "find", Both, chrome_mv3: true;
    FontSettings => "fontSettings", Both, chrome_mv3: true;
    Geolocation => "geolocation", Both, chrome_mv3: true;
    History => "history", Both, chrome_mv3: true;
    Identity => "identity", Both, chrome_mv3: true;
    IdentityEmail => "identity.email", Both, chrome_mv3: true;
    Idle => "idle", Both, chrome_mv3: true;
    Management => "management", Both, chrome_mv3: true;
    Menus => "menus", Both, chrome_mv3: true;
    NativeMessaging => "nativeMessaging", Both, chrome_mv3: true;
    Notifications => "notifications", Both, chrome_mv3: true;
    Offscreen => "offscreen", RequiredOnly, chrome_mv3: true;
    PageCapture => "pageCapture", Both, chrome_mv3: true;
    Power => "power", Both, chrome_mv3: true;
    Privacy => "privacy", Both, chrome_mv3: true;
    Proxy => "proxy", Both, chrome_mv3: true;
    ReadingList => "readingList", Both, chrome_mv3: true;
    Scripting => "scripting", RequiredOnly, chrome_mv3: true;
    Search => "search", Both, chrome_mv3: true;
    Sessions => "sessions", Both, chrome_mv3: true;
    SidePanel => "sidePanel", RequiredOnly, chrome_mv3: true;
    Storage => "storage", RequiredOnly, chrome_mv3: true;
    SystemCpu => "system.cpu", Both, chrome_mv3: true;
    SystemDisplay => "system.display", Both, chrome_mv3: true;
    SystemMemory => "system.memory", Both, chrome_mv3: true;
    SystemStorage => "system.storage", Both, chrome_mv3: true;
    TabCapture => "tabCapture", Both, chrome_mv3: true;
    TabGroups => "tabGroups", Both, chrome_mv3: true;
    TabHide => "tabHide", Both, chrome_mv3: true;
    Tabs => "tabs", Both, chrome_mv3: true;
    Theme => "theme", Both, chrome_mv3: true;
    TopSites => "topSites", Both, chrome_mv3: true;
    Tts => "tts", Both, chrome_mv3: true;
    TtsEngine => "ttsEngine", Both, chrome_mv3: true;
    UnlimitedStorage => "unlimitedStorage", RequiredOnly, chrome_mv3: true;
    UserScripts => "userScripts", Both, chrome_mv3: true;
    WebAuthenticationProxy => "webAuthenticationProxy", RequiredOnly, chrome_mv3: true;
    WebNavigation => "webNavigation", Both, chrome_mv3: true;
    WebRequest => "webRequest", Both, chrome_mv3: true;
    WebRequestAuthProvider => "webRequestAuthProvider", RequiredOnly, chrome_mv3: true;
    WebRequestBlocking => "webRequestBlocking", RequiredOnly, chrome_mv3: false;
    WebRequestFilterResponse => "webRequestFilterResponse", RequiredOnly, chrome_mv3: true;
}

impl ApiPermission {
    /// Whether the permission may be written in `list` (unknown names: yes, they get their own
    /// warning).
    pub fn placement_ok(&self, list: PermissionList) -> bool {
        self.placement().allows(list)
    }

    /// Whether the permission exists in this manifest version for this kind of extension:
    /// `webRequestBlocking` is gone from Chrome's MV3 and stays in Firefox's.
    pub fn available_in(&self, version: ManifestVersion, kind: ExtensionKind) -> bool {
        match (version, kind) {
            (ManifestVersion::V2, _) => true,
            (ManifestVersion::V3, ExtensionKind::Firefox) => true,
            (ManifestVersion::V3, ExtensionKind::Chrome) => self.chrome_mv3(),
        }
    }
}

impl Serialize for ApiPermission {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.name())
    }
}

impl<'de> Deserialize<'de> for ApiPermission {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Ok(ApiPermission::parse(&String::deserialize(deserializer)?))
    }
}

// ---- warnings ----------------------------------------------------------------------------

/// One line of an install or update prompt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Warning {
    /// A stable message id; warnings with the same id are shown once.
    pub id: &'static str,
    /// The text.
    pub text: String,
    /// Extra lines (for example the list of hosts behind "a number of websites").
    pub details: Vec<String>,
}

const CHROME_ALL_HOSTS: &str = "Read and change all your data on all websites";
const FIREFOX_ALL_HOSTS: &str = "Access your data for all websites";
const MAX_DETAIL_LINES: usize = 100;

type Messages = &'static [(&'static str, &'static str)];

fn chrome_messages(p: &ApiPermission) -> Messages {
    use ApiPermission as P;
    match p {
        P::AccessibilityFeaturesModify => {
            &[("accessibility_modify", "Change your accessibility settings")]
        }
        P::AccessibilityFeaturesRead => {
            &[("accessibility_read", "Read your accessibility settings")]
        }
        P::Bookmarks => &[("bookmarks", "Read and change your bookmarks")],
        P::ClipboardRead => &[("clipboard_read", "Read data you copy and paste")],
        P::ClipboardWrite => &[("clipboard_write", "Modify data you copy and paste")],
        P::ContentSettings => &[(
            "content_settings",
            "Change your settings that control websites' access to features such as cookies, JavaScript, plugins, geolocation, microphone, camera etc.",
        )],
        P::Debugger => &[
            ("debugger", "Access the page debugger backend"),
            ("all_hosts", CHROME_ALL_HOSTS),
        ],
        P::DeclarativeNetRequest => &[("block_content", "Block content on any page")],
        P::DeclarativeNetRequestFeedback | P::Tabs | P::WebNavigation => {
            &[("browsing_history_read", "Read your browsing history")]
        }
        P::DesktopCapture => &[("desktop_capture", "Capture content of your screen")],
        P::Downloads | P::DownloadsOpen | P::DownloadsUi => {
            &[("downloads", "Manage your downloads")]
        }
        P::Favicon => &[("favicon", "Read the icons of the websites you visit")],
        P::Geolocation => &[("geolocation", "Detect your physical location")],
        P::History => &[(
            "history",
            "Read and change your browsing history on all signed-in devices",
        )],
        P::IdentityEmail => &[("identity_email", "Know your email address")],
        P::Management => &[("management", "Manage your apps, extensions, and themes")],
        P::NativeMessaging => &[(
            "native_messaging",
            "Communicate with cooperating native applications",
        )],
        P::Notifications => &[("notifications", "Display notifications")],
        P::PageCapture | P::Proxy | P::TabCapture | P::WebAuthenticationProxy => {
            &[("all_hosts", CHROME_ALL_HOSTS)]
        }
        P::Privacy => &[("privacy", "Change your privacy-related settings")],
        P::ReadingList => &[(
            "reading_list",
            "Read and change entries in the reading list",
        )],
        P::SystemStorage => &[("system_storage", "Identify and eject storage devices")],
        P::TabGroups => &[("tab_groups", "View and manage your tab groups")],
        P::TopSites => &[(
            "top_sites",
            "Read a list of your most frequently visited websites",
        )],
        P::TtsEngine => &[(
            "tts_engine",
            "Read all text spoken using synthesized speech",
        )],
        _ => &[],
    }
}

fn firefox_messages(p: &ApiPermission) -> Messages {
    use ApiPermission as P;
    match p {
        P::Bookmarks => &[("bookmarks", "Read and modify bookmarks")],
        P::BrowserSettings => &[("browser_settings", "Read and modify browser settings")],
        P::ClipboardRead => &[("clipboard_read", "Get data from the clipboard")],
        P::ClipboardWrite => &[("clipboard_write", "Input data to the clipboard")],
        P::ContextualIdentities => &[("contextual_identities", "Access contextual identities")],
        P::Downloads | P::DownloadsOpen => &[(
            "downloads",
            "Download files and read and modify the browser's download history",
        )],
        P::Geolocation => &[("geolocation", "Access your location")],
        P::History | P::TopSites => &[("history", "Access browsing history")],
        P::Management => &[("management", "Monitor extension usage and manage themes")],
        P::NativeMessaging => &[(
            "native_messaging",
            "Exchange messages with programs other than Mercury",
        )],
        P::Notifications => &[("notifications", "Display notifications to you")],
        P::Privacy => &[("privacy", "Read and modify privacy settings")],
        P::Proxy => &[("proxy", "Control browser proxy settings")],
        P::Sessions => &[("sessions", "Access recently closed tabs")],
        P::TabHide => &[("tab_hide", "Hide and show browser tabs")],
        P::Tabs => &[("tabs", "Access browser tabs")],
        P::WebNavigation => &[(
            "web_navigation",
            "Access browser activity during navigation",
        )],
        _ => &[],
    }
}

fn host_warnings(hosts: &MatchPatternSet, kind: ExtensionKind, force_all: bool) -> Vec<Warning> {
    let all = force_all || hosts.iter().any(|p| matches!(p.host(), HostPattern::Any));
    let (all_text, lead) = match kind {
        ExtensionKind::Chrome => (CHROME_ALL_HOSTS, "Read and change your data on"),
        ExtensionKind::Firefox => (FIREFOX_ALL_HOSTS, "Access your data for"),
    };
    if all {
        return vec![Warning {
            id: "all_hosts",
            text: all_text.to_owned(),
            details: Vec::new(),
        }];
    }
    let mut names: BTreeSet<String> = BTreeSet::new();
    let mut files = false;
    for p in hosts {
        match p.host() {
            HostPattern::Subdomains(_) | HostPattern::Exact(_) => {
                if let Some(h) = p.display_host() {
                    names.insert(h);
                }
            }
            HostPattern::None => {
                if p.schemes().contains(&crate::match_pattern::Scheme::File) {
                    files = true;
                }
            }
            HostPattern::Any => {}
        }
    }
    let mut out = Vec::new();
    let names: Vec<String> = names.into_iter().collect();
    if !names.is_empty() {
        let (text, details) = match names.as_slice() {
            [a] => (format!("{lead} {a}"), Vec::new()),
            [a, b] => (format!("{lead} {a} and {b}"), Vec::new()),
            [a, b, c] => (format!("{lead} {a}, {b}, and {c}"), Vec::new()),
            many => (
                match kind {
                    ExtensionKind::Chrome => {
                        "Read and change your data on a number of websites".to_owned()
                    }
                    ExtensionKind::Firefox => "Access your data on a number of websites".to_owned(),
                },
                many.iter().take(MAX_DETAIL_LINES).cloned().collect(),
            ),
        };
        out.push(Warning {
            id: "hosts",
            text,
            details,
        });
    }
    if files {
        out.push(Warning {
            id: "file_hosts",
            text: "Read and change your data in files on this computer".to_owned(),
            details: Vec::new(),
        });
    }
    out
}

/// The warnings to show for `set`, for a Chrome or Firefox style extension, deduplicated by
/// message id and in a fixed order: host warnings first, then API warnings in permission order.
pub fn install_warnings(set: &PermissionSet, kind: ExtensionKind) -> Vec<Warning> {
    use ApiPermission as P;
    let forced_all = kind == ExtensionKind::Chrome
        && set
            .api
            .iter()
            .any(|p| chrome_messages(p).iter().any(|(id, _)| *id == "all_hosts"));
    let all_hosts = forced_all
        || set
            .hosts
            .iter()
            .any(|p| matches!(p.host(), HostPattern::Any));
    let mut out = host_warnings(&set.hosts, kind, forced_all);
    let mut seen: BTreeSet<&'static str> = out.iter().map(|w| w.id).collect();
    let has_history = set.api.contains(&P::History);
    let has_tabs = set.api.contains(&P::Tabs);
    let has_sessions = set.api.contains(&P::Sessions);
    for p in &set.api {
        let messages = match kind {
            ExtensionKind::Chrome => chrome_messages(p),
            ExtensionKind::Firefox => firefox_messages(p),
        };
        for (id, text) in messages {
            if *id == "all_hosts" {
                continue;
            }
            if kind == ExtensionKind::Chrome
                && *id == "browsing_history_read"
                && (all_hosts || has_history || (has_sessions && has_tabs))
            {
                continue;
            }
            if seen.insert(id) {
                out.push(Warning {
                    id,
                    text: (*text).to_owned(),
                    details: Vec::new(),
                });
            }
        }
    }
    if kind == ExtensionKind::Chrome
        && has_sessions
        && has_tabs
        && !has_history
        && !all_hosts
        && seen.insert("browsing_history_signed_in")
    {
        out.push(Warning {
            id: "browsing_history_signed_in",
            text: "Read your browsing history on all your signed-in devices".to_owned(),
            details: Vec::new(),
        });
    }
    out
}

// ---- sets --------------------------------------------------------------------------------

/// API permissions plus host patterns.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PermissionSet {
    /// API permissions.
    pub api: BTreeSet<ApiPermission>,
    /// Host patterns.
    pub hosts: MatchPatternSet,
}

impl PermissionSet {
    /// An empty set.
    pub fn new() -> Self {
        Self::default()
    }

    /// Whether the set holds nothing.
    pub fn is_empty(&self) -> bool {
        self.api.is_empty() && self.hosts.is_empty()
    }

    /// Everything in either set.
    #[must_use]
    pub fn union(&self, other: &PermissionSet) -> PermissionSet {
        PermissionSet {
            api: self.api.union(&other.api).cloned().collect(),
            hosts: self.hosts.union(&other.hosts),
        }
    }

    /// What is in this set but not in `other` (hosts: not covered by `other`).
    #[must_use]
    pub fn difference(&self, other: &PermissionSet) -> PermissionSet {
        PermissionSet {
            api: self.api.difference(&other.api).cloned().collect(),
            hosts: self.hosts.difference(&other.hosts),
        }
    }

    /// Whether this set holds everything `other` does.
    pub fn contains(&self, other: &PermissionSet) -> bool {
        other.api.is_subset(&self.api) && self.hosts.covers_set(&other.hosts)
    }

    /// Whether `other` holds everything this set does.
    pub fn is_subset(&self, other: &PermissionSet) -> bool {
        other.contains(self)
    }

    /// Whether the API permission is in the set.
    pub fn contains_api(&self, permission: &ApiPermission) -> bool {
        self.api.contains(permission)
    }

    /// Whether the hosts cover `pattern`.
    pub fn covers_host(&self, pattern: &MatchPattern) -> bool {
        self.hosts.covers(&pattern.with_any_path())
    }
}

/// What a manifest asks for.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ManifestPermissions {
    /// Granted at install: `permissions` and the required hosts. Permissions that cannot be
    /// listed there, or do not exist in this manifest version, are left out (the parse already
    /// warned).
    pub required: PermissionSet,
    /// What may be asked for later.
    pub optional: PermissionSet,
    /// The hosts of `content_scripts` (paths dropped): they count for warnings and updates, and
    /// manifest-declared scripts may run there.
    pub scriptable_hosts: MatchPatternSet,
}

impl From<&Manifest> for ManifestPermissions {
    fn from(m: &Manifest) -> Self {
        let keep = |list: &[ApiPermission], which: PermissionList| -> BTreeSet<ApiPermission> {
            list.iter()
                .filter(|p| p.placement_ok(which) && p.available_in(m.manifest_version, m.kind))
                .cloned()
                .collect()
        };
        let mut scriptable = Vec::new();
        for cs in &m.content_scripts {
            scriptable.extend(cs.matches.iter().map(MatchPattern::with_any_path));
        }
        ManifestPermissions {
            required: PermissionSet {
                api: keep(&m.permissions, PermissionList::Required),
                hosts: m.host_permissions.clone(),
            },
            optional: PermissionSet {
                api: keep(&m.optional_permissions, PermissionList::Optional),
                hosts: m.optional_host_permissions.clone(),
            },
            scriptable_hosts: MatchPatternSet::from_patterns(scriptable).unwrap_or_default(),
        }
    }
}

impl ManifestPermissions {
    /// The set the install prompt and update consent look at: the required permissions plus the
    /// content script hosts (Chrome warns for both).
    pub fn warning_set(&self) -> PermissionSet {
        PermissionSet {
            api: self.required.api.clone(),
            hosts: self.required.hosts.union(&self.scriptable_hosts),
        }
    }
}

// ---- site access -------------------------------------------------------------------------

/// Mercury's defaults for permission state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Defaults {
    /// Start with broad host patterns (`<all_urls>`, `*://*/*`, whole top-level domains) withheld,
    /// so the extension runs "on click" until the user allows it. Chrome grants requested hosts as
    /// requested; Mercury is conservative, so the default is `true`.
    pub withhold_broad_hosts: bool,
}

impl Default for Defaults {
    fn default() -> Self {
        Defaults {
            withhold_broad_hosts: true,
        }
    }
}

/// The site-access setting of an extension (Chrome's "on click / on specific sites / on all
/// sites").
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SiteAccess {
    /// No standing access; only `activeTab` after a gesture.
    OnClick,
    /// Access on these sites only.
    OnSpecificSites(MatchPatternSet),
    /// Access everywhere the extension asked for, broad hosts included.
    OnAllSites,
}

const MAX_DECISIONS: usize = 1_000;

/// The host access of one extension: what it asked for, what the user allowed and withheld.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostGrants {
    requested: MatchPatternSet,
    optional: MatchPatternSet,
    base: MatchPatternSet,
    decisions: Vec<(MatchPattern, bool)>,
}

impl HostGrants {
    /// The initial state: `requested` hosts (minus broad ones when the defaults withhold them) are
    /// granted; `optional` hosts can be granted later.
    pub fn new(
        requested: &MatchPatternSet,
        optional: &MatchPatternSet,
        defaults: &Defaults,
    ) -> Self {
        let base = if defaults.withhold_broad_hosts {
            MatchPatternSet::from_patterns(requested.iter().filter(|p| !p.is_broad()).cloned())
                .unwrap_or_default()
        } else {
            requested.clone()
        };
        HostGrants {
            requested: requested.clone(),
            optional: optional.clone(),
            base,
            decisions: Vec::new(),
        }
    }

    fn grantable(&self) -> MatchPatternSet {
        self.requested.union(&self.optional)
    }

    fn push_decision(&mut self, pattern: MatchPattern, allow: bool) {
        self.decisions.retain(|(q, _)| !pattern.covers(q));
        if self.decisions.len() >= MAX_DECISIONS {
            self.decisions.remove(0);
        }
        self.decisions.push((pattern, allow));
    }

    /// Grants access to `pattern` (its path is ignored); it must be within what the manifest
    /// requested or listed as optional.
    pub fn grant(&mut self, pattern: MatchPattern) -> Result<(), PermissionError> {
        let pattern = pattern.with_any_path();
        if !self.grantable().covers(&pattern) {
            return Err(PermissionError::NotRequested);
        }
        self.push_decision(pattern, true);
        Ok(())
    }

    /// Withholds access to `pattern`: the user turned it off.
    pub fn withhold(&mut self, pattern: MatchPattern) {
        self.push_decision(pattern.with_any_path(), false);
    }

    /// Whether the extension may act on `url` by host permission. This is authoritative.
    pub fn allows(&self, url: &Url) -> bool {
        for (pattern, allow) in self.decisions.iter().rev() {
            if pattern.matches(url) {
                return *allow;
            }
        }
        self.base.matches(url)
    }

    /// The effective patterns, for display: the base grants and later grants, minus whatever a
    /// later withhold covers entirely. A pattern that is only partly withheld stays listed;
    /// [`HostGrants::allows`] is the truth.
    pub fn effective_hosts(&self) -> MatchPatternSet {
        let mut set = self.base.clone();
        for (pattern, allow) in &self.decisions {
            if *allow {
                set.insert(pattern.clone());
            } else {
                set.remove_covered_by(pattern);
            }
        }
        set
    }

    /// The setting the effective hosts amount to.
    pub fn site_access(&self) -> SiteAccess {
        let eff = self.effective_hosts();
        if eff.is_empty() {
            SiteAccess::OnClick
        } else if eff.has_broad() {
            SiteAccess::OnAllSites
        } else {
            SiteAccess::OnSpecificSites(eff)
        }
    }

    /// Applies a site-access choice, forgetting earlier per-site decisions.
    pub fn set_site_access(&mut self, access: SiteAccess) {
        self.decisions.clear();
        self.base = match access {
            SiteAccess::OnClick => MatchPatternSet::new(),
            SiteAccess::OnAllSites => self.requested.clone(),
            SiteAccess::OnSpecificSites(sites) => {
                let grantable = self.grantable();
                let mut chosen = Vec::new();
                for q in &sites {
                    let q = q.with_any_path();
                    if grantable.covers(&q) {
                        chosen.push(q);
                    }
                }
                for r in &self.requested {
                    if sites.covers(r) {
                        chosen.push(r.clone());
                    }
                }
                MatchPatternSet::from_patterns(chosen).unwrap_or_default()
            }
        };
    }
}

// ---- activeTab ---------------------------------------------------------------------------

/// A tab, as the shell numbers them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TabId(pub i64);

/// Proof that a real user gesture happened. Only the shell may make one, from actual input (a
/// click on the extension's action or a context-menu item, a key press for a command, accepting
/// an omnibox suggestion); extension code, background messages, alarms and page loads never
/// produce one. The field is private so the value cannot be built by struct literal.
#[derive(Debug)]
pub struct UserGesture {
    _private: (),
}

impl UserGesture {
    /// Records that the user just did something. **The shell's job**: call this only from the
    /// handler of a real input event.
    pub fn from_user_input() -> UserGesture {
        UserGesture { _private: () }
    }
}

/// What the user did to grant `activeTab`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GrantReason {
    /// Clicked the extension's toolbar action.
    ActionClick,
    /// Chose one of the extension's context-menu items.
    ContextMenu,
    /// Pressed one of the extension's keyboard commands.
    KeyboardCommand,
    /// Accepted an omnibox suggestion of the extension.
    Omnibox,
}

/// One `activeTab` grant: a tab and an origin.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActiveTabGrant {
    /// The origin (`scheme://host:port`) the grant covers.
    pub origin: String,
    /// What the user did.
    pub reason: GrantReason,
}

/// Why an `activeTab` grant was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ActiveTabError {
    /// The page is a blocked surface.
    #[error("the page is a blocked surface")]
    Blocked(BlockedSurface),
    /// The page has no web origin (for example a file or data URL).
    #[error("the page has no web origin")]
    NoOrigin,
    /// The extension does not have the `activeTab` permission.
    #[error("the extension does not have activeTab")]
    NotDeclared,
}

fn origin_key(url: &Url) -> Option<String> {
    match url.scheme() {
        "http" | "https" | "ws" | "wss" | "ftp" => {
            let host = url.host()?;
            Some(format!(
                "{}://{}:{}",
                url.scheme(),
                host,
                url.port_or_known_default()?
            ))
        }
        _ => None,
    }
}

/// The `activeTab` grants of one extension. A grant covers one tab and the origin of the page that
/// was there; it ends when the tab navigates to another document (even of the same origin), when
/// the tab closes, and it never covers another origin, a blocked surface, or a file.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ActiveTabGrants {
    grants: BTreeMap<TabId, ActiveTabGrant>,
}

impl ActiveTabGrants {
    /// No grants.
    pub fn new() -> Self {
        Self::default()
    }

    /// Grants `activeTab` for `tab`, whose top page is `url`. Needs the proof of a gesture.
    pub fn grant(
        &mut self,
        tab: TabId,
        url: &Url,
        reason: GrantReason,
        _gesture: &UserGesture,
    ) -> Result<(), ActiveTabError> {
        if let SurfaceClass::Blocked(b) = classify_surface(url, None, true) {
            return Err(ActiveTabError::Blocked(b));
        }
        let origin = origin_key(url).ok_or(ActiveTabError::NoOrigin)?;
        self.grants.insert(tab, ActiveTabGrant { origin, reason });
        Ok(())
    }

    /// The grant of a tab, if any.
    pub fn get(&self, tab: TabId) -> Option<&ActiveTabGrant> {
        self.grants.get(&tab)
    }

    /// Whether `tab` has a grant covering `url`'s origin.
    pub fn covers(&self, tab: TabId, url: &Url) -> bool {
        if matches!(classify_surface(url, None, false), SurfaceClass::Blocked(_)) {
            return false;
        }
        match (self.grants.get(&tab), origin_key(url)) {
            (Some(g), Some(origin)) => g.origin == origin,
            _ => false,
        }
    }

    /// The tab navigated. A same-document navigation (fragment, history API) keeps the grant;
    /// any other navigation ends it, even to the same origin.
    pub fn on_navigation(&mut self, tab: TabId, _new_url: &Url, same_document: bool) {
        if !same_document {
            self.grants.remove(&tab);
        }
    }

    /// The tab closed.
    pub fn on_tab_closed(&mut self, tab: TabId) {
        self.grants.remove(&tab);
    }

    /// Ends one grant.
    pub fn revoke(&mut self, tab: TabId) {
        self.grants.remove(&tab);
    }

    /// Ends every grant (for example when the extension is disabled).
    pub fn clear(&mut self) {
        self.grants.clear();
    }

    /// How many tabs have a grant.
    pub fn len(&self) -> usize {
        self.grants.len()
    }

    /// Whether no tab has a grant.
    pub fn is_empty(&self) -> bool {
        self.grants.is_empty()
    }
}

// ---- optional permissions ----------------------------------------------------------------

/// Why a permission change was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum PermissionError {
    /// A permission request needs a user gesture.
    #[error("a permission request needs a user gesture")]
    NoUserGesture,
    /// The request names nothing.
    #[error("the permission request is empty")]
    EmptyRequest,
    /// The request is larger than the cap.
    #[error("the permission request is too large")]
    TooManyItems,
    /// Something requested is not listed in the manifest's optional permissions.
    #[error("a requested permission is not listed as optional in the manifest")]
    NotOptional,
    /// Something to remove is required by the manifest.
    #[error("a required permission cannot be removed")]
    RemoveRequired,
    /// A host grant is outside what the manifest requested or listed as optional.
    #[error("the host is not requested by the manifest")]
    NotRequested,
}

/// A `permissions.request` or `permissions.remove` argument.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PermissionRequest {
    /// API permissions.
    pub api: Vec<ApiPermission>,
    /// Host patterns.
    pub hosts: Vec<MatchPattern>,
}

/// What to do about a permission request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PermissionRequestPlan {
    /// What the user must be asked about (not yet granted).
    pub to_prompt: PermissionSet,
    /// What is already in place.
    pub already_granted: PermissionSet,
    /// The prompt lines for exactly `to_prompt`.
    pub warnings: Vec<Warning>,
}

fn request_limits(request: &PermissionRequest) -> Result<(), PermissionError> {
    if request.api.is_empty() && request.hosts.is_empty() {
        return Err(PermissionError::EmptyRequest);
    }
    if request.api.len() > MAX_PATTERNS_PER_SET || request.hosts.len() > MAX_PATTERNS_PER_SET {
        return Err(PermissionError::TooManyItems);
    }
    Ok(())
}

/// Plans a `permissions.request`. Refuses without a gesture, refuses anything the manifest does
/// not list as optional (a host is fine when an optional pattern covers it; a required one is
/// already part of the install), and returns only what is not granted yet.
pub fn plan_permission_request(
    manifest: &Manifest,
    current_grants: &PermissionSet,
    requested: &PermissionRequest,
    gesture: Option<&UserGesture>,
) -> Result<PermissionRequestPlan, PermissionError> {
    if gesture.is_none() {
        return Err(PermissionError::NoUserGesture);
    }
    request_limits(requested)?;
    let declared = ManifestPermissions::from(manifest);
    let mut to_prompt = PermissionSet::new();
    let mut already = PermissionSet::new();
    for api in &requested.api {
        let granted_optional =
            current_grants.api.contains(api) && declared.optional.api.contains(api);
        if declared.required.api.contains(api) || granted_optional {
            already.api.insert(api.clone());
        } else if declared.optional.api.contains(api) {
            to_prompt.api.insert(api.clone());
        } else {
            return Err(PermissionError::NotOptional);
        }
    }
    for host in &requested.hosts {
        let host = host.with_any_path();
        let allowed = declared.optional.hosts.covers(&host)
            || declared.required.hosts.covers(&host)
            || declared.scriptable_hosts.covers(&host);
        if !allowed {
            return Err(PermissionError::NotOptional);
        }
        if current_grants.hosts.covers(&host) {
            already.hosts.insert(host);
        } else {
            to_prompt.hosts.insert(host);
        }
    }
    let warnings = install_warnings(&to_prompt, manifest.kind);
    Ok(PermissionRequestPlan {
        to_prompt,
        already_granted: already,
        warnings,
    })
}

/// Plans a `permissions.remove`: returns what to remove, or refuses when something is required
/// (an API permission the manifest lists under `permissions`, or a host that overlaps a required
/// or content script host; the overlap test is conservative).
pub fn plan_permission_removal(
    manifest: &Manifest,
    request: &PermissionRequest,
) -> Result<PermissionSet, PermissionError> {
    request_limits(request)?;
    let declared = ManifestPermissions::from(manifest);
    let mut out = PermissionSet::new();
    for api in &request.api {
        if declared.required.api.contains(api) {
            return Err(PermissionError::RemoveRequired);
        }
        out.api.insert(api.clone());
    }
    for host in &request.hosts {
        let host = host.with_any_path();
        if declared.required.hosts.overlaps(&host) || declared.scriptable_hosts.overlaps(&host) {
            return Err(PermissionError::RemoveRequired);
        }
        out.hosts.insert(host);
    }
    Ok(out)
}

// ---- private windows ---------------------------------------------------------------------

/// The user's choice about running in private windows (default: not allowed).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum IncognitoPolicy {
    /// The extension does not run in private windows (the default).
    #[default]
    NotAllowed,
    /// The user turned it on for this extension.
    AllowedByUser,
}

/// Whether the extension may run in a private window: never when its manifest says
/// `incognito: "not_allowed"`, otherwise only when the user allowed it.
pub fn may_run_in_private_window(manifest: &Manifest, user_choice: IncognitoPolicy) -> bool {
    manifest.incognito != Incognito::NotAllowed && user_choice == IncognitoPolicy::AllowedByUser
}

// ---- blocked surfaces and the access check -----------------------------------------------

/// A page extensions can never touch, whatever was granted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BlockedSurface {
    /// `mercury:` and `about:` pages.
    InternalPage,
    /// A `mercury-extension:` page of another extension.
    OtherExtensionPage,
    /// `view-source:`.
    ViewSource,
    /// A top-level `data:` document.
    DataDocument,
    /// A top-level `blob:` document.
    BlobDocument,
    /// An extension store page.
    StorePage,
    /// Any other scheme (`chrome:`, `moz-extension:`, `javascript:`, ...): fail closed.
    UnsupportedScheme,
}

/// What kind of page a URL is, for the access check.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SurfaceClass {
    /// An ordinary web (or ftp, ws, urn, framed data) page: host permissions decide.
    Web,
    /// A page of the extension itself.
    OwnExtensionPage,
    /// A local file: needs the user's "Allow access to file URLs".
    File,
    /// Never accessible.
    Blocked(BlockedSurface),
}

fn percent_decode_lossy(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes.get(i).copied().unwrap_or(0);
        if b == b'%' {
            let hi = bytes.get(i + 1).and_then(|c| char::from(*c).to_digit(16));
            let lo = bytes.get(i + 2).and_then(|c| char::from(*c).to_digit(16));
            if let (Some(hi), Some(lo)) = (hi, lo)
                && let Ok(v) = u8::try_from(hi * 16 + lo)
            {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(b);
        i += 1;
    }
    String::from_utf8_lossy(&out).to_ascii_lowercase()
}

fn first_path_segment(url: &Url) -> String {
    let decoded = percent_decode_lossy(url.path());
    decoded
        .trim_start_matches('/')
        .split('/')
        .next()
        .unwrap_or("")
        .to_owned()
}

fn is_store_page(url: &Url) -> bool {
    let Some(host) = url.host_str() else {
        return false;
    };
    let host = host.trim_end_matches('.').to_ascii_lowercase();
    let under = |base: &str| host == base || host.ends_with(&format!(".{base}"));
    if under("chromewebstore.google.com") || under("addons.mozilla.org") {
        return true;
    }
    if host == "chrome.google.com" && first_path_segment(url) == "webstore" {
        return true;
    }
    host == "microsoftedge.microsoft.com" && first_path_segment(url) == "addons"
}

/// Classifies `url`. `own` is the extension asking (its own pages are fine); `frame_is_top` says
/// whether the document is a top-level one (`data:` and `blob:` are blocked only then).
pub fn classify_surface(url: &Url, own: Option<&ExtensionId>, frame_is_top: bool) -> SurfaceClass {
    match url.scheme() {
        "mercury" | "about" => SurfaceClass::Blocked(BlockedSurface::InternalPage),
        "view-source" => SurfaceClass::Blocked(BlockedSurface::ViewSource),
        "mercury-extension" => {
            let mine = own.is_some_and(|e| url.host_str() == Some(e.host_label().as_str()));
            if mine {
                SurfaceClass::OwnExtensionPage
            } else {
                SurfaceClass::Blocked(BlockedSurface::OtherExtensionPage)
            }
        }
        "data" if frame_is_top => SurfaceClass::Blocked(BlockedSurface::DataDocument),
        "blob" if frame_is_top => SurfaceClass::Blocked(BlockedSurface::BlobDocument),
        "data" | "blob" | "urn" => SurfaceClass::Web,
        "file" => SurfaceClass::File,
        "http" | "https" | "ws" | "wss" | "ftp" => {
            if is_store_page(url) {
                SurfaceClass::Blocked(BlockedSurface::StorePage)
            } else {
                SurfaceClass::Web
            }
        }
        _ => SurfaceClass::Blocked(BlockedSurface::UnsupportedScheme),
    }
}

/// The user's "Allow access to file URLs" toggle for one extension.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum FileAccess {
    /// `file:` pages are refused (the default).
    #[default]
    Denied,
    /// The user allowed `file:` pages.
    Allowed,
}

/// Everything the access check needs to know about one extension.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtensionPermissionState {
    /// The extension.
    pub extension: ExtensionId,
    /// Granted API permissions.
    pub api: BTreeSet<ApiPermission>,
    /// Host access.
    pub host_grants: HostGrants,
    /// `activeTab` grants.
    pub active_tab: ActiveTabGrants,
    /// "Allow access to file URLs".
    pub file_access: FileAccess,
    /// The manifest's `incognito` value.
    pub manifest_incognito: Incognito,
    /// The user's private-window choice.
    pub incognito_choice: IncognitoPolicy,
}

impl ExtensionPermissionState {
    /// The state right after install: the manifest's required permissions and hosts (broad ones
    /// withheld by the defaults), no `activeTab` grants, files and private windows off.
    pub fn new(extension: ExtensionId, manifest: &Manifest, defaults: &Defaults) -> Self {
        let declared = ManifestPermissions::from(manifest);
        let requested = declared.required.hosts.union(&declared.scriptable_hosts);
        ExtensionPermissionState {
            extension,
            api: declared.required.api.clone(),
            host_grants: HostGrants::new(&requested, &declared.optional.hosts, defaults),
            active_tab: ActiveTabGrants::new(),
            file_access: FileAccess::Denied,
            manifest_incognito: manifest.incognito,
            incognito_choice: IncognitoPolicy::NotAllowed,
        }
    }

    /// Grants `activeTab` for a tab after a user gesture; refuses when the extension does not have
    /// the permission.
    pub fn grant_active_tab(
        &mut self,
        tab: TabId,
        url: &Url,
        reason: GrantReason,
        gesture: &UserGesture,
    ) -> Result<(), ActiveTabError> {
        if !self.api.contains(&ApiPermission::ActiveTab) {
            return Err(ActiveTabError::NotDeclared);
        }
        self.active_tab.grant(tab, url, reason, gesture)
    }

    fn may_run_private(&self) -> bool {
        self.manifest_incognito != Incognito::NotAllowed
            && self.incognito_choice == IncognitoPolicy::AllowedByUser
    }
}

/// What the check is asked about.
#[derive(Debug, Clone, Copy)]
pub struct AccessContext<'a> {
    /// The extension that wants access.
    pub extension: &'a ExtensionId,
    /// The page.
    pub url: &'a Url,
    /// The tab the page is in, if any.
    pub tab: Option<TabId>,
    /// Whether the page is in a private window.
    pub is_private_window: bool,
    /// Whether the document is the top-level one of its tab.
    pub frame_is_top: bool,
}

/// Why access was allowed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AllowReason {
    /// A page of the extension itself.
    OwnPage,
    /// A host permission the user has not withheld.
    HostPermission,
    /// A live `activeTab` grant for this tab and origin.
    ActiveTab,
}

/// Why access was denied.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DenyReason {
    /// The state belongs to another extension.
    WrongExtension,
    /// A private window and the extension is not allowed there.
    PrivateWindowNotAllowed,
    /// A blocked surface.
    Blocked(BlockedSurface),
    /// A `file:` page and "Allow access to file URLs" is off.
    FileAccessNotGranted,
    /// No host permission and no `activeTab` grant covers the page.
    NoHostPermission,
}

/// The answer of [`can_access`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Access {
    /// Allowed, and why.
    Allowed(AllowReason),
    /// Denied, and why.
    Denied(DenyReason),
}

impl Access {
    /// Whether access is allowed.
    pub fn is_allowed(&self) -> bool {
        matches!(self, Access::Allowed(_))
    }
}

/// THE check: may this extension inject into, fetch from or observe this page? Blocked surfaces
/// are refused before anything else is looked at.
pub fn can_access(ctx: &AccessContext<'_>, state: &ExtensionPermissionState) -> Access {
    if ctx.extension != &state.extension {
        return Access::Denied(DenyReason::WrongExtension);
    }
    let class = classify_surface(ctx.url, Some(&state.extension), ctx.frame_is_top);
    if let SurfaceClass::Blocked(b) = class {
        return Access::Denied(DenyReason::Blocked(b));
    }
    if ctx.is_private_window && !state.may_run_private() {
        return Access::Denied(DenyReason::PrivateWindowNotAllowed);
    }
    match class {
        SurfaceClass::OwnExtensionPage => return Access::Allowed(AllowReason::OwnPage),
        SurfaceClass::File if state.file_access != FileAccess::Allowed => {
            return Access::Denied(DenyReason::FileAccessNotGranted);
        }
        _ => {}
    }
    if state.host_grants.allows(ctx.url) {
        return Access::Allowed(AllowReason::HostPermission);
    }
    if class != SurfaceClass::File
        && let Some(tab) = ctx.tab
        && state.api.contains(&ApiPermission::ActiveTab)
        && state.active_tab.covers(tab, ctx.url)
    {
        return Access::Allowed(AllowReason::ActiveTab);
    }
    Access::Denied(DenyReason::NoHostPermission)
}

// ---- kill switch -------------------------------------------------------------------------

/// Crash breaker settings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KillSwitchConfig {
    /// Crashes within `window` that disable an extension (default 3).
    pub max_crashes: u32,
    /// The window (default 10 minutes).
    pub window: Duration,
}

impl Default for KillSwitchConfig {
    fn default() -> Self {
        KillSwitchConfig {
            max_crashes: 3,
            window: Duration::from_secs(600),
        }
    }
}

/// Whether an extension may run, as far as the kill switch is concerned.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KillState {
    /// It may run.
    Enabled,
    /// "Disable all extensions" is on.
    GloballyDisabled,
    /// The crash breaker tripped; only [`KillSwitch::reset`] (a user action) re-enables it.
    CrashLoop,
}

/// What recording a crash did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CrashOutcome {
    /// Counted; this many crashes are inside the window.
    Recorded {
        /// Crashes inside the window, this one included.
        count: u32,
    },
    /// This crash tripped the breaker.
    Disabled,
    /// It was already disabled.
    AlreadyDisabled,
}

/// The global kill switch and the per-extension crash breaker. Time is passed in.
#[derive(Debug, Clone, Default)]
pub struct KillSwitch {
    config: KillSwitchConfig,
    all_disabled: bool,
    crashes: BTreeMap<ExtensionId, Vec<SystemTime>>,
    tripped: BTreeSet<ExtensionId>,
}

impl KillSwitch {
    /// A switch with the given settings.
    pub fn new(config: KillSwitchConfig) -> Self {
        KillSwitch {
            config,
            ..KillSwitch::default()
        }
    }

    /// Turns "disable all extensions" on or off.
    pub fn set_all_disabled(&mut self, disabled: bool) {
        self.all_disabled = disabled;
    }

    /// Whether "disable all extensions" is on.
    pub fn all_disabled(&self) -> bool {
        self.all_disabled
    }

    /// Records a crash at `now`.
    pub fn record_crash(&mut self, extension: &ExtensionId, now: SystemTime) -> CrashOutcome {
        if self.tripped.contains(extension) {
            return CrashOutcome::AlreadyDisabled;
        }
        let window = self.config.window;
        let max = usize::try_from(self.config.max_crashes.max(1)).unwrap_or(usize::MAX);
        let list = self.crashes.entry(extension.clone()).or_default();
        list.retain(|t| match now.duration_since(*t) {
            Ok(age) => age <= window,
            Err(_) => true,
        });
        list.push(now);
        if list.len() >= max {
            self.crashes.remove(extension);
            self.tripped.insert(extension.clone());
            CrashOutcome::Disabled
        } else {
            CrashOutcome::Recorded {
                count: u32::try_from(list.len()).unwrap_or(u32::MAX),
            }
        }
    }

    /// The state of one extension.
    pub fn state(&self, extension: &ExtensionId) -> KillState {
        if self.all_disabled {
            KillState::GloballyDisabled
        } else if self.tripped.contains(extension) {
            KillState::CrashLoop
        } else {
            KillState::Enabled
        }
    }

    /// Whether the extension may run.
    pub fn is_enabled(&self, extension: &ExtensionId) -> bool {
        self.state(extension) == KillState::Enabled
    }

    /// Re-enables an extension after a user action and forgets its crashes.
    pub fn reset(&mut self, extension: &ExtensionId) {
        self.tripped.remove(extension);
        self.crashes.remove(extension);
    }

    /// Forgets an extension (it was uninstalled).
    pub fn forget(&mut self, extension: &ExtensionId) {
        self.reset(extension);
    }
}

// ---- updates -----------------------------------------------------------------------------

/// What an update adds.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PermissionDelta {
    /// API permissions the old version did not have.
    pub added_api: BTreeSet<ApiPermission>,
    /// Host patterns the old version's hosts do not cover.
    pub added_hosts: MatchPatternSet,
}

impl PermissionDelta {
    /// Whether nothing was added.
    pub fn is_empty(&self) -> bool {
        self.added_api.is_empty() && self.added_hosts.is_empty()
    }

    /// The prompt lines for exactly what was added.
    pub fn warnings(&self, kind: ExtensionKind) -> Vec<Warning> {
        install_warnings(
            &PermissionSet {
                api: self.added_api.clone(),
                hosts: self.added_hosts.clone(),
            },
            kind,
        )
    }
}

/// What `new` asks for that `old` did not: required API permissions and required or content
/// script hosts (a host that an old pattern covers does not count).
pub fn permissions_added(old: &Manifest, new: &Manifest) -> PermissionDelta {
    let old_set = ManifestPermissions::from(old).warning_set();
    let new_set = ManifestPermissions::from(new).warning_set();
    let diff = new_set.difference(&old_set);
    PermissionDelta {
        added_api: diff.api,
        added_hosts: diff.hosts,
    }
}

/// Whether an update must wait for the user: it adds any API permission or host.
pub fn update_needs_consent(delta: &PermissionDelta) -> bool {
    !delta.is_empty()
}
