// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
use std::collections::HashMap;

/// The most rows a history popup lists before it says there is more.
pub const POPUP_ROWS: usize = 15;
/// How long a press on Back or Forward takes to open the popup instead of going.
pub const LONG_PRESS_MS: u64 = 500;

const LABEL_CHARS: usize = 60;
const URL_HEAD_CHARS: usize = 28;
const URL_TAIL_CHARS: usize = 29;
const NEW_TAB_LABEL: &str = "New tab";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direction {
    Back,
    Forward,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HistoryEntry {
    pub url: String,
    pub title: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BackList {
    entries: Vec<HistoryEntry>,
    current: usize,
    titles: HashMap<String, String>,
}

impl BackList {
    /// Replaces the entries; titles are kept for addresses still in the list. `current` is
    /// clamped to the list, so it is 0 for an empty list.
    pub fn set_entries(&mut self, urls: Vec<String>, current: usize) {
        self.entries = urls
            .into_iter()
            .map(|url| HistoryEntry {
                title: self.titles.get(&url).cloned(),
                url,
            })
            .collect();
        self.current = current.min(self.entries.len().saturating_sub(1));
    }

    /// Remembers the title of the current entry by its address; `None` (or a blank title)
    /// forgets it.
    pub fn set_current_title(&mut self, title: Option<String>) {
        let title = title.filter(|title| !title.trim().is_empty());
        let Some(entry) = self.entries.get_mut(self.current) else {
            return;
        };
        match &title {
            Some(title) => {
                self.titles.insert(entry.url.clone(), title.clone());
            }
            None => {
                self.titles.remove(&entry.url);
            }
        }
        entry.title = title;
    }

    pub fn entries(&self) -> &[HistoryEntry] {
        &self.entries
    }

    pub fn current(&self) -> usize {
        self.current
    }

    pub fn can_go_back(&self) -> bool {
        self.current > 0
    }

    pub fn can_go_forward(&self) -> bool {
        self.current + 1 < self.entries.len()
    }

    /// The entries one way from the current one, nearest first, at most `POPUP_ROWS` of them.
    pub fn popup(&self, direction: Direction) -> HistoryPopup {
        let steps: Box<dyn Iterator<Item = usize>> = match direction {
            Direction::Back => Box::new((0..self.current).rev()),
            Direction::Forward => Box::new(self.current + 1..self.entries.len()),
        };
        let mut rows = Vec::new();
        let mut remaining = 0;
        for index in steps {
            if rows.len() == POPUP_ROWS {
                remaining += 1;
                continue;
            }
            let entry = &self.entries[index];
            let distance = index.abs_diff(self.current);
            let steps = match direction {
                Direction::Back => -i32::try_from(distance).unwrap_or(i32::MAX),
                Direction::Forward => i32::try_from(distance).unwrap_or(i32::MAX),
            };
            rows.push(PopupRow {
                label: row_label(&entry.url, entry.title.as_deref()),
                steps,
            });
        }
        HistoryPopup {
            direction,
            rows,
            truncated: remaining > 0,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PopupRow {
    pub label: String,
    /// Negative for a back step.
    pub steps: i32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HistoryPopup {
    pub direction: Direction,
    pub rows: Vec<PopupRow>,
    pub truncated: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PopupAction {
    Traverse(i32),
    ShowFullHistory,
}

impl HistoryPopup {
    pub const SHOW_FULL_HISTORY_LABEL: &'static str = "Show full history";

    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    /// `row == rows.len()` is the "Show full history" row.
    pub fn action(&self, row: usize) -> Option<PopupAction> {
        match row.cmp(&self.rows.len()) {
            std::cmp::Ordering::Less => Some(PopupAction::Traverse(self.rows[row].steps)),
            std::cmp::Ordering::Equal => Some(PopupAction::ShowFullHistory),
            std::cmp::Ordering::Greater => None,
        }
    }
}

/// The popup label of an entry: its title, else its address without the scheme; `New tab`
/// for a blank page. Longer labels are shortened to 60 characters.
pub fn row_label(url: &str, title: Option<&str>) -> String {
    if let Some(title) = title.map(str::trim).filter(|title| !title.is_empty()) {
        return elide_end(title, LABEL_CHARS);
    }
    if url.is_empty() || url == "about:blank" {
        return NEW_TAB_LABEL.to_owned();
    }
    let address = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))
        .unwrap_or(url);
    elide_middle(address, URL_HEAD_CHARS, URL_TAIL_CHARS)
}

fn elide_end(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        return text.to_owned();
    }
    let kept: String = text.chars().take(max_chars - 1).collect();
    format!("{kept}…")
}

fn elide_middle(text: &str, head: usize, tail: usize) -> String {
    let chars: Vec<char> = text.chars().collect();
    if chars.len() <= LABEL_CHARS {
        return text.to_owned();
    }
    let start: String = chars[..head].iter().collect();
    let end: String = chars[chars.len() - tail..].iter().collect();
    format!("{start}…{end}")
}
