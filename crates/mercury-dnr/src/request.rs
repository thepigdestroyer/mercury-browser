// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! What the evaluator needs to know about a request: the EC38 metadata, resource types and methods.
//!
//! [`RequestMetadata`] mirrors the fields the net layer records for every request (`top_level_site`,
//! `initiator_origin`, `is_third_party`, see `servo-embedder-traits/mercury/net.rs` on lane net-1). This
//! crate does not depend on that crate; the engine lane converts.
//!
//! Behaviour per <https://developer.chrome.com/docs/extensions/reference/api/declarativeNetRequest>
//! (`ResourceType`, `RequestMethod`, `RuleCondition`).

use url::Url;

/// A network request's resource type (Chrome's `declarativeNetRequest.ResourceType`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ResourceType {
    /// A top-level navigation.
    MainFrame,
    /// A frame or iframe navigation.
    SubFrame,
    /// A stylesheet.
    Stylesheet,
    /// A script.
    Script,
    /// An image.
    Image,
    /// A font.
    Font,
    /// An `<object>` or `<embed>`.
    Object,
    /// `XMLHttpRequest` and `fetch()`.
    XmlHttpRequest,
    /// A hyperlink audit ping (`ping=`, `sendBeacon`).
    Ping,
    /// A CSP violation report.
    CspReport,
    /// Audio or video.
    Media,
    /// A WebSocket.
    WebSocket,
    /// A WebTransport session.
    WebTransport,
    /// A web bundle.
    WebBundle,
    /// Anything else.
    Other,
}

impl ResourceType {
    /// Every type, in declaration order.
    pub const ALL: [ResourceType; 15] = [
        ResourceType::MainFrame,
        ResourceType::SubFrame,
        ResourceType::Stylesheet,
        ResourceType::Script,
        ResourceType::Image,
        ResourceType::Font,
        ResourceType::Object,
        ResourceType::XmlHttpRequest,
        ResourceType::Ping,
        ResourceType::CspReport,
        ResourceType::Media,
        ResourceType::WebSocket,
        ResourceType::WebTransport,
        ResourceType::WebBundle,
        ResourceType::Other,
    ];

    /// The name used in rule JSON.
    pub fn name(self) -> &'static str {
        match self {
            ResourceType::MainFrame => "main_frame",
            ResourceType::SubFrame => "sub_frame",
            ResourceType::Stylesheet => "stylesheet",
            ResourceType::Script => "script",
            ResourceType::Image => "image",
            ResourceType::Font => "font",
            ResourceType::Object => "object",
            ResourceType::XmlHttpRequest => "xmlhttprequest",
            ResourceType::Ping => "ping",
            ResourceType::CspReport => "csp_report",
            ResourceType::Media => "media",
            ResourceType::WebSocket => "websocket",
            ResourceType::WebTransport => "webtransport",
            ResourceType::WebBundle => "webbundle",
            ResourceType::Other => "other",
        }
    }

    /// Parses the name used in rule JSON (exact, lowercase).
    pub fn from_name(name: &str) -> Option<ResourceType> {
        ResourceType::ALL.into_iter().find(|t| t.name() == name)
    }

    /// Whether this is a document load (`main_frame` or `sub_frame`).
    pub fn is_frame(self) -> bool {
        matches!(self, ResourceType::MainFrame | ResourceType::SubFrame)
    }

    fn bit(self) -> u16 {
        1u16 << (self as u32)
    }
}

/// A set of [`ResourceType`]s.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ResourceTypeSet(u16);

impl ResourceTypeSet {
    /// The empty set.
    pub const fn empty() -> Self {
        ResourceTypeSet(0)
    }

    /// Every type.
    pub fn all() -> Self {
        ResourceType::ALL.into_iter().collect()
    }

    /// Every type except `main_frame`: what a rule matches when it names no resource types (as Chrome).
    pub fn rule_default() -> Self {
        let mut set = Self::all();
        set.remove(ResourceType::MainFrame);
        set
    }

