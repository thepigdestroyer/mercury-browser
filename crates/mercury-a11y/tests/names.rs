// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! Accessible names and descriptions: the table of cases in `data/names.txt`, the prohibited roles,
//! the limits, ten malformed-input cases, a seeded fuzz over random markup, and the reverse map
//! (`dependants`). Item 2 (`core-names`).

use a11y_dom::{Arena, Node, NodeId, NodeKind, descendants, elements};
use accesskit::Role;
use mercury_a11y::names::{self, DependantsIndex, MAX_NAME_DEPTH, MAX_TEXT_BYTES, dependants};
use mercury_a11y::roles::ARIA_ROLES;
use mercury_a11y::testing::TestDom;
use mercury_a11y::{NodeSemantics, Relations, compute};

const CASES: &str = include_str!("data/names.txt");

struct Case {
    id: String,
    html: String,
    target: String,
    name: Option<String>,
    /// `None`: the case does not check the description.
    description: Option<Option<String>>,
}

fn value(text: &str) -> Option<String> {
    (text != "<none>").then(|| text.to_string())
}

fn parse_cases(text: &str) -> Vec<Case> {
    let mut cases = Vec::new();
    for block in text.split("\n\n") {
        let mut id = None;
        let mut html = None;
        let mut target = "t".to_string();
        let mut name = None;
        let mut description = None;
        for line in block.lines().map(str::trim) {
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let (key, rest) = line
                .split_once(": ")
                .or_else(|| line.strip_suffix(':').map(|key| (key, "")))
                .unwrap_or_else(|| panic!("a case line is `key: value`: {line}"));
            match key {
                "case" => id = Some(rest.to_string()),
                "html" => html = Some(rest.to_string()),
                "target" => target = rest.to_string(),
                "name" => name = Some(value(rest)),
                "desc" => description = Some(value(rest)),
                "note" => {}
                other => panic!("unknown key {other} in a case of names.txt"),
            }
        }
        let Some(id) = id else { continue };
        cases.push(Case {
            html: html.unwrap_or_else(|| panic!("case {id} has no html")),
            name: name.unwrap_or_else(|| panic!("case {id} has no name")),
            id,
            target,
            description,
        });
    }
    cases
}

fn semantics(html: &str, target: &str) -> NodeSemantics {
    TestDom::from_html(html).semantics_of_id(target)
}

fn label_of(semantics: &NodeSemantics) -> Option<String> {
    semantics.node.label().map(str::to_string)
}

fn description_of(semantics: &NodeSemantics) -> Option<String> {
    semantics.node.description().map(str::to_string)
}

#[test]
fn every_case_of_the_name_table_matches() {
    let cases = parse_cases(CASES);
    assert!(cases.len() >= 100, "the table has {} cases", cases.len());
    let mut ids = std::collections::HashSet::new();
    let mut failures = Vec::new();
    for case in &cases {
        assert!(ids.insert(case.id.clone()), "case {} twice", case.id);
        let result = semantics(&case.html, &case.target);
        let name = label_of(&result);
        if name != case.name {
            failures.push(format!(
                "{}: {}\n    name: expected {:?}, got {:?}",
                case.id, case.html, case.name, name
            ));
        }
        if let Some(expected) = &case.description {
            let description = description_of(&result);
            if &description != expected {
                failures.push(format!(
                    "{}: {}\n    description: expected {:?}, got {:?}",
                    case.id, case.html, expected, description
                ));
            }
        }
    }
    assert!(failures.is_empty(), "\n{}", failures.join("\n"));
}

#[test]
fn a_prohibited_role_never_gets_a_label() {
    for (name, role) in ARIA_ROLES {
        if !accname::name_is_prohibited(name) {
            continue;
        }
        let result = semantics(
            &format!(
                "<div id=t role={name} aria-label=X aria-labelledby=l title=Y>text</div><i id=l>L</i>"
            ),
            "t",
        );
        assert_eq!(result.node.role(), *role, "role={name}");
        assert_eq!(label_of(&result), None, "role={name}");
        assert_eq!(description_of(&result).as_deref(), Some("Y"), "role={name}");
    }
}

