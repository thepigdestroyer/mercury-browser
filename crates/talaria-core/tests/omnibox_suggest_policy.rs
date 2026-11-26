// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

use talaria_core::{REMOTE_SUGGEST_ENGINE_IDS, remote_suggestions_allowed};

#[test]
fn only_the_four_privacy_friendly_engines_may_fetch_suggestions() {
    assert_eq!(
        REMOTE_SUGGEST_ENGINE_IDS,
        ["duckduckgo", "ecosia", "startpage", "wikipedia"]
    );

    for id in REMOTE_SUGGEST_ENGINE_IDS {
        assert!(remote_suggestions_allowed(id), "{id} should be allowed");
    }
}

#[test]
fn other_engines_never_fetch_suggestions() {
    for id in ["google", "bing", "brave", "", "DuckDuckGo", "custom-1"] {
        assert!(!remote_suggestions_allowed(id), "{id:?} should be refused");
    }
}