    /// Adds a type.
    pub fn insert(&mut self, t: ResourceType) {
        self.0 |= t.bit();
    }

    /// Removes a type.
    pub fn remove(&mut self, t: ResourceType) {
        self.0 &= !t.bit();
    }

    /// Whether the type is in the set.
    pub fn contains(self, t: ResourceType) -> bool {
        self.0 & t.bit() != 0
    }

    /// Whether the set has no member.
    pub fn is_empty(self) -> bool {
        self.0 == 0
    }

    /// Whether the two sets share a member.
    pub fn intersects(self, other: Self) -> bool {
        self.0 & other.0 != 0
    }

    /// Whether every member of `self` is in `other`.
    pub fn is_subset_of(self, other: Self) -> bool {
        self.0 & !other.0 == 0
    }

    /// The members of `self` that are not in `other`.
    pub fn minus(self, other: Self) -> Self {
        ResourceTypeSet(self.0 & !other.0)
    }

    /// The members, in declaration order.
    pub fn iter(self) -> impl Iterator<Item = ResourceType> {
        ResourceType::ALL
            .into_iter()
            .filter(move |t| self.contains(*t))
    }
}

impl FromIterator<ResourceType> for ResourceTypeSet {
    fn from_iter<I: IntoIterator<Item = ResourceType>>(iter: I) -> Self {
        let mut set = ResourceTypeSet::empty();
        for t in iter {
            set.insert(t);
        }
        set
    }
}

/// An HTTP request method as Chrome's `RequestMethod` knows it. Everything else is `Other`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum RequestMethod {
    /// `CONNECT`.
    Connect,
    /// `DELETE`.
    Delete,
    /// `GET`.
    Get,
    /// `HEAD`.
    Head,
    /// `OPTIONS`.
    Options,
    /// `PATCH`.
    Patch,
    /// `POST`.
    Post,
    /// `PUT`.
    Put,
    /// Any other method.
    Other,
}

impl RequestMethod {
    /// Every method, in declaration order.
    pub const ALL: [RequestMethod; 9] = [
        RequestMethod::Connect,
        RequestMethod::Delete,
        RequestMethod::Get,
        RequestMethod::Head,
        RequestMethod::Options,
        RequestMethod::Patch,
        RequestMethod::Post,
        RequestMethod::Put,
        RequestMethod::Other,
    ];

    /// The lowercase name used in rule JSON.
    pub fn name(self) -> &'static str {
        match self {
            RequestMethod::Connect => "connect",
            RequestMethod::Delete => "delete",
            RequestMethod::Get => "get",
            RequestMethod::Head => "head",
            RequestMethod::Options => "options",
            RequestMethod::Patch => "patch",
            RequestMethod::Post => "post",
            RequestMethod::Put => "put",
            RequestMethod::Other => "other",
        }
    }

    /// Parses the lowercase name used in rule JSON (exact).
    pub fn from_name(name: &str) -> Option<RequestMethod> {
        RequestMethod::ALL.into_iter().find(|m| m.name() == name)
    }

    /// Maps an HTTP method string, in any letter case, to the enum; unknown methods are `Other`.
    pub fn from_http(method: &str) -> RequestMethod {
        RequestMethod::ALL
            .into_iter()
            .filter(|m| *m != RequestMethod::Other)
            .find(|m| m.name().eq_ignore_ascii_case(method))
            .unwrap_or(RequestMethod::Other)
    }

    fn bit(self) -> u16 {
        1u16 << (self as u32)
    }
}

/// A set of [`RequestMethod`]s.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct MethodSet(u16);

impl MethodSet {
    /// Every method.
    pub fn all() -> Self {
        RequestMethod::ALL.into_iter().collect()
    }

    /// Adds a method.
    pub fn insert(&mut self, m: RequestMethod) {
        self.0 |= m.bit();
    }

    /// Whether the method is in the set.
    pub fn contains(self, m: RequestMethod) -> bool {
        self.0 & m.bit() != 0
    }

