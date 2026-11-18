// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! The star button's state: which page it is about, whether that page is bookmarked, and
//! which lookup's answer is still wanted. Pure; the caller sends the lookups and delivers
//! their answers.

use mercury_store::{BookmarkId, MAX_URL_BYTES};

/// What the star shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum StarState {
    /// The page cannot be bookmarked (or the database is not available).
    Disabled,
    /// Waiting for the first answer about this page: shown empty, but it can be clicked.
    Checking,
    Unbookmarked,
    Bookmarked(BookmarkId),
}

/// A `bookmark_for_url` lookup to send; its answer goes to [`StarModel::answer`] with `token`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Lookup {
    pub(crate) token: u64,
    pub(crate) url: String,
}

#[derive(Debug)]
pub(crate) struct StarModel {
    page: Option<String>,
    token: u64,
    state: StarState,
}

impl Default for StarModel {
    fn default() -> Self {
        Self {
            page: None,
            token: 0,
            state: StarState::Disabled,
        }
    }
}

impl StarModel {
    pub(crate) fn state(&self) -> StarState {
        self.state
    }

    /// The page the star is about, if it can be bookmarked.
    pub(crate) fn page(&self) -> Option<&str> {
        self.page.as_deref()
    }

    /// The active tab shows `page` (`None`: a page that cannot be bookmarked). Returns the
    /// lookup to send when the page changed.
    pub(crate) fn show_page(&mut self, page: Option<&str>) -> Option<Lookup> {
        if self.page.as_deref() == page {
            return None;
        }
        self.page = page.map(str::to_owned);
        self.token += 1;
        match &self.page {
            None => {
                self.state = StarState::Disabled;
                None
            }
            Some(url) => {
                self.state = StarState::Checking;
                Some(Lookup {
                    token: self.token,
                    url: url.clone(),
                })
            }
        }
    }

    /// The bookmarks changed: asks again about the current page, keeping what is shown until
    /// the answer comes.
    pub(crate) fn refresh(&mut self) -> Option<Lookup> {
        let url = self.page.clone()?;
        self.token += 1;
        Some(Lookup {
            token: self.token,
            url,
        })
    }

    /// The answer to the lookup `token`: `Err` if the database could not answer. Returns
    /// whether it was applied (answers to older lookups are not).
    pub(crate) fn answer(&mut self, token: u64, answer: Result<Option<BookmarkId>, ()>) -> bool {
        if token != self.token || self.page.is_none() {
            return false;
        }
        self.state = match answer {
            Ok(Some(id)) => StarState::Bookmarked(id),
            Ok(None) => StarState::Unbookmarked,
            Err(()) => StarState::Disabled,
        };
        true
    }

    /// `url` was bookmarked as `id` by the star itself.
    pub(crate) fn added(&mut self, url: &str, id: BookmarkId) -> bool {
        self.settle(url, StarState::Bookmarked(id))
    }

    /// The star's bookmark of `url` was removed.
    pub(crate) fn removed(&mut self, url: &str) -> bool {
        self.settle(url, StarState::Unbookmarked)
    }

    /// Shows `state` for `url` if it is the current page, outdating pending lookups.
    fn settle(&mut self, url: &str, state: StarState) -> bool {
        if self.page.as_deref() != Some(url) {
            return false;
        }
        self.token += 1;
        self.state = state;
        true
    }
}

/// The URL the star can bookmark for a page, or `None`: only `http`, `https` and `file`
/// pages, as history records; not a failed load or the engine's error page (recognised by
/// its title, as history does); not a URL longer than the store accepts.
pub(crate) fn bookmarkable_url<'a>(
    url: &'a str,
    title: &str,
    failed: bool,
    error_page_titles: &[String],
) -> Option<&'a str> {
    let scheme = url.split_once(':')?.0;
    let web_or_file = ["http", "https", "file"]
        .iter()
        .any(|allowed| scheme.eq_ignore_ascii_case(allowed));
    let error_page = error_page_titles.iter().any(|error| error == title);
    (web_or_file && !failed && !error_page && url.len() <= MAX_URL_BYTES).then_some(url)
}

#[cfg(test)]
mod tests {
    use super::*;

    const A: &str = "https://a.test/";
    const B: &str = "https://b.test/";

    fn lookup(model: &mut StarModel, page: &str) -> Lookup {
        model
            .show_page(Some(page))
            .expect("a new page is looked up")
    }