#[test]
fn the_roles_that_take_a_name_from_the_author_get_it() {
    for (name, role) in ARIA_ROLES {
        if accname::name_is_prohibited(name) {
            continue;
        }
        let result = semantics(
            &format!("<div id=t role={name} aria-label=X>text</div>"),
            "t",
        );
        assert_eq!(result.node.role(), *role, "role={name}");
        assert_eq!(label_of(&result).as_deref(), Some("X"), "role={name}");
    }
}

#[test]
fn a_text_node_gets_no_name_and_no_description() {
    let dom = TestDom::from_html("<p title=x>hello</p>");
    let ids = dom.id_index();
    let text = descendants(dom.first("p"))
        .find(|node| node.kind() == NodeKind::Text)
        .expect("a text node");
    let mut semantics = NodeSemantics {
        node: accesskit::Node::new(Role::TextRun),
        relations: Relations::default(),
        hidden: false,
    };
    names::apply(&dom, text, &ids, &mut semantics);
    assert_eq!(semantics.node.label(), None);
    assert_eq!(semantics.node.description(), None);
}

#[test]
fn names_and_descriptions_are_cut_at_the_limit() {
    let long = "x".repeat(MAX_TEXT_BYTES + 5_000);
    let result = semantics(&format!("<button id=t>{long}</button>"), "t");
    assert_eq!(result.node.label().map(str::len), Some(MAX_TEXT_BYTES));

    let long = "é".repeat(MAX_TEXT_BYTES);
    let result = semantics(&format!("<button id=t title=\"{long}\">x</button>"), "t");
    let description = result.node.description().expect("a description");
    assert!(description.len() <= MAX_TEXT_BYTES);
    assert!(
        description.len() >= MAX_TEXT_BYTES - 1,
        "cut at the last whole character"
    );
    assert!(description.chars().all(|c| c == 'é'));

    let long = "w ".repeat(MAX_TEXT_BYTES);
    let result = semantics(
        &format!("<p id=h>{long}</p><button id=t aria-describedby=h>x</button>"),
        "t",
    );
    assert!(
        result
            .node
            .description()
            .is_some_and(|d| d.len() <= MAX_TEXT_BYTES)
    );
}

#[test]
fn a_label_changes_with_the_document_and_not_with_the_role_function() {
    // The same markup with and without the id index entry: the name follows the document.
    let named = semantics("<label for=t>A</label><input id=t>", "t");
    let unnamed = semantics("<input id=t>", "t");
    assert_eq!(label_of(&named).as_deref(), Some("A"));
    assert_eq!(label_of(&unnamed), None);
}

// ---------------------------------------------------------------------------------------------
// Malformed and hostile input: nothing panics, nothing loops, nothing overflows the stack.
// ---------------------------------------------------------------------------------------------

/// `<outer id=t attrs>` holding `depth` nested divs and a text at the bottom, in a `body`.
fn nested_inside(outer: &str, attrs: &[(&str, &str)], depth: usize) -> TestDom {
    let mut builder = Arena::builder().open("body").open(outer).attr("id", "t");
    for (name, value) in attrs {
        builder = builder.attr(name, value);
    }
    for _ in 0..depth {
        builder = builder.open("div");
    }
    builder = builder.text("deep");
    for _ in 0..depth {
        builder = builder.close();
    }
    TestDom(builder.close().close().build())
}

#[test]
fn malformed_1_a_ring_of_labelledby_ends() {
    // b0 -> b1 -> ... -> b99 -> b0, every one a button with its own text.
    let mut html = String::new();
    for index in 0..100 {
        html.push_str(&format!(
            "<button id=b{index} aria-labelledby=b{}>{index}</button>",
            (index + 1) % 100
        ));
    }
    let dom = TestDom::from_html(&html);
    for index in 0..100 {
        let result = dom.semantics_of_id(&format!("b{index}"));
        assert_eq!(
            label_of(&result),
            Some(((index + 1) % 100).to_string()),
            "b{index}"
        );
    }
}

#[test]
fn malformed_2_a_node_that_names_itself_uses_its_content() {
    let result = semantics(
        "<button id=t aria-labelledby=t aria-describedby=t>Self</button>",
        "t",
    );
    assert_eq!(label_of(&result).as_deref(), Some("Self"));
    assert_eq!(description_of(&result), None);
    let result = semantics("<label id=t for=t>x</label>", "t");
    assert_eq!(label_of(&result), None);
}

