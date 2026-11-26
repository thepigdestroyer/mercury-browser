// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! Paste and go: what "Paste and Go" and "Paste and Search" do with the clipboard text.

use url::Url;

use crate::address::{NavigationTarget, classify_address};

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
        match self {
            Self::Nothing => None,
            Self::Go(_) | Self::GoMany(_) => Some("Paste and Go"),
            Self::Search(_) => Some("Paste and Search"),
        }
    }
}

/// What "Paste and Go" does with `clipboard`.
pub fn paste_and_go(clipboard: &str) -> Paste {
    if clipboard.len() > MAX_PASTE_BYTES || clipboard.chars().any(is_refused_control) {
        return Paste::Nothing;
    }

    let lines: Vec<&str> = clipboard
        .split(['\n', '\r'])
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect();

    match lines.as_slice() {
        [] => Paste::Nothing,
        [line] => match classify_address(line) {
            Ok(NavigationTarget::Url(url)) => Paste::Go(url),
            Ok(NavigationTarget::SearchQuery(query)) => Paste::Search(query),
            Err(_) => Paste::Nothing,
        },
        _ => {
            let addresses: Option<Vec<Url>> = lines
                .iter()
                .map(|line| match classify_address(line) {
                    Ok(NavigationTarget::Url(url)) => Some(url),
                    _ => None,
                })
                .collect();

            match addresses {
                Some(urls) if urls.len() <= MAX_PASTE_URLS => Paste::GoMany(urls),
                Some(_) => Paste::Nothing,
                None => Paste::Search(lines.join(" ")),
            }
        }
    }
}

fn is_refused_control(character: char) -> bool {
    character.is_control() && !matches!(character, '\n' | '\r' | '\t')
}
