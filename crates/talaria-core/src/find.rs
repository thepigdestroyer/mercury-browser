//! Find in page: the find bar's state for one tab.
//!
//! The engine has no find API, so the search runs as a script in the page (see
//! `talaria-servo`'s `find_in_page`). The script keeps no state between calls: each request
//! carries the query, and the page's own selection marks the current match. This model holds what
//! the bar shows and which answer it is waiting for; answers to anything older are ignored.

use crate::BrowserCommand;

/// Where a search moves the current match.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FindStep {
    /// The first match at or after the current selection; with no selection, the first match in
    /// view. Used for a new query, so that typing more letters keeps the same match.
    Here,
    Next,
    Previous,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FindRequest {
    pub id: u64,
    pub query: String,
    pub match_case: bool,
    pub step: FindStep,
    /// How many matches `Next` or `Previous` moves (presses made while the page was still
    /// answering an earlier search); 1 for `Here`.
    pub repeat: u32,
    /// An explicit action (Return, a button, a shortcut) rather than typing: the script may spend
    /// more time on it.
    pub thorough: bool,
    /// The current match as last reported, for when the page's selection no longer marks it.
    pub current: Option<usize>,
}

/// Sent when the bar closes after it searched.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EndFindRequest {
    pub query: String,
    pub match_case: bool,
    /// The page's selection from before the bar opened, as the script saved it; it is put back if
    /// the page still has the current match selected.
    pub restore_selection: Option<String>,
    /// The last search found nothing before its limit and left a caret where it stopped; the
    /// script treats a collapsed selection as that caret and puts the page's selection back.
    pub caret_left: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FindMatches {
    pub count: usize,
    /// Zero-based position in the document; `None` when there are no matches, or when the search
    /// stopped at its limit, which leaves the position unknown.
    pub current: Option<usize>,
    /// The search stopped at its limit, so there may be more matches than `count`.
    pub truncated: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FindOutcome {
    Matches(FindMatches),
    /// The script could not run or its answer made no sense (for example, the page replaced the
    /// built-ins it needs).
    Failed,
}

/// The page's selection when the bar opened.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FindSelection {
    /// The selected text if it is short and on one line, else empty.
    pub text: String,
    /// An opaque description of the selection that the script can restore.
    pub saved: Option<String>,
}

/// What the bar shows next to the field.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum FindStatus {
    /// Nothing searched for the current page yet.
    #[default]
    Idle,
    Matches(FindMatches),
    Failed,
    /// The page has not answered the last search in time.
    NotResponding,
}

impl FindStatus {
    fn current(&self) -> Option<usize> {
        match self {
            Self::Matches(matches) => matches.current,
            Self::Idle | Self::Failed | Self::NotResponding => None,
        }
    }
}

/// An explicit search that finds nothing before its limit is continued from where it stopped,
/// this many times more (so `1 + CONTINUATION_ROUNDS` script runs of at most 200 ms each).
pub const CONTINUATION_ROUNDS: u8 = 4;

/// A search waiting for the page to answer the one before it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PendingSearch {
    Here {
        thorough: bool,
    },
    /// Positive: Next that many times; negative: Previous.
    Steps {
        count: i64,
    },
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FindState {
    open: bool,
    query: String,
    match_case: bool,
    status: FindStatus,
    next_request_id: u64,
    /// At most one search is in the page at a time; later ones wait in `pending`.
    awaited_search: Option<u64>,
    /// Whether the awaited search was explicit (and so may leave a caret and be continued).
    awaited_thorough: bool,
    awaited_selection: Option<u64>,
    /// Automatic continuations sent since the user's last search.
    continuation_rounds: u8,
    caret_left: bool,
    pending: Option<PendingSearch>,
    /// The query or page changed since the last search, so the next one starts `Here`.
    search_due: bool,
    /// The query was typed on another document: it is shown, but only searched on request.
    query_from_other_page: bool,
    /// A search to queue now (after an answer or a selection read); the controller takes it.
    follow_up: Option<FindRequest>,
    edited_since_open: bool,
    searched_since_open: bool,
    saved_selection: Option<String>,
}

impl FindState {
    pub const fn is_open(&self) -> bool {
        self.open
    }

    pub fn query(&self) -> &str {
        &self.query
    }

    pub const fn match_case(&self) -> bool {
        self.match_case
    }

    pub const fn status(&self) -> &FindStatus {
        &self.status
    }

