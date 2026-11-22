// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! What the address bar does with typed text beyond a plain address or search: scopes
//! (`@tabs`, `@bookmarks`, `@history`, `@actions`), tab-to-search, modifier-Return, paste and go,
//! switch-to-tab rows and the search terms shown on a results page.
//!
//! Pure and engine-free. The application supplies the engine list and the scope words as data
//! (`SearchModes`), the open tabs and the clipboard text; nothing here reads settings, the
//! clipboard or the clock. The suggestion popup is another lane's; see
//! `.superpowers/sdd/plans/omnibox-core-spec.md` section 4 for how it calls this.

mod modes;
mod paste;
mod results_page;
mod returns;
mod suggest_policy;
mod tabs;

pub use modes::{
    AtCandidate, CompletionMode, EngineEntry, Scope, ScopeNames, SearchModes, TabCompletion, Typed,
};
pub use paste::{MAX_PASTE_BYTES, MAX_PASTE_URLS, Paste, paste_and_go};
pub use returns::{OpenIn, Platform, ReturnKeys, ReturnResolution, ReturnTarget, return_plan};
pub use suggest_policy::{REMOTE_SUGGEST_ENGINE_IDS, remote_suggestions_allowed};
pub use tabs::{OpenTab, SWITCH_TO_TAB_ROWS, TabMatch, switch_to_tab_matches};
