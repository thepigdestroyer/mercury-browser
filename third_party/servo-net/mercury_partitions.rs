// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
// Mercury: the partitioned cookie jar (EC7, storage side, net-2 lane, decisions N2-D1 to N2-D3). A new Mercury
// file, written from the CHIPS draft and the net-2 spec; nothing in it is copied from Servo or another project.
// The hooks are in `cookie_storage.rs` and `lib.rs`; `MERCURY.md` lists them.

//! Cookies that are kept per top-level site (CHIPS, the `Partitioned` attribute).
//!
//! A `Set-Cookie` with `Partitioned` is kept in a **partition**, keyed by the top-level site of the request
//! that set it (the network partition key). A partition is a [`CookieStorage`] of its own, so every rule of
//! the jar (the `Secure` rule, the per-host cap, eviction, expiry) applies to it unchanged. A partitioned
//! cookie that is not `Secure`, or that has no partition key, is refused: nothing is stored, nothing is sent.
//!
//! The unpartitioned cookies stay in the main jar and are sent only in [`CookieAccess::Full`] mode. In
//! [`CookieAccess::PartitionedOnly`] (a third-party request with blocking on) only the partition of the
//! top-level site is used. In [`CookieAccess::Nothing`] (the top-level site is unknown or opaque) nothing is
//! stored and nothing is sent.
//!
//! At most [`MAX_PARTITIONS`] partitions exist; a new one over the cap is refused, and the cookie with it, while
//! the existing partitions keep working.

use cookie::Cookie;
use log::{info, warn};
use net_traits::CookieSource;
use servo_url::ServoUrl;

use crate::cookie::ServoCookie;
use crate::cookie_storage::{CookieStorage, is_cookie_expired, reg_host};
use crate::net_policy::SiteKey;

/// The most partitions one jar holds. A partition beyond this is refused (fail closed).
pub const MAX_PARTITIONS: usize = 2048;

/// Which cookies a request may use, from the sites of the request (decision N2-D3).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CookieAccess {
    /// First party, or allowed: the unpartitioned jar and the partition of the top-level site.
    Full,
    /// Third party with blocking on, top-level site known: the partition of the top-level site only.
    PartitionedOnly,
    /// Top-level site unknown or opaque with blocking on: no cookie is stored or sent.
    Nothing,
}

/// The cookie access of one request or document: its mode and the partition key (the top-level site).
#[derive(Clone, Debug)]
pub struct CookieScope {
    pub access: CookieAccess,
    pub partition: Option<SiteKey>,
}

impl CookieStorage {
    /// Stores a cookie that a request or document set, under the rules of its [`CookieScope`].
    pub fn push_scoped(
        &mut self,
        cookie: ServoCookie,
        url: &ServoUrl,
        source: CookieSource,
        scope: &CookieScope,
    ) {
        if scope.access == CookieAccess::Nothing {
            return;
        }
        if cookie.cookie.partitioned() != Some(true) {
            if scope.access == CookieAccess::Full {
                self.push(cookie, url, source);
            }
            return;
        }
        let Some(key) = scope.partition.as_ref() else {
            return;
        };
        if !cookie.cookie.secure().unwrap_or(false) {
            return;
        }
        if !self.partitions.contains_key(key) {
            if self.partitions.len() >= MAX_PARTITIONS {
                warn!(" === PARTITION REFUSED: the jar holds {} partitions", MAX_PARTITIONS);
                return;
            }
            self.version = 2;
        }
        let max_per_host = self.max_per_host;
        self.partitions
            .entry(key.clone())
            .or_insert_with(|| CookieStorage::new(max_per_host))
            .push(cookie, url, source);
    }

    /// The cookie string a request or document may send, under the rules of its [`CookieScope`]: the cookies that
    /// `may_send` accepts, sorted as RFC 6265 section 5.4 sorts them, in one string.
    pub fn cookies_for_scope(
        &mut self,
        url: &ServoUrl,
        source: CookieSource,
        scope: &CookieScope,
        may_send: &dyn Fn(&Cookie<'static>) -> bool,
    ) -> Option<String> {
        if scope.access == CookieAccess::Nothing {
            return None;
        }
        let domain = reg_host(url.host_str().unwrap_or(""));
        let CookieStorage {
            cookies_map,
            partitions,
            ..
        } = self;

        let mut matched: Vec<&mut ServoCookie> = Vec::new();
        if scope.access == CookieAccess::Full {
            if let Some(cookies) = cookies_map.get_mut(&domain) {
                matched.extend(
                    cookies
                        .iter_mut()
                        .filter(|c| may_be_sent(c, url, source, may_send)),
                );
            }
        }
        if let Some(partition) = scope.partition.as_ref().and_then(|key| partitions.get_mut(key)) {
            if let Some(cookies) = partition.cookies_map.get_mut(&domain) {
                matched.extend(
                    cookies
                        .iter_mut()
                        .filter(|c| may_be_sent(c, url, source, may_send)),
                );
            }
        }
        matched.sort_by(|a, b| CookieStorage::cookie_comparator(a, b));

        let sent_count = matched.len();
        let cookie_string = matched
            .into_iter()
            .map(|c| {
                c.touch();
                format!("{}={}", c.cookie.name(), c.cookie.value())
            })
            .collect::<Vec<_>>()
            .join("; ");
        // Mercury: the count only, never the string (EC13), as `cookies_for_request` does.
        info!(" === COOKIES SENT: {} cookie(s)", sent_count);

        match cookie_string.len() {
            0 => None,
            _ => Some(cookie_string),
        }
    }

    /// How many partitions the jar holds.
    pub fn partition_count(&self) -> usize {
        self.partitions.len()
    }
}

/// Whether a cookie of a jar may go with a request to `url`: not expired, it matches the URL, and the caller's
/// rule accepts it. Expired cookies are never sent, even before the jar's clean-up has run.
fn may_be_sent(
    cookie: &ServoCookie,
    url: &ServoUrl,
    source: CookieSource,
    may_send: &dyn Fn(&Cookie<'static>) -> bool,
) -> bool {
    !is_cookie_expired(cookie) &&
        cookie.appropriate_for_url(url, source) &&
        may_send(&cookie.cookie)
}