    /// Whether an answer from the page is still expected, or a search waits to be sent.
    pub const fn is_awaiting_answer(&self) -> bool {
        self.awaited_search.is_some() || self.awaited_selection.is_some() || self.pending.is_some()
    }

    /// The search the page has not answered yet.
    pub const fn awaited_search(&self) -> Option<u64> {
        self.awaited_search
    }

    /// The query was carried over from another document and is not searched until the user asks.
    pub const fn is_query_from_other_page(&self) -> bool {
        self.query_from_other_page
    }

    /// Opens the bar and asks the page for its selection. `None` if it was already open.
    pub(crate) fn open(&mut self) -> Option<BrowserCommand> {
        if !self.reopen() {
            return None;
        }
        let request_id = self.next_id();
        self.awaited_selection = Some(request_id);
        Some(BrowserCommand::BeginFind { request_id })
    }

    /// Opens the bar without reading the selection; false if it was already open.
    fn reopen(&mut self) -> bool {
        if self.open {
            return false;
        }
        self.open = true;
        self.edited_since_open = false;
        self.searched_since_open = false;
        self.saved_selection = None;
        true
    }

    /// The user edited the field. The last status stays until the new query is answered.
    pub(crate) fn set_query(&mut self, query: &str) {
        self.edited_since_open = true;
        self.query_from_other_page = false;
        if self.query == query {
            return;
        }
        self.query = query.to_owned();
        if self.query.is_empty() {
            self.status = FindStatus::Idle;
        }
        self.search_due = true;
    }

    /// The search a debounce timer or a selection read is waiting for, if any. A query from
    /// another document is never searched automatically.
    pub(crate) fn due_search(&mut self) -> Option<BrowserCommand> {
        if !self.open || !self.search_due || self.query_from_other_page {
            return None;
        }
        self.search(FindStep::Here, 1, false)
    }

    pub(crate) fn take_follow_up(&mut self) -> Option<BrowserCommand> {
        self.follow_up.take().map(BrowserCommand::Find)
    }

    /// Next or previous match; a query not searched yet is searched `Here` first. Reopens a
    /// closed bar that still has a query.
    pub(crate) fn step(&mut self, forward: bool) -> Option<BrowserCommand> {
        if self.query.is_empty() {
            return None;
        }
        self.reopen();
        self.query_from_other_page = false;
        if self.search_due {
            return self.search(FindStep::Here, 1, true);
        }
        let step = if forward {
            FindStep::Next
        } else {
            FindStep::Previous
        };
        self.search(step, 1, true)
    }

    pub(crate) fn toggle_match_case(&mut self) -> Option<BrowserCommand> {
        self.match_case = !self.match_case;
        self.query_from_other_page = false;
        self.search_due = true;
        if !self.open {
            return None;
        }
        self.search(FindStep::Here, 1, true)
    }

    pub(crate) fn close(&mut self) -> Option<BrowserCommand> {
        if !self.open {
            return None;
        }
        self.open = false;
        self.status = FindStatus::Idle;
        self.forget_answers();
        let restore_selection = self.saved_selection.take();
        let caret_left = std::mem::take(&mut self.caret_left);
        std::mem::take(&mut self.searched_since_open).then(|| {
            BrowserCommand::EndFind(EndFindRequest {
                query: self.query.clone(),
                match_case: self.match_case,
                restore_selection,
                caret_left,
            })
        })
    }

    pub(crate) fn apply_result(&mut self, request_id: u64, outcome: FindOutcome) {
        if self.awaited_search != Some(request_id) {
            return;
        }
        self.awaited_search = None;
        let nothing_found_yet = matches!(
            outcome,
            FindOutcome::Matches(FindMatches {
                count: 0,
                truncated: true,
                ..
            })
        );
        // Only explicit searches leave a caret (the script decides the same way).
        self.caret_left = self.awaited_thorough && nothing_found_yet;
        self.status = match outcome {
            FindOutcome::Matches(matches) => FindStatus::Matches(matches),
            FindOutcome::Failed => FindStatus::Failed,
        };
        let follow_up = match self.pending.take() {
            Some(PendingSearch::Here { thorough }) => self.search(FindStep::Here, 1, thorough),
            Some(PendingSearch::Steps { count }) => {
                let step = if count > 0 {
                    FindStep::Next
                } else {
                    FindStep::Previous
                };
                let repeat = u32::try_from(count.unsigned_abs()).unwrap_or(u32::MAX);
                self.search(step, repeat, true)
            }
            None if self.caret_left && self.continuation_rounds < CONTINUATION_ROUNDS => {
                // Go on from the caret by ourselves, giving the page's thread a break between
                // rounds, instead of making the user press Return again.
                let rounds = self.continuation_rounds + 1;
                let command = self.search(FindStep::Here, 1, true);
                self.continuation_rounds = rounds;
                command
            }
            None => None,
        };
        self.follow_up = find_request(follow_up);
    }

