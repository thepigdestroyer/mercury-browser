// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! The engine table the omnibox tests share.

use talaria_core::{EngineEntry, ScopeNames, SearchModes};

fn entry(id: &str, name: &str, keyword: &str, url_template: &str) -> EngineEntry {
    EngineEntry {
        id: id.to_owned(),
        name: name.to_owned(),
        keyword: keyword.to_owned(),
        url_template: url_template.to_owned(),
    }
}

/// `duckduckgo`, `wikipedia`, `bing`, a custom `docs` (`d`) and a custom `path` (`@path`).
pub fn engines() -> Vec<EngineEntry> {
    vec![
        entry(
            "duckduckgo",
            "DuckDuckGo",
            "@duckduckgo",
            "https://duckduckgo.com/?q={searchTerms}",
        ),
        entry(
            "wikipedia",
            "Wikipedia",
            "@wikipedia",
            "https://en.wikipedia.org/w/index.php?title=Special:Search&search={searchTerms}",
        ),
        entry(
            "bing",
            "Bing",
            "@bing",
            "https://www.bing.com/search?q={searchTerms}",
        ),
        entry(
            "docs",
            "Docs",
            "d",
            "https://docs.test/search?q={searchTerms}",
        ),
        entry(
            "path",
            "Path",
            "@path",
            "https://example.test/search/{searchTerms}",
        ),
    ]
}

/// The modes over [`engines`], default `duckduckgo`, default scope words.
pub fn modes() -> SearchModes {
    SearchModes::new(engines(), "duckduckgo", ScopeNames::default())
}
