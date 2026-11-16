//! The address bar's suggestion list: what it shows for an input, which row is selected, where
//! a choice leads, and the inline completion of a host. Pure: the wiring is in `mod.rs`.

use mercury_store::Suggestion;
use talaria_core::NavigationTarget;
use url::Url;

/// How long typing must pause before the store is asked.
pub(crate) const SUGGEST_DEBOUNCE_MS: u64 = 40;
/// Rows of history and bookmarks under the search row.
pub(crate) const SUGGESTION_ROWS: u32 = 8;

/// The search engine's name, as the search row shows it.
// settings lane: read `SettingsHandle::default_search_engine()` here once it is merged.
pub(crate) const SEARCH_ENGINE_NAME: &str = "DuckDuckGo";

/// One row of the list, before it becomes a Slint `SuggestionRow`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Row {
    pub(crate) is_search: bool,
    pub(crate) is_bookmark: bool,
    /// The title, split around the first match of the input.
    pub(crate) title: Emphasis,
    /// The URL as shown, split likewise; empty for the search row.
    pub(crate) url: Emphasis,
    /// Where choosing the row goes.
    pub(crate) target: String,
}

/// A text split around its emphasised part.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Emphasis {
    pub(crate) before: String,
    pub(crate) matched: String,
    pub(crate) after: String,
}

impl Emphasis {
    pub(crate) fn plain(text: &str) -> Self {
        Self {
            before: text.to_owned(),
            ..Self::default()
        }
    }
}

/// `text` split around the first occurrence of `needle`, ignoring case where lower-casing
/// keeps byte offsets (otherwise matching exactly).
pub(crate) fn emphasise(text: &str, needle: &str) -> Emphasis {
    let needle = needle.trim();
    if needle.is_empty() {
        return Emphasis::plain(text);
    }
    let found = {
        let lower_text = text.to_lowercase();
        let lower_needle = needle.to_lowercase();
        if lower_text.len() == text.len() && lower_needle.len() == needle.len() {
            lower_text
                .find(&lower_needle)
                .filter(|&at| text.is_char_boundary(at) && text.is_char_boundary(at + needle.len()))
                .map(|at| (at, at + needle.len()))
        } else {
            text.find(needle).map(|at| (at, at + needle.len()))
        }
    };
    match found {
        Some((start, end)) => Emphasis {
            before: text[..start].to_owned(),
            matched: text[start..end].to_owned(),
            after: text[end..].to_owned(),
        },
        None => Emphasis::plain(text),
    }
}

/// A URL as the list shows it: without `https://`, `http://` and a trailing `/` on a bare host.
pub(crate) fn display_url(url: &str) -> String {
    let shown = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))
        .unwrap_or(url);
    match shown.strip_suffix('/') {
        Some(host) if !host.contains('/') => host.to_owned(),
        _ => shown.to_owned(),
    }
}

/// The rows for `input`: the search row first, then the suggestions, best first.
pub(crate) fn rows(input: &str, suggestions: &[Suggestion]) -> Vec<Row> {
    let input = input.trim();
    if input.is_empty() {
        return Vec::new();
    }
    let search = NavigationTarget::SearchQuery(input.to_owned())
        .into_url()
        .map(String::from)
        .unwrap_or_default();
    let mut rows = vec![Row {
        is_search: true,
        is_bookmark: false,
        title: Emphasis::plain(&format!("Search {SEARCH_ENGINE_NAME} for “{input}”")),
        url: Emphasis::default(),
        target: search,
    }];
    rows.extend(suggestions.iter().map(|suggestion| {
        let url = display_url(&suggestion.url);
        let title = suggestion
            .title
            .as_deref()
            .filter(|title| !title.trim().is_empty())
            .unwrap_or(&url);
        Row {
            is_search: false,
            is_bookmark: suggestion.is_bookmark,
            title: emphasise(title, input),
            url: emphasise(&url, input),
            target: suggestion.url.clone(),
        }
    }));
    rows
}

/// The selection after Up (`down == false`) or Down in a list of `len` rows. `None` is the
/// typed text itself: Up from the first row returns to it.
pub(crate) fn move_selection(selected: Option<usize>, len: usize, down: bool) -> Option<usize> {
    if len == 0 {
        return None;
    }
    match (selected, down) {
        (None, true) => Some(0),
        (None, false) => Some(len - 1),
        (Some(index), true) => Some((index + 1).min(len - 1)),
        (Some(0), false) => None,
        (Some(index), false) => Some(index - 1),
    }
}

/// The text to add after `typed` so that it reads as the top suggestion's host, if that host
/// starts with what was typed (`exa` → `mple.com`). Only for something that looks like the
/// start of a host: no spaces, slashes or scheme.
pub(crate) fn inline_completion(typed: &str, top: &Suggestion) -> Option<String> {
    if typed.is_empty() || typed.contains([' ', '/', ':', '?', '#']) {
        return None;
    }
    let host = Url::parse(&top.url).ok()?.host_str()?.to_owned();
    let lower = typed.to_lowercase();
    let host = if lower.starts_with("www.") {
        host
    } else {
        host.strip_prefix("www.")
            .map_or(host.clone(), str::to_owned)
    };
    (host.len() > lower.len() && host.starts_with(&lower) && lower.len() == typed.len())
        .then(|| host[typed.len()..].to_owned())
}

