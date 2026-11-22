// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! The update-check client model: no network, no clock.
//!
//! Behaviour per Chromium's update protocol (`omaha-protocol`'s documentation of update2) and
//! MDN's "Updates" page for the Firefox update manifest. The client builds requests, the shell
//! fetches them through an [`UpdateTransport`], and this module judges what came back. Update
//! checks are outside services: nothing here runs until the shell enables them (OD-32), and
//! Mercury never calls the Chrome Web Store (D3): [`store_update_url`] is a tested builder that
//! nothing uses.
//!
//! What an accepted update is: an offer for the same id, newer than the installed version, from
//! an `https` URL without credentials or fragment. It is only a promise to download. After the
//! download the shell verifies the hash ([`UpdateHash::verify`]), then the package signature, then
//! checks the new manifest's id with [`check_update_identity`], and the permission delta
//! (`permissions::permissions_added`) decides consent.
//!
//! Safety limits ([`UpdateLimits`]): responses of at most 1 MiB, at most 1,000 apps or updates,
//! XML nesting of at most 16 levels. XML with a `DOCTYPE` is refused outright, so no entity can be
//! defined, and entity references in text are never expanded. Nothing is parsed from text nodes;
//! only attributes are read. Error messages never carry response content.

use std::cmp::Ordering;
use std::collections::BTreeMap;

use omaha::protocol::{Sha256 as OmahaSha256, Version as OmahaVersion};
use quick_xml::XmlVersion;
use quick_xml::events::{BytesStart, Event};
use quick_xml::reader::Reader;
use serde_json::Value;
use sha2::{Digest, Sha256, Sha512};
use url::Url;

use crate::id::{ExtensionId, ExtensionKind};
use crate::manifest::{exceeds_nesting, is_toolkit_version};

/// Caps for the update parsers and requests.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdateLimits {
    /// Largest response accepted, in bytes (default 1 MiB).
    pub max_response_bytes: usize,
    /// Most apps in an update2 response (default 1,000).
    pub max_apps: usize,
    /// Most updates for one add-on in a Firefox update manifest (default 1,000).
    pub max_updates: usize,
    /// Deepest XML nesting (default 16).
    pub max_xml_depth: usize,
    /// Longest attribute value, in bytes (default 4096).
    pub max_attr_len: usize,
    /// Deepest JSON nesting (default 64).
    pub max_json_depth: usize,
    /// Longest request URL, in bytes; longer requests are split (default 2000, Chrome's cap).
    pub max_request_url_len: usize,
}

impl Default for UpdateLimits {
    fn default() -> Self {
        UpdateLimits {
            max_response_bytes: 1 << 20,
            max_apps: 1_000,
            max_updates: 1_000,
            max_xml_depth: 16,
            max_attr_len: 4096,
            max_json_depth: 64,
            max_request_url_len: 2000,
        }
    }
}

/// Why an update response or request was refused. The messages carry no response content.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum UpdateError {
    /// The response is larger than the cap.
    #[error("the update response is too large")]
    TooLarge,
    /// The response is not well-formed XML of the expected shape.
    #[error("the update response is not valid XML")]
    NotXml,
    /// The XML has a `DOCTYPE`, which is refused.
    #[error("the update response has a DOCTYPE, which is not allowed")]
    DoctypeNotAllowed,
    /// The XML nests deeper than the cap.
    #[error("the update response nests too deeply")]
    TooDeep,
    /// The response lists more apps than the cap.
    #[error("the update response lists too many apps")]
    TooManyApps,
    /// The response lists more updates than the cap.
    #[error("the update response lists too many updates")]
    TooManyUpdates,
    /// The response is not valid JSON.
    #[error("the update manifest is not valid JSON")]
    NotJson,
    /// The JSON does not have the expected shape.
    #[error("the update manifest has an unexpected shape")]
    BadShape,
    /// An input to a builder is not valid (an id, a version).
    #[error("an update input is invalid")]
    InvalidInput,
    /// A version string is not valid for this kind of extension.
    #[error("a version string is invalid")]
    InvalidVersion,
    /// The package's id is not the installed extension's id.
    #[error("the update is for a different extension id")]
    IdentityMismatch,
    /// The request URL is not allowed by the policy.
    #[error("the update URL is not allowed")]
    UrlNotAllowed,
    /// The server answered with an error for this extension.
    #[error("the update server reported an error")]
    Server,
    /// The response did not mention a requested extension.
    #[error("the update response does not mention the extension")]
    MissingResult,
    /// The transport failed.
    #[error("the update transport failed: {0}")]
    Transport(#[from] TransportError),
}