#[test]
fn malformed_3_ten_thousand_levels_inside_a_button_do_not_overflow_the_stack() {
    let dom = nested_inside("button", &[], 10_000);
    let result = dom.semantics_of_id("t");
    assert_eq!(result.node.role(), Role::Button);
    assert_eq!(label_of(&result), None, "too deep to read the content");

    let dom = nested_inside("button", &[("aria-label", "x"), ("title", "tip")], 10_000);
    let result = dom.semantics_of_id("t");
    assert_eq!(label_of(&result).as_deref(), Some("x"));
    assert_eq!(description_of(&result).as_deref(), Some("tip"));

    let dom = nested_inside("h2", &[("title", "heading title")], 10_000);
    assert_eq!(
        label_of(&dom.semantics_of_id("t")).as_deref(),
        Some("heading title")
    );
}

#[test]
fn malformed_4_the_depth_limit_is_a_limit_and_not_a_cliff_below_it() {
    let dom = nested_inside("a", &[("href", "x")], MAX_NAME_DEPTH - 2);
    assert_eq!(label_of(&dom.semantics_of_id("t")).as_deref(), Some("deep"));
    let dom = nested_inside("a", &[("href", "x")], MAX_NAME_DEPTH + 2);
    assert_eq!(label_of(&dom.semantics_of_id("t")), None);
}

#[test]
fn malformed_5_ten_thousand_levels_of_ancestors_above_a_button() {
    let mut builder = Arena::builder().open("body");
    for _ in 0..10_000 {
        builder = builder.open("div");
    }
    builder = builder.open("button").attr("id", "t").text("ok").close();
    for _ in 0..10_000 {
        builder = builder.close();
    }
    let dom = TestDom(builder.close().build());
    let result = dom.semantics_of_id("t");
    assert_eq!(result.node.role(), Role::Button);
    assert_eq!(label_of(&result).as_deref(), Some("ok"));
}

#[test]
fn malformed_6_ten_thousand_levels_in_an_aria_labelledby_target() {
    let mut builder = Arena::builder().open("body").open("div").attr("id", "l");
    for _ in 0..10_000 {
        builder = builder.open("span");
    }
    builder = builder.text("deep");
    for _ in 0..10_000 {
        builder = builder.close();
    }
    builder = builder
        .close()
        .open("input")
        .attr("id", "t")
        .attr("aria-labelledby", "l")
        .attr("aria-label", "own")
        .attr("title", "tip")
        .close();
    let dom = TestDom(builder.close().build());
    let result = dom.semantics_of_id("t");
    assert_eq!(label_of(&result).as_deref(), Some("own"));
    assert_eq!(description_of(&result).as_deref(), Some("tip"));
}

#[test]
fn malformed_7_a_huge_attribute_value() {
    let big_label = "a ".repeat(2_500_000);
    let big_title = "b ".repeat(2_500_000);
    let dom = TestDom(
        Arena::builder()
            .open("body")
            .open("button")
            .attr("id", "t")
            .attr("aria-label", &big_label)
            .attr("title", &big_title)
            .attr("aria-describedby", &big_title)
            .close()
            .close()
            .build(),
    );
    let result = dom.semantics_of_id("t");
    assert_eq!(result.node.label().map(str::len), Some(MAX_TEXT_BYTES));
    // The huge describedby names nothing, so the title is the description.
    assert_eq!(
        result.node.description().map(str::len),
        Some(MAX_TEXT_BYTES)
    );
}

#[test]
fn malformed_8_a_hundred_thousand_idrefs_to_one_target() {
    let refs = "l ".repeat(100_000);
    let dom = TestDom::from_html(&format!(
        "<span id=l>Label</span><button id=t aria-labelledby=\"{refs}\" aria-describedby=\"{refs}\"></button>"
    ));
    let result = dom.semantics_of_id("t");
    assert_eq!(label_of(&result).as_deref(), Some("Label"));
    assert_eq!(description_of(&result).as_deref(), Some("Label"));
}

#[test]
fn malformed_9_fifty_thousand_siblings_inside_a_link() {
    let mut builder = Arena::builder()
        .open("body")
        .open("a")
        .attr("id", "t")
        .attr("href", "x");
    for _ in 0..50_000 {
        builder = builder.open("span").text("x").close();
    }
    let dom = TestDom(builder.close().close().build());
    let result = dom.semantics_of_id("t");
    assert_eq!(result.node.label().map(str::len), Some(MAX_TEXT_BYTES));
}

