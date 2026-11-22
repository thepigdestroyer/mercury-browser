// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! Engine keywords, scopes and Tab completion (stub: the modes task fills in the bodies).

use url::Url;

use crate::address::AddressResolutionError;

/// A search engine as the address bar needs to know it; the application builds the list from its
/// settings (`SearchSettings::engines()`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EngineEntry {
    /// The engine's stable id.
    pub id: String,
    /// The name shown to the user.
    pub name: String,
    /// The address-bar keyword (`@wikipedia`, or a custom one such as `w`); empty for none.
    pub keyword: String,
    /// The results page URL with `{searchTerms}` where the query goes.
    pub url_template: String,
}

impl EngineEntry {
    /// The engine's home page: the origin of `url_template` with the path `/`.
    pub fn home_url(&self) -> Option<Url> {
        None
    }
}

/// The words that select a suggestion source (the first word of the typed text).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScopeNames {
    /// Open tabs.
    pub tabs: String,
    /// Bookmarks.
    pub bookmarks: String,
    /// History.
    pub history: String,
    /// Browser actions (clear data, settings, mute...).
    pub actions: String,
}

impl Default for ScopeNames {
    fn default() -> Self {
        Self {
            tabs: "@tabs".to_owned(),
            bookmarks: "@bookmarks".to_owned(),
            history: "@history".to_owned(),
            actions: "@actions".to_owned(),
        }
    }
}

/// A suggestion source the typed text can be narrowed to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    /// Open tabs.
    Tabs,
    /// Bookmarks.
    Bookmarks,
    /// History.
    History,
    /// Browser actions.
    Actions,
}

/// What typed text means to the address bar.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Typed {
    /// An address to open.
    Address(Url),
    /// A search. `query` has the keyword removed; `engine_id` is the default engine when no
    /// keyword was used.
    Search {
        /// The engine that searches.
        engine_id: String,
        /// The text to search for.
        query: String,
        /// Whether a keyword chose the engine.
        by_keyword: bool,
    },
    /// Text narrowed to one source (`query` is empty for the bare scope word).
    Scope {
        /// The source.
        scope: Scope,
        /// What to look for in it.
        query: String,
    },
}

/// What Tab does to the typed text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TabCompletion {
    /// The new field text: the keyword or scope word and one space.
    pub text: String,
    /// What the text now selects.
    pub mode: CompletionMode,
}

/// What a completed word selects.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CompletionMode {
    /// A search engine.
    Engine {
        /// The engine's id.
        engine_id: String,
    },
    /// A suggestion source.
    Scope(Scope),
}

/// One entry of the list a bare `@` shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AtCandidate {
    /// The word to put in the field.
    pub text: String,
    /// What to show beside it (the engine's name, or "Tabs" ...).
    pub label: String,
    /// What it selects.
    pub mode: CompletionMode,
}

/// The engines, the default engine and the scope words, with the rules that use them.
#[derive(Debug, Clone)]
pub struct SearchModes {
    pub(crate) engines: Vec<EngineEntry>,
    pub(crate) default_engine: String,
    pub(crate) scopes: ScopeNames,
}

impl SearchModes {
    /// `default_engine` is an engine id; if no engine has it the first engine is the default.
    pub fn new(engines: Vec<EngineEntry>, default_engine: &str, scopes: ScopeNames) -> Self {
        Self {
            engines,
            default_engine: default_engine.to_owned(),
            scopes,
        }
    }

    /// What `typed` means. The same rule as `classify_address` for addresses; a scope word or an
    /// engine keyword as the first word, followed by white space, narrows or chooses an engine.
    pub fn interpret(&self, _typed: &str) -> Result<Typed, AddressResolutionError> {
        Err(AddressResolutionError::EmptyInput)
    }

    /// What Tab does: complete an exact or unique-prefix keyword or scope word.
    pub fn complete_with_tab(&self, _typed: &str) -> Option<TabCompletion> {
        None
    }

    /// The scope words and engine keywords that start with `typed` (which starts with `@`), scopes
    /// first, then engines in list order.
    pub fn at_candidates(&self, _typed: &str) -> Vec<AtCandidate> {
        Vec::new()
    }

    /// Whether an engine keyword spelt `keyword` would be shadowed by a scope word.
    pub fn keyword_clashes_with_scope(&self, _keyword: &str) -> bool {
        false
    }
}