/// Why the shell could not fetch a URL.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum TransportError {
    /// The network failed.
    #[error("network error")]
    Network,
    /// The request timed out.
    #[error("timeout")]
    Timeout,
    /// The server answered with this HTTP status.
    #[error("HTTP status {0}")]
    HttpStatus(u16),
    /// The body was larger than the cap.
    #[error("response too large")]
    TooLarge,
    /// The shell refused to make the request (updates are off, or the URL is not allowed).
    #[error("request refused")]
    Refused,
}

/// How the shell fetches a URL. Implemented by the shell; tests use a fake. The implementation
/// must stop reading after `max_bytes`, send no cookies and no credentials, and follow redirects
/// only to `https`.
pub trait UpdateTransport {
    /// Fetches `url` with a GET and returns the body.
    fn fetch(&self, url: &Url, max_bytes: u64) -> Result<Vec<u8>, TransportError>;
}

// ---- versions ----------------------------------------------------------------------------

/// Compares two Chrome extension versions component by component, numerically; a missing trailing
/// component counts as 0 (`1.0 == 1`). Each must be 1 to 4 dot-separated integers.
pub fn compare_chrome_versions(a: &str, b: &str) -> Result<Ordering, UpdateError> {
    let parse = |s: &str| -> Result<OmahaVersion, UpdateError> {
        let v: OmahaVersion = s.parse().map_err(|_| UpdateError::InvalidVersion)?;
        if v.is_valid_extension_version() {
            Ok(v)
        } else {
            Err(UpdateError::InvalidVersion)
        }
    };
    Ok(parse(a)?.cmp(&parse(b)?))
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
struct ToolkitPart {
    num_a: i64,
    str_b: Option<String>,
    num_c: i64,
    extra_d: Option<String>,
}

/// `strtol`-like: an optional sign and digits at the start; returns the value (saturating) and
/// the rest.
fn leading_integer(s: &str) -> (i64, &str) {
    let bytes = s.as_bytes();
    let mut i = 0;
    let negative = match bytes.first() {
        Some(b'-') => {
            i = 1;
            true
        }
        Some(b'+') => {
            i = 1;
            false
        }
        _ => false,
    };
    let start = i;
    let mut value: i64 = 0;
    while let Some(d) = bytes.get(i).filter(|b| b.is_ascii_digit()) {
        value = value
            .saturating_mul(10)
            .saturating_add(i64::from(d - b'0'))
            .min(i64::from(i32::MAX));
        i += 1;
    }
    if i == start {
        return (0, s);
    }
    (
        if negative { -value } else { value },
        s.get(i..).unwrap_or(""),
    )
}

fn parse_toolkit_part(s: &str) -> ToolkitPart {
    if s == "*" {
        return ToolkitPart {
            num_a: i64::from(i32::MAX),
            str_b: Some(String::new()),
            ..ToolkitPart::default()
        };
    }
    let (mut num_a, rest) = leading_integer(s);
    let mut part = ToolkitPart::default();
    if rest.is_empty() {
        part.num_a = num_a;
        return part;
    }
    if rest.starts_with('+') {
        num_a = num_a.saturating_add(1);
        part.str_b = Some("pre".to_owned());
    } else {
        match rest.find(|c: char| c.is_ascii_digit() || c == '+' || c == '-') {
            None => part.str_b = Some(rest.to_owned()),
            Some(at) => {
                part.str_b = Some(rest.get(..at).unwrap_or("").to_owned());
                let (num_c, extra) = leading_integer(rest.get(at..).unwrap_or(""));
                part.num_c = num_c;
                if !extra.is_empty() {
                    part.extra_d = Some(extra.to_owned());
                }
            }
        }
    }
    part.num_a = num_a;
    part
}

/// A missing string is greater than a present one (`1.1 > 1.1pre`).
fn cmp_optional_str(a: &Option<String>, b: &Option<String>) -> Ordering {
    match (a, b) {
        (None, None) => Ordering::Equal,
        (None, Some(_)) => Ordering::Greater,
        (Some(_), None) => Ordering::Less,
        (Some(x), Some(y)) => x.as_bytes().cmp(y.as_bytes()),
    }
}

fn cmp_toolkit_part(a: &ToolkitPart, b: &ToolkitPart) -> Ordering {
    a.num_a
        .cmp(&b.num_a)
        .then_with(|| cmp_optional_str(&a.str_b, &b.str_b))
        .then_with(|| a.num_c.cmp(&b.num_c))
        .then_with(|| cmp_optional_str(&a.extra_d, &b.extra_d))
}

fn toolkit_parts(v: &str) -> Vec<ToolkitPart> {
    let mut pieces: Vec<&str> = v.split('.').collect();
    if pieces.len() > 1 && pieces.last() == Some(&"") {
        pieces.pop();
    }
    pieces.into_iter().map(parse_toolkit_part).collect()
}

/// Compares two versions in Firefox's toolkit version format (MDN "Toolkit version format"): parts
/// separated by dots, each `<number><string><number><string>`; `*` is the largest number; a
/// trailing `+` means "one more, pre"; a part with a string sorts before the same part without
/// (`1.1pre < 1.1`); missing parts are 0 (`1 == 1.0 == 1.0.0`).
pub fn compare_toolkit_versions(a: &str, b: &str) -> Ordering {
    let pa = toolkit_parts(a);
    let pb = toolkit_parts(b);
    let zero = ToolkitPart::default();
    for i in 0..pa.len().max(pb.len()) {
        let o = cmp_toolkit_part(pa.get(i).unwrap_or(&zero), pb.get(i).unwrap_or(&zero));
        if o != Ordering::Equal {
            return o;
        }
    }
    Ordering::Equal
}

/// Compares versions with the rule of the extension's kind: Chrome's numeric rule for a Chrome
/// id, the toolkit rule for a Firefox id. A malformed version is an error.
pub fn compare_versions(kind: ExtensionKind, a: &str, b: &str) -> Result<Ordering, UpdateError> {
    match kind {
        ExtensionKind::Chrome => compare_chrome_versions(a, b),
        ExtensionKind::Firefox => {
            if is_toolkit_version(a) && is_toolkit_version(b) {
                Ok(compare_toolkit_versions(a, b))
            } else {
                Err(UpdateError::InvalidVersion)
            }
        }
    }
}

// ---- hashes ------------------------------------------------------------------------------

/// A hash an update promises for its package.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UpdateHash {
    /// SHA-256.
    Sha256([u8; 32]),
    /// SHA-512.
    Sha512([u8; 64]),
}

