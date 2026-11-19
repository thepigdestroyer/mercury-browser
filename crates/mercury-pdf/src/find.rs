// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! Find in the document: hits per page, the current hit, next and previous, and "n of m". Pure
//! state, no I/O: the helper process searches, the shell passes what it finds to
//! [`FindState::hits`] as it arrives, page by page, in whatever order the pages are answered.
//!
//! - A **find id** names one search. A new query, or a change of the options, is a new
//!   [`FindState::start`]: it replaces everything, and an answer that carries an old id is ignored
//!   (the helper may still be answering the last search while the person types the next).
//! - Hits are kept **per page, in document order**, whatever order the pages are answered in
//!   (the helper starts at the page on screen and wraps), so "3 of 12" counts in reading order.
//! - The **current hit** is the first hit that arrives, and then only moves with
//!   [`FindState::select_next`] and [`FindState::select_previous`], which wrap around the end. It is kept by
//!   identity (page and first character), so hits of earlier pages arriving later change its
//!   number but not the hit.
//! - **Searching** is "not finished": the last answer of a search carries `finished`.
//! - At most [`budget::FIND_HITS`] hits are kept; later ones are dropped.

use std::collections::BTreeMap;

use crate::cache::budget;
use crate::types::{FindOptions, SearchHit};

/// What the find bar shows: "current of total", and whether the search is still running.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FindProgress {
    /// The number of the current hit among all the hits found so far, counted from 1; 0 when
    /// there is none.
    pub current: usize,
    /// How many hits there are so far.
    pub total: usize,
    /// Whether every page has been searched.
    pub finished: bool,
}

impl FindProgress {
    /// True while pages are still being searched (the count can still grow).
    pub fn searching(&self) -> bool {
        !self.finished
    }
}

/// What [`FindState::hits`] did with an answer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HitsOutcome {
    /// The answer was for another search than the current one (or for none): ignored.
    Stale,
    /// The answer was taken. `current_changed` is true when it made a hit the current one (the
    /// first hit of the search).
    Accepted { current_changed: bool },
}

/// The identity of a hit: its page and where it starts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct HitId {
    page: u32,
    char_start: u32,
}

/// The state of the document's current find.
#[derive(Clone, Debug, Default)]
pub struct FindState {
    next_id: u64,
    id: u64,
    query: String,
    options: FindOptions,
    pages: BTreeMap<u32, Vec<SearchHit>>,
    total: usize,
    finished: bool,
    current: Option<HitId>,
}

impl FindState {
    pub fn new() -> Self {
        Self::default()
    }

    /// Starts a new search, replacing the old one, and returns its id. An empty query is a
    /// search that is finished with no hits (nothing needs to be asked of the helper).
    pub fn start(&mut self, query: &str, options: FindOptions) -> u64 {
        self.next_id += 1;
        self.id = self.next_id;
        self.query = query.to_owned();
        self.options = options;
        self.pages.clear();
        self.total = 0;
        self.current = None;
        self.finished = query.is_empty();
        self.id
    }

    /// Takes `id` as the id of the current search. The client that runs the search picks its own
    /// ids, so the shell tells the model which one to expect: [`Self::start`], then the client's
    /// `find`, then this with what the client returned. (Answers cannot arrive in between: they
    /// are delivered on the interface thread.)
    pub fn adopt_id(&mut self, id: u64) {
        if self.id != 0 {
            self.id = id;
        }
    }

    /// Forgets the search; the answers still on their way are ignored.
    pub fn clear(&mut self) {
        self.id = 0;
        self.query.clear();
        self.pages.clear();
        self.total = 0;
        self.current = None;
        self.finished = false;
    }

    /// The id of the current search, or 0 when there is none.
    pub fn id(&self) -> u64 {
        self.id
    }

    pub fn is_active(&self) -> bool {
        self.id != 0
    }

    pub fn query(&self) -> &str {
        &self.query
    }

    pub fn options(&self) -> FindOptions {
        self.options
    }

