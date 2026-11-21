// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! Highlighting with tree-sitter (`tree-sitter-highlight` 0.27 with the html, javascript and css
//! grammars). Item 1 (source-page) writes this module; nothing in it is public API of the crate.
//!
//! The highlighter parses text a page's server sent, with C code, inside the browser process. So
//! it is held on a short rein (brief, risk 4):
//!
//! - nothing above [`MAX_HIGHLIGHT_BYTES`] is parsed at all;
//! - the work runs on a thread of its own, never on the caller's (which may be a runtime worker);
//! - the caller waits [`HIGHLIGHT_TIMEOUT`], then sets the cancellation flag that the parser and
//!   the highlighter poll, and waits a short grace for the thread to answer; after that it
//!   leaves the thread to finish by itself and shows the text plain;
//! - a panic, an error or a cancellation all give [`Outcome::Failed`], never a broken page.
//!
//! The 2 s "timer" of the brief is the caller's own `recv_timeout`: it sets the flag exactly when a
//! timer thread would, without a third thread and without a timer that has to be woken up.
//!
//! What it gives back is a flat list of non-overlapping [`Span`]s (a class for a byte range) and
//! the byte ranges of `href` and `src` values found in the HTML syntax tree. The page turns both
//! into markup.

use std::ops::Range;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, OnceLock, mpsc};
use std::thread;
use std::time::Duration;

use tree_sitter_highlight::{HighlightConfiguration, HighlightEvent, Highlighter};

use crate::source::SourceKind;
use crate::{MAX_HIGHLIGHT_BYTES, MAX_LINKS};

/// How long the highlighter may run before it is told to stop (the brief: 2 s).
pub(crate) const HIGHLIGHT_TIMEOUT: Duration = Duration::from_secs(2);
/// How long a cancelled highlighter may take to answer before the thread is left alone.
const CANCEL_GRACE: Duration = Duration::from_millis(500);
/// The worker's stack. The grammars and the highlighter are iterative, but this is untrusted input
/// in C code: a roomy stack costs nothing, and a thread's pages are only touched when used.
const WORKER_STACK: usize = 8 * 1024 * 1024;
/// More `href` and `src` candidates than this are not looked at (`MAX_LINKS` become links).
const MAX_LINK_CANDIDATES: usize = 4 * MAX_LINKS;

/// A syntax class of the page: the CSS class and the palette token that colours it.
pub(crate) struct Class {
    /// The class attribute value (short: it is repeated for every token).
    pub css: &'static str,
    /// The name of the colour in `style::PALETTE`.
    pub token: &'static str,
}

const COMMENT: u8 = 0;
const KEYWORD: u8 = 1;
const STRING: u8 = 2;
const SPECIAL: u8 = 3;
const NUMBER: u8 = 4;
const FUNCTION: u8 = 5;
const TYPE: u8 = 6;
const TAG: u8 = 7;
const ATTRIBUTE: u8 = 8;
const TAG_ERROR: u8 = 9;

/// Every class, indexed by the numbers above.
pub(crate) const CLASSES: &[Class] = &[
    Class {
        css: "c",
        token: "comment",
    },
    Class {
        css: "k",
        token: "keyword",
    },
    Class {
        css: "s",
        token: "string",
    },
    Class {
        css: "r",
        token: "special",
    },
    Class {
        css: "n",
        token: "number",
    },
    Class {
        css: "f",
        token: "function",
    },
    Class {
        css: "y",
        token: "type",
    },
    Class {
        css: "t",
        token: "tag",
    },
    Class {
        css: "a",
        token: "attribute",
    },
    Class {
        css: "e",
        token: "tag-error",
    },
];

/// The highlight name of the value of an `href` or `src` attribute: coloured as a string, and
/// reported as a link range (see [`LINK_QUERY`]).
const LINK_NAME: &str = "string.link";