impl UpdateHash {
    /// Whether `data` has this hash.
    pub fn verify(&self, data: &[u8]) -> bool {
        match self {
            UpdateHash::Sha256(h) => Sha256::digest(data).as_slice() == h.as_slice(),
            UpdateHash::Sha512(h) => Sha512::digest(data).as_slice() == h.as_slice(),
        }
    }

    /// Parses `sha256:<hex>` or `sha512:<hex>` (what the Firefox update manifest writes). Any
    /// other algorithm, including `sha1` and `md5`, is refused.
    pub fn parse_prefixed(text: &str) -> Option<UpdateHash> {
        let (algo, hex) = text.split_once(':')?;
        match algo.to_ascii_lowercase().as_str() {
            "sha256" => decode_hex::<32>(hex).map(UpdateHash::Sha256),
            "sha512" => decode_hex::<64>(hex).map(UpdateHash::Sha512),
            _ => None,
        }
    }
}

fn decode_hex<const N: usize>(hex: &str) -> Option<[u8; N]> {
    let bytes = hex.as_bytes();
    if bytes.len() != N * 2 {
        return None;
    }
    let mut out = [0u8; N];
    for (slot, pair) in out.iter_mut().zip(bytes.chunks(2)) {
        let hi = char::from(*pair.first()?).to_digit(16)?;
        let lo = char::from(*pair.get(1)?).to_digit(16)?;
        *slot = u8::try_from(hi * 16 + lo).ok()?;
    }
    Some(out)
}

// ---- update2 (CRX) -----------------------------------------------------------------------

/// An installed extension, as far as the update check cares.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstalledExtension {
    /// The id.
    pub id: ExtensionId,
    /// The installed `version`.
    pub version: String,
    /// The manifest's `update_url` (or gecko `update_url`), if any.
    pub update_url: Option<Url>,
}

/// What the browser tells the update server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BrowserInfo {
    /// The product name (`prod`).
    pub prod: String,
    /// The browser version (`prodversion`).
    pub prodversion: String,
    /// The operating system (`os`).
    pub os: String,
    /// The CPU architecture (`arch`).
    pub arch: String,
    /// The UI language (`lang`), if any.
    pub lang: Option<String>,
}

/// One update2 request: a GET URL and the ids it asks about.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdateRequest {
    /// The URL to fetch.
    pub url: Url,
    /// The extensions it asks about.
    pub ids: Vec<ExtensionId>,
}

fn request_url(base: &Url, browser: &BrowserInfo, members: &[&InstalledExtension]) -> Url {
    let mut url = base.clone();
    {
        let mut q = url.query_pairs_mut();
        q.append_pair("os", &browser.os);
        q.append_pair("arch", &browser.arch);
        q.append_pair("prod", &browser.prod);
        q.append_pair("prodversion", &browser.prodversion);
        if let Some(lang) = &browser.lang {
            q.append_pair("lang", lang);
        }
        q.append_pair("acceptformat", "crx3");
        for e in members {
            q.append_pair("x", &format!("id={}&v={}&uc", e.id, e.version));
        }
    }
    url
}

