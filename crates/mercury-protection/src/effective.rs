// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! The effective protection table for one top-level site (R13 MH12, MH13).

use crate::exceptions::ExceptionList;
use crate::level::{Categories, Category, ProtectionLevel, Setting};
use crate::settings::{LnaPolicy, ProtectionSettings};
use crate::site::Site;

/// What applies to one top-level site: the result of [`ProtectionSettings::resolve`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Effective {
    /// The site, if the page has one.
    pub site: Option<Site>,
    /// Whether protections are off here (shields down).
    pub shields_down: bool,
    /// The level the table was taken from.
    pub level: ProtectionLevel,
    /// The setting of every category.
    pub categories: Categories,
    /// Global Privacy Control.
    pub gpc: bool,
    /// What happens to a request for the local network.
    pub lna: LnaPolicy,
}

impl Effective {
    /// The setting of one category.
    pub fn setting(&self, category: Category) -> Setting {
        self.categories.get(category)
    }

    /// Whether a category applies at all.
    pub fn active(&self, category: Category) -> bool {
        self.setting(category).is_on()
    }

    /// Whether ad and tracker rules block a request. Standard blocks third-party requests only; Strict blocks
    /// first-party ones too.
    pub fn blocks_ads_for(&self, third_party: bool) -> bool {
        match self.setting(Category::AdsAndTrackers) {
            Setting::Off => false,
            Setting::Standard => third_party,
            Setting::Strict => true,
        }
    }
}

impl ProtectionSettings {
    /// The effective table for a top-level site. Shields down turns every category off; otherwise the site's
    /// level (or the global one) gives the table, and the site's overrides apply on top. `None` (a page with no
    /// site) gets the global table and no exception.
    pub fn resolve(&self, exceptions: &ExceptionList, site: Option<&Site>) -> Effective {
        let exception = site.and_then(|site| exceptions.get(site));
        let level = exception.and_then(|e| e.level).unwrap_or(self.level);
        let shields_down = exception.is_some_and(|e| e.shields_down);
        let categories = if shields_down {
            Categories::all(Setting::Off)
        } else {
            let mut table = level.table(&self.custom);
            if let Some(exception) = exception {
                for (category, setting) in exception.overrides.iter() {
                    table = table.with(category, setting);
                }
            }
            table
        };
        Effective {
            site: site.cloned(),
            shields_down,
            level,
            categories,
            gpc: self.gpc,
            lna: self.lna,
        }
    }
}