/// An extra pattern for the HTML highlight query, after the grammar's own: the value of an `href`
/// or `src` attribute, quoted or not. It is found in the same syntax tree, by the same parse, as
/// the colours, so cancelling the highlighter cancels this too, and no second parse is needed. The
/// last pattern that matches a node wins, so these values are `string.link`, not `string`; the
/// recognised name maps to the string class. The attribute name is captured as `@attribute` again,
/// not under a private name: the last pattern that matches a node wins for it too, and a capture
/// that is not recognised would take the colour away from `href` and `src` themselves.
const LINK_QUERY: &str = "(attribute (attribute_name) @attribute \
    [(attribute_value) @string.link (quoted_attribute_value (attribute_value) @string.link)] \
    (#match? @attribute \"(?i)^(href|src)$\"))";

/// The highlight names the highlighter is told to recognise, in the order `Highlight(i)` counts
/// them. `Some(class)` is coloured. `None` is recognised and shown in the default colour on purpose
/// (the named allow-list of the capture-coverage test):
///
/// - `punctuation.*` and `operator`: about a third of all tokens of a script, each would cost a
///   span (24 bytes and a node) and colour carries little meaning there;
/// - `variable`: every identifier of a script;
/// - `embedded`: the `${...}` of a template string, which should not stay string-coloured.
///
/// `tree-sitter-highlight` matches a capture to the recognised name with the most parts that are
/// all parts of the capture (`function.builtin` beats `function`), so a more specific name here
/// takes the captures it names out of the broader one.
pub(crate) const RECOGNIZED: &[(&str, Option<u8>)] = &[
    ("comment", Some(COMMENT)),
    ("keyword", Some(KEYWORD)),
    ("variable.builtin", Some(KEYWORD)),
    ("string", Some(STRING)),
    ("string.special", Some(SPECIAL)),
    (LINK_NAME, Some(STRING)),
    ("number", Some(NUMBER)),
    ("constant", Some(NUMBER)),
    ("constant.builtin", Some(NUMBER)),
    ("function", Some(FUNCTION)),
    ("function.builtin", Some(FUNCTION)),
    ("function.method", Some(FUNCTION)),
    ("type", Some(TYPE)),
    ("constructor", Some(TYPE)),
    ("tag", Some(TAG)),
    ("tag.error", Some(TAG_ERROR)),
    ("attribute", Some(ATTRIBUTE)),
    ("property", Some(ATTRIBUTE)),
    // Shown in the default colour.
    ("punctuation.bracket", None),
    ("punctuation.delimiter", None),
    ("punctuation.special", None),
    ("operator", None),
    ("variable", None),
    ("embedded", None),
];

/// Captures that the highlighter reads for itself (injections and local variables) and that are
/// never a highlight, plus `glimmer`: a capture of the JavaScript injection query that names a
/// language for an editor's Handlebars support and that this crate does not use. Captures that
/// start with `_` are private to a query, by convention, and are not listed.
#[cfg(test)]
pub(crate) const INTERNAL: &[&str] = &[
    "injection.content",
    "injection.language",
    "local.scope",
    "local.definition",
    "local.reference",
    "glimmer",
];

/// One highlighted range of the text.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Span {
    /// First byte.
    pub start: usize,
    /// One past the last byte.
    pub end: usize,
    /// An index into [`CLASSES`].
    pub class: u8,
}

/// What highlighting found.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Highlighted {
    /// Ordered, non-overlapping, on character boundaries.
    pub spans: Vec<Span>,
    /// Byte ranges of the values of `href` and `src` attributes of the HTML syntax tree, in
    /// document order (HTML and XML only). They are raw text: entities are not decoded.
    pub links: Vec<Range<usize>>,
}

/// What [`highlight`] did.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Outcome {
    /// The kind of source has no grammar (plain text, binary): nothing was tried.
    NoGrammar,
    /// More than `MAX_HIGHLIGHT_BYTES`: nothing was tried.
    TooLarge,
    /// An error, a panic, a cancellation or no thread: the text is shown plain.
    Failed,
    /// Done (possibly with nothing to colour).
    Done(Highlighted),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Lang {
    Html,
    JavaScript,
    Css,
}