/// Builds the update2 requests for the Chrome-style extensions among `installed`, one group per
/// update URL, split so that no URL is longer than [`UpdateLimits::max_request_url_len`] (an
/// extension that cannot fit alone is left out). Firefox-style ids, extensions without an
/// `update_url`, and URLs that are not `http` or `https` are skipped. The result is ordered by URL.
pub fn build_update2_request(
    installed: &[InstalledExtension],
    browser: &BrowserInfo,
) -> Vec<UpdateRequest> {
    build_update2_request_with(installed, browser, &UpdateLimits::default())
}

/// [`build_update2_request`] with explicit limits.
pub fn build_update2_request_with(
    installed: &[InstalledExtension],
    browser: &BrowserInfo,
    limits: &UpdateLimits,
) -> Vec<UpdateRequest> {
    let mut groups: BTreeMap<String, (Url, Vec<&InstalledExtension>)> = BTreeMap::new();
    for e in installed {
        let version_ok = e
            .version
            .parse::<OmahaVersion>()
            .is_ok_and(|v| v.is_valid_extension_version());
        if e.id.kind() != ExtensionKind::Chrome || !version_ok {
            continue;
        }
        let Some(url) = &e.update_url else {
            continue;
        };
        if !matches!(url.scheme(), "http" | "https") || url.host().is_none() {
            continue;
        }
        groups
            .entry(url.as_str().to_owned())
            .or_insert_with(|| (url.clone(), Vec::new()))
            .1
            .push(e);
    }
    let mut out = Vec::new();
    for (_, (base, members)) in groups {
        let mut chunk: Vec<&InstalledExtension> = Vec::new();
        for e in members {
            chunk.push(e);
            if request_url(&base, browser, &chunk).as_str().len() > limits.max_request_url_len {
                chunk.pop();
                if !chunk.is_empty() {
                    out.push(finish_request(&base, browser, &chunk));
                }
                chunk = vec![e];
                if request_url(&base, browser, &chunk).as_str().len() > limits.max_request_url_len {
                    chunk.clear();
                }
            }
        }
        if !chunk.is_empty() {
            out.push(finish_request(&base, browser, &chunk));
        }
    }
    out
}

fn finish_request(
    base: &Url,
    browser: &BrowserInfo,
    members: &[&InstalledExtension],
) -> UpdateRequest {
    UpdateRequest {
        url: request_url(base, browser, members),
        ids: members.iter().map(|e| e.id.clone()).collect(),
    }
}

/// The Chrome Web Store download URL for an extension, in Chrome's form
/// (`.../update2/crx?response=redirect&prodversion=...&acceptformat=crx2,crx3&x=id%3D<id>%26uc`).
/// Mercury does not call the store (D3); this is a documented builder with tests, and unused.
pub fn store_update_url(id: &ExtensionId, prodversion: &str) -> Result<Url, UpdateError> {
    if id.kind() != ExtensionKind::Chrome
        || prodversion.is_empty()
        || prodversion.len() > 32
        || !prodversion.bytes().all(|b| b.is_ascii_digit() || b == b'.')
    {
        return Err(UpdateError::InvalidInput);
    }
    Url::parse(&format!(
        "https://clients2.google.com/service/update2/crx?response=redirect&prodversion={prodversion}&acceptformat=crx2,crx3&x=id%3D{id}%26uc"
    ))
    .map_err(|_| UpdateError::InvalidInput)
}

/// What an update2 response says about one app.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UpdateCheckResult {
    /// The installed version is current.
    UpToDate {
        /// The app id.
        id: String,
    },
    /// A newer version is offered.
    Available {
        /// The app id.
        id: String,
        /// The offered version.
        version: String,
        /// Where to download the package.
        codebase: Url,
        /// The package's SHA-256, as the server states it.
        sha256: [u8; 32],
        /// The package size, if stated.
        size: Option<u64>,
    },
    /// The server reported an error, or the answer is unusable.
    Error {
        /// The app id (may be empty if it was not usable).
        id: String,
        /// What was wrong.
        kind: UpdateErrorKind,
    },
}

/// Why an app's answer is unusable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UpdateErrorKind {
    /// The server's status, such as `error-unknownApplication` or `restricted` (only letters,
    /// digits, `-`, `_` and at most 64 of them are kept).
    Status(String),
    /// The `app` has no `updatecheck`.
    MissingUpdateCheck,
    /// The app id is not a valid extension id.
    InvalidAppId,
    /// The offered version is not valid.
    InvalidVersion,
    /// The `codebase` is not an `http` or `https` URL.
    InvalidCodebase,
    /// The offer has no `hash_sha256`; Mercury refuses an unhashed download.
    MissingHash,
    /// The `hash_sha256` is not 64 hex digits.
    InvalidHash,
}

fn sanitize_status(s: &str) -> String {
    s.chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
        .take(64)
        .collect()
}