#[test]
fn malformed_10_odd_attribute_values_and_duplicate_ids() {
    let html = "<div id=t role=\"\t\n \" aria-labelledby=\"\u{0}  \u{a0}\"><span id=x>1</span><span id=x>2</span>\
                <span id=\"\">3</span><label for=\"\">4</label><label for=x>5</label><input id=\"\" aria-labelledby=\"\"></div>\
                <button id=b aria-labelledby=x>b</button><button id=c aria-labelledby=\"\" aria-describedby=\" \">c</button>";
    let dom = TestDom::from_html(html);
    let ids = dom.id_index();
    for node in elements(&dom) {
        let result = compute(&dom, node, &ids);
        let _ = (result.node.label(), result.node.description());
    }
    // The first element with a duplicated id is the target.
    assert_eq!(label_of(&dom.semantics_of_id("b")).as_deref(), Some("1"));
    assert_eq!(label_of(&dom.semantics_of_id("c")).as_deref(), Some("c"));
}

#[test]
fn malformed_11_every_element_of_a_page_with_many_interlinked_references() {
    // 400 controls, each labelled by the next one's label and described by the one before it.
    let mut html = String::new();
    for index in 0..400 {
        html.push_str(&format!(
            "<label id=l{index} for=i{index}>Label {index}</label>\
             <input id=i{index} aria-labelledby=\"l{} l{index}\" aria-describedby=l{}>",
            (index + 1) % 400,
            (index + 399) % 400
        ));
    }
    let dom = TestDom::from_html(&html);
    let ids = dom.id_index();
    let mut named = 0;
    for node in elements(&dom) {
        let result = compute(&dom, node, &ids);
        named += usize::from(result.node.label().is_some());
    }
    assert!(named >= 400, "{named} named");
    let result = dom.semantics_of_id("i7");
    assert_eq!(label_of(&result).as_deref(), Some("Label 8 Label 7"));
    assert_eq!(description_of(&result).as_deref(), Some("Label 6"));
}

/// A tiny deterministic generator (a linear congruential one) so that the fuzz is reproducible.
struct Lcg(u64);

impl Lcg {
    fn next(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        self.0 >> 33
    }

    fn below(&mut self, bound: usize) -> usize {
        (self.next() % bound as u64) as usize
    }

    fn pick<'a>(&mut self, pool: &[&'a str]) -> &'a str {
        pool[self.below(pool.len())]
    }
}

