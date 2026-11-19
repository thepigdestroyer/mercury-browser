// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! What the find bar says about a search of a PDF: the same words as for a page.

use mercury_pdf::find::FindProgress;

/// "3 of 7", "No matches", and while pages are still being searched "3 of 7+", "Searching…" or
/// "5+ matches".
pub(super) fn status_text(progress: &FindProgress) -> String {
    let plus = if progress.finished { "" } else { "+" };
    match (progress.total, progress.current) {
        (0, _) if progress.finished => "No matches".to_owned(),
        (0, _) => "Searching\u{2026}".to_owned(),
        (total, 0) => format!("{total}{plus} matches"),
        (total, current) => format!("{current} of {total}{plus}"),
    }
}

/// Whether the bar should show its "nothing found" colour: only once every page was searched.
pub(super) fn has_no_matches(progress: &FindProgress) -> bool {
    progress.finished && progress.total == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn progress(current: usize, total: usize, finished: bool) -> FindProgress {
        FindProgress {
            current,
            total,
            finished,
        }
    }

    #[test]
    fn the_bar_says_where_the_current_match_is() {
        assert_eq!(status_text(&progress(3, 7, true)), "3 of 7");
        assert_eq!(status_text(&progress(1, 1, true)), "1 of 1");
    }

    #[test]
    fn while_pages_are_searched_the_count_may_still_grow() {
        assert_eq!(status_text(&progress(0, 0, false)), "Searching\u{2026}");
        assert_eq!(status_text(&progress(1, 4, false)), "1 of 4+");
        assert_eq!(status_text(&progress(0, 5, false)), "5+ matches");
    }

    #[test]
    fn nothing_is_found_only_once_every_page_was_searched() {
        assert_eq!(status_text(&progress(0, 0, true)), "No matches");
        assert!(has_no_matches(&progress(0, 0, true)));
        assert!(!has_no_matches(&progress(0, 0, false)));
        assert!(!has_no_matches(&progress(1, 2, true)));
    }
}