/// Whether an edit added to the text (typing), as opposed to deleting: only typing completes,
/// so Backspace can remove a completion.
pub(crate) fn extends(previous: &str, edited: &str) -> bool {
    edited.len() > previous.len() && edited.starts_with(previous)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn suggestion(url: &str, title: Option<&str>, is_bookmark: bool) -> Suggestion {
        Suggestion {
            url: url.to_owned(),
            title: title.map(str::to_owned),
            is_bookmark,
            frecency: 100,
        }
    }

    #[test]
    fn the_match_is_emphasised_whatever_its_case() {
        assert_eq!(
            emphasise("Example Domain", "exa"),
            Emphasis {
                before: String::new(),
                matched: "Exa".into(),
                after: "mple Domain".into(),
            }
        );
        assert_eq!(
            emphasise("docs.rs/serde", "RS/"),
            Emphasis {
                before: "docs.".into(),
                matched: "rs/".into(),
                after: "serde".into(),
            }
        );
        assert_eq!(
            emphasise("Nothing here", "zz"),
            Emphasis::plain("Nothing here")
        );
        assert_eq!(emphasise("anything", "  "), Emphasis::plain("anything"));
    }

    #[test]
    fn text_whose_case_changes_its_length_is_matched_exactly() {
        // "İ" lower-cases to two characters: offsets would not line up.
        assert_eq!(emphasise("İstanbul", "stan").matched, "stan");
        assert_eq!(emphasise("İstanbul", "STAN"), Emphasis::plain("İstanbul"));
    }

    #[test]
    fn urls_are_shown_without_the_scheme_or_a_bare_hosts_slash() {
        assert_eq!(display_url("https://example.com/"), "example.com");
        assert_eq!(display_url("http://example.com/a/"), "example.com/a/");
        assert_eq!(display_url("file:///tmp/a.html"), "file:///tmp/a.html");
    }

    #[test]
    fn the_search_row_comes_first_then_the_suggestions() {
        let rows = rows(
            " exa ",
            &[
                suggestion("https://example.com/", Some("Example Domain"), true),
                suggestion("https://www.examine.test/page", None, false),
            ],
        );

        assert_eq!(rows.len(), 3);
        assert!(rows[0].is_search);
        assert_eq!(
            rows[0].title.before,
            format!("Search {SEARCH_ENGINE_NAME} for “exa”")
        );
        assert!(rows[0].target.starts_with("https://duckduckgo.com/?q=exa"));
        assert!(rows[1].is_bookmark);
        assert_eq!(rows[1].title.matched, "Exa");
        assert_eq!(rows[1].url.matched, "exa");
        assert_eq!(rows[1].target, "https://example.com/");
        // No title: the URL stands in for it.
        assert_eq!(rows[2].title.before, "www.");
        assert_eq!(rows[2].title.matched, "exa");
        assert_eq!(rows[2].target, "https://www.examine.test/page");
    }

    #[test]
    fn a_search_that_looks_like_an_address_still_searches() {
        let rows = rows("example.com", &[]);
        assert_eq!(rows[0].target, "https://duckduckgo.com/?q=example.com");
    }

    #[test]
    fn nothing_typed_shows_nothing() {
        assert!(rows("   ", &[suggestion("https://a.test/", None, false)]).is_empty());
    }

    #[test]
    fn up_and_down_move_through_the_rows_and_back_to_the_typed_text() {
        assert_eq!(move_selection(None, 3, true), Some(0));
        assert_eq!(move_selection(Some(0), 3, true), Some(1));
        assert_eq!(move_selection(Some(2), 3, true), Some(2));
        assert_eq!(move_selection(Some(1), 3, false), Some(0));
        assert_eq!(move_selection(Some(0), 3, false), None);
        assert_eq!(move_selection(None, 3, false), Some(2));
        assert_eq!(move_selection(None, 0, true), None);
    }

    #[test]
    fn the_top_suggestions_host_completes_what_was_typed() {
        let top = suggestion("https://www.example.com/some/page", None, false);

        assert_eq!(inline_completion("exa", &top).as_deref(), Some("mple.com"));
        assert_eq!(
            inline_completion("www.ex", &top).as_deref(),
            Some("ample.com")
        );
        // What was typed keeps its case.
        assert_eq!(inline_completion("EXA", &top).as_deref(), Some("mple.com"));
        assert_eq!(inline_completion("example.com", &top), None);
        assert_eq!(inline_completion("example.com/so", &top), None);
        assert_eq!(inline_completion("ex ample", &top), None);
        assert_eq!(inline_completion("xam", &top), None);
        assert_eq!(inline_completion("", &top), None);
    }

    #[test]
    fn only_typing_completes() {
        assert!(extends("exa", "exam"));
        assert!(!extends("example.com", "exa"));
        assert!(!extends("exa", "exa"));
        assert!(!extends("exa", "ex"));
    }
}