fn random_element(rng: &mut Lcg, depth: usize, out: &mut String) {
    const TAGS: &[&str] = &[
        "div",
        "span",
        "a",
        "button",
        "input",
        "label",
        "select",
        "option",
        "table",
        "tr",
        "td",
        "th",
        "ul",
        "li",
        "h1",
        "p",
        "img",
        "svg",
        "title",
        "legend",
        "fieldset",
        "figure",
        "figcaption",
        "details",
        "summary",
        "textarea",
        "output",
        "section",
        "header",
        "nav",
        "form",
        "dl",
        "dt",
        "dd",
        "iframe",
        "abbr",
        "caption",
        "progress",
    ];
    const IDS: &[&str] = &["a", "b", "c", "d", "e", "f"];
    const ROLES: &[&str] = &[
        "button",
        "none",
        "presentation",
        "img",
        "image",
        "heading",
        "textbox",
        "combobox",
        "foo",
        "doc-toc",
        "listitem",
        "region",
        "main",
        "generic",
        "tab",
    ];
    const TYPES: &[&str] = &[
        "text", "password", "checkbox", "image", "submit", "hidden", "number",
    ];
    const STYLES: &[&str] = &[
        "display:none",
        "visibility:hidden",
        "display:block",
        "x:y;;",
    ];
    const WORDS: &[&str] = &[
        "one",
        "two words",
        "",
        " ",
        "x&amp;y",
        "naïve",
        "&nbsp;",
        "<!-- c -->",
    ];
    let tag = rng.pick(TAGS);
    out.push('<');
    out.push_str(tag);
    for _ in 0..rng.below(5) {
        match rng.below(14) {
            0 => out.push_str(&format!(" id={}", rng.pick(IDS))),
            1 => out.push_str(&format!(" for={}", rng.pick(IDS))),
            2 => out.push_str(&format!(
                " aria-labelledby=\"{} {}\"",
                rng.pick(IDS),
                rng.pick(IDS)
            )),
            3 => out.push_str(&format!(" aria-describedby={}", rng.pick(IDS))),
            4 => out.push_str(&format!(
                " aria-label=\"{}\"",
                rng.pick(WORDS).replace('"', "")
            )),
            5 => out.push_str(&format!(" role=\"{}\"", rng.pick(ROLES))),
            6 => out.push_str(" hidden"),
            7 => out.push_str(&format!(" style=\"{}\"", rng.pick(STYLES))),
            8 => out.push_str(&format!(" type={}", rng.pick(TYPES))),
            9 => out.push_str(" href=x"),
            10 => out.push_str(&format!(" alt=\"{}\"", rng.pick(WORDS).replace('"', ""))),
            11 => out.push_str(&format!(" title=\"{}\"", rng.pick(WORDS).replace('"', ""))),
            12 => out.push_str(&format!(" placeholder={}", rng.pick(IDS))),
            _ => out.push_str(" tabindex=0 contenteditable"),
        }
    }
    out.push('>');
    if depth > 0 {
        for _ in 0..rng.below(4) {
            if rng.below(3) == 0 {
                out.push_str(rng.pick(WORDS));
            } else {
                random_element(rng, depth - 1, out);
            }
        }
    }
    out.push_str("</");
    out.push_str(tag);
    out.push('>');
}

#[test]
fn a_seeded_fuzz_over_random_markup_never_panics() {
    let mut rng = Lcg(0x004d_6572_6375_7279);
    let mut computed = 0usize;
    for _ in 0..400 {
        let mut html = String::new();
        for _ in 0..1 + rng.below(3) {
            random_element(&mut rng, 5, &mut html);
        }
        let dom = TestDom::from_html(&html);
        let ids = dom.id_index();
        let index = DependantsIndex::build(&dom, &ids);
        for node in elements(&dom) {
            let result = compute(&dom, node, &ids);
            assert!(
                result
                    .node
                    .label()
                    .is_none_or(|l| l.len() <= MAX_TEXT_BYTES)
            );
            let _ = index.dependants(node, &ids);
            computed += 1;
        }
    }
    assert!(computed > 2_000, "{computed} nodes computed");
}

#[test]
fn garbage_that_is_not_html_does_not_panic_the_parser_or_the_names() {
    for html in [
        "",
        "<",
        "<>",
        "</>",
        "<<<>>>",
        "&&&;;;&#;&#x;&#xZ;&#99999999999;",
        "<a b='c",
        "<a b=\"c",
        "<a =>",
        "<a / / />",
        "text only",
        "<!-- unterminated",
        "<![CDATA[x]]>",
        "<input type=password value=",
        "<p id=t title='\u{0}'>\u{1}</p>",
    ] {
        let dom = TestDom::from_html(html);
        let ids = dom.id_index();
        for node in elements(&dom) {
            let _ = compute(&dom, node, &ids);
        }
    }
}

// ---------------------------------------------------------------------------------------------
// dependants: the reverse map item 4 uses to damage the nodes whose name depends on another.
// ---------------------------------------------------------------------------------------------

fn element_ids(dom: &TestDom, list: Vec<NodeId>) -> Vec<String> {
    list.into_iter()
        .map(|id| {
            dom.0
                .get(id)
                .and_then(|node| node.attr("id"))
                .unwrap_or("?")
                .to_string()
        })
        .collect()
}

fn dependants_of(html: &str, id: &str) -> Vec<String> {
    let dom = TestDom::from_html(html);
    let ids = dom.id_index();
    element_ids(&dom, dependants(&dom, dom.by_id(id), &ids))
}

fn dependants_of_text_in(html: &str, id: &str) -> Vec<String> {
    let dom = TestDom::from_html(html);
    let ids = dom.id_index();
    let text = descendants(dom.by_id(id))
        .find(|node| node.kind() == NodeKind::Text)
        .expect("a text node");
    element_ids(&dom, dependants(&dom, text, &ids))
}

