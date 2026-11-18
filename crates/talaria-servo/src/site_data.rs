// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! Clearing what the engine keeps for sites, for "Clear browsing data": cookies, web storage
//! (`localStorage`, `sessionStorage`) and the HTTP cache.
//!
//! Servo 0.8 cannot clear any of these by date (`SiteDataManager` clears by site or all,
//! `NetworkManager::clear_cache` clears everything), so each kind is cleared entirely, whatever
//! time range the user chose for history.

use std::panic::AssertUnwindSafe;

use servo::StorageType;

use crate::runtime::ServoRuntime;

/// What to clear.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SiteDataKinds {
    /// Every cookie, and every site's local and session storage.
    pub cookies_and_site_data: bool,
    /// The HTTP cache.
    pub cache: bool,
}

impl ServoRuntime {
    /// Clears the chosen kinds of site data, all of it. Returns false if the engine's resource
    /// threads did not answer (the failure is logged).
    pub fn clear_site_data(&self, kinds: SiteDataKinds) -> bool {
        let servo = self.servo();
        contained("clearing site data", || {
            if kinds.cookies_and_site_data {
                let manager = servo.site_data_manager();
                manager.clear_cookies(None);
                let storage = StorageType::Local | StorageType::Session;
                let sites = manager
                    .site_data(storage)
                    .iter()
                    .map(|site| site.name())
                    .collect::<Vec<_>>();
                let sites = sites.iter().map(String::as_str).collect::<Vec<_>>();
                manager.clear_site_data(&sites, storage);
            }
            if kinds.cache {
                servo.network_manager().clear_cache();
            }
            true
        })
    }
}

/// Runs `call`, which talks to the engine's site-data or network manager. Those calls are
/// synchronous round trips to the resource threads and panic when the threads do not answer
/// (during shutdown, or after the network process has gone). Such a failure is logged as
/// "`what` failed" and gives `T::default()`, so the browser carries on.
pub(crate) fn contained<T: Default>(what: &str, call: impl FnOnce() -> T) -> T {
    std::panic::catch_unwind(AssertUnwindSafe(call)).unwrap_or_else(|_| {
        log::error!("{what} failed (the engine's resource threads did not answer)");
        T::default()
    })
}