#[derive(Default)]
struct XmlCheck {
    status: Option<String>,
    codebase: Option<String>,
    version: Option<String>,
    hash_sha256: Option<String>,
    size: Option<String>,
}

#[derive(Default)]
struct XmlApp {
    id: String,
    status: Option<String>,
    check: Option<XmlCheck>,
}

fn local_name(name: &str) -> &str {
    name.rsplit(':').next().unwrap_or(name)
}

fn attributes(
    e: &BytesStart<'_>,
    limits: &UpdateLimits,
) -> Result<Vec<(String, String)>, UpdateError> {
    let mut out = Vec::new();
    for attr in e.attributes() {
        let attr = attr.map_err(|_| UpdateError::NotXml)?;
        let value = attr
            .normalized_value(XmlVersion::Implicit1_0)
            .map_err(|_| UpdateError::NotXml)?;
        if value.len() > limits.max_attr_len {
            return Err(UpdateError::TooLarge);
        }
        out.push((
            local_name(attr.key.into_inner()).to_owned(),
            value.into_owned(),
        ));
    }
    Ok(out)
}

fn resolve_app(app: XmlApp) -> UpdateCheckResult {
    let err = |kind| UpdateCheckResult::Error {
        id: app.id.clone(),
        kind,
    };
    if !omaha::protocol::is_valid_crx_id(&app.id) {
        return UpdateCheckResult::Error {
            id: String::new(),
            kind: UpdateErrorKind::InvalidAppId,
        };
    }
    if let Some(status) = &app.status
        && status != "ok"
    {
        return err(UpdateErrorKind::Status(sanitize_status(status)));
    }
    let Some(check) = &app.check else {
        return err(UpdateErrorKind::MissingUpdateCheck);
    };
    match check.status.as_deref() {
        Some("noupdate") => {
            return UpdateCheckResult::UpToDate { id: app.id.clone() };
        }
        Some("ok") | None => {}
        Some(other) => return err(UpdateErrorKind::Status(sanitize_status(other))),
    }
    let Some(version) = &check.version else {
        return err(UpdateErrorKind::InvalidVersion);
    };
    match version.parse::<OmahaVersion>() {
        Ok(v) if v.is_valid_extension_version() => {}
        _ => return err(UpdateErrorKind::InvalidVersion),
    }
    let codebase = match check.codebase.as_deref().map(Url::parse) {
        Some(Ok(u)) if matches!(u.scheme(), "http" | "https") && u.host().is_some() => u,
        _ => return err(UpdateErrorKind::InvalidCodebase),
    };
    let sha256 = match check.hash_sha256.as_deref() {
        None => return err(UpdateErrorKind::MissingHash),
        Some(h) => match h.parse::<OmahaSha256>() {
            Ok(h) => *h.as_bytes(),
            Err(_) => return err(UpdateErrorKind::InvalidHash),
        },
    };
    UpdateCheckResult::Available {
        id: app.id.clone(),
        version: version.clone(),
        codebase,
        sha256,
        size: check.size.as_deref().and_then(|s| s.parse::<u64>().ok()),
    }
}

/// Parses a `gupdate` update2 response. Refuses a `DOCTYPE`, nesting over the cap, more apps than
/// the cap and anything over the size cap; reads attributes only.
pub fn parse_update2_response(
    bytes: &[u8],
    limits: &UpdateLimits,
) -> Result<Vec<UpdateCheckResult>, UpdateError> {
    if bytes.len() > limits.max_response_bytes {
        return Err(UpdateError::TooLarge);
    }
    let text = std::str::from_utf8(bytes).map_err(|_| UpdateError::NotXml)?;
    scan_update2(text, limits)
}

