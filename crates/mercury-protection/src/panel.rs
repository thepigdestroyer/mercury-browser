// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! The shield panel's view of one site: what applies and what was blocked here (MH14, MH12).

use crate::counters::TabCounters;
use crate::effective::Effective;
use crate::exceptions::SiteException;
use crate::level::{Category, ProtectionLevel, Setting};
use crate::site::Site;

/// One category as the panel shows it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PanelRow {
    /// The category.
    pub category: Category,
    /// The setting that applies to the site.
    pub setting: Setting,
    /// How many requests this category blocked on the page.
    pub count: u64,
    /// Whether the site's exception overrides this category.
    pub overridden: bool,
}

/// One blocked host as the panel lists it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BlockedHostRow {
    /// The host.
    pub host: String,
    /// The category that blocked it.
    pub category: Category,
    /// How many requests to it were blocked.
    pub count: u64,
}

/// Everything the shield panel shows for one page.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PanelView {
    /// The page's site, if it has one.
    pub site: Option<Site>,
    /// Whether protections are off here.
    pub shields_down: bool,
    /// The level the settings came from.
    pub level: ProtectionLevel,
    /// Every blocked request on the page.
    pub total_blocked: u64,
    /// One row per category, in [`Category::ALL`] order.
    pub categories: Vec<PanelRow>,
    /// The blocked hosts, in the order they were first seen.
    pub hosts: Vec<BlockedHostRow>,
}

/// Shapes the effective table and the tab's counts for the shield panel.
pub fn panel_view(
    effective: &Effective,
    exception: Option<&SiteException>,
    counters: Option<&TabCounters>,
) -> PanelView {
    let categories = Category::ALL
        .into_iter()
        .map(|category| PanelRow {
            category,
            setting: effective.setting(category),
            count: counters.map_or(0, |c| c.by_category[category.index()]),
            overridden: exception.is_some_and(|e| e.overrides.get(category).is_some()),
        })
        .collect();
    let hosts = counters
        .map(|c| {
            c.hosts()
                .map(|h| BlockedHostRow {
                    host: h.host.to_owned(),
                    category: h.category,
                    count: h.count,
                })
                .collect()
        })
        .unwrap_or_default();
    PanelView {
        site: effective.site.clone(),
        shields_down: effective.shields_down,
        level: effective.level,
        total_blocked: counters.map_or(0, |c| c.total),
        categories,
        hosts,
    }
}