    #[test]
    fn a_new_page_is_looked_up_once() {
        let mut star = StarModel::default();
        assert_eq!(star.state(), StarState::Disabled);
        let first = lookup(&mut star, A);
        assert_eq!(first.url, A);
        assert_eq!(star.state(), StarState::Checking);
        assert_eq!(star.show_page(Some(A)), None, "the same page again");
        assert!(star.answer(first.token, Ok(Some(BookmarkId(4)))));
        assert_eq!(star.state(), StarState::Bookmarked(BookmarkId(4)));
    }

    #[test]
    fn an_answer_for_an_older_page_is_discarded() {
        let mut star = StarModel::default();
        let old = lookup(&mut star, A);
        let new = lookup(&mut star, B);
        assert!(!star.answer(old.token, Ok(Some(BookmarkId(1)))));
        assert_eq!(star.state(), StarState::Checking);
        assert!(star.answer(new.token, Ok(None)));
        assert_eq!(star.state(), StarState::Unbookmarked);
        assert_eq!(star.page(), Some(B));
    }

    #[test]
    fn going_back_to_a_page_discards_the_answer_about_it_from_before() {
        let mut star = StarModel::default();
        let first = lookup(&mut star, A);
        let _b = lookup(&mut star, B);
        let again = lookup(&mut star, A);
        assert_ne!(first.token, again.token);
        assert!(!star.answer(first.token, Ok(None)));
        assert!(star.answer(again.token, Ok(Some(BookmarkId(2)))));
    }

    #[test]
    fn a_page_that_cannot_be_bookmarked_disables_the_star_and_drops_answers() {
        let mut star = StarModel::default();
        let pending = lookup(&mut star, A);
        assert_eq!(star.show_page(None), None);
        assert_eq!(star.state(), StarState::Disabled);
        assert!(!star.answer(pending.token, Ok(Some(BookmarkId(1)))));
        assert_eq!(star.state(), StarState::Disabled);
        assert_eq!(star.refresh(), None, "nothing to refresh");
    }

    #[test]
    fn an_unavailable_database_disables_the_star() {
        let mut star = StarModel::default();
        let pending = lookup(&mut star, A);
        assert!(star.answer(pending.token, Err(())));
        assert_eq!(star.state(), StarState::Disabled);
    }

    #[test]
    fn a_refresh_keeps_the_state_until_its_answer_and_supersedes_older_lookups() {
        let mut star = StarModel::default();
        let first = lookup(&mut star, A);
        assert!(star.answer(first.token, Ok(Some(BookmarkId(3)))));
        let refresh = star.refresh().expect("the page is looked up again");
        assert_eq!(refresh.url, A);
        assert_eq!(star.state(), StarState::Bookmarked(BookmarkId(3)));
        assert!(star.answer(refresh.token, Ok(None)));
        assert_eq!(star.state(), StarState::Unbookmarked);
    }

    #[test]
    fn the_stars_own_edits_apply_at_once_and_outdate_pending_lookups() {
        let mut star = StarModel::default();
        let pending = lookup(&mut star, A);
        assert!(star.added(A, BookmarkId(7)));
        assert_eq!(star.state(), StarState::Bookmarked(BookmarkId(7)));
        assert!(!star.answer(pending.token, Ok(None)));
        assert!(!star.added(B, BookmarkId(8)), "another page's bookmark");
        assert_eq!(star.state(), StarState::Bookmarked(BookmarkId(7)));
        assert!(star.removed(A));
        assert_eq!(star.state(), StarState::Unbookmarked);
        assert!(!star.removed(B));
    }

    #[test]
    fn only_web_and_file_pages_can_be_bookmarked() {
        let errors = vec!["Error loading page".to_owned()];
        let ok = |url, title| bookmarkable_url(url, title, false, &errors);
        assert_eq!(ok(A, "A"), Some(A));
        assert_eq!(ok("http://a.test/x", ""), Some("http://a.test/x"));
        assert_eq!(ok("file:///tmp/x.html", "x"), Some("file:///tmp/x.html"));
        assert_eq!(ok("HTTPS://A.TEST/", "x"), Some("HTTPS://A.TEST/"));
        for refused in [
            "",
            "about:blank",
            "data:text/html,x",
            "javascript:x",
            "blob:x",
            "a.test",
        ] {
            assert_eq!(ok(refused, "x"), None, "{refused}");
        }
    }

    #[test]
    fn error_pages_failed_loads_and_overlong_urls_cannot_be_bookmarked() {
        let errors = vec!["Error loading page".to_owned()];
        assert_eq!(
            bookmarkable_url(A, "Error loading page", false, &errors),
            None
        );
        assert_eq!(bookmarkable_url(A, "A", true, &errors), None);
        let long = format!("https://a.test/{}", "x".repeat(MAX_URL_BYTES));
        assert_eq!(bookmarkable_url(&long, "A", false, &errors), None);
    }
}