fn scan_update2(text: &str, limits: &UpdateLimits) -> Result<Vec<UpdateCheckResult>, UpdateError> {
    let mut reader = Reader::from_str(text);
    let mut depth = 0usize;
    let mut saw_root = false;
    let mut current: Option<XmlApp> = None;
    let mut results: Vec<UpdateCheckResult> = Vec::new();
    let mut app_count = 0usize;
    loop {
        let event = reader.read_event().map_err(|_| UpdateError::NotXml)?;
        let (start, empty) = match &event {
            Event::Start(e) => (Some(e), false),
            Event::Empty(e) => (Some(e), true),
            _ => (None, false),
        };
        if let Some(e) = start {
            if !empty {
                depth += 1;
                if depth > limits.max_xml_depth {
                    return Err(UpdateError::TooDeep);
                }
            }
            let name = local_name(e.name().into_inner()).to_owned();
            if !saw_root {
                if name != "gupdate" {
                    return Err(UpdateError::NotXml);
                }
                saw_root = true;
            }
            match name.as_str() {
                "app" => {
                    app_count += 1;
                    if app_count > limits.max_apps {
                        return Err(UpdateError::TooManyApps);
                    }
                    let mut app = XmlApp::default();
                    for (k, v) in attributes(e, limits)? {
                        match k.as_str() {
                            "appid" => app.id = v,
                            "status" => app.status = Some(v),
                            _ => {}
                        }
                    }
                    if empty {
                        results.push(resolve_app(app));
                    } else {
                        current = Some(app);
                    }
                }
                "updatecheck" => {
                    if let Some(app) = current.as_mut()
                        && app.check.is_none()
                    {
                        let mut check = XmlCheck::default();
                        for (k, v) in attributes(e, limits)? {
                            match k.as_str() {
                                "status" => check.status = Some(v),
                                "codebase" => check.codebase = Some(v),
                                "version" => check.version = Some(v),
                                "hash_sha256" => check.hash_sha256 = Some(v),
                                "size" => check.size = Some(v),
                                _ => {}
                            }
                        }
                        app.check = Some(check);
                    }
                }
                _ => {}
            }
            continue;
        }
        match event {
            Event::Eof => break,
            Event::DocType(_) => return Err(UpdateError::DoctypeNotAllowed),
            Event::End(e) => {
                depth = depth.saturating_sub(1);
                if local_name(e.name().into_inner()) == "app"
                    && let Some(app) = current.take()
                {
                    results.push(resolve_app(app));
                }
            }
            _ => {}
        }
    }
    if depth != 0 || !saw_root {
        return Err(UpdateError::NotXml);
    }
    Ok(results)
}

// ---- Firefox update manifest -------------------------------------------------------------

/// One update an add-on's Firefox update manifest offers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AvailableUpdate {
    /// The version.
    pub version: String,
    /// Where to download the `.xpi`.
    pub update_link: Url,
    /// The promised hash, if any (`sha256` or `sha512`; weaker algorithms make the entry unusable
    /// and it is left out).
    pub update_hash: Option<UpdateHash>,
    /// `applications.gecko.strict_min_version`.
    pub strict_min_version: Option<String>,
    /// `applications.gecko.strict_max_version`.
    pub strict_max_version: Option<String>,
}

/// Parses a Firefox update manifest (`{"addons": {"<id>": {"updates": [...]}}}`) and returns the
/// usable updates for `addon_id`: entries with a missing or invalid version or link, a link that
/// is not `http`/`https`, or a hash of an algorithm other than sha256/sha512 are left out. The
/// entries are in file order.
pub fn parse_firefox_update_manifest(
    bytes: &[u8],
    addon_id: &ExtensionId,
    limits: &UpdateLimits,
) -> Result<Vec<AvailableUpdate>, UpdateError> {
    if bytes.len() > limits.max_response_bytes {
        return Err(UpdateError::TooLarge);
    }
    if exceeds_nesting(bytes, limits.max_json_depth) {
        return Err(UpdateError::TooDeep);
    }
    let value: Value = serde_json::from_slice(bytes).map_err(|_| UpdateError::NotJson)?;
    let Value::Object(root) = value else {
        return Err(UpdateError::BadShape);
    };
    let Some(Value::Object(addons)) = root.get("addons") else {
        return Err(UpdateError::BadShape);
    };
    let Some(entry) = addons.get(addon_id.as_str()) else {
        return Ok(Vec::new());
    };
    let Some(Value::Array(updates)) = entry.get("updates") else {
        return Err(UpdateError::BadShape);
    };
    if updates.len() > limits.max_updates {
        return Err(UpdateError::TooManyUpdates);
    }
    let mut out = Vec::new();
    for u in updates {
        let Some(version) = u.get("version").and_then(Value::as_str) else {
            continue;
        };
        if !is_toolkit_version(version) {
            continue;
        }
        let Some(link) = u.get("update_link").and_then(Value::as_str) else {
            continue;
        };
        let link = match Url::parse(link) {
            Ok(l) if matches!(l.scheme(), "http" | "https") && l.host().is_some() => l,
            _ => continue,
        };
        let update_hash = match u.get("update_hash") {
            None => None,
            Some(Value::String(h)) => match UpdateHash::parse_prefixed(h) {
                Some(h) => Some(h),
                None => continue,
            },
            Some(_) => continue,
        };
        let gecko = u.get("applications").and_then(|a| a.get("gecko"));
        let version_of = |key: &str| {
            gecko
                .and_then(|g| g.get(key))
                .and_then(Value::as_str)
                .filter(|s| is_toolkit_version(s))
                .map(str::to_owned)
        };
        out.push(AvailableUpdate {
            version: version.to_owned(),
            update_link: link,
            update_hash,
            strict_min_version: version_of("strict_min_version"),
            strict_max_version: version_of("strict_max_version"),
        });
    }
    Ok(out)
}

