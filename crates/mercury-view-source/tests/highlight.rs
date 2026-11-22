// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! Highlighting as a reader of the page sees it: which classes appear for which source, that the
//! `<script>` and `<style>` of a page are highlighted as JavaScript and CSS, that a token that
//! runs over a line break is closed and opened again on every line, that the colours the page
//! carries are readable in both schemes, and that hostile input costs bounded time.
//!
//! The capture-name coverage (every capture of the three highlight queries has a class or is on a
//! named allow-list), the forced cancellation and the panic fall-back are unit tests inside
//! `src/highlight.rs`, where the internals are visible.

use std::collections::BTreeSet;
use std::time::{Duration, Instant};

use mercury_view_source::{
    MAX_HIGHLIGHT_BYTES, SourceBody, SourceDocument, SourceKind, ViewSourceUrl, render_page,
};
use tree_sitter::{Parser, Tree};
use url::Url;

const PAGE_ADDRESS: &str = "https://example.test/dir/page.html";

fn render(kind: SourceKind, text: &str) -> String {
    let request = ViewSourceUrl::parse(&format!("view-source:{PAGE_ADDRESS}")).expect("address");
    let document = SourceDocument {
        final_url: Url::parse(PAGE_ADDRESS).expect("url"),
        status: 200,
        content_type: None,
        kind,
        body: SourceBody::Text {
            text: text.to_owned(),
            bytes: text.len() as u64,
            truncated: false,
            charset: "UTF-8",
        },
    };
    render_page(&document, &request).html
}

fn parse(html: &str) -> Tree {
    let mut parser = Parser::new();
    parser
        .set_language(&tree_sitter_html::LANGUAGE.into())
        .expect("html grammar");
    parser.parse(html, None).expect("a parse")
}

fn assert_no_error_node(html: &str) {
    let tree = parse(html);
    assert!(
        !tree.root_node().has_error(),
        "the page does not parse cleanly"
    );
}

/// The lines of the page, each with its markup (between `<span>` and `</span></div>`).
fn line_markup(html: &str) -> Vec<&str> {
    html.split("<div class=\"l\" data-n=\"")
        .skip(1)
        .map(|chunk| {
            let after = chunk.split_once("\"><span>").expect("line markup").1;
            after.split_once("</span></div>").expect("line end").0
        })
        .collect()
}

/// The token classes that appear in the page's lines.
fn classes_used(html: &str) -> BTreeSet<String> {
    let mut found = BTreeSet::new();
    for chunk in html.split("<span class=\"").skip(1) {
        if let Some((class, _)) = chunk.split_once('"') {
            // Header and gutter classes are not on `span`; only token classes are.
            found.insert(class.to_owned());
        }
    }
    found
}

fn has_token(html: &str, class: &str, text: &str) -> bool {
    html.contains(&format!("<span class=\"{class}\">{text}</span>"))
}

// ---------------------------------------------------------------------------------------------
// What is coloured.
// ---------------------------------------------------------------------------------------------

const EVERYTHING: &str = "<!doctype html>\n<!-- a comment -->\n<div id=\"x\"></p></div>\n\
<script>class Box { static make() { return new Box(this.x, \"s\", /re+/g, 42, true); } }\n\
function go() { return console.log(go()); }</script>\n\
<style>a { color: #fff; width: 1px }</style>\n";

#[test]
fn a_page_with_everything_uses_every_class() {
    let html = render(SourceKind::Html, EVERYTHING);
    assert_no_error_node(&html);
    let used = classes_used(&html);
    let expected: BTreeSet<String> = ["c", "k", "s", "r", "n", "f", "y", "t", "a", "e"]
        .iter()
        .map(|c| (*c).to_owned())
        .collect();
    assert_eq!(used, expected, "classes used");
    // Spot checks of what lands in which class.
    for (class, text) in [
        ("t", "div"),
        ("a", "id"),
        ("s", "x"),
        ("e", "p"),
        ("k", "class"),
        ("y", "Box"),
        ("f", "make"),
        ("k", "this"),
        ("r", "re+"),
        ("r", "g"),
        ("n", "42"),
        ("n", "true"),
        ("f", "go"),
        ("t", "a"),
        ("a", "color"),
        ("r", "fff"),
        ("y", "px"),
    ] {
        assert!(has_token(&html, class, text), "{class}: {text}\n{html}");
    }
    assert!(html.contains("<span class=\"c\">&lt;!-- a comment --&gt;</span>"));
}

#[test]
fn script_and_style_elements_are_highlighted_as_javascript_and_css() {
    let html = render(
        SourceKind::Html,
        "<script>var answer = 42; // note\n</script><style>p { margin: 0 } /* c */</style>",
    );
    assert_no_error_node(&html);
    assert!(has_token(&html, "k", "var"));
    assert!(has_token(&html, "n", "42"));
    assert!(has_token(&html, "c", "// note"));
    assert!(has_token(&html, "a", "margin"));
    assert!(has_token(&html, "n", "0"));
    assert!(has_token(&html, "c", "/* c */"));
    // The html grammar's own captures still apply around them.
    assert!(has_token(&html, "t", "script") && has_token(&html, "t", "style"));
}

