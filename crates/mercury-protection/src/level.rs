// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! Protection levels, categories and the table that says what each level switches on (MH13).

use std::collections::BTreeMap;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// One kind of protection. The ids (`Category::id`) are stored in profiles and never change.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Category {
    /// Ad and tracker lists. Standard applies the rules to third-party requests; Strict also to first-party ones.
    AdsAndTrackers,
    /// Social-media trackers and embeds' tracking.
    SocialTrackers,
    /// Cookies in a third-party context (owner decision O-18: blocked by default).
    CrossSiteCookies,
    /// Fingerprinting: farbling and rounded screen geometry (R14c D3: on at Standard).
    Fingerprinting,
    /// Cryptocurrency miners.
    Cryptominers,
    /// Cookie-consent banners (the cookie-banner list; O-18: on).
    CookieBanners,
    /// Bounce tracking: storage of redirect-only sites is purged.
    BounceTracking,
    /// Tracking parameters stripped from cross-site navigations.
    UrlCleaning,
    /// Local-network access: a public page cannot reach loopback or the local network. Turning it off for a site
    /// is that site's "allow local network" grant.
    LocalNetwork,
}

impl Category {
    /// How many categories there are.
    pub const COUNT: usize = 9;

    /// Every category, in the order the panel lists them.
    pub const ALL: [Category; Category::COUNT] = [
        Category::AdsAndTrackers,
        Category::SocialTrackers,
        Category::CrossSiteCookies,
        Category::Fingerprinting,
        Category::Cryptominers,
        Category::CookieBanners,
        Category::BounceTracking,
        Category::UrlCleaning,
        Category::LocalNetwork,
    ];

    /// The permanent text id.
    pub fn id(self) -> &'static str {
        match self {
            Category::AdsAndTrackers => "ads_and_trackers",
            Category::SocialTrackers => "social_trackers",
            Category::CrossSiteCookies => "cross_site_cookies",
            Category::Fingerprinting => "fingerprinting",
            Category::Cryptominers => "cryptominers",
            Category::CookieBanners => "cookie_banners",
            Category::BounceTracking => "bounce_tracking",
            Category::UrlCleaning => "url_cleaning",
            Category::LocalNetwork => "local_network",
        }
    }

    /// The category with this id, if this build knows it.
    pub fn from_id(id: &str) -> Option<Category> {
        Self::ALL.into_iter().find(|category| category.id() == id)
    }

    /// The category's position in [`Category::ALL`].
    pub fn index(self) -> usize {
        self as usize
    }
}

/// How strongly one category is applied.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Setting {
    /// Not applied.
    Off,
    /// Applied in its usual form.
    Standard,
    /// Applied in its stricter form (where the category has one; see [`ProtectionLevel::table`]).
    Strict,
}

impl Setting {
    /// The permanent text id.
    pub fn id(self) -> &'static str {
        match self {
            Setting::Off => "off",
            Setting::Standard => "standard",
            Setting::Strict => "strict",
        }
    }

    /// The setting with this id, if this build knows it.
    pub fn from_id(id: &str) -> Option<Setting> {
        [Setting::Off, Setting::Standard, Setting::Strict]
            .into_iter()
            .find(|setting| setting.id() == id)
    }

    /// Whether the category is applied at all.
    pub fn is_on(self) -> bool {
        self != Setting::Off
    }
}

/// A named set of settings: the two built-in levels, or the user's own table.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProtectionLevel {
    /// The default (owner decision O-18).
    #[default]
    Standard,
    /// Everything at its stricter reading.
    Strict,
    /// The user's own settings, one per category.
    Custom,
}

impl ProtectionLevel {
    /// The permanent text id.
    pub fn id(self) -> &'static str {
        match self {
            ProtectionLevel::Standard => "standard",
            ProtectionLevel::Strict => "strict",
            ProtectionLevel::Custom => "custom",
        }
    }

    /// The level with this id, if this build knows it.
    pub fn from_id(id: &str) -> Option<ProtectionLevel> {
        [
            ProtectionLevel::Standard,
            ProtectionLevel::Strict,
            ProtectionLevel::Custom,
        ]
        .into_iter()
        .find(|level| level.id() == id)
    }

    /// What this level switches on. `custom` is the user's table, returned as it is for [`ProtectionLevel::Custom`].
    ///
    /// Strict differs from Standard like this: ads and trackers also apply to first-party requests; social
    /// trackers, cookies, fingerprinting and cryptominers take their stricter form; bounce tracking purges after
    /// 10 minutes instead of an hour; tracking-parameter stripping includes the conditional rules. Cookie banners
    /// and local-network access are the same in both.
    pub fn table(self, custom: &Categories) -> Categories {
        match self {
            ProtectionLevel::Custom => *custom,
            ProtectionLevel::Standard => Categories::all(Setting::Standard),
            ProtectionLevel::Strict => Categories::all(Setting::Strict)
                .with(Category::CookieBanners, Setting::Standard)
                .with(Category::LocalNetwork, Setting::Standard),
        }
    }
}

/// One [`Setting`] for every [`Category`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Categories([Setting; Category::COUNT]);

impl Categories {
    /// A table with the same setting everywhere.
    pub const fn all(setting: Setting) -> Self {
        Self([setting; Category::COUNT])
    }

    /// The setting of one category.
    pub fn get(&self, category: Category) -> Setting {
        self.0[category.index()]
    }

    /// The table with one category changed.
    #[must_use]
    pub fn with(mut self, category: Category, setting: Setting) -> Self {
        self.0[category.index()] = setting;
        self
    }

    /// The categories and settings, in [`Category::ALL`] order.
    pub fn iter(&self) -> impl Iterator<Item = (Category, Setting)> + '_ {
        Category::ALL
            .into_iter()
            .map(|category| (category, self.get(category)))
    }
}

impl Default for Categories {
    fn default() -> Self {
        Self::all(Setting::Standard)
    }
}

impl Serialize for Categories {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_map(
            self.iter()
                .map(|(category, setting)| (category.id(), setting.id())),
        )
    }
}

/// Reads a map of ids to settings. A category this build does not know is ignored, a missing category takes
/// `Standard`, and a setting it does not know takes `Standard`, so a profile written by a newer build loads.
impl<'de> Deserialize<'de> for Categories {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let map = BTreeMap::<String, serde_json::Value>::deserialize(deserializer)?;
        let mut table = Categories::default();
        for (id, value) in map {
            let (Some(category), Some(setting)) = (
                Category::from_id(&id),
                value.as_str().and_then(Setting::from_id),
            ) else {
                continue;
            };
            table = table.with(category, setting);
        }
        Ok(table)
    }
}