    pub(crate) fn apply_selection(&mut self, request_id: u64, selection: Option<FindSelection>) {
        if self.awaited_selection != Some(request_id) {
            return;
        }
        self.awaited_selection = None;
        let (text, saved) = selection.map_or((String::new(), None), |selection| {
            (selection.text, selection.saved)
        });
        self.saved_selection = saved;
        if self.edited_since_open {
            return;
        }
        if !text.is_empty() && text != self.query {
            // Text from this page: no longer a query from another document.
            self.query = text;
            self.query_from_other_page = false;
        }
        self.search_due = true;
        let follow_up = self.due_search();
        self.follow_up = find_request(follow_up);
    }

    /// The page has not answered `request_id` in time. Nothing more is sent to it until it
    /// answers; the bar says so.
    pub(crate) fn answer_overdue(&mut self, request_id: u64) {
        if self.awaited_search == Some(request_id) {
            self.status = FindStatus::NotResponding;
        }
    }

    /// The tab shows another document, or reloads: its matches are gone, and a query typed on
    /// the old document is not sent to the new one until the user asks.
    pub(crate) fn page_changed(&mut self) {
        self.status = FindStatus::Idle;
        self.forget_answers();
        self.caret_left = false;
        self.saved_selection = None;
        self.search_due = true;
        self.query_from_other_page = !self.query.is_empty();
    }

    fn forget_answers(&mut self) {
        self.awaited_search = None;
        self.awaited_selection = None;
        self.pending = None;
        self.follow_up = None;
    }

    /// Sends a search, or, while the page is still answering one, merges it into the pending
    /// search: steps add up; a new query (`Here`) replaces steps.
    fn search(&mut self, step: FindStep, repeat: u32, thorough: bool) -> Option<BrowserCommand> {
        if self.query.is_empty() {
            self.search_due = false;
            return None;
        }
        self.search_due = false;
        if self.awaited_search.is_some() {
            let steps = i64::from(repeat);
            self.pending = match (self.pending, step) {
                // A waiting new query stays `Here`, with the larger budget of the two.
                (Some(PendingSearch::Here { thorough: waiting }), _) => Some(PendingSearch::Here {
                    thorough: waiting || thorough,
                }),
                (_, FindStep::Here) => Some(PendingSearch::Here { thorough }),
                (Some(PendingSearch::Steps { count }), FindStep::Next) => {
                    Some(PendingSearch::Steps {
                        count: count + steps,
                    })
                }
                (Some(PendingSearch::Steps { count }), FindStep::Previous) => {
                    Some(PendingSearch::Steps {
                        count: count - steps,
                    })
                }
                (None, FindStep::Next) => Some(PendingSearch::Steps { count: steps }),
                (None, FindStep::Previous) => Some(PendingSearch::Steps { count: -steps }),
            };
            if self.pending == Some(PendingSearch::Steps { count: 0 }) {
                self.pending = None;
            }
            return None;
        }
        let id = self.next_id();
        self.awaited_search = Some(id);
        self.awaited_thorough = thorough;
        // A continuation sets the count again after this.
        self.continuation_rounds = 0;
        self.searched_since_open = true;
        Some(BrowserCommand::Find(FindRequest {
            id,
            query: self.query.clone(),
            match_case: self.match_case,
            step,
            repeat,
            thorough,
            current: self.status.current(),
        }))
    }

    fn next_id(&mut self) -> u64 {
        self.next_request_id += 1;
        self.next_request_id
    }
}

fn find_request(command: Option<BrowserCommand>) -> Option<FindRequest> {
    match command {
        Some(BrowserCommand::Find(request)) => Some(request),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_ids_never_repeat() {
        let mut find = FindState::default();
        let first = find.open();
        find.close();
        let second = find.open();

        assert_eq!(first, Some(BrowserCommand::BeginFind { request_id: 1 }));
        assert_eq!(second, Some(BrowserCommand::BeginFind { request_id: 2 }));
    }
}