#[test]
fn javascript_css_and_svg_files_are_highlighted_by_their_own_grammar() {
    let js = render(
        SourceKind::JavaScript,
        "export const f = async (a) => { await a; };\n",
    );
    assert!(
        has_token(&js, "k", "export")
            && has_token(&js, "k", "const")
            && has_token(&js, "k", "await")
    );
    assert!(
        !js.contains("<span class=\"t\">"),
        "no html tags in a script"
    );
    let css = render(
        SourceKind::Css,
        "h1, .a:hover { background: url(x.png) no-repeat }\n",
    );
    assert!(has_token(&css, "t", "h1") && has_token(&css, "a", "background"));
    let svg = render(SourceKind::Xml, "<svg width=\"10\"><circle r=\"4\"/></svg>");
    assert!(
        has_token(&svg, "t", "svg")
            && has_token(&svg, "t", "circle")
            && has_token(&svg, "a", "width")
    );
    assert_no_error_node(&svg);
}

#[test]
fn plain_text_json_and_binary_kinds_get_no_token_markup() {
    let json = render(
        SourceKind::Plain,
        "{\"a\": [1, 2, true, null], \"b\": \"<x>\"}",
    );
    assert!(classes_used(&json).is_empty(), "{:?}", classes_used(&json));
    assert!(
        !json.contains("Colours and links are not shown"),
        "plain is not a failure"
    );
}

#[test]
fn a_token_over_a_line_break_is_closed_and_opened_again_on_each_line() {
    let html = render(
        SourceKind::JavaScript,
        "/* one\ntwo\nthree */ let x = `a\nb\nc`;\n",
    );
    assert_no_error_node(&html);
    let lines = line_markup(&html);
    assert_eq!(lines.len(), 5, "{lines:?}");
    assert_eq!(lines[0], "<span class=\"c\">/* one</span>");
    assert_eq!(lines[1], "<span class=\"c\">two</span>");
    assert!(
        lines[2].starts_with("<span class=\"c\">three */</span> <span class=\"k\">let</span>"),
        "{}",
        lines[2]
    );
    assert!(
        lines[2].ends_with("<span class=\"s\">`a</span>"),
        "{}",
        lines[2]
    );
    assert_eq!(lines[3], "<span class=\"s\">b</span>");
    assert_eq!(lines[4], "<span class=\"s\">c`</span>;");
}

#[test]
fn a_template_string_substitution_is_not_string_coloured() {
    let html = render(SourceKind::JavaScript, "`a${b}c`");
    let line = line_markup(&html)[0];
    // The `${` and `}` and the expression are in the default colour; the string resumes after.
    assert_eq!(
        line,
        "<span class=\"s\">`a</span>${b}<span class=\"s\">c`</span>"
    );
}

