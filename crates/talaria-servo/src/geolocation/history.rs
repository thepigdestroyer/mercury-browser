//! When a history change means the document may have changed without Servo reporting a new
//! document.
//!
//! A back/forward traversal to a page in the back-forward cache restores its document without
//! `LoadStatus::Started`, and the page left stays alive in the cache with its watches. So, fail
//! safe: forget the web view's requests and watches when the current entry moves
//! - with no document start since the last history notification, or
//! - with the entries unchanged (a traversal). This also covers Servo reporting the history of a
//!   new document before its `Started`, which would otherwise leave a stale "started" behind for
//!   the next traversal.
//!
//! The cost: a same-document move (`pushState`, a fragment, an SPA's `history.back()`) also ends
//! the page's watches. The page is told (its watches get POSITION_UNAVAILABLE) and may watch
//! again. That costs function, never privacy.

use url::Url;

#[derive(Default)]
pub(crate) struct HistoryRule {
    last: Option<(Vec<Url>, usize)>,
    document_started_since: bool,
}

impl HistoryRule {
    pub(crate) fn document_started(&mut self) {
        self.document_started_since = true;
    }

    /// Whether the web view's requests and watches must be forgotten.
    pub(crate) fn history_changed(&mut self, entries: &[Url], current: usize) -> bool {
        let forget = match &self.last {
            Some((last_entries, last_current)) if *last_current != current => {
                last_entries.as_slice() == entries || !self.document_started_since
            }
            _ => false,
        };
        self.document_started_since = false;
        self.last = Some((entries.to_vec(), current));
        forget
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    enum Event {
        Started,
        History(&'static [&'static str], usize),
    }
    use Event::{History, Started};

    /// The history notifications (by position among them) after which the rule forgets.
    fn forgets(events: &[Event]) -> Vec<usize> {
        let mut rule = HistoryRule::default();
        let mut forgotten = Vec::new();
        let mut history_index = 0;
        for event in events {
            match event {
                Started => rule.document_started(),
                History(entries, current) => {
                    let entries = entries
                        .iter()
                        .map(|entry| Url::parse(&format!("https://{entry}.test/")).expect("valid"))
                        .collect::<Vec<_>>();
                    if rule.history_changed(&entries, *current) {
                        forgotten.push(history_index);
                    }
                    history_index += 1;
                }
            }
        }
        forgotten
    }

    #[test]
    fn the_first_notification_never_forgets() {
        assert!(forgets(&[History(&["a"], 0)]).is_empty());
        assert!(forgets(&[Started, History(&["a"], 0)]).is_empty());
    }

    #[test]
    fn a_new_document_reported_before_its_history_is_left_to_the_document_start() {
        assert!(
            forgets(&[
                Started,
                History(&["a"], 0),
                Started,
                History(&["a", "b"], 1)
            ])
            .is_empty()
        );
    }

    #[test]
    fn a_new_documents_history_reported_before_its_start_forgets() {
        assert_eq!(
            forgets(&[
                Started,
                History(&["a"], 0),
                History(&["a", "b"], 1),
                Started
            ]),
            [1]
        );
    }

    #[test]
    fn back_to_a_cached_page_forgets() {
        assert_eq!(
            forgets(&[
                Started,
                History(&["a"], 0),
                Started,
                History(&["a", "b"], 1),
                History(&["a", "b"], 0),
            ]),
            [2]
        );
    }

    #[test]
    fn a_start_reported_after_its_history_does_not_shield_the_next_traversal() {
        assert_eq!(
            forgets(&[
                Started,
                History(&["a"], 0),
                History(&["a", "b"], 1),
                Started,
                History(&["a", "b"], 0),
            ]),
            [1, 2]
        );
    }

    #[test]
    fn a_traversal_that_reloads_an_evicted_page_still_forgets() {
        assert_eq!(
            forgets(&[
                Started,
                History(&["a"], 0),
                Started,
                History(&["a", "b"], 1),
                Started,
                History(&["a", "b"], 0),
            ]),
            [2]
        );
    }

    #[test]
    fn a_same_document_entry_forgets_and_replacing_the_entry_does_not() {
        // pushState / fragment: a new entry with no document start.
        assert_eq!(
            forgets(&[Started, History(&["a"], 0), History(&["a", "a2"], 1)]),
            [1]
        );
        // replaceState: the current entry changes, its index does not.
        assert!(forgets(&[Started, History(&["a"], 0), History(&["a2"], 0)]).is_empty());
    }
}