    /// Takes the answer for one page: its hits (replacing an earlier answer for that page) and
    /// whether the search is finished with it.
    pub fn hits(
        &mut self,
        find: u64,
        page: u32,
        hits: Vec<SearchHit>,
        finished: bool,
    ) -> HitsOutcome {
        if self.id == 0 || find != self.id {
            return HitsOutcome::Stale;
        }
        let mut hits: Vec<SearchHit> = hits.into_iter().filter(|hit| hit.page == page).collect();
        hits.sort_by_key(|hit| (hit.char_start, hit.char_len));
        let replaced = self.pages.remove(&page).map_or(0, |old| old.len());
        self.total -= replaced;
        hits.truncate(budget::FIND_HITS.saturating_sub(self.total));
        self.total += hits.len();
        let first_of_page = hits.first().map(|hit| HitId {
            page,
            char_start: hit.char_start,
        });
        if !hits.is_empty() {
            self.pages.insert(page, hits);
        }
        if finished {
            self.finished = true;
        }
        // A replaced page may have held the current hit: fall back to the first of the new answer.
        if self.current.is_some_and(|current| !self.contains(current)) {
            self.current = None;
        }
        let mut current_changed = false;
        if self.current.is_none() && first_of_page.is_some() {
            self.current = first_of_page;
            current_changed = true;
        }
        HitsOutcome::Accepted { current_changed }
    }

    fn contains(&self, id: HitId) -> bool {
        self.pages
            .get(&id.page)
            .is_some_and(|hits| hits.iter().any(|hit| hit.char_start == id.char_start))
    }

    /// The hits of one page, in order.
    pub fn hits_on_page(&self, page: u32) -> &[SearchHit] {
        self.pages.get(&page).map_or(&[], Vec::as_slice)
    }

    /// The current hit.
    pub fn current(&self) -> Option<&SearchHit> {
        let id = self.current?;
        self.pages
            .get(&id.page)?
            .iter()
            .find(|hit| hit.char_start == id.char_start)
    }

    /// Whether this hit is the current one.
    pub fn is_current(&self, hit: &SearchHit) -> bool {
        self.current
            .is_some_and(|id| id.page == hit.page && id.char_start == hit.char_start)
    }

    /// The hit after the current one, wrapping from the last to the first; makes it current.
    /// `None` when there are no hits.
    pub fn select_next(&mut self) -> Option<&SearchHit> {
        let id = self.current?;
        self.current = Some(self.step(id, true)?);
        self.current()
    }

    /// The hit before the current one, wrapping from the first to the last.
    pub fn select_previous(&mut self) -> Option<&SearchHit> {
        let id = self.current?;
        self.current = Some(self.step(id, false)?);
        self.current()
    }

    /// The hit after (or before) `from` in document order, wrapping around the ends.
    fn step(&self, from: HitId, forward: bool) -> Option<HitId> {
        let id_of = |page: u32, hit: &SearchHit| HitId {
            page,
            char_start: hit.char_start,
        };
        let on_page = self.pages.get(&from.page);
        let position = on_page
            .and_then(|hits| {
                hits.iter()
                    .position(|hit| hit.char_start == from.char_start)
            })
            .unwrap_or(0);
        if forward {
            if let Some(hit) = on_page.and_then(|hits| hits.get(position + 1)) {
                return Some(id_of(from.page, hit));
            }
            let later = self.pages.range(from.page.saturating_add(1)..).next();
            let (page, hits) = later.or_else(|| self.pages.iter().next())?;
            Some(id_of(*page, hits.first()?))
        } else {
            if position > 0
                && let Some(hit) = on_page.and_then(|hits| hits.get(position - 1))
            {
                return Some(id_of(from.page, hit));
            }
            let earlier = self.pages.range(..from.page).next_back();
            let (page, hits) = earlier.or_else(|| self.pages.iter().next_back())?;
            Some(id_of(*page, hits.last()?))
        }
    }

    /// "current of total" and whether the search is still running.
    pub fn progress(&self) -> FindProgress {
        let current = self.current.map_or(0, |id| {
            let before: usize = self
                .pages
                .range(..id.page)
                .map(|(_, hits)| hits.len())
                .sum();
            let within = self
                .pages
                .get(&id.page)
                .and_then(|hits| hits.iter().position(|hit| hit.char_start == id.char_start))
                .unwrap_or(0);
            before + within + 1
        });
        FindProgress {
            current,
            total: self.total,
            finished: self.finished,
        }
    }
}
