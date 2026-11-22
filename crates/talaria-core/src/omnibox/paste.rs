// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! Paste and go (stub: the paste task fills it in).

use url::Url;

/// The most bytes of clipboard text looked at.
pub const MAX_PASTE_BYTES: usize = 4096;
/// The most addresses one paste opens.
pub const MAX_PASTE_URLS: usize = 20;

/// What pasting into the address bar and going does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Paste {
    /// Nothing usable (empty, too long, control characters, a scheme that must not be opened).
    Nothing,
    /// Open one address.
    Go(Url),
    /// Search for the text.
    Search(String),
    /// Open each address in its own tab.
    GoMany(Vec<Url>),
}

impl Paste {
    /// The context-menu item's name: "Paste and Go" or "Paste and Search"; `None` hides the item.
    pub fn menu_label(&self) -> Option<&'static str> {
        None
    }
}

/// What "Paste and Go" does with `clipboard`.
pub fn paste_and_go(_clipboard: &str) -> Paste {
    Paste::Nothing
}
