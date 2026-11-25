// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! Roles: the ARIA table against an independent record of the W3C tables, the element and context
//! rows in `data/roles.txt`, and the rules between them (presentational conflict, token lists,
//! inheritance, editing hosts). Item 2 (`core-names`).

use a11y_dom::{Document, Node, NodeKind, elements, self_and_descendants};
use accesskit::Role;
use mercury_a11y::roles::{
    self, ARIA_ROLES, aria_name_of, aria_role, name_prohibited, role_for_aria,
};
use mercury_a11y::testing::TestDom;

const ARIA_TABLE: &str = include_str!("data/aria-roles.txt");
const ROWS: &str = include_str!("data/roles.txt");

/// The data lines of a table file: number, trimmed text, comments and blank lines left out.
fn data_lines(text: &str) -> impl Iterator<Item = (usize, &str)> {
    text.lines()
        .enumerate()
        .map(|(index, line)| (index + 1, line.trim()))
        .filter(|(_, line)| !line.is_empty() && !line.starts_with('#'))
}

fn role_of(html: &str) -> Role {
    let dom = TestDom::from_html(html);
    roles::role(&dom, dom.by_id("t"))
}

fn aria_role_of(html: &str) -> Option<&'static str> {
    let dom = TestDom::from_html(html);
    aria_role(&dom, dom.by_id("t"))
}

/// Split `a => b` at the last arrow.
fn split_row(line: &str) -> (&str, &str) {
    let (left, right) = line
        .rsplit_once(" => ")
        .unwrap_or_else(|| panic!("a table row needs ` => `: {line}"));
    (left.trim(), right.trim())
}

