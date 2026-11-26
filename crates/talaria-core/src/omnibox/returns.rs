// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! Modifier-Return (draft for src/omnibox/returns.rs).

use url::Url;

use crate::address::{AddressResolutionError, NavigationTarget, classify_address};
use crate::omnibox::modes::{Scope, SearchModes, Typed};

/// The modifier keys held with Return. `alt` is Option on macOS; `meta` is Command.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ReturnKeys {
    /// Shift.
    pub shift: bool,
    /// Alt / Option.
    pub alt: bool,
    /// Control.
    pub ctrl: bool,
    /// Command (macOS) or Windows/Super.
    pub meta: bool,
}

/// Which key table applies.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Platform {
    /// macOS.
    Mac,
    /// Linux and Windows.
    Other,
}

/// Where the result opens.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpenIn {
    /// The tab the user is typing in.
    CurrentTab,
    /// A new tab; `foreground` says whether it is selected.
    NewTab {
        /// Whether the new tab becomes the selected one.
        foreground: bool,
    },
    /// A new window.
    NewWindow,
}

/// What the Return key does with the typed text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReturnResolution {
    /// What to open.
    pub target: ReturnTarget,
    /// Where to open it.
    pub open_in: OpenIn,
}

/// What a Return opens.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReturnTarget {
    /// An address.
    Address(Url),
    /// A search for `query` with the engine.
    Search {
        /// The engine's id.
        engine_id: String,
        /// The text to search for.
        query: String,
    },
    /// A scope word with its query; the popup narrows, nothing opens.
    Scope {
        /// The source.
        scope: Scope,
        /// What to look for in it.
        query: String,
    },
    /// The engine's home page (Shift+Return on a bare keyword).
    EngineHome {
        /// The engine's id.
        engine_id: String,
        /// Its home page.
        url: Url,
    },
}

/// The key table: where the result opens, and whether `www.` and `.com` are added.
pub fn return_plan(keys: ReturnKeys, platform: Platform) -> (OpenIn, bool) {
    let ReturnKeys {
        shift,
        alt,
        ctrl,
        meta,
    } = keys;

    match platform {
        Platform::Mac => match (shift, alt, ctrl, meta) {
            (false, false, false, false) => (OpenIn::CurrentTab, false),
            (false, false, false, true) => (OpenIn::NewTab { foreground: false }, false),
            (false, true, false, false) => (OpenIn::NewTab { foreground: true }, false),
            (true, _, _, true) if !alt && !ctrl => (OpenIn::NewTab { foreground: true }, false),
            (true, false, false, false) => (OpenIn::NewWindow, false),
            (false, false, true, false) => (OpenIn::CurrentTab, true),
            (true, false, true, false) => (OpenIn::NewWindow, false),
            _ => (OpenIn::CurrentTab, false),
        },
        Platform::Other => match (shift, alt, ctrl, meta) {
            (false, false, false, false) => (OpenIn::CurrentTab, false),
            (false, true, false, false) => (OpenIn::NewTab { foreground: true }, false),
            (true, true, false, false) => (OpenIn::NewTab { foreground: false }, false),
            (true, false, false, false) => (OpenIn::NewWindow, false),
            (false, false, true, false) => (OpenIn::CurrentTab, true),
            (true, false, true, false) => (OpenIn::NewWindow, false),
            _ => (OpenIn::CurrentTab, false),
        },
    }
}

impl SearchModes {
    /// What Return with `keys` held does with `typed`.
    pub fn resolve_return(
        &self,
        typed: &str,
        keys: ReturnKeys,
        platform: Platform,
    ) -> Result<ReturnResolution, AddressResolutionError> {
        let (mut open_in, www_dot_com) = return_plan(keys, platform);
        let trimmed = typed.trim();

        let interpreted = self.interpret(trimmed)?;

        // Shift on a bare keyword: the engine's home page, in the current tab.
        if keys.shift
            && let Some(engine) = self.bare_keyword_engine(trimmed)
            && let Some(url) = engine.home_url()
        {
            return Ok(ReturnResolution {
                target: ReturnTarget::EngineHome {
                    engine_id: engine.id.clone(),
                    url,
                },
                open_in: OpenIn::CurrentTab,
            });
        }

        let target = match interpreted {
            Typed::Scope { scope, query } => ReturnTarget::Scope { scope, query },
            Typed::Search {
                engine_id,
                query,
                by_keyword,
            } => {
                if by_keyword && keys.shift {
                    open_in = OpenIn::NewWindow;
                }
                ReturnTarget::Search { engine_id, query }
            }
            Typed::Address(url) => ReturnTarget::Address(url),
        };

        // The www./.com rule applies only where the text was a search for a bare word.
        if www_dot_com
            && let ReturnTarget::Search { .. } = target
            && let Some(url) = www_dot_com_address(trimmed)
        {
            return Ok(ReturnResolution {
                target: ReturnTarget::Address(url),
                open_in,
            });
        }

        Ok(ReturnResolution { target, open_in })
    }

    /// The engine when `trimmed` is exactly one engine keyword and nothing else.
    fn bare_keyword_engine(&self, trimmed: &str) -> Option<&crate::omnibox::modes::EngineEntry> {
        let lowered = trimmed.to_lowercase();
        self.engines
            .iter()
            .find(|engine| !engine.keyword.is_empty() && engine.keyword.to_lowercase() == lowered)
    }
}

/// `mercury` or `mercury/docs?x=1` -> `https://www.mercury.com/...`; `None` for anything with a
/// dot, a space, a scheme or other characters, so only a single bare token is rewritten.
fn www_dot_com_address(trimmed: &str) -> Option<Url> {
    let token_end = trimmed.find(['/', '?', '#']).unwrap_or(trimmed.len());
    let (host, rest) = trimmed.split_at(token_end);

    let mut chars = host.chars();
    let first = chars.next()?;
    let is_token = first.is_alphanumeric()
        && chars.all(|character| character.is_alphanumeric() || character == '-')
        && !host.contains('.');
    if !is_token {
        return None;
    }

    // Only when classify_address would have searched; an address like `localhost` keeps its form.
    match classify_address(trimmed) {
        Ok(NavigationTarget::SearchQuery(_)) => {}
        _ => return None,
    }

    Url::parse(&format!("https://www.{host}.com{rest}")).ok()
}
