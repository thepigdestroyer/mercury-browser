// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! The address bar's suggestion list: what it shows for an input, which row is selected, where
//! a choice leads, and the inline completion of a host. Pure: the wiring is in `mod.rs`.

use mercury_store::Suggestion;
use url::Url;

/// How long typing must pause before the store is asked.
pub(crate) const SUGGEST_DEBOUNCE_MS: u64 = 40;
/// Rows of history and bookmarks under the search row.
pub(crate) const SUGGESTION_ROWS: u32 = 8;

/// The search row: where searching for what was typed goes, and the engine's name. Built by
/// the caller from the controller's search builder and the settings; without one (no search
/// URL can be built for the text) the list has no search row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SearchRow {
    pub(crate) engine: String,
    pub(crate) url: String,
}

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
    /// Choosing the row counts as typing its address: the search row, and pages the user
    /// typed or bookmarked before (those inline completion may complete). Others were only
    /// reached by links or redirects, and choosing them must not earn them completion.
    pub(crate) typed: bool,
}

/// A row chosen from the list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Choice {
    pub(crate) target: String,
    /// See [`Row::typed`].
    pub(crate) typed: bool,
}

impl Row {
    fn choice(&self) -> Choice {
        Choice {
            target: self.target.clone(),
            typed: self.typed,
        }
    }
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

/// The rows for `input`: the search row first (if there is one), then the suggestions, best
/// first.
pub(crate) fn rows(
    input: &str,
    search: Option<&SearchRow>,
    suggestions: &[Suggestion],
) -> Vec<Row> {
    let input = input.trim();
    if input.is_empty() {
        return Vec::new();
    }
    let mut rows: Vec<Row> = search
        .map(|search| Row {
            is_search: true,
            is_bookmark: false,
            title: Emphasis::plain(&format!("Search {} for “{input}”", search.engine)),
            url: Emphasis::default(),
            target: search.url.clone(),
            typed: true,
        })
        .into_iter()
        .collect();
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
            typed: suggestion.typed || suggestion.is_bookmark,
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

/// The text to add after `typed` so that it reads as a suggested host, if one starts with what
/// was typed (`exa` → `mple.com`). Only for something that looks like the start of a host (no
/// spaces, slashes or scheme), and only:
///
/// - to a host the user **typed or bookmarked** (a host reached only by a link or a redirect
///   could be a look-alike such as `paypal.com.evil.example`), the first such suggestion that
///   matches;
/// - in the host's **Unicode** form (`bü` completes `bücher.example`; typing `xn` never runs
///   into punycode);
/// - **never past a completed label**: once what was typed holds a dot, a host that goes on
///   with `.` right after it is not completed (`example.com` never becomes `example.com.au`),
///   while a whole first label still completes (`github` → `github.com`).
pub(crate) fn inline_completion(typed: &str, suggestions: &[Suggestion]) -> Option<String> {
    if typed.is_empty() || typed.contains([' ', '/', ':', '?', '#']) {
        return None;
    }
    let lower = typed.to_lowercase();
    if lower.len() != typed.len() {
        return None;
    }
    suggestions
        .iter()
        .filter(|suggestion| suggestion.typed || suggestion.is_bookmark)
        .find_map(|suggestion| {
            let host = unicode_host(&suggestion.url)?;
            let host = if lower.starts_with("www.") {
                host
            } else {
                host.strip_prefix("www.")
                    .map_or(host.clone(), str::to_owned)
            };
            let rest = host.strip_prefix(lower.as_str())?;
            let extends_a_label = rest.starts_with('.') && lower.contains('.');
            (!rest.is_empty() && !extends_a_label).then(|| rest.to_owned())
        })
}

/// A URL's host in lower-case Unicode, as the address bar shows and the store indexes it.
fn unicode_host(url: &str) -> Option<String> {
    let url = Url::parse(url).ok()?;
    let host = url.host_str()?;
    let unicode = url::quirks::domain_to_unicode(host);
    Some(
        if unicode.is_empty() {
            host.to_owned()
        } else {
            unicode
        }
        .to_lowercase(),
    )
}

/// Whether an edit added to the text (typing), as opposed to deleting: only typing completes,
/// so Backspace can remove a completion.
pub(crate) fn extends(previous: &str, edited: &str) -> bool {
    edited.len() > previous.len() && edited.starts_with(previous)
}

/// What an edit of the address field asks for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Edit {
    /// Nothing to suggest: close the list.
    Close,
    /// Ask the store about `input`; the reply carries `generation`.
    Ask { generation: u64, input: String },
}

/// Text to put in the address field, with the part from `start` to `end` (UTF-8 offsets)
/// selected.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Completion {
    pub(crate) text: String,
    pub(crate) start: usize,
    pub(crate) end: usize,
}