#[test]
fn dependants_of_an_aria_labelledby_target() {
    let html = "<span id=l>Lbl</span><button id=t aria-labelledby=l></button>";
    assert_eq!(dependants_of(html, "l"), ["t"]);
    assert_eq!(dependants_of_text_in(html, "l"), ["t"]);
    assert!(dependants_of(html, "t").is_empty(), "never the node itself");
}

#[test]
fn dependants_of_an_aria_describedby_target() {
    let html = "<p id=h>Hint</p><input id=t aria-describedby=h>";
    assert_eq!(dependants_of(html, "h"), ["t"]);
    assert_eq!(dependants_of_text_in(html, "h"), ["t"]);
}

#[test]
fn dependants_include_the_referrers_of_an_ancestor() {
    let html = "<div id=box><span id=s>x</span></div><button id=t aria-labelledby=box></button>";
    assert_eq!(dependants_of(html, "s"), ["t"]);
    assert_eq!(dependants_of_text_in(html, "s"), ["t"]);
}

#[test]
fn dependants_of_a_label() {
    let html = "<label id=l for=t>Name</label><input id=t>";
    assert_eq!(dependants_of(html, "l"), ["t"]);
    assert_eq!(dependants_of_text_in(html, "l"), ["t"]);
    let html = "<label id=l>Name <input id=t></label>";
    assert_eq!(dependants_of(html, "l"), ["t"]);
    assert_eq!(dependants_of_text_in(html, "l"), ["t"]);
    // The control inside its own label does not depend on itself.
    assert!(dependants_of(html, "t").is_empty());
    // A label for something that cannot be labelled names nothing.
    assert!(dependants_of("<label id=l for=d>x</label><div id=d></div>", "l").is_empty());
}

#[test]
fn dependants_of_the_native_text_alternatives() {
    assert_eq!(
        dependants_of("<fieldset id=f><legend id=l>L</legend></fieldset>", "l"),
        ["f"]
    );
    assert_eq!(
        dependants_of("<figure id=f><figcaption id=c>C</figcaption></figure>", "c"),
        ["f"]
    );
    assert_eq!(
        dependants_of("<table id=f><caption id=c>C</caption></table>", "c"),
        ["f"]
    );
    assert_eq!(
        dependants_of("<svg id=f><title id=c>C</title></svg>", "c"),
        ["f"]
    );
}

#[test]
fn dependants_include_the_ancestors_that_name_themselves_from_content() {
    let html = "<div id=d><a id=a href=x><span id=s>text</span></a></div>";
    assert_eq!(dependants_of(html, "s"), ["a"]);
    assert_eq!(dependants_of_text_in(html, "s"), ["a"]);
    assert!(dependants_of(html, "a").is_empty());
    let html = "<h2 id=h>Title <em id=e>one</em></h2>";
    assert_eq!(dependants_of(html, "e"), ["h"]);
}

#[test]
fn dependants_are_sorted_and_without_duplicates() {
    let html = "<span id=l>x</span><button id=b2 aria-labelledby=l aria-describedby=l></button>\
                <button id=b1 aria-describedby=l></button>";
    assert_eq!(dependants_of(html, "l"), ["b2", "b1"]);
}

#[test]
fn dependants_survive_a_cycle() {
    let html = "<button id=a aria-labelledby=b>a</button><button id=b aria-labelledby=a>b</button>";
    assert_eq!(dependants_of(html, "a"), ["b"]);
    assert_eq!(dependants_of(html, "b"), ["a"]);
}

#[test]
fn the_index_answers_what_the_function_answers() {
    let dom = TestDom::from_html(
        "<label id=l for=i>L</label><input id=i aria-describedby=h><p id=h>H <a id=a href=x>A</a></p>\
         <fieldset id=f><legend id=g>G</legend><button aria-labelledby=g id=b>b</button></fieldset>",
    );
    let ids = dom.id_index();
    let index = DependantsIndex::build(&dom, &ids);
    for node in elements(&dom) {
        assert_eq!(
            index.dependants(node, &ids),
            dependants(&dom, node, &ids),
            "{:?}",
            node.attr("id")
        );
    }
    assert!(
        DependantsIndex::default()
            .dependants(dom.first("p"), &ids)
            .is_empty()
    );
}