impl Lang {
    const fn of(kind: SourceKind) -> Option<Self> {
        match kind {
            // SVG and XML go through the HTML grammar, which tolerates them.
            SourceKind::Html | SourceKind::Xml => Some(Self::Html),
            SourceKind::JavaScript => Some(Self::JavaScript),
            SourceKind::Css => Some(Self::Css),
            SourceKind::Plain | SourceKind::Binary => None,
        }
    }
}

struct Configs {
    html: HighlightConfiguration,
    javascript: HighlightConfiguration,
    css: HighlightConfiguration,
}

impl Configs {
    const fn get(&self, lang: Lang) -> &HighlightConfiguration {
        match lang {
            Lang::Html => &self.html,
            Lang::JavaScript => &self.javascript,
            Lang::Css => &self.css,
        }
    }

    /// The configuration for an injected language, by the name the injection query gives: the
    /// HTML grammar names `javascript` and `css`; a tagged template literal in a script names
    /// whatever its tag is called (`css`, `html`, and others that are not highlighted).
    fn injection(&self, name: &str) -> Option<&HighlightConfiguration> {
        match name {
            "javascript" | "js" => Some(&self.javascript),
            "css" => Some(&self.css),
            "html" => Some(&self.html),
            _ => None,
        }
    }
}

/// A highlight configuration for a grammar (a macro, because the grammar crates give a `LanguageFn`
/// whose type this crate does not depend on: `.into()` finds the `tree-sitter` type by itself).
macro_rules! config {
    ($name:literal, $grammar:ident, $highlights:expr, $injections:expr, $locals:expr) => {{
        let config = HighlightConfiguration::new(
            $grammar::LANGUAGE.into(),
            $name,
            $highlights,
            $injections,
            $locals,
        );
        match config {
            Ok(mut config) => {
                let names: Vec<&str> = RECOGNIZED.iter().map(|(name, _)| *name).collect();
                config.configure(&names);
                config
            }
            Err(error) => {
                log::error!(
                    "view source: the {} highlight queries do not load: {error}",
                    $name
                );
                return None;
            }
        }
    }};
}

fn build_configs() -> Option<Configs> {
    Some(Configs {
        html: config!(
            "html",
            tree_sitter_html,
            &format!("{}\n{LINK_QUERY}", tree_sitter_html::HIGHLIGHTS_QUERY),
            tree_sitter_html::INJECTIONS_QUERY,
            ""
        ),
        javascript: config!(
            "javascript",
            tree_sitter_javascript,
            tree_sitter_javascript::HIGHLIGHT_QUERY,
            tree_sitter_javascript::INJECTIONS_QUERY,
            tree_sitter_javascript::LOCALS_QUERY
        ),
        css: config!(
            "css",
            tree_sitter_css,
            tree_sitter_css::HIGHLIGHTS_QUERY,
            "",
            ""
        ),
    })
}

/// The three configurations, built once (parsing the queries takes a few milliseconds).
fn configs() -> Option<&'static Configs> {
    static CONFIGS: OnceLock<Option<Configs>> = OnceLock::new();
    CONFIGS.get_or_init(build_configs).as_ref()
}

/// Highlights `text` as `kind`, within [`HIGHLIGHT_TIMEOUT`].
pub(crate) fn highlight(kind: SourceKind, text: &str) -> Outcome {
    highlight_within(kind, text, HIGHLIGHT_TIMEOUT)
}

