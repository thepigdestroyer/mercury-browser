// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! What a history notification means for a web view's watches.
//!
//! Servo's history is the joint session history: an iframe's navigation adds an entry too (with
//! the parent's URL, `servo-constellation` `constellation.rs` `notify_history_changed`), and a
//! back/forward traversal to a page in the back-forward cache restores its document without
//! `LoadStatus::Started`. So the rule tells only two things apart:
//! - a **traversal**: the entries are unchanged and the current one moved. Watches registered at
//!   another entry are parked (the page there may be in the back-forward cache), and those
//!   registered at the new entry resume.
//! - a change of **entries** (a new document, `pushState`, a fragment, an iframe navigation,
//!   `replaceState`, pruning): nothing is ended (a new document is handled by its start signal),
//!   but parked watches registered at an entry that changed or went are dropped.
//!
//! The cost: a same-document traversal (an SPA's `history.back()`, an iframe's back/forward)
//! parks the page's watches registered at the other entry; they are told (POSITION_UNAVAILABLE)
//! and resume if the user comes back to that entry. That costs function, never privacy.

use url::Url;

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum HistoryChange {
    /// Same entries, another current one.
    Traversal { to: usize },
    /// The entries changed (or this is the first notification). `changed` lists the indexes
    /// of the previous entries that no longer hold the same entry.
    Entries { current: usize, changed: Vec<usize> },
    /// Nothing that matters to watches.
    None,
}

#[derive(Default)]
pub(crate) struct HistoryRule {
    last: Option<(Vec<Url>, usize)>,
}

impl HistoryRule {
    pub(crate) fn history_changed(&mut self, entries: &[Url], current: usize) -> HistoryChange {
        let change = match &self.last {
            Some((last_entries, last_current)) if last_entries.as_slice() == entries => {
                if *last_current == current {
                    HistoryChange::None
                } else {
                    HistoryChange::Traversal { to: current }
                }
            }
            Some((last_entries, _)) => HistoryChange::Entries {
                current,
                changed: (0..last_entries.len())
                    .filter(|&index| entries.get(index) != Some(&last_entries[index]))
                    .collect(),
            },
            None => HistoryChange::Entries {
                current,
                changed: Vec::new(),
            },
        };
        self.last = Some((entries.to_vec(), current));
        change
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn urls(entries: &[&str]) -> Vec<Url> {
        entries
            .iter()
            .map(|entry| Url::parse(&format!("https://{entry}.test/")).expect("valid"))
            .collect()
    }

    fn changes(steps: &[(&[&str], usize)]) -> Vec<HistoryChange> {
        let mut rule = HistoryRule::default();
        steps
            .iter()
            .map(|(entries, current)| rule.history_changed(&urls(entries), *current))
            .collect()
    }

    fn entries(current: usize, changed: &[usize]) -> HistoryChange {
        HistoryChange::Entries {
            current,
            changed: changed.to_vec(),
        }
    }

    #[test]
    fn the_first_notification_sets_the_entries() {
        assert_eq!(changes(&[(&["a"], 0)]), [entries(0, &[])]);
    }

    #[test]
    fn a_new_document_adds_an_entry() {
        assert_eq!(
            changes(&[(&["a"], 0), (&["a", "b"], 1)]),
            [entries(0, &[]), entries(1, &[])]
        );
    }

    #[test]
    fn an_iframe_navigation_adds_an_entry_and_is_no_traversal() {
        // The iframe's entry carries the parent's URL.
        assert_eq!(
            changes(&[(&["a"], 0), (&["a", "a"], 1)]),
            [entries(0, &[]), entries(1, &[])]
        );
    }

    #[test]
    fn back_and_forward_are_traversals() {
        assert_eq!(
            changes(&[(&["a", "b"], 1), (&["a", "b"], 0), (&["a", "b"], 1)]),
            [
                entries(1, &[]),
                HistoryChange::Traversal { to: 0 },
                HistoryChange::Traversal { to: 1 },
            ]
        );
    }

    #[test]
    fn the_same_entry_again_is_nothing() {
        assert_eq!(
            changes(&[(&["a", "b"], 1), (&["a", "b"], 1)]),
            [entries(1, &[]), HistoryChange::None]
        );
    }

    #[test]
    fn navigating_after_going_back_drops_the_forward_entries() {
        assert_eq!(
            changes(&[(&["a", "b", "c"], 0), (&["a", "d"], 1)]),
            [entries(0, &[]), entries(1, &[1, 2])]
        );
    }

    #[test]
    fn replacing_an_entry_changes_only_that_one() {
        assert_eq!(
            changes(&[(&["a", "b", "c"], 0), (&["a2", "b", "c"], 0)]),
            [entries(0, &[]), entries(0, &[0])]
        );
    }

    #[test]
    fn trimming_the_oldest_entry_changes_every_shifted_one() {
        assert_eq!(
            changes(&[(&["a", "b", "c"], 2), (&["b", "c", "d"], 2)]),
            [entries(2, &[]), entries(2, &[0, 1, 2])]
        );
    }
}
