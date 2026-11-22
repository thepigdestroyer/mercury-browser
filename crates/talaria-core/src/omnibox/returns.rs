// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! Modifier-Return (stub: the modes task fills in the bodies).

use url::Url;

use crate::address::AddressResolutionError;
use crate::omnibox::modes::{Scope, SearchModes};

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
pub fn return_plan(_keys: ReturnKeys, _platform: Platform) -> (OpenIn, bool) {
    (OpenIn::CurrentTab, false)
}

impl SearchModes {
    /// What Return with `keys` held does with `typed`.
    pub fn resolve_return(
        &self,
        _typed: &str,
        _keys: ReturnKeys,
        _platform: Platform,
    ) -> Result<ReturnResolution, AddressResolutionError> {
        Err(AddressResolutionError::EmptyInput)
    }
}