/// [`highlight`] with the time limit as a parameter (the tests use a short one).
pub(crate) fn highlight_within(kind: SourceKind, text: &str, timeout: Duration) -> Outcome {
    let Some(lang) = Lang::of(kind) else {
        return Outcome::NoGrammar;
    };
    if text.len() > MAX_HIGHLIGHT_BYTES {
        return Outcome::TooLarge;
    }
    if text.is_empty() {
        return Outcome::Done(Highlighted::default());
    }
    let cancel = Arc::new(AtomicUsize::new(0));
    let (sender, receiver) = mpsc::sync_channel(1);
    let owned = text.to_owned();
    let worker_cancel = Arc::clone(&cancel);
    let spawned = thread::Builder::new()
        .name("view-source-highlight".to_owned())
        .stack_size(WORKER_STACK)
        .spawn(move || {
            let result = catch_unwind(AssertUnwindSafe(|| run(lang, &owned, &worker_cancel)));
            let _ = sender.send(result.unwrap_or(None));
        });
    if let Err(error) = spawned {
        log::warn!("view source: no thread for the highlighter: {error}");
        return Outcome::Failed;
    }
    let answer = match receiver.recv_timeout(timeout) {
        Ok(answer) => answer,
        Err(mpsc::RecvTimeoutError::Timeout) => {
            log::debug!("view source: highlighting is taking too long; cancelling it");
            cancel.store(1, Ordering::SeqCst);
            receiver.recv_timeout(CANCEL_GRACE).ok().flatten()
        }
        Err(mpsc::RecvTimeoutError::Disconnected) => None,
    };
    answer.map_or(Outcome::Failed, Outcome::Done)
}

/// A text with this in it makes the worker panic (the tests of the fall-back).
#[cfg(test)]
pub(crate) const PANIC_MARKER: &str = "PANIC-IN-THE-HIGHLIGHTER";

/// A highlight frame of the event stream: what colours its text, and, for a link, where it is.
struct Frame {
    class: Option<u8>,
    /// The frame of an `href` or `src` value ([`LINK_NAME`]): its text is a link.
    link: Option<Range<usize>>,
    is_link: bool,
}

/// The worker: highlights and finds links. `None` is any failure.
fn run(lang: Lang, text: &str, cancel: &AtomicUsize) -> Option<Highlighted> {
    #[cfg(test)]
    if text.contains(PANIC_MARKER) {
        panic!("a test asked the highlighter to panic");
    }
    let configs = configs()?;
    let config = configs.get(lang);
    let mut highlighter = Highlighter::new();
    let events = highlighter
        .highlight(config, text.as_bytes(), None, Some(cancel), |name: &str| {
            configs.injection(name)
        })
        .ok()?;
    let mut spans: Vec<Span> = Vec::new();
    let mut links: Vec<Range<usize>> = Vec::new();
    // Highlights nest (a template substitution inside a string); the innermost one colours.
    let mut stack: Vec<Frame> = Vec::new();
    for event in events {
        match event.ok()? {
            HighlightEvent::HighlightStart(highlight) => {
                let recognized = RECOGNIZED.get(highlight.0);
                stack.push(Frame {
                    class: recognized.and_then(|(_, class)| *class),
                    link: None,
                    is_link: recognized.is_some_and(|(name, _)| *name == LINK_NAME),
                });
            }
            HighlightEvent::HighlightEnd => {
                if let Some(Frame {
                    is_link: true,
                    link: Some(range),
                    ..
                }) = stack.pop()
                    && links.len() < MAX_LINK_CANDIDATES
                {
                    links.push(range);
                }
            }
            HighlightEvent::Source { start, end } => {
                if let Some(frame) = stack.last_mut() {
                    if frame.is_link {
                        let range = frame.link.get_or_insert(start..end);
                        range.end = end;
                    }
                    if let Some(class) = frame.class {
                        push_span(&mut spans, text, start, end, class);
                    }
                }
            }
        }
    }
    Some(Highlighted { spans, links })
}

