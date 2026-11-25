// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! Images per site: which sites have images turned off. Pure data; the file is
//! `mercury_settings::SiteImagesStore`, the engine's guard is in `talaria-servo`.
//!
//! The key of a site is the exact host of the tab's top-level page (not the registrable domain,
//! not the origin; brief decision O9): `mail.example.com` is not `maps.example.com`, and `http`
//! and `https` of one host are one site, as are two ports.
//!
//! The rule that turns a host into a key exists twice, because `mercury-settings` does not
//! depend on this crate: here ([`image_site_key`] and the private `key_of_entry`) and in
//! `mercury_settings::site_images` (`key_of_page_host` and `key_of_entry`). Both files carry the
//! same table of cases in their tests; change one, change the other.

use std::collections::BTreeSet;

use url::Url;

/// The most sites kept; further ones are refused.
pub const MAX_IMAGE_BLOCKED_SITES: usize = 1000;

/// The longest key kept, in bytes: the longest name DNS allows. A longer host cannot be a
/// resolvable site, so it has no key.
const MAX_KEY_BYTES: usize = 253;

/// The longest stored entry looked at, in bytes, before any parsing (a punycode name is longer
/// than the Unicode name it came from, so this is wider than [`MAX_KEY_BYTES`]).
const MAX_ENTRY_BYTES: usize = 1024;

/// Why a site could not be changed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SiteImagesError {
    /// The page is not an `http(s)` page.
    NotAWebPage,
    /// [`MAX_IMAGE_BLOCKED_SITES`] sites are already blocked.
    TooMany,
}

/// The key of `page` as a slice of its own host text, without allocating: `Some` for an `http(s)`
/// page with a host, `None` otherwise.
///
/// The `url` crate already gives what the key needs for these two schemes: the host is lower
/// case ASCII (IDNA, so Unicode names are punycode), percent-escapes are decoded, odd IPv4
/// forms (`127.1`, `0x7f.1`) are dotted decimal, an IPv6 literal is compressed lower case and
/// bracketed, and the port is not part of the host. What is left to do here is the trailing dot
/// (`example.com.` is `example.com`) and the length limit.
fn key_of_page_host(page: &Url) -> Option<&str> {
    if !matches!(page.scheme(), "http" | "https") {
        return None;
    }
    let key = page.host_str()?.trim_end_matches('.');
    (!key.is_empty() && key.len() <= MAX_KEY_BYTES).then_some(key)
}

/// The key of a stored entry: the text must be one host and nothing else (no scheme, port,
/// path, user, escape or space), and it is then normalised exactly as the host of a page is.
/// `None` for anything else.
fn key_of_entry(entry: &str) -> Option<String> {
    if entry.is_empty() || entry.len() > MAX_ENTRY_BYTES {
        return None;
    }
    // The URL parser strips tabs and newlines and decodes escapes, so those must be refused
    // before it sees the text: what it would make of them is not what the entry says.
    let plain = |c: char| {
        !(c.is_whitespace()
            || c.is_control()
            || matches!(c, '/' | '\\' | '?' | '#' | '@' | '%' | ':' | '[' | ']'))
    };
    let shaped = match entry
        .strip_prefix('[')
        .and_then(|rest| rest.strip_suffix(']'))
    {
        Some(literal) => literal
            .chars()
            .all(|c| c.is_ascii_hexdigit() || matches!(c, ':' | '.')),
        None => entry.chars().all(plain),
    };
    if !shaped {
        return None;
    }
    let url = Url::parse(&format!("https://{entry}/")).ok()?;
    // Whatever shape the entry had, it must have been consumed as a bare host.
    if url.path() != "/"
        || url.query().is_some()
        || url.fragment().is_some()
        || url.port().is_some()
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return None;
    }
    key_of_page_host(&url).map(str::to_owned)
}

/// The key of the site of `page`: the host of an `http(s)` page in ASCII (punycode), lower case,
/// without port or trailing dot, IPv6 in brackets; `None` for anything else (`file:`, `about:`,
/// `data:`, `view-source:`, a host over 253 bytes).
pub fn image_site_key(page: &Url) -> Option<String> {
    key_of_page_host(page).map(str::to_owned)
}

/// The sites whose images are turned off.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SiteImages {
    blocked: BTreeSet<String>,
}

impl SiteImages {
    /// A set from stored keys: each is normalised like the host of a page, invalid entries (not
    /// a bare host) and duplicates are dropped, and the first [`MAX_IMAGE_BLOCKED_SITES`] valid
    /// ones in the order given are kept.
    pub fn from_hosts(hosts: impl IntoIterator<Item = String>) -> Self {
        let mut blocked = BTreeSet::new();
        for host in hosts {
            if blocked.len() >= MAX_IMAGE_BLOCKED_SITES {
                break;
            }
            if let Some(key) = key_of_entry(&host) {
                blocked.insert(key);
            }
        }
        Self { blocked }
    }

    /// The engine's one question: is this load an image of a page whose site has images off?
    /// `page` is the tab's top-level page; `None` (no page yet) never blocks, nor does a load
    /// that is not an image. An empty set answers before looking at the page.
    pub fn blocks_image_load(&self, page: Option<&Url>, is_image: bool) -> bool {
        if !is_image || self.blocked.is_empty() {
            return false;
        }
        page.is_some_and(|page| self.blocks(page))
    }

    /// Whether the site of `page` has images off. Exact host only: a subdomain is another site.
    pub fn blocks(&self, page: &Url) -> bool {
        key_of_page_host(page).is_some_and(|key| self.blocked.contains(key))
    }

    /// Turns images off (`true`) or on (`false`) for the site of `page`. `Ok(true)` if that
    /// changed something, `Ok(false)` if the site was already so.
    ///
    /// # Errors
    ///
    /// [`SiteImagesError::NotAWebPage`] when `page` has no key (the set is untouched);
    /// [`SiteImagesError::TooMany`] when a new site would be the one after
    /// [`MAX_IMAGE_BLOCKED_SITES`] (the set is untouched; removing is always allowed).
    pub fn set_blocked(&mut self, page: &Url, blocked: bool) -> Result<bool, SiteImagesError> {
        let key = key_of_page_host(page).ok_or(SiteImagesError::NotAWebPage)?;
        if !blocked {
            return Ok(self.blocked.remove(key));
        }
        if self.blocked.contains(key) {
            return Ok(false);
        }
        if self.blocked.len() >= MAX_IMAGE_BLOCKED_SITES {
            return Err(SiteImagesError::TooMany);
        }
        Ok(self.blocked.insert(key.to_owned()))
    }

    /// The keys, sorted.
    pub fn hosts(&self) -> impl Iterator<Item = &str> {
        self.blocked.iter().map(String::as_str)
    }

    /// Whether no site is blocked (the engine's guard is then one comparison).
    pub fn is_empty(&self) -> bool {
        self.blocked.is_empty()
    }
}