/// The suggestion list's life. Every edit, and every close (the field losing focus, Return,
/// Escape, a choice), starts a new generation: a reply for an older one is dropped, so a list
/// closed before its reply arrived never comes back, and nothing acts on rows no longer shown.
#[derive(Debug, Default)]
pub(crate) struct Session {
    generation: u64,
    /// What the user typed, without any completion.
    typed: String,
    /// Whether the last edit added to the text (only then is a host completed).
    typing: bool,
    rows: Vec<Row>,
    selected: Option<usize>,
    open: bool,
}

impl Session {
    pub(crate) fn edited(&mut self, text: &str) -> Edit {
        self.typing = extends(&self.typed, text);
        self.typed = text.to_owned();
        if text.trim().is_empty() {
            self.close();
            return Edit::Close;
        }
        self.generation += 1;
        self.selected = None;
        Edit::Ask {
            generation: self.generation,
            input: text.to_owned(),
        }
    }

    /// A reply for `generation`. `None` if it is stale (dropped); otherwise the list is open
    /// and this is the completion to apply, if any. `field` is what the address field holds
    /// now: a completion is offered only if that is still exactly what was typed. `search` is
    /// the search row for what was typed, if a search URL could be built.
    pub(crate) fn replied(
        &mut self,
        generation: u64,
        search: Option<SearchRow>,
        suggestions: &[Suggestion],
        field: &str,
    ) -> Option<Option<Completion>> {
        if generation != self.generation {
            return None;
        }
        self.rows = rows(&self.typed, search.as_ref(), suggestions);
        self.selected = None;
        self.open = !self.rows.is_empty();
        let completion = (self.typing && field == self.typed)
            .then(|| inline_completion(&self.typed, suggestions))
            .flatten()
            .map(|rest| Completion {
                text: format!("{}{rest}", self.typed),
                start: self.typed.len(),
                end: self.typed.len() + rest.len(),
            });
        Some(completion)
    }

    /// What to ask the store for generation `generation`, unless it went stale.
    pub(crate) fn pending_input(&self, generation: u64) -> Option<String> {
        (generation == self.generation).then(|| self.typed.clone())
    }

    /// Closes the list; a reply on its way is dropped.
    pub(crate) fn close(&mut self) {
        self.generation += 1;
        self.open = false;
        self.selected = None;
    }

    pub(crate) fn is_open(&self) -> bool {
        self.open
    }

    pub(crate) fn rows(&self) -> &[Row] {
        &self.rows
    }

    pub(crate) fn selected(&self) -> Option<usize> {
        self.selected
    }

    pub(crate) fn move_selection(&mut self, down: bool) {
        if self.open {
            self.selected = move_selection(self.selected, self.rows.len(), down);
        }
    }

    /// Return: the selected row's target if the list is open and a row is selected; the list
    /// closes either way (the field then navigates to what it holds).
    pub(crate) fn accept(&mut self) -> Option<Choice> {
        let choice = self
            .selected
            .filter(|_| self.open)
            .and_then(|index| self.rows.get(index).map(Row::choice));
        self.close();
        choice
    }

