// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! Images per site: which sites have images turned off. Pure data; the file is
//! `mercury_settings::SiteImagesStore`, the engine's guard is in `talaria-servo`. Item 4 (images)
//! fills in the bodies.
//!
//! The key of a site is the exact host of the tab's top-level page (not the registrable domain,
//! not the origin; brief decision O9).

use std::collections::BTreeSet;

use url::Url;

/// The most sites kept; further ones are refused.
pub const MAX_IMAGE_BLOCKED_SITES: usize = 1000;

/// Why a site could not be changed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SiteImagesError {
    /// The page is not an `http(s)` page.
    NotAWebPage,
    /// [`MAX_IMAGE_BLOCKED_SITES`] sites are already blocked.
    TooMany,
}

/// The key of the site of `page`: the host of an `http(s)` page in ASCII (punycode), lower case,
/// without port or trailing dot, IPv6 in brackets; `None` for anything else.
pub fn image_site_key(_page: &Url) -> Option<String> {
    todo!("item 4: images")
}

/// The sites whose images are turned off.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SiteImages {
    blocked: BTreeSet<String>,
}

impl SiteImages {
    /// A set from stored keys: normalised, invalid and duplicate entries dropped, capped at
    /// [`MAX_IMAGE_BLOCKED_SITES`].
    pub fn from_hosts(_hosts: impl IntoIterator<Item = String>) -> Self {
        todo!("item 4: images")
    }

    /// The engine's one question: is this load an image of a page whose site has images off?
    /// `page` is the tab's top-level page; `None` (no page yet) never blocks.
    pub fn blocks_image_load(&self, _page: Option<&Url>, _is_image: bool) -> bool {
        todo!("item 4: images")
    }

    /// Whether the site of `page` has images off.
    pub fn blocks(&self, _page: &Url) -> bool {
        todo!("item 4: images")
    }

    /// Turns images off (`true`) or on (`false`) for the site of `page`. `Ok(true)` if that
    /// changed something.
    pub fn set_blocked(&mut self, _page: &Url, _blocked: bool) -> Result<bool, SiteImagesError> {
        todo!("item 4: images")
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
