// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! The bounce-tracking state machine (MH32). It finds the sites a tab's navigation passed through, that stored
//! data, and that the user never interacted with. It never reads the clock: every time-dependent method takes
//! `now_ms`.

use std::collections::{BTreeMap, HashMap};

use crate::counters::TabKey;
use crate::level::Setting;
use crate::site::Site;

const HOUR_MS: i64 = 3_600_000;
const DAY_MS: i64 = 86_400_000;

/// The timing and size limits of the state machine.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BounceConfig {
    /// How long after a bounce the site becomes due for purging.
    pub grace_period_ms: i64,
    /// How long a user activation protects a site.
    pub activation_lifetime_ms: i64,
    /// How soon after the previous commit a script navigation still continues the chain.
    pub client_redirect_window_ms: i64,
    /// The most hops a chain keeps.
    pub max_chain: usize,
    /// The most site records kept.
    pub max_sites: usize,
}

impl BounceConfig {
    /// The configuration for a setting. `Strict` shortens the grace period to ten minutes; `Off` gets the
    /// `Standard` configuration (the caller feeds no events while the category is off).
    pub fn for_setting(setting: Setting) -> Self {
        let grace_period_ms = match setting {
            Setting::Strict => 10 * 60_000,
            Setting::Off | Setting::Standard => HOUR_MS,
        };
        Self {
            grace_period_ms,
            activation_lifetime_ms: 45 * DAY_MS,
            client_redirect_window_ms: 10_000,
            max_chain: 32,
            max_sites: 4096,
        }
    }
}

/// What kind of navigation a hop is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NavKind {
    /// Typed, bookmark, link click with a gesture, back or forward, reload.
    User,
    /// An HTTP 3xx hop.
    Redirect,
    /// A page-started navigation with no gesture.
    Script,
}

/// What the tracker remembers about one site.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SiteRecord {
    /// The site.
    pub site: Site,
    /// The last user activation on the site, if any.
    pub last_activation_ms: Option<i64>,
    /// When the site was bounced (the earliest time, if it bounced more than once), if it is pending.
    pub bounced_at_ms: Option<i64>,
}

#[derive(Clone, Debug)]
struct Hop {
    site: Site,
    stored: bool,
}

#[derive(Clone, Debug)]
struct Chain {
    hops: Vec<Hop>,
    last_commit_ms: i64,
}

impl Chain {
    fn start(site: Site, now_ms: i64) -> Self {
        Self {
            hops: vec![Hop {
                site,
                stored: false,
            }],
            last_commit_ms: now_ms,
        }
    }

    fn push(&mut self, site: Site, now_ms: i64, max_chain: usize) {
        self.hops.push(Hop {
            site,
            stored: false,
        });
        self.last_commit_ms = now_ms;
        while self.hops.len() > max_chain && self.hops.len() > 2 {
            self.hops.remove(1);
        }
    }
}

/// The bounce-tracking state machine: one chain per tab, and one record per site the tracker still needs.
#[derive(Clone, Debug)]
pub struct BounceTracker {
    cfg: BounceConfig,
    chains: HashMap<TabKey, Chain>,
    sites: BTreeMap<Site, SiteRecord>,
}

impl BounceTracker {
    /// An empty tracker.
    pub fn new(cfg: BounceConfig) -> Self {
        Self::from_records(cfg, std::iter::empty())
    }

    /// A tracker holding the records of an earlier run (see [`BounceTracker::records`]).
    pub fn from_records(cfg: BounceConfig, rows: impl IntoIterator<Item = SiteRecord>) -> Self {
        let mut rows: Vec<SiteRecord> = rows
            .into_iter()
            .filter(|row| row.last_activation_ms.is_some() || row.bounced_at_ms.is_some())
            .collect();
        rows.sort_by_key(retention);
        let excess = rows.len().saturating_sub(cfg.max_sites);
        let sites = rows
            .into_iter()
            .skip(excess)
            .map(|row| (row.site.clone(), row))
            .collect();
        Self {
            cfg,
            chains: HashMap::new(),
            sites,
        }
    }

    /// One hop of a top-level navigation on `tab`. `None` is a page with no site (`data:`, `about:`, `file:`): it
    /// closes the chain and starts none.
    pub fn navigated(&mut self, tab: TabKey, site: Option<Site>, kind: NavKind, now_ms: i64) {
        let Some(site) = site else {
            self.close(tab, now_ms);
            return;
        };
        let extends = match (kind, self.chains.get(&tab)) {
            (NavKind::User, _) | (_, None) => false,
            (NavKind::Redirect, Some(_)) => true,
            (NavKind::Script, Some(chain)) => {
                now_ms.saturating_sub(chain.last_commit_ms) <= self.cfg.client_redirect_window_ms
            }
        };
        if extends {
            if let Some(chain) = self.chains.get_mut(&tab) {
                chain.push(site, now_ms, self.cfg.max_chain);
            }
            return;
        }
        self.close(tab, now_ms);
        self.chains.insert(tab, Chain::start(site, now_ms));
    }

