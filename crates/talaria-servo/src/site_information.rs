// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! Cookies and site data of one site, for the site-information popover.
//!
//! The engine calls run on the UI thread and are guarded like "Clear browsing data"
//! (`site_data::contained`): if the resource threads do not answer, the popover shows no site
//! data and "Clear" clears nothing, instead of the browser panicking.

use mercury_site_security::site_covers_host;
use servo::{CookieSource, Servo, StorageType};
use url::Url;

use crate::site_data::contained;

/// What the engine keeps for the site of an address.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SiteDataSummary {
    /// Cookies the site's pages would be sent.
    pub cookies: usize,
    pub local_storage: bool,
    pub session_storage: bool,
}

impl SiteDataSummary {
    pub const fn is_empty(&self) -> bool {
        self.cookies == 0 && !self.local_storage && !self.session_storage
    }
}

/// The engine's sites (registrable domains) that hold data for `url`'s host.
fn sites_for(servo: &Servo, url: &Url, storage_types: StorageType) -> Vec<(String, StorageType)> {
    let Some(host) = url.host_str() else {
        return Vec::new();
    };
    servo
        .site_data_manager()
        .site_data(storage_types)
        .into_iter()
        .filter(|site| site_covers_host(&site.name(), host))
        .map(|site| (site.name(), site.storage_types()))
        .collect()
}

pub(crate) fn site_data_summary(servo: &Servo, url: &Url) -> SiteDataSummary {
    if !matches!(url.scheme(), "http" | "https") {
        return SiteDataSummary::default();
    }
    contained("reading the site data of one site", || {
        let cookies = servo
            .site_data_manager()
            .cookies_for_url(url.clone(), CookieSource::HTTP)
            .len();
        let stored = sites_for(servo, url, StorageType::Local | StorageType::Session)
            .into_iter()
            .fold(StorageType::empty(), |all, (_, types)| all | types);
        SiteDataSummary {
            cookies,
            local_storage: stored.contains(StorageType::Local),
            session_storage: stored.contains(StorageType::Session),
        }
    })
}

/// Clears the cookies and web storage of `url`'s site. Returns how many sites were cleared
/// (0 if the engine did not answer).
pub(crate) fn clear_data_for_site(servo: &Servo, url: &Url) -> usize {
    contained("clearing the data of one site", || {
        let all = StorageType::Cookies | StorageType::Local | StorageType::Session;
        let sites = sites_for(servo, url, all)
            .into_iter()
            .map(|(name, _)| name)
            .collect::<Vec<_>>();
        if sites.is_empty() {
            return 0;
        }
        let names = sites.iter().map(String::as_str).collect::<Vec<_>>();
        servo.site_data_manager().clear_site_data(&names, all);
        log::info!("cleared the cookies and site data of {}", names.join(", "));
        sites.len()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_failing_engine_call_gives_the_empty_answer() {
        let summary: SiteDataSummary = contained("reading site data", || {
            panic!("the resource thread is gone")
        });
        assert_eq!(summary, SiteDataSummary::default());
        assert_eq!(contained("clearing", || 3_usize), 3);
    }
}
