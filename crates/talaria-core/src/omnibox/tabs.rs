// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! "Switch to tab" rows: the open tabs the typed text points at.

use url::Url;

use crate::address::{NavigationTarget, classify_address};
use crate::controller::tab_matches_query;
use crate::tab::TabId;

/// How many "Switch to tab" rows the list shows at most.
pub const SWITCH_TO_TAB_ROWS: usize = 3;

/// An open tab as the matcher needs it. The caller passes only the tabs the window may show
/// (a private window passes its private tabs only).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenTab {
    /// The tab.
    pub id: TabId,
    /// Its title.
    pub title: String,
    /// Its address.
    pub url: String,
    /// Whether it is the tab the user is typing in.
    pub is_active: bool,
}

/// A tab to offer a "Switch to tab" row for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TabMatch {
    /// The tab.
    pub tab: TabId,
    /// Whether the typed text is exactly the tab's address.
    pub exact_url: bool,
}

/// The tabs to offer for `typed`, best first, at most `limit`.
///
/// Tabs whose address is exactly the typed address come first, then the tabs whose title or
/// address contains every typed word, in strip order. The active tab and `about:` tabs are never
/// offered, and nothing is offered for text without a word of two or more characters.
pub fn switch_to_tab_matches(typed: &str, tabs: &[OpenTab], limit: usize) -> Vec<TabMatch> {
    let typed = typed.trim();
    if limit == 0 || typed.is_empty() {
        return Vec::new();
    }

    let typed_address = match classify_address(typed) {
        Ok(NavigationTarget::Url(url)) => Some(url),
        _ => None,
    };
    let has_word = typed
        .split_whitespace()
        .any(|word| word.chars().count() >= 2);

    let mut exact = Vec::new();
    let mut word = Vec::new();

    for tab in tabs {
        if tab.is_active || tab.url.to_ascii_lowercase().starts_with("about:") {
            continue;
        }

        if typed_address
            .as_ref()
            .is_some_and(|address| same_address(address, &tab.url))
        {
            exact.push(TabMatch {
                tab: tab.id,
                exact_url: true,
            });
        } else if has_word && tab_matches_query(&tab.title, &tab.url, typed) {
            word.push(TabMatch {
                tab: tab.id,
                exact_url: false,
            });
        }
    }

    exact.into_iter().chain(word).take(limit).collect()
}

/// Equal in scheme, host, port, path (a trailing `/` ignored), and query; the fragment is ignored.
fn same_address(typed: &Url, tab_url: &str) -> bool {
    let Ok(tab) = Url::parse(tab_url) else {
        return false;
    };

    typed.scheme() == tab.scheme()
        && typed.host_str() == tab.host_str()
        && typed.port() == tab.port()
        && typed.path().trim_end_matches('/') == tab.path().trim_end_matches('/')
        && typed.query() == tab.query()
}