#[test]
fn every_row_of_the_element_and_context_table_matches() {
    let mut failures = Vec::new();
    let mut rows = 0;
    for (number, line) in data_lines(ROWS) {
        let (html, expected) = split_row(line);
        rows += 1;
        let actual = format!("{:?}", role_of(html));
        if actual != expected {
            failures.push(format!(
                "roles.txt:{number}: {html}\n    expected {expected}, got {actual}"
            ));
        }
    }
    assert!(rows >= 300, "the table has {rows} rows");
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn the_aria_table_equals_the_independent_record() {
    let mut recorded = Vec::new();
    for (number, line) in data_lines(ARIA_TABLE) {
        let (name, expected) = split_row(line);
        let actual = role_for_aria(name).map(|role| format!("{role:?}"));
        assert_eq!(
            actual.as_deref(),
            Some(expected),
            "aria-roles.txt:{number}: role {name}"
        );
        recorded.push(name);
    }
    assert_eq!(recorded.len(), ARIA_ROLES.len(), "one entry per role");
    for (name, role) in ARIA_ROLES {
        assert!(
            recorded.contains(name),
            "the role {name} ({role:?}) is in src/roles.rs but not in aria-roles.txt"
        );
    }
}

#[test]
fn the_table_has_the_expected_size() {
    // ARIA 1.2 concrete roles (83), `image`, DPub (41), Graphics (3).
    assert_eq!(ARIA_ROLES.len(), 83 + 1 + 41 + 3);
    let names: std::collections::HashSet<_> = ARIA_ROLES.iter().map(|(name, _)| name).collect();
    assert_eq!(names.len(), ARIA_ROLES.len(), "no name twice");
}

#[test]
fn an_explicit_role_on_a_div_maps_as_the_table_says() {
    for (name, expected) in ARIA_ROLES {
        let actual = role_of(&format!("<div id=t role={name}></div>"));
        assert_eq!(actual, *expected, "role={name}");
    }
}

#[test]
fn the_table_has_an_inverse_where_the_role_has_one_name() {
    for (name, role) in ARIA_ROLES {
        let canonical = aria_name_of(*role).unwrap_or_else(|| panic!("{role:?} has no name"));
        assert_eq!(
            role_for_aria(canonical),
            Some(*role),
            "the canonical name of {role:?} maps back"
        );
        let names_of_role = ARIA_ROLES.iter().filter(|(_, r)| r == role).count();
        if names_of_role == 1 {
            assert_eq!(canonical, *name, "{role:?} has one name");
        }
    }
    // The roles with more than one name, and which one is canonical.
    assert_eq!(aria_name_of(Role::GenericContainer), Some("generic"));
    assert_eq!(aria_name_of(Role::Image), Some("img"));
    assert_eq!(aria_name_of(Role::List), Some("list"));
    assert_eq!(aria_name_of(Role::Splitter), Some("separator"));
    // Roles with no ARIA name.
    for role in [
        Role::Unknown,
        Role::TextRun,
        Role::Label,
        Role::PasswordInput,
        Role::DateInput,
        Role::RootWebArea,
        Role::Iframe,
    ] {
        assert_eq!(aria_name_of(role), None, "{role:?}");
    }
}

#[test]
fn abstract_roles_and_misspellings_are_not_roles() {
    for name in [
        "command",
        "composite",
        "input",
        "landmark",
        "range",
        "roletype",
        "section",
        "sectionhead",
        "select",
        "structure",
        "widget",
        "window",
        "Button",
        "buton",
        "doc-",
        "",
    ] {
        assert_eq!(role_for_aria(name), None, "{name:?}");
    }
}

#[test]
fn the_table_agrees_with_the_accname_crate_on_the_aria_1_2_roles() {
    // accname decides prohibited names and name from content from its own role; the two must agree
    // on every role both know, so that a name is never computed for a role this crate hides.
    let accname_roles = [
        "alert",
        "alertdialog",
        "application",
        "article",
        "banner",
        "blockquote",
        "button",
        "caption",
        "cell",
        "checkbox",
        "code",
        "columnheader",
        "combobox",
        "complementary",
        "contentinfo",
        "definition",
        "deletion",
        "dialog",
        "directory",
        "document",
        "emphasis",
        "feed",
        "figure",
        "form",
        "generic",
        "grid",
        "gridcell",
        "group",
        "heading",
        "img",
        "insertion",
        "link",
        "list",
        "listbox",
        "listitem",
        "log",
        "main",
        "mark",
        "marquee",
        "math",
        "menu",
        "menubar",
        "menuitem",
        "menuitemcheckbox",
        "menuitemradio",
        "meter",
        "navigation",
        "none",
        "note",
        "option",
        "paragraph",
        "presentation",
        "progressbar",
        "radio",
        "radiogroup",
        "region",
        "row",
        "rowgroup",
        "rowheader",
        "scrollbar",
        "search",
        "searchbox",
        "separator",
        "slider",
        "spinbutton",
        "status",
        "strong",
        "subscript",
        "superscript",
        "switch",
        "tab",
        "table",
        "tablist",
        "tabpanel",
        "term",
        "textbox",
        "time",
        "timer",
        "toolbar",
        "tooltip",
        "tree",
        "treegrid",
        "treeitem",
    ];
    assert_eq!(accname_roles.len(), 83);
    for name in accname_roles {
        let dom = TestDom::from_html(&format!("<div id=t role={name}></div>"));
        let node = dom.by_id("t");
        assert_eq!(
            accname::role(node),
            Some(name),
            "accname role of role={name}"
        );
        assert_eq!(
            aria_role(&dom, node),
            Some(name),
            "aria_role of role={name}"
        );
        let role = role_for_aria(name).expect("in the table");
        assert_eq!(
            name_prohibited(role),
            accname::name_is_prohibited(name),
            "name prohibition of {name}"
        );
    }
    // The roles `accname` does not know: it takes the element for a generic div.
    for name in [
        "image",
        "doc-chapter",
        "doc-toc",
        "graphics-document",
        "graphics-symbol",
    ] {
        let dom = TestDom::from_html(&format!("<div id=t role={name}></div>"));
        assert_eq!(accname::role(dom.by_id("t")), Some("generic"), "{name}");
        assert_eq!(aria_role(&dom, dom.by_id("t")), Some(name), "{name}");
    }
}

#[test]
fn every_role_name_accname_answers_for_an_element_is_in_the_table() {
    let mut seen = std::collections::BTreeSet::new();
    for (_, line) in data_lines(ROWS) {
        let (html, _) = split_row(line);
        let dom = TestDom::from_html(html);
        for node in elements(&dom) {
            if let Some(name) = accname::implicit(node) {
                seen.insert(name);
                assert!(
                    role_for_aria(name).is_some(),
                    "accname::implicit answers {name} for {html}, which is not in the table"
                );
            }
        }
    }
    assert!(
        seen.len() > 40,
        "the rows exercise {} implicit roles",
        seen.len()
    );
}

#[test]
fn name_prohibition_is_the_accname_list() {
    let prohibited: Vec<Role> = ARIA_ROLES
        .iter()
        .filter(|(name, _)| accname::name_is_prohibited(name))
        .map(|(_, role)| *role)
        .collect();
    for role in [
        Role::GenericContainer,
        Role::Paragraph,
        Role::Caption,
        Role::Code,
        Role::ContentDeletion,
        Role::ContentInsertion,
        Role::Emphasis,
        Role::Strong,
        Role::Term,
        Role::Time,
    ] {
        assert!(name_prohibited(role), "{role:?}");
        assert!(prohibited.contains(&role), "{role:?}");
    }
    for role in [
        Role::Button,
        Role::Link,
        Role::Heading,
        Role::Image,
        Role::List,
        Role::Group,
        Role::Region,
        Role::TextInput,
        Role::DocChapter,
        Role::Unknown,
        Role::TextRun,
    ] {
        assert!(!name_prohibited(role), "{role:?}");
    }
}

#[test]
fn a_text_node_is_a_text_run_and_has_no_aria_role() {
    let dom = TestDom::from_html("<p>hello <b>world</b></p>");
    let texts: Vec<_> = self_and_descendants(dom.root())
        .filter(|node| node.kind() == NodeKind::Text)
        .collect();
    assert_eq!(texts.len(), 2);
    for text in texts {
        assert_eq!(roles::role(&dom, text), Role::TextRun);
        assert_eq!(aria_role(&dom, text), None);
    }
}

#[test]
fn aria_role_answers_the_name_for_aria_roles_and_nothing_for_the_rest() {
    assert_eq!(aria_role_of("<div id=t role=button></div>"), Some("button"));
    assert_eq!(aria_role_of("<button id=t></button>"), Some("button"));
    assert_eq!(aria_role_of("<a id=t href=x></a>"), Some("link"));
    assert_eq!(aria_role_of("<div id=t></div>"), Some("generic"));
    assert_eq!(aria_role_of("<abbr id=t></abbr>"), Some("generic"));
    assert_eq!(aria_role_of("<img id=t alt=''>"), Some("presentation"));
    assert_eq!(aria_role_of("<input id=t type=text>"), Some("textbox"));
    assert_eq!(aria_role_of("<input id=t type=number>"), Some("spinbutton"));
    assert_eq!(aria_role_of("<input id=t type=password>"), None);
    assert_eq!(aria_role_of("<input id=t type=email>"), None);
    assert_eq!(aria_role_of("<iframe id=t></iframe>"), None);
    assert_eq!(aria_role_of("<dl id=t></dl>"), None);
    assert_eq!(aria_role_of("<div id=t role=none></div>"), Some("none"));
    assert_eq!(
        aria_role_of("<h1 id=t role=none aria-label=x></h1>"),
        Some("heading")
    );
}

#[test]
fn a_password_is_a_password_input_and_leaks_nothing_through_compute() {
    let dom = TestDom::from_html(
        "<form aria-label=Login><input id=t type=password value=hunter2 data-test-value=s3cret \
         placeholder=Secret></form>",
    );
    let semantics = dom.semantics_of_id("t");
    assert_eq!(semantics.node.role(), Role::PasswordInput);
    assert_eq!(semantics.node.value(), None);
    assert_eq!(semantics.node.numeric_value(), None);
    let everything = format!("{semantics:?}");
    for secret in ["hunter2", "s3cret"] {
        assert!(
            !everything.contains(secret),
            "the password {secret} reached a node property: {everything}"
        );
    }
}

#[test]
fn compute_gives_the_role_of_the_role_function() {
    let dom = TestDom::from_html(
        "<nav aria-label=Main><a href=x>Home</a><input type=email><ul><li>a</li></ul></nav>",
    );
    let ids = dom.id_index();
    for node in elements(&dom) {
        let semantics = mercury_a11y::compute(&dom, node, &ids);
        assert_eq!(
            semantics.node.role(),
            roles::role(&dom, node),
            "{}",
            node.local_name()
        );
    }
}

#[test]
fn a_deep_ancestor_chain_does_not_slow_or_break_the_role() {
    // 3,000 nested divs, the role of the innermost header and of a `th` at the bottom.
    let mut builder = a11y_dom::Arena::builder().open("body");
    for _ in 0..3000 {
        builder = builder.open("div");
    }
    builder = builder.open("header").attr("id", "t").close();
    for _ in 0..3000 {
        builder = builder.close();
    }
    let dom = TestDom(builder.close().build());
    assert_eq!(roles::role(&dom, dom.by_id("t")), Role::Banner);
}

#[test]
fn an_unknown_element_and_every_tag_name_is_a_role_without_panic() {
    // Tag names that are not HTML: custom elements, SVG and MathML names, odd characters.
    for tag in [
        "x-foo",
        "foreignObject",
        "annotation-xml",
        "my:tag",
        "a1",
        "h7",
        "INPUT",
        "Button",
    ] {
        let dom = TestDom::from_html(&format!("<{tag} id=t></{tag}>"));
        let _ = roles::role(&dom, dom.by_id("t"));
    }
}
