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
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FindMatches {
    pub count: usize,
    /// Zero-based; `None` when there are no matches.
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
    /// Nothing searched for the current query and page yet.
    #[default]
    Idle,
    Matches(FindMatches),
    Failed,
}

impl FindStatus {
    fn current(&self) -> Option<usize> {
        match self {
            Self::Matches(matches) => matches.current,
            Self::Idle | Self::Failed => None,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FindState {
    open: bool,
    query: String,
    match_case: bool,
    status: FindStatus,
    next_request_id: u64,
    awaited_search: Option<u64>,
    awaited_selection: Option<u64>,
    /// The query or page changed since the last search, so the next one starts `Here`.
    search_due: bool,
    /// Set when a selection read asks for a search; the controller queues it.
    search_requested: bool,
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

    /// Whether an answer from the page is still expected.
    pub const fn is_awaiting_answer(&self) -> bool {
        self.awaited_search.is_some() || self.awaited_selection.is_some()
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

    pub(crate) fn set_query(&mut self, query: &str) {
        self.edited_since_open = true;
        if self.query == query {
            return;
        }
        self.query = query.to_owned();
        self.status = FindStatus::Idle;
        self.awaited_search = None;
        self.search_due = true;
    }

    /// The search a debounce timer or a selection read is waiting for, if any.
    pub(crate) fn due_search(&mut self) -> Option<BrowserCommand> {
        if !self.open || !self.search_due {
            return None;
        }
        self.search(FindStep::Here)
    }

    pub(crate) fn take_requested_search(&mut self) -> Option<BrowserCommand> {
        if !std::mem::take(&mut self.search_requested) {
            return None;
        }
        self.due_search()
    }

    /// Next or previous match; a query not searched yet is searched `Here` first. Reopens a
    /// closed bar that still has a query.
    pub(crate) fn step(&mut self, forward: bool) -> Option<BrowserCommand> {
        if self.query.is_empty() {
            return None;
        }
        self.reopen();
        if self.search_due {
            return self.search(FindStep::Here);
        }
        self.search(if forward {
            FindStep::Next
        } else {
            FindStep::Previous
        })
    }

    pub(crate) fn toggle_match_case(&mut self) -> Option<BrowserCommand> {
        self.match_case = !self.match_case;
        let current = self.status.current();
        self.status = FindStatus::Idle;
        self.search_due = true;
        let mut command = self.due_search();
        if let Some(BrowserCommand::Find(request)) = &mut command {
            request.current = current;
        }
        command
    }

    pub(crate) fn close(&mut self) -> Option<BrowserCommand> {
        if !self.open {
            return None;
        }
        self.open = false;
        self.status = FindStatus::Idle;
        self.awaited_search = None;
        self.awaited_selection = None;
        self.search_requested = false;
        let restore_selection = self.saved_selection.take();
        std::mem::take(&mut self.searched_since_open).then(|| {
            BrowserCommand::EndFind(EndFindRequest {
                query: self.query.clone(),
                match_case: self.match_case,
                restore_selection,
            })
        })
    }

    pub(crate) fn apply_result(&mut self, request_id: u64, outcome: FindOutcome) {
        if self.awaited_search != Some(request_id) {
            return;
        }
        self.awaited_search = None;
        self.status = match outcome {
            FindOutcome::Matches(matches) => FindStatus::Matches(matches),
            FindOutcome::Failed => FindStatus::Failed,
        };
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
            self.query = text;
            self.status = FindStatus::Idle;
        }
        self.search_due = true;
        self.search_requested = true;
    }

    /// The tab shows another document: its matches are gone.
    pub(crate) fn page_changed(&mut self) {
        self.status = FindStatus::Idle;
        self.awaited_search = None;
        self.saved_selection = None;
        self.search_due = true;
    }

    fn search(&mut self, step: FindStep) -> Option<BrowserCommand> {
        if self.query.is_empty() {
            self.search_due = false;
            return None;
        }
        let id = self.next_id();
        self.awaited_search = Some(id);
        self.search_due = false;
        self.searched_since_open = true;
        Some(BrowserCommand::Find(FindRequest {
            id,
            query: self.query.clone(),
            match_case: self.match_case,
            step,
            current: self.status.current(),
        }))
    }

    fn next_id(&mut self) -> u64 {
        self.next_request_id += 1;
        self.next_request_id
    }
}

/// Whether two addresses name different documents; a change of fragment alone does not.
pub(crate) fn is_other_document(old_url: &str, new_url: &str) -> bool {
    fn without_fragment(url: &str) -> &str {
        url.split_once('#').map_or(url, |(base, _)| base)
    }
    without_fragment(old_url) != without_fragment(new_url)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fragments_do_not_make_another_document() {
        assert!(!is_other_document("https://a.test/p", "https://a.test/p#x"));
        assert!(!is_other_document(
            "https://a.test/p#x",
            "https://a.test/p#y"
        ));
        assert!(is_other_document("https://a.test/p", "https://a.test/q"));
        assert!(is_other_document("", "https://a.test/"));
    }

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