/// Appends a span, joining it to the one before when they touch and share a class. Anything that
/// is out of order, out of range or not on character boundaries is dropped (never a panic).
fn push_span(spans: &mut Vec<Span>, text: &str, start: usize, end: usize, class: u8) {
    if start >= end
        || end > text.len()
        || !text.is_char_boundary(start)
        || !text.is_char_boundary(end)
    {
        return;
    }
    if let Some(last) = spans.last_mut() {
        if last.end > start {
            return;
        }
        if last.end == start && last.class == class {
            last.end = end;
            return;
        }
    }
    spans.push(Span { start, end, class });
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;
    use std::time::Instant;

    use super::*;

    /// What `HighlightConfiguration::configure` does with a capture name, written out: the
    /// recognised name with the most dot-separated parts, all of which are parts of the capture;
    /// on a tie the first in the list.
    fn resolve(capture: &str) -> Option<usize> {
        let parts: Vec<&str> = capture.split('.').collect();
        let mut best: Option<(usize, usize)> = None;
        for (index, (name, _)) in RECOGNIZED.iter().enumerate() {
            let wanted: Vec<&str> = name.split('.').collect();
            if wanted.iter().all(|part| parts.contains(part))
                && best.is_none_or(|(_, len)| wanted.len() > len)
            {
                best = Some((index, wanted.len()));
            }
        }
        best.map(|(index, _)| index)
    }

    fn is_internal(capture: &str) -> bool {
        INTERNAL.contains(&capture) || capture.starts_with('_')
    }

    fn all_captures() -> Vec<(&'static str, BTreeSet<String>)> {
        let configs = configs().expect("the highlight queries load");
        [
            ("html", &configs.html),
            ("javascript", &configs.javascript),
            ("css", &configs.css),
        ]
        .into_iter()
        .map(|(name, config)| {
            (
                name,
                config.names().iter().map(|n| (*n).to_owned()).collect(),
            )
        })
        .collect()
    }

    #[test]
    fn every_capture_of_the_three_highlight_queries_has_a_class_or_a_named_reason() {
        let mut coloured = BTreeSet::new();
        let mut default_colour = BTreeSet::new();
        let mut internal = BTreeSet::new();
        let mut dropped = Vec::new();
        for (language, captures) in all_captures() {
            for capture in captures {
                if is_internal(&capture) {
                    internal.insert(capture);
                } else {
                    match resolve(&capture).map(|index| RECOGNIZED[index].1) {
                        Some(Some(_)) => {
                            coloured.insert(capture);
                        }
                        Some(None) => {
                            default_colour.insert(capture);
                        }
                        None => dropped.push(format!("{language}: {capture}")),
                    }
                }
            }
        }
        assert!(
            dropped.is_empty(),
            "captures that match no recognised name, so are silently dropped: {dropped:?}"
        );
        eprintln!("coloured captures: {coloured:?}");
        eprintln!("default-colour captures (the allow-list): {default_colour:?}");
        eprintln!("internal captures: {internal:?}");
        // Each of the three kinds is in use, so the lists are not stale.
        assert!(!coloured.is_empty() && !default_colour.is_empty() && !internal.is_empty());
    }

    #[test]
    fn the_allow_lists_name_only_captures_the_queries_use() {
        let all: BTreeSet<String> = all_captures().into_iter().flat_map(|(_, c)| c).collect();
        for name in INTERNAL {
            assert!(
                all.contains(*name),
                "INTERNAL lists {name}, which no query uses"
            );
        }
        for (name, class) in RECOGNIZED {
            assert!(
                all.iter()
                    .any(|capture| resolve(capture).map(|i| RECOGNIZED[i].0) == Some(*name)),
                "RECOGNIZED lists {name}, which no capture resolves to"
            );
            if let Some(class) = class {
                assert!(
                    usize::from(*class) < CLASSES.len(),
                    "{name}: class {class} does not exist"
                );
            }
        }
    }

    #[test]
    fn every_class_is_used_and_class_names_are_unique_and_short() {
        let used: BTreeSet<u8> = RECOGNIZED.iter().filter_map(|(_, class)| *class).collect();
        for (index, class) in CLASSES.iter().enumerate() {
            assert!(
                used.contains(&u8::try_from(index).unwrap()),
                "class .{} is never used",
                class.css
            );
            assert!(class.css.len() <= 2 && class.css.bytes().all(|b| b.is_ascii_lowercase()));
        }
        let names: BTreeSet<&str> = CLASSES.iter().map(|class| class.css).collect();
        assert_eq!(names.len(), CLASSES.len());
        // Class names must not collide with the page's own classes.
        for page_class in [
            "l", "src", "addr", "facts", "note", "notice", "err", "d1", "d2",
        ] {
            assert!(
                !names.contains(page_class),
                "{page_class} is a class of the page"
            );
        }
    }

    /// The text of each span with its class name, for readable assertions.
    fn labelled(kind: SourceKind, text: &str) -> Vec<(String, &'static str)> {
        match highlight_within(kind, text, Duration::from_secs(20)) {
            Outcome::Done(done) => done
                .spans
                .iter()
                .map(|span| {
                    (
                        text[span.start..span.end].to_owned(),
                        CLASSES[usize::from(span.class)].css,
                    )
                })
                .collect(),
            other => panic!("expected a highlight, got {other:?}"),
        }
    }

    fn has(spans: &[(String, &'static str)], text: &str, class: &str) -> bool {
        spans.iter().any(|(t, c)| t == text && *c == class)
    }

    #[test]
    fn html_tags_attributes_values_comments_and_doctype_are_coloured() {
        let spans = labelled(
            SourceKind::Html,
            "<!doctype html><!-- note --><a href=\"/x\" class=y>link</a>",
        );
        assert!(has(&spans, "a", "t"), "{spans:?}");
        assert!(has(&spans, "href", "a"), "{spans:?}");
        assert!(has(&spans, "/x", "s"), "{spans:?}");
        assert!(has(&spans, "class", "a"), "{spans:?}");
        assert!(has(&spans, "<!-- note -->", "c"), "{spans:?}");
        assert!(has(&spans, "<!doctype html", "n"), "{spans:?}");
    }

    #[test]
    fn a_script_element_is_highlighted_as_javascript_and_a_style_element_as_css() {
        let source = "<script>var answer = 42; // note\nfunction go() { return \"s\"; }</script>\
                      <style>a.b { color: red; margin: 10px } /* c */</style>";
        let spans = labelled(SourceKind::Html, source);
        // JavaScript inside <script>.
        assert!(has(&spans, "var", "k"), "{spans:?}");
        assert!(has(&spans, "42", "n"), "{spans:?}");
        assert!(has(&spans, "// note", "c"), "{spans:?}");
        assert!(has(&spans, "function", "k"), "{spans:?}");
        assert!(has(&spans, "go", "f"), "{spans:?}");
        assert!(has(&spans, "\"s\"", "s"), "{spans:?}");
        // CSS inside <style>.
        assert!(has(&spans, "color", "a"), "{spans:?}");
        assert!(has(&spans, "margin", "a"), "{spans:?}");
        assert!(has(&spans, "10", "n"), "{spans:?}");
        assert!(has(&spans, "/* c */", "c"), "{spans:?}");
        assert!(has(&spans, "b", "a"), "{spans:?}");
    }

    #[test]
    fn javascript_and_css_files_are_highlighted_without_html() {
        let js = labelled(
            SourceKind::JavaScript,
            "class Box extends Base { static make() { return new Box(this.x, `a${1}b`, /re+/g, null); } }",
        );
        assert!(has(&js, "class", "k"), "{js:?}");
        assert!(has(&js, "Box", "y"), "{js:?}");
        assert!(has(&js, "make", "f"), "{js:?}");
        assert!(has(&js, "this", "k"), "{js:?}");
        assert!(has(&js, "null", "n"), "{js:?}");
        assert!(has(&js, "re+", "r") && has(&js, "g", "r"), "{js:?}");
        assert!(
            js.iter().any(|(t, c)| *c == "s" && t.starts_with('`')),
            "{js:?}"
        );
        let css = labelled(
            SourceKind::Css,
            "@media (min-width: 10px) { .a > #b:hover { color: #fff; width: calc(1px + 2em) } }",
        );
        assert!(has(&css, "@media", "k"), "{css:?}");
        assert!(has(&css, "color", "a"), "{css:?}");
        assert!(has(&css, "fff", "r"), "{css:?}");
        assert!(has(&css, "em", "y"), "{css:?}");
        assert!(has(&css, "calc", "f"), "{css:?}");
    }

    #[test]
    fn xml_and_svg_use_the_html_grammar() {
        let spans = labelled(
            SourceKind::Xml,
            "<svg width=\"10\"><path d=\"M0 0\"/></svg>",
        );
        assert!(has(&spans, "svg", "t"), "{spans:?}");
        assert!(has(&spans, "width", "a"), "{spans:?}");
        assert!(has(&spans, "10", "s"), "{spans:?}");
    }

    #[test]
    fn plain_and_binary_never_reach_the_highlighter() {
        assert_eq!(
            highlight(SourceKind::Plain, "{\"a\": 1}"),
            Outcome::NoGrammar
        );
        assert_eq!(highlight(SourceKind::Binary, "PNG"), Outcome::NoGrammar);
    }

    #[test]
    fn empty_text_has_nothing_to_colour() {
        assert_eq!(
            highlight(SourceKind::Html, ""),
            Outcome::Done(Highlighted::default())
        );
    }

    #[test]
    fn spans_are_ordered_disjoint_in_range_and_on_character_boundaries() {
        // Multi-byte text in every position, broken markup, and a lone surrogate-like mess.
        let samples = [
            "<p title=\"héllo ☃\">é<script>var é = '☃'; /* ü */</script><style>é{ü:ö}</style></p",
            "<<<>>><a href='x<script>'>&&&</b></a><!--",
            "var s = `é${'☃'}é`; // ü\n/* ☃ */ x = /é+/;",
            "a{b:'☃';c:url(é)} @media{",
        ];
        for sample in samples {
            for kind in [
                SourceKind::Html,
                SourceKind::JavaScript,
                SourceKind::Css,
                SourceKind::Xml,
            ] {
                let Outcome::Done(done) = highlight_within(kind, sample, Duration::from_secs(20))
                else {
                    panic!("{kind:?}: {sample:?} did not highlight");
                };
                let mut previous_end = 0;
                for span in &done.spans {
                    assert!(span.start < span.end && span.end <= sample.len());
                    assert!(span.start >= previous_end, "overlap in {kind:?} {sample:?}");
                    assert!(
                        sample.is_char_boundary(span.start) && sample.is_char_boundary(span.end)
                    );
                    assert!(usize::from(span.class) < CLASSES.len());
                    previous_end = span.end;
                }
                for link in &done.links {
                    assert!(link.start < link.end && sample.is_char_boundary(link.start));
                    assert!(sample.is_char_boundary(link.end));
                }
            }
        }
    }

    #[test]
    fn touching_spans_of_one_class_are_joined() {
        let text = "x";
        let mut spans = Vec::new();
        push_span(&mut spans, text, 0, 1, KEYWORD);
        let text = "xy";
        push_span(&mut spans, text, 1, 2, KEYWORD);
        push_span(&mut spans, text, 1, 2, STRING); // overlaps the joined span: dropped
        assert_eq!(
            spans,
            vec![Span {
                start: 0,
                end: 2,
                class: KEYWORD
            }]
        );
        // Out of range and inside a character: dropped, not a panic.
        let mut spans = Vec::new();
        push_span(&mut spans, "é", 0, 1, KEYWORD);
        push_span(&mut spans, "é", 0, 9, KEYWORD);
        push_span(&mut spans, "é", 1, 1, KEYWORD);
        assert!(spans.is_empty());
    }

    fn links_of(source: &str) -> Vec<String> {
        let Outcome::Done(done) =
            highlight_within(SourceKind::Html, source, Duration::from_secs(20))
        else {
            panic!("no highlight");
        };
        done.links
            .iter()
            .map(|range| source[range.clone()].to_owned())
            .collect()
    }

    #[test]
    fn links_come_from_href_and_src_attribute_nodes_not_from_text() {
        let source = "<a href=\"/one\">href=\"/no\"</a><IMG SRC='/two'><script src=/three></script>\
                      <p data-href=\"/no\" title=\"href=/no\" xlink:href=\"/no\">src=\"/no\"</p>\
                      <script>var s = '<a href=\"/no\">';</script><a href=\"\"><a href>\
                      <!-- <a href=\"/no\"> --><link rel=x HREF=\"/four\">";
        assert_eq!(links_of(source), ["/one", "/two", "/three", "/four"]);
    }

    #[test]
    fn deep_nesting_still_finds_its_links() {
        let source = format!("{}<a href=\"/deep\">", "<div>".repeat(1_500));
        assert_eq!(links_of(&source), ["/deep"]);
    }

    #[test]
    fn more_candidates_than_the_cap_are_not_all_collected() {
        let source = "<a href=\"/x\"></a>".repeat(MAX_LINK_CANDIDATES + 100);
        assert!(source.len() <= MAX_HIGHLIGHT_BYTES);
        let found = links_of(&source).len();
        assert_eq!(found, MAX_LINK_CANDIDATES);
    }

    #[test]
    fn more_than_the_highlight_limit_is_not_even_tried() {
        let text = "a".repeat(MAX_HIGHLIGHT_BYTES + 1);
        let started = Instant::now();
        assert_eq!(highlight(SourceKind::JavaScript, &text), Outcome::TooLarge);
        assert!(
            started.elapsed() < Duration::from_millis(250),
            "it must answer at once"
        );
        // The limit itself is still highlighted.
        let at_limit = "a".repeat(MAX_HIGHLIGHT_BYTES);
        assert!(matches!(
            highlight(SourceKind::JavaScript, &at_limit),
            Outcome::Done(_)
        ));
    }

    /// The pathological input of the brief: deeply nested `<div>` repeated up to the limit.
    fn nested_divs() -> String {
        "<div>".repeat(MAX_HIGHLIGHT_BYTES / 5)
    }

    #[test]
    fn a_cancelled_highlight_falls_back_at_once() {
        let text = nested_divs();
        assert!(text.len() <= MAX_HIGHLIGHT_BYTES);
        let started = Instant::now();
        // A limit of zero sets the cancellation flag before the parse can finish.
        let outcome = highlight_within(SourceKind::Html, &text, Duration::ZERO);
        let took = started.elapsed();
        assert_eq!(outcome, Outcome::Failed);
        assert!(took < Duration::from_secs(2), "cancelling took {took:?}");
    }

    #[test]
    fn the_real_limit_bounds_the_pathological_input() {
        let text = nested_divs();
        let started = Instant::now();
        let outcome = highlight(SourceKind::Html, &text);
        let took = started.elapsed();
        eprintln!(
            "nested divs, {} bytes: {took:?}, {}",
            text.len(),
            match &outcome {
                Outcome::Done(done) => format!("done, {} spans", done.spans.len()),
                other => format!("{other:?}"),
            }
        );
        // 2 s limit plus the 500 ms grace; a generous margin for a loaded machine.
        assert!(
            took < HIGHLIGHT_TIMEOUT + CANCEL_GRACE + Duration::from_secs(3),
            "took {took:?}"
        );
        assert!(matches!(outcome, Outcome::Failed | Outcome::Done(_)));
    }

    #[test]
    fn a_panic_in_the_worker_is_a_failure_not_a_crash() {
        let text = format!("<p>{PANIC_MARKER}</p>");
        assert_eq!(highlight(SourceKind::Html, &text), Outcome::Failed);
        // And the next call is unaffected.
        assert!(matches!(
            highlight(SourceKind::Html, "<p>fine</p>"),
            Outcome::Done(_)
        ));
    }

    #[test]
    fn the_worker_does_not_hold_the_callers_thread() {
        // Highlighting from a thread with a tiny stack still works: the work is on its own thread.
        let handle = thread::Builder::new()
            .stack_size(64 * 1024)
            .spawn(|| highlight(SourceKind::Html, &"<div>".repeat(2000)))
            .expect("thread");
        assert!(matches!(handle.join().expect("no panic"), Outcome::Done(_)));
    }
}