    /// Whether the set has no member.
    pub fn is_empty(self) -> bool {
        self.0 == 0
    }

    /// Whether the two sets share a member.
    pub fn intersects(self, other: Self) -> bool {
        self.0 & other.0 != 0
    }

    /// The members of `self` that are not in `other`.
    pub fn minus(self, other: Self) -> Self {
        MethodSet(self.0 & !other.0)
    }
}

impl FromIterator<RequestMethod> for MethodSet {
    fn from_iter<I: IntoIterator<Item = RequestMethod>>(iter: I) -> Self {
        let mut set = MethodSet::default();
        for m in iter {
            set.insert(m);
        }
        set
    }
}

/// Everything the evaluator reads about one request (the EC38 fields plus the method and tab).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RequestMetadata {
    /// The request URL as the network stack sees it.
    pub url: Url,
    /// The HTTP method, in any letter case.
    pub method: String,
    /// The resource type.
    pub resource_type: ResourceType,
    /// The origin that started the request, serialised (`https://example.com`, or `null` when opaque);
    /// `None` when the request has no initiator (a typed navigation).
    pub initiator_origin: Option<String>,
    /// The site of the top-level document the request belongs to (`https://example.com`).
    pub top_level_site: Option<String>,
    /// Whether the request is third party to its initiator. `None` means unknown, which the evaluator
    /// treats as third party: a `firstParty` rule does not match, a `thirdParty` rule does (the fail-closed
    /// direction for blocking rules).
    pub is_third_party: Option<bool>,
    /// The tab the request belongs to; `None` (or `-1`) for requests without a tab.
    pub tab_id: Option<i64>,
}

impl RequestMetadata {
    /// A request with no initiator, no top-level site, unknown party and no tab.
    pub fn new(url: Url, method: impl Into<String>, resource_type: ResourceType) -> Self {
        RequestMetadata {
            url,
            method: method.into(),
            resource_type,
            initiator_origin: None,
            top_level_site: None,
            is_third_party: None,
            tab_id: None,
        }
    }

    /// Sets the initiator origin.
    pub fn with_initiator(mut self, origin: impl Into<String>) -> Self {
        self.initiator_origin = Some(origin.into());
        self
    }

    /// Sets the top-level site.
    pub fn with_top_level_site(mut self, site: impl Into<String>) -> Self {
        self.top_level_site = Some(site.into());
        self
    }

    /// Sets whether the request is third party.
    pub fn with_third_party(mut self, third_party: bool) -> Self {
        self.is_third_party = Some(third_party);
        self
    }

    /// Sets the tab id.
    pub fn with_tab(mut self, tab_id: i64) -> Self {
        self.tab_id = Some(tab_id);
        self
    }

    /// The method as the enum.
    pub fn request_method(&self) -> RequestMethod {
        RequestMethod::from_http(&self.method)
    }

    /// The tab id with `-1` for "no tab" (Chrome's `TAB_ID_NONE`).
    pub fn effective_tab_id(&self) -> i64 {
        self.tab_id.unwrap_or(-1)
    }
}

/// The host of an origin or site string (`https://a.example.com:8080` gives `a.example.com`); `None`
/// for `null`, opaque origins, and anything that does not parse to a host.
pub(crate) fn host_of_origin(origin: &str) -> Option<String> {
    if origin == "null" {
        return None;
    }
    let url = Url::parse(origin).ok()?;
    let host = url.host_str()?;
    Some(normalise_host(host))
}

/// Lowercase host without one trailing dot.
pub(crate) fn normalise_host(host: &str) -> String {
    let lower = host.to_ascii_lowercase();
    match lower.strip_suffix('.') {
        Some(stripped) if !stripped.is_empty() => stripped.to_owned(),
        _ => lower,
    }
}