#[test]
fn token_classes_never_nest() {
    // The highlighter's events nest (a string inside an embedded expression inside a string); the
    // page gets a flat list, so a span never contains another token span.
    let html = render(
        SourceKind::JavaScript,
        "const s = `a ${ `b ${ \"c\" } d` } e`; // `x`\n",
    );
    for line in line_markup(&html) {
        let mut depth = 0_i32;
        for part in line.split("<span class=\"").skip(1) {
            depth += 1;
            assert!(depth <= 1, "nested token spans in {line}");
            if part.contains("</span>") {
                depth -= 1;
            }
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Colours that are readable.
// ---------------------------------------------------------------------------------------------

fn style_of(html: &str) -> &str {
    let after = html.split_once("<style>").expect("style").1;
    after.split_once("</style>").expect("end of style").0
}

/// `#rrggbb` of `--name` in the first token block after `marker`.
fn token(css: &str, marker: &str, name: &str) -> [u8; 3] {
    let from = css.find(marker).expect("block");
    let block = &css[from..from + css[from..].find('}').expect("end of block")];
    let key = format!("--{name}:#");
    let start = block
        .find(&key)
        .unwrap_or_else(|| panic!("--{name} missing"))
        + key.len();
    let hex = &block[start..start + 6];
    let channel = |i: usize| u8::from_str_radix(&hex[i..i + 2], 16).expect("hex");
    [channel(0), channel(2), channel(4)]
}

fn luminance(rgb: [u8; 3]) -> f64 {
    let lin = |c: u8| {
        let c = f64::from(c) / 255.0;
        if c <= 0.04045 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * lin(rgb[0]) + 0.7152 * lin(rgb[1]) + 0.0722 * lin(rgb[2])
}

fn contrast(a: [u8; 3], b: [u8; 3]) -> f64 {
    let (a, b) = (luminance(a), luminance(b));
    (a.max(b) + 0.05) / (a.min(b) + 0.05)
}

#[test]
fn every_class_in_use_and_every_ui_colour_reaches_4_5_to_1_in_both_schemes() {
    let html = render(SourceKind::Html, EVERYTHING);
    let css = style_of(&html);
    let classes = classes_used(&html);
    assert_eq!(classes.len(), 10);
    for (scheme, marker) in [
        ("light", ":root{"),
        ("dark", "(prefers-color-scheme:dark){:root{"),
    ] {
        let background = token(css, marker, "bg");
        for class in &classes {
            // `.k{color:var(--keyword)}`
            let rule = format!(".{class}{{color:var(--");
            let after = css
                .split_once(&rule)
                .unwrap_or_else(|| panic!("no rule for .{class}"))
                .1;
            let name = after.split_once(')').expect("end of var").0;
            let ratio = contrast(token(css, marker, name), background);
            assert!(
                ratio >= 4.5,
                "{scheme}: .{class} (--{name}) is {ratio:.2}:1 on the page"
            );
        }
        for (text, on) in [
            ("fg", "bg"),
            ("muted", "bg"),
            ("muted", "surface"),
            ("link", "bg"),
            ("link", "surface"),
            ("error", "surface"),
            ("notice-fg", "notice-bg"),
        ] {
            let ratio = contrast(token(css, marker, text), token(css, marker, on));
            assert!(ratio >= 4.5, "{scheme}: --{text} on --{on} is {ratio:.2}:1");
        }
    }
    // The page declares both schemes and its own background.
    assert!(css.contains("@media (prefers-color-scheme:dark)"));
    assert!(css.contains("html{background:var(--bg)}"));
    assert!(html.contains("<meta name=\"color-scheme\" content=\"light dark\">"));
}

// ---------------------------------------------------------------------------------------------
// Bounds.
// ---------------------------------------------------------------------------------------------

#[test]
fn a_source_one_byte_over_the_highlight_limit_is_plain_at_once() {
    let text = format!("var a = '{}';", "x".repeat(MAX_HIGHLIGHT_BYTES));
    assert!(text.len() > MAX_HIGHLIGHT_BYTES);
    let started = Instant::now();
    let html = render(SourceKind::JavaScript, &text);
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "took {:?}",
        started.elapsed()
    );
    assert!(classes_used(&html).is_empty());
    assert!(
        html.contains(
            "Colours and links are not shown because the source is larger than 512.0 KiB."
        )
    );
}

#[test]
fn a_source_at_the_limit_is_highlighted() {
    let mut text = String::from("var a = 1;\n");
    while text.len() < MAX_HIGHLIGHT_BYTES - 16 {
        text.push_str("var b = 2;\n");
    }
    assert!(text.len() <= MAX_HIGHLIGHT_BYTES);
    let html = render(SourceKind::JavaScript, &text);
    assert!(has_token(&html, "k", "var"));
    assert!(!html.contains("Colours and links are not shown"));
}

#[test]
fn deeply_nested_markup_is_bounded_in_time_and_the_page_is_still_whole() {
    // The pathological input of the brief: nested <div>, repeated up to the limit.
    let text = "<div>".repeat(MAX_HIGHLIGHT_BYTES / 5);
    let started = Instant::now();
    let html = render(SourceKind::Html, &text);
    let took = started.elapsed();
    let coloured = classes_used(&html).contains("t");
    eprintln!(
        "nested divs ({} bytes): {took:?}, {}",
        text.len(),
        if coloured { "coloured" } else { "plain" }
    );
    // 2 s limit, 0.5 s grace, and the work around it, with room for a loaded machine.
    assert!(took < Duration::from_secs(8), "took {took:?}");
    let lines = line_markup(&html);
    assert_eq!(lines.len(), 1);
    assert!(
        coloured
            || html
                .contains("Colours and links are not shown because highlighting did not finish.")
    );
    assert_no_error_node(&html);
}

#[test]
fn hostile_inputs_do_not_break_the_page_or_the_process() {
    // (kind, input): each grammar with the inputs that are worst for it. Several run into the 2 s
    // limit, so the list is short.
    let samples: Vec<(SourceKind, String)> = vec![
        (SourceKind::Html, "<".repeat(100_000)),
        (SourceKind::Html, "<a href=\"".repeat(20_000)),
        (SourceKind::Html, "<!--".repeat(50_000)),
        (SourceKind::Html, "<script>".repeat(30_000)),
        (SourceKind::Html, "<a href=x>".repeat(30_000)),
        (SourceKind::Html, "<p>\r\n".repeat(50_000)),
        (SourceKind::Xml, "</div>".repeat(80_000)),
        (SourceKind::JavaScript, "{".repeat(100_000)),
        (SourceKind::JavaScript, "`${".repeat(30_000)),
        (SourceKind::JavaScript, "/*".repeat(100_000)),
        (SourceKind::JavaScript, "\"".repeat(200_000)),
        (SourceKind::Css, "(".repeat(100_000)),
        (SourceKind::Css, "/*".repeat(100_000)),
        (SourceKind::Css, "\u{0}".repeat(1000)),
        (SourceKind::Html, "é".repeat(200_000)),
    ];
    {
        for (kind, sample) in &samples {
            let kind = *kind;
            let started = Instant::now();
            let html = render(kind, sample);
            let took = started.elapsed();
            assert!(
                took < Duration::from_secs(10),
                "{kind:?} on {:?}: {took:?}",
                &sample[..sample.len().min(12)]
            );
            assert_no_error_node(&html);
            assert!(!html.contains("<script"), "{kind:?}");
        }
    }
}
