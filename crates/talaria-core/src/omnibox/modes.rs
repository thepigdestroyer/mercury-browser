// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! Engine keywords, scopes and Tab completion.

use url::Url;

use crate::address::{AddressResolutionError, NavigationTarget, classify_address};

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
        let mut url = Url::parse(&self.url_template).ok()?;
        url.set_path("/");
        url.set_query(None);
        url.set_fragment(None);
        Some(url)
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
    pub fn interpret(&self, typed: &str) -> Result<Typed, AddressResolutionError> {
        let trimmed = typed.trim();
        if trimmed.is_empty() {
            return Err(AddressResolutionError::EmptyInput);
        }

        if let Some((scope, query)) = self.scope_and_query(trimmed) {
            return Ok(Typed::Scope { scope, query });
        }

        if let Some((engine, query)) = self.keyword_and_query(trimmed) {
            return Ok(Typed::Search {
                engine_id: engine.id.clone(),
                query: query.to_owned(),
                by_keyword: true,
            });
        }

        match classify_address(trimmed)? {
            NavigationTarget::Url(url) => Ok(Typed::Address(url)),
            NavigationTarget::SearchQuery(query) => {
                let engine = self.default().ok_or_else(|| {
                    AddressResolutionError::Search("no search engine is configured".to_owned())
                })?;
                Ok(Typed::Search {
                    engine_id: engine.id.clone(),
                    query,
                    by_keyword: false,
                })
            }
        }
    }

    /// What Tab does: complete an exact or unique-prefix keyword or scope word.
    pub fn complete_with_tab(&self, typed: &str) -> Option<TabCompletion> {
        let word = typed.trim();
        if word.is_empty() || word.contains(char::is_whitespace) {
            return None;
        }

        let lowered = word.to_lowercase();
        let has_at = lowered.starts_with('@');
        let all = self.all_candidates();

        let matches =
            |candidate: &AtCandidate| has_at && candidate.text.to_lowercase().starts_with(&lowered);

        let exact = |candidate: &AtCandidate| {
            let text = candidate.text.to_lowercase();
            let bare = text.strip_prefix('@').unwrap_or(&text);
            text == lowered || (!has_at && bare == lowered)
        };

        if let Some(candidate) = all.iter().find(|candidate| exact(candidate)) {
            return Some(TabCompletion {
                text: format!("{} ", candidate.text),
                mode: candidate.mode.clone(),
            });
        }

        let prefixed: Vec<&AtCandidate> =
            all.iter().filter(|candidate| matches(candidate)).collect();
        match prefixed.as_slice() {
            [only] => Some(TabCompletion {
                text: format!("{} ", only.text),
                mode: only.mode.clone(),
            }),
            _ => None,
        }
    }

    /// The scope words and engine keywords that start with `typed` (which starts with `@`), scopes
    /// first, then engines in list order.
    pub fn at_candidates(&self, typed: &str) -> Vec<AtCandidate> {
        let typed = typed.trim().to_lowercase();
        if !typed.starts_with('@') {
            return Vec::new();
        }

        self.all_candidates()
            .into_iter()
            .filter(|candidate| candidate.text.to_lowercase().starts_with(&typed))
            .collect()
    }

    /// Whether an engine keyword spelt `keyword` would be shadowed by a scope word.
    pub fn keyword_clashes_with_scope(&self, keyword: &str) -> bool {
        let keyword = keyword.to_lowercase();
        if keyword.is_empty() {
            return false;
        }

        self.scope_names()
            .iter()
            .any(|(name, _)| name.to_lowercase() == keyword)
    }

    fn default(&self) -> Option<&EngineEntry> {
        self.engines
            .iter()
            .find(|engine| engine.id == self.default_engine)
            .or_else(|| self.engines.first())
    }

    fn scope_names(&self) -> [(&str, Scope); 4] {
        [
            (self.scopes.tabs.as_str(), Scope::Tabs),
            (self.scopes.bookmarks.as_str(), Scope::Bookmarks),
            (self.scopes.history.as_str(), Scope::History),
            (self.scopes.actions.as_str(), Scope::Actions),
        ]
    }

    /// The first word, if it is a scope word, and the rest of the text.
    fn scope_and_query(&self, trimmed: &str) -> Option<(Scope, String)> {
        let (word, rest) = split_first_word(trimmed)?;
        let word = word.to_lowercase();
        let scope = self
            .scope_names()
            .into_iter()
            .find(|(name, _)| name.to_lowercase() == word)
            .map(|(_, scope)| scope)?;
        Some((scope, rest.trim().to_owned()))
    }

    /// The engine whose keyword is the first word followed by white space and a non-empty rest.
    fn keyword_and_query<'a>(&'a self, trimmed: &'a str) -> Option<(&'a EngineEntry, &'a str)> {
        let (word, rest) = trimmed.split_once(char::is_whitespace)?;
        let query = rest.trim();
        if word.is_empty() || query.is_empty() {
            return None;
        }

        let word = word.to_lowercase();
        self.engines
            .iter()
            .find(|engine| !engine.keyword.is_empty() && engine.keyword.to_lowercase() == word)
            .map(|engine| (engine, query))
    }

    /// Every scope word then every engine keyword, as completion candidates.
    fn all_candidates(&self) -> Vec<AtCandidate> {
        let scopes = self
            .scope_names()
            .into_iter()
            .map(|(name, scope)| AtCandidate {
                text: name.to_owned(),
                label: scope_label(scope).to_owned(),
                mode: CompletionMode::Scope(scope),
            });
        let engines = self
            .engines
            .iter()
            .filter(|engine| !engine.keyword.is_empty())
            .map(|engine| AtCandidate {
                text: engine.keyword.clone(),
                label: engine.name.clone(),
                mode: CompletionMode::Engine {
                    engine_id: engine.id.clone(),
                },
            });

        scopes.chain(engines).collect()
    }
}

fn split_first_word(trimmed: &str) -> Option<(&str, &str)> {
    let end = trimmed.find(char::is_whitespace).unwrap_or(trimmed.len());
    let (word, rest) = trimmed.split_at(end);
    (!word.is_empty()).then_some((word, rest))
}

fn scope_label(scope: Scope) -> &'static str {
    match scope {
        Scope::Tabs => "Tabs",
        Scope::Bookmarks => "Bookmarks",
        Scope::History => "History",
        Scope::Actions => "Actions",
    }
}