/// Whether `host` equals `domain` or is a subdomain of it, on a label boundary. IP literals match only
/// exactly.
pub(crate) fn host_matches_domain(host: &str, domain: &str) -> bool {
    if host == domain {
        return true;
    }
    if host.starts_with('[') || host.parse::<std::net::Ipv4Addr>().is_ok() {
        return false;
    }
    host.len() > domain.len()
        && host.ends_with(domain)
        && host
            .as_bytes()
            .get(host.len() - domain.len() - 1)
            .is_some_and(|b| *b == b'.')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resource_type_names_round_trip() {
        for t in ResourceType::ALL {
            assert_eq!(ResourceType::from_name(t.name()), Some(t));
        }
        assert_eq!(ResourceType::from_name("Script"), None);
        assert_eq!(ResourceType::from_name("fetch"), None);
    }

    #[test]
    fn rule_default_types_exclude_main_frame_only() {
        let set = ResourceTypeSet::rule_default();
        assert!(!set.contains(ResourceType::MainFrame));
        assert!(set.contains(ResourceType::SubFrame));
        assert_eq!(set.iter().count(), 14);
    }

    #[test]
    fn resource_type_set_algebra() {
        let frames: ResourceTypeSet = [ResourceType::MainFrame, ResourceType::SubFrame]
            .into_iter()
            .collect();
        assert!(frames.is_subset_of(ResourceTypeSet::all()));
        assert!(!ResourceTypeSet::all().is_subset_of(frames));
        assert!(frames.intersects(ResourceTypeSet::all()));
        assert!(
            ResourceTypeSet::all()
                .minus(ResourceTypeSet::all())
                .is_empty()
        );
    }

    #[test]
    fn request_method_is_case_insensitive_and_unknown_is_other() {
        assert_eq!(RequestMethod::from_http("get"), RequestMethod::Get);
        assert_eq!(RequestMethod::from_http("GET"), RequestMethod::Get);
        assert_eq!(RequestMethod::from_http("PoSt"), RequestMethod::Post);
        assert_eq!(RequestMethod::from_http("PROPFIND"), RequestMethod::Other);
        assert_eq!(RequestMethod::from_http(""), RequestMethod::Other);
    }

    #[test]
    fn rule_method_names_are_exact_lowercase() {
        assert_eq!(RequestMethod::from_name("get"), Some(RequestMethod::Get));
        assert_eq!(RequestMethod::from_name("GET"), None);
        assert_eq!(
            RequestMethod::from_name("other"),
            Some(RequestMethod::Other)
        );
    }

    #[test]
    fn missing_tab_is_tab_id_none() {
        let url = Url::parse("https://a.test/").expect("url");
        let r = RequestMetadata::new(url.clone(), "GET", ResourceType::Script);
        assert_eq!(r.effective_tab_id(), -1);
        assert_eq!(r.with_tab(7).effective_tab_id(), 7);
    }

    #[test]
    fn origin_host_extraction() {
        assert_eq!(
            host_of_origin("https://A.Example.com:8080"),
            Some("a.example.com".into())
        );
        assert_eq!(host_of_origin("null"), None);
        assert_eq!(host_of_origin("not a url"), None);
        assert_eq!(host_of_origin(""), None);
    }

    #[test]
    fn domain_suffix_matches_on_label_boundary() {
        assert!(host_matches_domain("example.com", "example.com"));
        assert!(host_matches_domain("a.b.example.com", "example.com"));
        assert!(!host_matches_domain("notexample.com", "example.com"));
        assert!(!host_matches_domain("example.com.evil.test", "example.com"));
        assert!(!host_matches_domain("example.com", "a.example.com"));
    }

    #[test]
    fn ip_literals_match_only_exactly() {
        assert!(host_matches_domain("127.0.0.1", "127.0.0.1"));
        assert!(!host_matches_domain("127.0.0.1", "0.0.1"));
        assert!(host_matches_domain("[::1]", "[::1]"));
    }

    #[test]
    fn one_trailing_dot_is_dropped_from_hosts() {
        assert_eq!(normalise_host("Example.COM."), "example.com");
        assert_eq!(normalise_host("."), ".");
    }
}