// ---- judging -----------------------------------------------------------------------------

/// What the policy allows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct UpdatePolicy {
    /// Allow `http` to a loopback host (`localhost`, `127.0.0.0/8`, `::1`). For tests and local
    /// development only; default `false`.
    pub allow_insecure_loopback: bool,
}

impl UpdatePolicy {
    /// Whether `url` may be fetched or downloaded from under this policy.
    pub fn url_allowed(&self, url: &Url) -> bool {
        match url.scheme() {
            "https" => url.host().is_some(),
            "http" => self.allow_insecure_loopback && is_loopback(url),
            _ => false,
        }
    }
}

fn is_loopback(url: &Url) -> bool {
    match url.host() {
        Some(url::Host::Domain(d)) => d == "localhost",
        Some(url::Host::Ipv4(a)) => a.is_loopback(),
        Some(url::Host::Ipv6(a)) => a.is_loopback(),
        None => false,
    }
}

/// An update on offer, from either protocol.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdateOffer {
    /// The extension the offer is for.
    pub id: ExtensionId,
    /// The offered version.
    pub version: String,
    /// Where to download the package.
    pub url: Url,
    /// The promised hash.
    pub hash: Option<UpdateHash>,
    /// The promised size.
    pub size: Option<u64>,
    /// Firefox: `strict_min_version`.
    pub strict_min_version: Option<String>,
    /// Firefox: `strict_max_version`.
    pub strict_max_version: Option<String>,
}

/// Why an offer was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RefuseReason {
    /// The offer is for another id than the installed extension.
    IdMismatch,
    /// The URL is not `https` (or loopback `http` when the policy allows it).
    NotHttps,
    /// The URL carries a user name or password.
    CredentialsInUrl,
    /// The URL has a fragment.
    FragmentInUrl,
    /// An `http` XPI offer without a hash.
    MissingHash,
    /// The offered version is older than the installed one.
    Downgrade,
    /// The offered version is the installed one.
    SameVersion,
    /// A version string is malformed.
    InvalidVersion,
}

/// The verdict on an offer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UpdateDecision {
    /// Download it, verify the hash, then the signature, then the identity, then the permission
    /// delta.
    Accept,
    /// Do not.
    Refuse(RefuseReason),
}

/// Judges an offer against the installed extension.
pub fn judge_update(
    installed: &InstalledExtension,
    offer: &UpdateOffer,
    policy: &UpdatePolicy,
) -> UpdateDecision {
    use RefuseReason as R;
    if installed.id != offer.id {
        return UpdateDecision::Refuse(R::IdMismatch);
    }
    if !policy.url_allowed(&offer.url) {
        return UpdateDecision::Refuse(R::NotHttps);
    }
    if !offer.url.username().is_empty() || offer.url.password().is_some() {
        return UpdateDecision::Refuse(R::CredentialsInUrl);
    }
    if offer.url.fragment().is_some() {
        return UpdateDecision::Refuse(R::FragmentInUrl);
    }
    let kind = installed.id.kind();
    if kind == ExtensionKind::Firefox && offer.url.scheme() == "http" && offer.hash.is_none() {
        return UpdateDecision::Refuse(R::MissingHash);
    }
    match compare_versions(kind, &offer.version, &installed.version) {
        Err(_) => UpdateDecision::Refuse(R::InvalidVersion),
        Ok(Ordering::Less) => UpdateDecision::Refuse(R::Downgrade),
        Ok(Ordering::Equal) => UpdateDecision::Refuse(R::SameVersion),
        Ok(Ordering::Greater) => UpdateDecision::Accept,
    }
}

/// After the download: the package's id must be the installed extension's.
pub fn check_update_identity(
    old_id: &ExtensionId,
    new_id: &ExtensionId,
) -> Result<(), UpdateError> {
    if old_id == new_id {
        Ok(())
    } else {
        Err(UpdateError::IdentityMismatch)
    }
}

// ---- the check ---------------------------------------------------------------------------

/// What an update check found for one extension.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UpdateStatus {
    /// Nothing newer.
    UpToDate,
    /// An acceptable update.
    Available(UpdateOffer),
    /// An update was offered and refused.
    Refused {
        /// The offer.
        offer: UpdateOffer,
        /// Why.
        reason: RefuseReason,
    },
    /// The extension has no usable `update_url`.
    NoUpdateUrl,
    /// The check failed.
    Failed(UpdateError),
}

/// The outcome for one installed extension.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdateOutcome {
    /// The extension.
    pub id: ExtensionId,
    /// What was found.
    pub status: UpdateStatus,
}

