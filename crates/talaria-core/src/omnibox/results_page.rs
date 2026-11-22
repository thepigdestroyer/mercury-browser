// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! The search terms shown in place of a results page's URL (stub: the modes task fills it in).

use url::Url;

use crate::omnibox::modes::SearchModes;

impl SearchModes {
    /// The search terms to show in the field instead of `url`, when `url` is a results page of the
    /// default engine; `None` shows the URL.
    pub fn search_terms_for_url(&self, _url: &Url) -> Option<String> {
        None
    }
}