    /// The site set a cookie or used storage while `tab` was in a chain.
    pub fn storage_accessed(&mut self, tab: TabKey, site: &Site) {
        if let Some(chain) = self.chains.get_mut(&tab) {
            for hop in chain.hops.iter_mut().filter(|hop| hop.site == *site) {
                hop.stored = true;
            }
        }
    }

    /// A click or key press on the page of `site`: the site is no longer a candidate.
    pub fn user_activated(&mut self, site: &Site, now_ms: i64) {
        let record = self
            .sites
            .entry(site.clone())
            .or_insert_with(|| SiteRecord {
                site: site.clone(),
                last_activation_ms: None,
                bounced_at_ms: None,
            });
        record.last_activation_ms = Some(now_ms);
        record.bounced_at_ms = None;
        self.enforce_bounds();
    }

    /// The tab closed: its chain closes.
    pub fn tab_closed(&mut self, tab: TabKey, now_ms: i64) {
        self.close(tab, now_ms);
    }

    /// The sites whose bounce is past the grace period, sorted, and that are not `exempt`. Pure: a due site stays
    /// due until [`BounceTracker::purged`].
    pub fn due(&self, now_ms: i64, exempt: &dyn Fn(&Site) -> bool) -> Vec<Site> {
        self.sites
            .values()
            .filter(|record| {
                record
                    .bounced_at_ms
                    .is_some_and(|at| now_ms.saturating_sub(at) >= self.cfg.grace_period_ms)
                    && !exempt(&record.site)
            })
            .map(|record| record.site.clone())
            .collect()
    }

    /// The site's data was cleared: forget its bounce.
    pub fn purged(&mut self, site: &Site) {
        let forget = match self.sites.get_mut(site) {
            Some(record) => {
                record.bounced_at_ms = None;
                record.last_activation_ms.is_none()
            }
            None => false,
        };
        if forget {
            self.sites.remove(site);
        }
    }

    /// Drops the activations older than the lifetime; pending bounces are kept.
    pub fn prune(&mut self, now_ms: i64) {
        let lifetime = self.cfg.activation_lifetime_ms;
        for record in self.sites.values_mut() {
            if record
                .last_activation_ms
                .is_some_and(|at| now_ms.saturating_sub(at) > lifetime)
            {
                record.last_activation_ms = None;
            }
        }
        self.sites.retain(|_, record| {
            record.last_activation_ms.is_some() || record.bounced_at_ms.is_some()
        });
    }

    /// The records for persistence, sorted by site.
    pub fn records(&self) -> Vec<SiteRecord> {
        self.sites.values().cloned().collect()
    }

    fn close(&mut self, tab: TabKey, now_ms: i64) {
        if let Some(chain) = self.chains.remove(&tab) {
            self.bounce_chain(&chain, now_ms);
        }
    }

    fn bounce_chain(&mut self, chain: &Chain, now_ms: i64) {
        let hops = &chain.hops;
        let (Some(first), Some(last)) = (hops.first(), hops.last()) else {
            return;
        };
        let candidates: Vec<Site> = hops
            .windows(3)
            .filter_map(|window| {
                let [prev, hop, next] = window else {
                    return None;
                };
                let is_candidate = hop.stored
                    && hop.site != first.site
                    && hop.site != last.site
                    && hop.site != prev.site
                    && hop.site != next.site
                    && !self.protected(&hop.site, now_ms);
                is_candidate.then(|| hop.site.clone())
            })
            .collect();
        for site in candidates {
            let record = self
                .sites
                .entry(site.clone())
                .or_insert_with(|| SiteRecord {
                    site,
                    last_activation_ms: None,
                    bounced_at_ms: None,
                });
            record.bounced_at_ms = Some(record.bounced_at_ms.map_or(now_ms, |at| at.min(now_ms)));
        }
        self.enforce_bounds();
    }

    fn protected(&self, site: &Site, now_ms: i64) -> bool {
        self.sites
            .get(site)
            .and_then(|record| record.last_activation_ms)
            .is_some_and(|at| now_ms.saturating_sub(at) <= self.cfg.activation_lifetime_ms)
    }

    fn enforce_bounds(&mut self) {
        while self.sites.len() > self.cfg.max_sites {
            let victim = self
                .sites
                .values()
                .min_by_key(|record| retention(record))
                .map(|record| record.site.clone());
            let Some(victim) = victim else {
                break;
            };
            self.sites.remove(&victim);
        }
    }
}

/// The eviction order: records without a pending bounce go first, then the oldest by their latest time.
fn retention(record: &SiteRecord) -> (bool, i64) {
    (
        record.bounced_at_ms.is_some(),
        record
            .last_activation_ms
            .or(record.bounced_at_ms)
            .unwrap_or(i64::MIN),
    )
}