/// Checks `installed` for updates through `transport`. Outcomes are in the order of `installed`.
/// Only `https` URLs are fetched (loopback `http` when the policy allows it); nothing else is
/// touched. Never panics on malformed responses.
pub fn check_for_updates<T: UpdateTransport>(
    transport: &T,
    installed: &[InstalledExtension],
    browser: &BrowserInfo,
    policy: &UpdatePolicy,
    limits: &UpdateLimits,
) -> Vec<UpdateOutcome> {
    let mut statuses: BTreeMap<ExtensionId, UpdateStatus> = BTreeMap::new();
    let cap = u64::try_from(limits.max_response_bytes).unwrap_or(u64::MAX);

    for request in build_update2_request_with(installed, browser, limits) {
        let fetched = if policy.url_allowed(&request.url) {
            transport
                .fetch(&request.url, cap)
                .map_err(UpdateError::from)
                .and_then(|b| parse_update2_response(&b, limits))
        } else {
            Err(UpdateError::UrlNotAllowed)
        };
        for id in &request.ids {
            let Some(ext) = installed.iter().find(|e| &e.id == id) else {
                continue;
            };
            let status = match &fetched {
                Err(e) => UpdateStatus::Failed(e.clone()),
                Ok(results) => chrome_status(ext, results, policy),
            };
            statuses.entry(id.clone()).or_insert(status);
        }
    }

    for ext in installed {
        if ext.id.kind() != ExtensionKind::Firefox {
            continue;
        }
        let status = match &ext.update_url {
            None => UpdateStatus::NoUpdateUrl,
            Some(url) if !policy.url_allowed(url) => {
                UpdateStatus::Failed(UpdateError::UrlNotAllowed)
            }
            Some(url) => match transport
                .fetch(url, cap)
                .map_err(UpdateError::from)
                .and_then(|b| parse_firefox_update_manifest(&b, &ext.id, limits))
            {
                Err(e) => UpdateStatus::Failed(e),
                Ok(updates) => firefox_status(ext, updates, policy),
            },
        };
        statuses.entry(ext.id.clone()).or_insert(status);
    }

    installed
        .iter()
        .map(|e| UpdateOutcome {
            id: e.id.clone(),
            status: statuses
                .get(&e.id)
                .cloned()
                .unwrap_or(UpdateStatus::NoUpdateUrl),
        })
        .collect()
}

fn chrome_status(
    ext: &InstalledExtension,
    results: &[UpdateCheckResult],
    policy: &UpdatePolicy,
) -> UpdateStatus {
    let mine = results.iter().find(|r| match r {
        UpdateCheckResult::UpToDate { id }
        | UpdateCheckResult::Available { id, .. }
        | UpdateCheckResult::Error { id, .. } => id == ext.id.as_str(),
    });
    match mine {
        None => UpdateStatus::Failed(UpdateError::MissingResult),
        Some(UpdateCheckResult::UpToDate { .. }) => UpdateStatus::UpToDate,
        Some(UpdateCheckResult::Error { .. }) => UpdateStatus::Failed(UpdateError::Server),
        Some(UpdateCheckResult::Available {
            version,
            codebase,
            sha256,
            size,
            ..
        }) => {
            let offer = UpdateOffer {
                id: ext.id.clone(),
                version: version.clone(),
                url: codebase.clone(),
                hash: Some(UpdateHash::Sha256(*sha256)),
                size: *size,
                strict_min_version: None,
                strict_max_version: None,
            };
            match judge_update(ext, &offer, policy) {
                UpdateDecision::Accept => UpdateStatus::Available(offer),
                UpdateDecision::Refuse(reason) => UpdateStatus::Refused { offer, reason },
            }
        }
    }
}

fn firefox_status(
    ext: &InstalledExtension,
    mut updates: Vec<AvailableUpdate>,
    policy: &UpdatePolicy,
) -> UpdateStatus {
    updates.sort_by(|a, b| compare_toolkit_versions(&b.version, &a.version));
    let mut first_refusal: Option<(UpdateOffer, RefuseReason)> = None;
    for u in updates {
        let offer = UpdateOffer {
            id: ext.id.clone(),
            version: u.version,
            url: u.update_link,
            hash: u.update_hash,
            size: None,
            strict_min_version: u.strict_min_version,
            strict_max_version: u.strict_max_version,
        };
        match judge_update(ext, &offer, policy) {
            UpdateDecision::Accept => return UpdateStatus::Available(offer),
            UpdateDecision::Refuse(reason) => {
                // A downgrade or the same version just means "nothing newer".
                if !matches!(reason, RefuseReason::Downgrade | RefuseReason::SameVersion)
                    && first_refusal.is_none()
                {
                    first_refusal = Some((offer, reason));
                }
            }
        }
    }
    match first_refusal {
        Some((offer, reason)) => UpdateStatus::Refused { offer, reason },
        None => UpdateStatus::UpToDate,
    }
}