    /// A click on row `index` of the list shown.
    pub(crate) fn choose(&mut self, index: usize) -> Option<Choice> {
        let choice = self.rows.get(index).filter(|_| self.open).map(Row::choice);
        self.close();
        choice
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn suggestion(url: &str, title: Option<&str>, is_bookmark: bool) -> Suggestion {
        Suggestion {
            url: url.to_owned(),
            title: title.map(str::to_owned),
            is_bookmark,
            typed: false,
            frecency: 100,
        }
    }

    /// What the default settings' search builder makes for `text`.
    fn ddg(text: &str) -> Option<SearchRow> {
        Some(SearchRow {
            engine: "DuckDuckGo".into(),
            url: format!("https://duckduckgo.com/?q={text}"),
        })
    }

    fn typed(url: &str) -> Suggestion {
        Suggestion {
            typed: true,
            ..suggestion(url, None, false)
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
            ddg("exa").as_ref(),
            &[
                suggestion("https://example.com/", Some("Example Domain"), true),
                suggestion("https://www.examine.test/page", None, false),
            ],
        );

        assert_eq!(rows.len(), 3);
        assert!(rows[0].is_search);
        assert_eq!(rows[0].title.before, "Search DuckDuckGo for “exa”");
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
    fn without_a_search_url_there_is_no_search_row() {
        let rows = rows(
            "exa",
            None,
            &[suggestion("https://example.com/", None, false)],
        );

        assert_eq!(rows.len(), 1);
        assert!(!rows[0].is_search);
        assert!(super::rows("exa", None, &[]).is_empty());
    }

    #[test]
    fn the_search_row_names_the_engine_it_goes_to() {
        let search = SearchRow {
            engine: "Startpage".into(),
            url: "https://www.startpage.com/do/search?q=exa".into(),
        };
        let rows = rows("exa", Some(&search), &[]);

        assert_eq!(rows[0].title.before, "Search Startpage for “exa”");
        assert_eq!(rows[0].target, search.url);
    }

    #[test]
    fn nothing_typed_shows_nothing() {
        assert!(
            rows(
                "   ",
                ddg("").as_ref(),
                &[suggestion("https://a.test/", None, false)]
            )
            .is_empty()
        );
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
    fn a_typed_hosts_rest_completes_what_was_typed() {
        let top = [typed("https://www.example.com/some/page")];

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
    fn only_hosts_the_user_typed_or_bookmarked_complete() {
        // Once redirected through a look-alike host: never offered as a completion.
        let redirected = [suggestion("https://paypal.com.evil.example/", None, false)];
        assert_eq!(inline_completion("paypal", &redirected), None);
        assert_eq!(inline_completion("paypal.com", &redirected), None);

        let bookmarked = [suggestion("https://example.com/", None, true)];
        assert_eq!(
            inline_completion("exa", &bookmarked).as_deref(),
            Some("mple.com")
        );

        // The first suggestion that may complete is used.
        let mixed = [
            suggestion("https://example.com/", None, false),
            typed("https://exam.test/"),
        ];
        assert_eq!(inline_completion("exa", &mixed).as_deref(), Some("m.test"));
    }

    #[test]
    fn a_whole_first_label_completes_to_its_host() {
        let github = [typed("https://github.com/")];

        assert_eq!(
            inline_completion("github", &github).as_deref(),
            Some(".com")
        );
        assert_eq!(
            inline_completion("example", &[typed("https://example.com/")]).as_deref(),
            Some(".com")
        );
    }

    #[test]
    fn a_host_with_a_dot_typed_is_never_extended_by_a_label() {
        let longer = [typed("https://example.com.au/")];

        assert_eq!(inline_completion("example.com", &longer), None);
        assert_eq!(
            inline_completion("exam", &longer).as_deref(),
            Some("ple.com.au")
        );
        assert_eq!(
            inline_completion("example.co", &longer).as_deref(),
            Some("m.au")
        );
    }

    #[test]
    fn a_chosen_row_counts_as_typed_only_if_its_host_could_complete() {
        let mut session = Session::default();
        let generation = ask(&mut session, "exa");
        session.replied(
            generation,
            ddg("exa"),
            &[
                suggestion("https://example.com/", None, false),
                suggestion("https://example.org/", None, true),
                typed("https://example.net/"),
            ],
            "exa",
        );
        let rows = session.rows().to_vec();
        let choice = |index: usize| {
            let mut session = Session {
                open: true,
                rows: rows.clone(),
                ..Session::default()
            };
            session.choose(index).map(|choice| choice.typed)
        };

        // The search row searches what was typed.
        assert_eq!(choice(0), Some(true));
        // Reached only by links: following it is not typing it.
        assert_eq!(choice(1), Some(false));
        assert_eq!(choice(2), Some(true));
        assert_eq!(choice(3), Some(true));
    }

    #[test]
    fn hosts_complete_in_unicode_not_punycode() {
        let unicode = [typed("https://xn--bcher-kva.example/")];

        assert_eq!(inline_completion("xn", &unicode), None);
        assert_eq!(
            inline_completion("bü", &unicode).as_deref(),
            Some("cher.example")
        );
        assert_eq!(
            inline_completion("BÜ", &unicode).as_deref(),
            Some("cher.example")
        );
    }

    #[test]
    fn only_typing_completes() {
        assert!(extends("exa", "exam"));
        assert!(!extends("example.com", "exa"));
        assert!(!extends("exa", "exa"));
        assert!(!extends("exa", "ex"));
    }

    // The list's life: what it shows, and that a stale one never comes back.

    fn rows_of(session: &Session) -> usize {
        session.rows().len()
    }

    fn ask(session: &mut Session, text: &str) -> u64 {
        match session.edited(text) {
            Edit::Ask { generation, input } => {
                assert_eq!(input, text);
                generation
            }
            Edit::Close => panic!("{text:?} should ask for suggestions"),
        }
    }

    #[test]
    fn a_reply_to_the_latest_edit_opens_the_list() {
        let mut session = Session::default();
        let generation = ask(&mut session, "exa");

        let shown = session.replied(
            generation,
            ddg("exa"),
            &[typed("https://example.com/")],
            "exa",
        );

        assert_eq!(
            shown,
            Some(Some(Completion {
                text: "example.com".into(),
                start: 3,
                end: 11,
            }))
        );
        assert!(session.is_open());
        assert_eq!(rows_of(&session), 2);
    }

    #[test]
    fn a_reply_to_an_older_edit_is_dropped() {
        let mut session = Session::default();
        let first = ask(&mut session, "ex");
        let second = ask(&mut session, "exa");

        assert_eq!(session.replied(first, ddg("exa"), &[], "exa"), None);
        assert!(!session.is_open());
        assert!(session.replied(second, ddg("exa"), &[], "exa").is_some());
    }

    #[test]
    fn losing_focus_closes_the_list_and_drops_the_reply_on_its_way() {
        let mut session = Session::default();
        let generation = ask(&mut session, "exa");

        session.close();
        assert_eq!(
            session.replied(
                generation,
                ddg("exa"),
                &[typed("https://example.com/")],
                "exa"
            ),
            None
        );

        // Focus comes back (Cmd+L): nothing reopens until the next edit.
        assert!(!session.is_open());
        assert_eq!(session.accept(), None);
        session.move_selection(true);
        assert_eq!(session.selected(), None);
    }

    #[test]
    fn return_before_the_first_reply_leaves_no_list() {
        let mut session = Session::default();
        let generation = ask(&mut session, "exa");

        assert_eq!(session.accept(), None);

        assert_eq!(session.replied(generation, ddg("exa"), &[], "exa"), None);
        assert!(!session.is_open());
        assert_eq!(session.pending_input(generation), None);
    }

    #[test]
    fn return_opens_the_selected_row_and_closes_the_list() {
        let mut session = Session::default();
        let generation = ask(&mut session, "exa");
        session.replied(
            generation,
            ddg("exa"),
            &[typed("https://example.com/")],
            "example.com",
        );

        session.move_selection(true);
        session.move_selection(true);

        assert_eq!(
            session.accept().map(|choice| choice.target).as_deref(),
            Some("https://example.com/")
        );
        assert!(!session.is_open());
    }

    #[test]
    fn a_completion_needs_typing_and_an_unchanged_field() {
        let suggestions = [typed("https://example.com/")];
        let mut session = Session::default();
        let generation = ask(&mut session, "exa");
        // The field changed since (a page event replaced it).
        assert_eq!(
            session.replied(generation, ddg("exa"), &suggestions, "other"),
            Some(None)
        );

        let generation = ask(&mut session, "exam");
        assert!(
            session
                .replied(generation, ddg("exam"), &suggestions, "exam")
                .unwrap()
                .is_some()
        );
        // Deleting back to "exa" does not complete again.
        let generation = ask(&mut session, "exa");
        assert_eq!(
            session.replied(generation, ddg("exa"), &suggestions, "exa"),
            Some(None)
        );
    }

    #[test]
    fn clearing_the_field_closes_the_list() {
        let mut session = Session::default();
        let generation = ask(&mut session, "exa");
        session.replied(generation, ddg("exa"), &[], "exa");

        assert_eq!(session.edited("  "), Edit::Close);
        assert!(!session.is_open());
    }
}
