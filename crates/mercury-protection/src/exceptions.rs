// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! Per-site exceptions: shields down, a site's own level and per-category overrides (R13 MH12).

use std::collections::BTreeMap;

use crate::level::{Category, ProtectionLevel, Setting};
use crate::site::Site;

/// How many sites can have an exception.
pub const MAX_EXCEPTIONS: usize = 5_000;

/// Per-category settings for one site. `None` keeps the setting the site's level gives.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Overrides([Option<Setting>; Category::COUNT]);

impl Overrides {
    /// The override for one category, if there is one.
    pub fn get(&self, category: Category) -> Option<Setting> {
        self.0[category.index()]
    }

    /// Sets the override for one category; `None` clears it.
    pub fn set(&mut self, category: Category, setting: Option<Setting>) {
        self.0[category.index()] = setting;
    }

    /// Whether no category is overridden.
    pub fn is_empty(&self) -> bool {
        self.0.iter().all(Option::is_none)
    }

    /// The overrides, in [`Category::ALL`] order.
    pub fn iter(&self) -> impl Iterator<Item = (Category, Setting)> + '_ {
        Category::ALL
            .into_iter()
            .filter_map(|category| self.get(category).map(|setting| (category, setting)))
    }

    /// The overrides as text, `ads_and_trackers=off,cross_site_cookies=strict`, with the stable ids.
    pub fn to_text(&self) -> String {
        self.iter()
            .map(|(category, setting)| format!("{}={}", category.id(), setting.id()))
            .collect::<Vec<_>>()
            .join(",")
    }

    /// Reads the text [`Overrides::to_text`] writes. Also returns how many pairs were dropped: an id this build
    /// does not know (a newer build's category or setting), or a pair with no `=`.
    pub fn from_text(text: &str) -> (Overrides, usize) {
        let mut overrides = Overrides::default();
        let mut dropped = 0;
        for pair in text
            .split(',')
            .map(str::trim)
            .filter(|pair| !pair.is_empty())
        {
            let parsed = pair.split_once('=').and_then(|(id, setting)| {
                Some((
                    Category::from_id(id.trim())?,
                    Setting::from_id(setting.trim())?,
                ))
            });
            match parsed {
                Some((category, setting)) => overrides.set(category, Some(setting)),
                None => dropped += 1,
            }
        }
        (overrides, dropped)
    }
}

/// One site's exception record.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SiteException {
    /// The site.
    pub site: Site,
    /// Whether protections are off on this site (shields down).
    pub shields_down: bool,
    /// The level for this site, if it has its own.
    pub level: Option<ProtectionLevel>,
    /// Per-category overrides, applied on top of the level.
    pub overrides: Overrides,
    /// When the record last changed, in milliseconds since the Unix epoch.
    pub updated_at_ms: i64,
}

impl SiteException {
    /// Whether the record changes anything. One that does not is removed instead of stored.
    fn changes_anything(&self) -> bool {
        self.shields_down || self.level.is_some() || !self.overrides.is_empty()
    }
}

/// Why an exception was not stored.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ExceptionError {
    /// [`MAX_EXCEPTIONS`] sites already have an exception.
    #[error("too many site exceptions")]
    Full,
}

/// The per-site exceptions, one per site, at most [`MAX_EXCEPTIONS`].
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ExceptionList(BTreeMap<Site, SiteException>);

impl ExceptionList {
    /// The exception for a site.
    pub fn get(&self, site: &Site) -> Option<&SiteException> {
        self.0.get(site)
    }

    /// Stores an exception. One that changes nothing removes the site's record instead. Fails with
    /// [`ExceptionError::Full`] only for a new site when the list is at [`MAX_EXCEPTIONS`].
    pub fn set(&mut self, exception: SiteException) -> Result<(), ExceptionError> {
        if !exception.changes_anything() {
            self.0.remove(&exception.site);
            return Ok(());
        }
        if self.0.len() >= MAX_EXCEPTIONS && !self.0.contains_key(&exception.site) {
            return Err(ExceptionError::Full);
        }
        self.0.insert(exception.site.clone(), exception);
        Ok(())
    }

    /// Removes a site's exception and returns it.
    pub fn remove(&mut self, site: &Site) -> Option<SiteException> {
        self.0.remove(site)
    }

    /// The exceptions, ordered by site.
    pub fn iter(&self) -> impl Iterator<Item = &SiteException> {
        self.0.values()
    }

    /// How many sites have an exception.
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Whether no site has an exception.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}
