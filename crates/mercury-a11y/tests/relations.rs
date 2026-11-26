// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! Relations between nodes (`aria-controls`, `aria-labelledby`, `aria-owns`, ...), checked through
//! `compute` on `TestDom`s written as HTML. Test names start with `relations_`.

use a11y_dom::{Node, NodeId};
use mercury_a11y::relations::MAX_RELATED;
use mercury_a11y::testing::TestDom;
use mercury_a11y::{NodeSemantics, Relations};

/// The DOM ids of the elements with these `id` attributes, in the order given.
fn ids(dom: &TestDom, names: &[&str]) -> Vec<NodeId> {
    names.iter().map(|name| dom.by_id(name).id()).collect()
}

/// The relations of the element `x` of this fragment, with the DOM for resolving ids.
fn relations_of(html: &str) -> (TestDom, Relations) {
    let dom = TestDom::from_html(html);
    let relations = dom.semantics_of_id("x").relations;
    (dom, relations)
}

#[test]
fn relations_none_without_attributes() {
    let (_, relations) = relations_of(r#"<div id=x>t</div><p id=a>a</p>"#);
    assert_eq!(relations, Relations::default());
}

#[test]
fn relations_idref_lists_resolve_in_the_order_written() {
    let html = r#"
        <div id=x aria-controls="c b a" aria-describedby="b a" aria-details=a aria-flowto="b c"
             aria-labelledby="c a"></div>
        <p id=a>a</p><p id=b>b</p><p id=c>c</p>"#;
    let (dom, relations) = relations_of(html);
    assert_eq!(relations.controls, ids(&dom, &["c", "b", "a"]));
    assert_eq!(relations.described_by, ids(&dom, &["b", "a"]));
    assert_eq!(relations.details, ids(&dom, &["a"]));
    assert_eq!(relations.flow_to, ids(&dom, &["b", "c"]));
    assert_eq!(relations.labelled_by, ids(&dom, &["c", "a"]));
}

#[test]
fn relations_unresolved_duplicate_and_self_ids_are_dropped() {
    let html = r#"
        <div id=x aria-controls="missing a a  x   nothing b"
             aria-describedby="a x a"></div><p id=a>a</p><p id=b>b</p>"#;
    let (dom, relations) = relations_of(html);
    assert_eq!(relations.controls, ids(&dom, &["a", "b"]));
    assert_eq!(relations.described_by, ids(&dom, &["a"]));
    // An attribute that names nothing is an empty relation.
    let (_, relations) = relations_of(r#"<div id=x aria-flowto="nope also-nope"></div>"#);
    assert!(relations.flow_to.is_empty());
    let (_, relations) = relations_of(r#"<div id=x aria-flowto=""></div>"#);
    assert!(relations.flow_to.is_empty());
}

#[test]
fn relations_id_list_is_bounded() {
    let mut html = String::from(r#"<div id=x aria-controls=""#);
    for index in 0..(MAX_RELATED + 40) {
        html.push_str(&format!("t{index} "));
    }
    html.push_str(r#""></div>"#);
    for index in 0..(MAX_RELATED + 40) {
        html.push_str(&format!(r#"<i id=t{index}></i>"#));
    }
    let (dom, relations) = relations_of(&html);
    assert_eq!(relations.controls.len(), MAX_RELATED);
    assert_eq!(relations.controls[0], dom.by_id("t0").id());
}

#[test]
fn relations_active_descendant_is_the_first_id() {
    let html =
        r#"<div id=x role=listbox aria-activedescendant="b a"><i id=a></i><i id=b></i></div>"#;
    let (dom, relations) = relations_of(html);
    assert_eq!(relations.active_descendant, Some(dom.by_id("b").id()));
    let (_, relations) = relations_of(r#"<div id=x aria-activedescendant=gone></div>"#);
    assert_eq!(relations.active_descendant, None);
    let (_, relations) = relations_of(r#"<div id=x aria-activedescendant=x></div>"#);
    assert_eq!(relations.active_descendant, None);
    // A combobox points into a popup it only controls.
    let html = r#"<input id=x role=combobox aria-controls=pop aria-activedescendant=opt><ul id=pop><li id=opt>a</li></ul>"#;
    let (dom, relations) = relations_of(html);
    assert_eq!(relations.active_descendant, Some(dom.by_id("opt").id()));
}

#[test]
fn relations_error_message_only_on_an_invalid_node() {
    let html = r#"<input id=x aria-invalid=true aria-errormessage="e1 e2"><p id=e1>bad</p><p id=e2>worse</p>"#;
    let (dom, relations) = relations_of(html);
    assert_eq!(relations.error_message, ids(&dom, &["e1"]));
    // Not invalid: the message does not apply.
    let html = r#"<input id=x aria-errormessage=e1><p id=e1>bad</p>"#;
    let (_, relations) = relations_of(html);
    assert!(relations.error_message.is_empty());
    let html = r#"<input id=x aria-invalid=false aria-errormessage=e1><p id=e1>bad</p>"#;
    let (_, relations) = relations_of(html);
    assert!(relations.error_message.is_empty());
    // Invalid through the native `:invalid`.
    let html = r#"<input id=x required aria-errormessage=e1><p id=e1>bad</p>"#;
    let (dom, relations) = relations_of(html);
    assert_eq!(relations.error_message, ids(&dom, &["e1"]));
    // A message that is not in the document.
    let html = r#"<input id=x aria-invalid=true aria-errormessage=nowhere>"#;
    let (_, relations) = relations_of(html);
    assert!(relations.error_message.is_empty());
}

// ---------------------------------------------------------------------------------------------
// labelled by
// ---------------------------------------------------------------------------------------------

#[test]
fn relations_labelled_by_native_label_for() {
    let html = r#"<label id=l1 for=x>One</label><input id=x><label id=l2 for=x>Two</label>"#;
    let (dom, relations) = relations_of(html);
    assert_eq!(relations.labelled_by, ids(&dom, &["l1", "l2"]));
}

#[test]
fn relations_labelled_by_wrapping_label() {
    let html = r#"<label id=l>Name <input id=x></label>"#;
    let (dom, relations) = relations_of(html);
    assert_eq!(relations.labelled_by, ids(&dom, &["l"]));
    // The first labelable descendant is the control; a second one is not labelled.
    let html = r#"<label id=l><input id=first> and <input id=x></label>"#;
    let (_, relations) = relations_of(html);
    assert!(relations.labelled_by.is_empty());
    // A label with `for` labels what it names, not what it wraps.
    let html = r#"<label id=l for=other>Name <input id=x></label><input id=other>"#;
    let (_, relations) = relations_of(html);
    assert!(relations.labelled_by.is_empty());
    // A label for and a wrapper together.
    let html = r#"<label id=a for=x>A</label><label id=b><input id=x></label>"#;
    let (dom, relations) = relations_of(html);
    assert_eq!(relations.labelled_by, ids(&dom, &["a", "b"]));
}

#[test]
fn relations_labelled_by_aria_wins_over_native_labels() {
    let html =
        r#"<label id=l for=x>native</label><input id=x aria-labelledby=t><span id=t>aria</span>"#;
    let (dom, relations) = relations_of(html);
    assert_eq!(relations.labelled_by, ids(&dom, &["t"]));
    // An aria-labelledby that names nothing leaves the native labels.
    let html = r#"<label id=l for=x>native</label><input id=x aria-labelledby=nothing>"#;
    let (dom, relations) = relations_of(html);
    assert_eq!(relations.labelled_by, ids(&dom, &["l"]));
}

#[test]
fn relations_labelled_by_only_for_labelable_elements() {
    // A `div` with an id a label points at is not labelled by it.
    let html = r#"<label id=l for=x>L</label><div id=x></div>"#;
    let (_, relations) = relations_of(html);
    assert!(relations.labelled_by.is_empty());
    // A hidden input is not labelable.
    let html = r#"<label id=l for=x>L</label><input id=x type=hidden>"#;
    let (_, relations) = relations_of(html);
    assert!(relations.labelled_by.is_empty());
    // Others are: button, select, textarea, meter, progress, output.
    for tag in [
        "button", "select", "textarea", "meter", "progress", "output",
    ] {
        let html = format!(r#"<label id=l for=x>L</label><{tag} id=x></{tag}>"#);
        let (dom, relations) = relations_of(&html);
        assert_eq!(relations.labelled_by, ids(&dom, &["l"]), "{tag}");
    }
}

#[test]
fn relations_labelled_by_captions() {
    let (dom, relations) =
        relations_of(r#"<fieldset id=x><p>a</p><legend id=l>Group</legend></fieldset>"#);
    assert_eq!(relations.labelled_by, ids(&dom, &["l"]));
    let (dom, relations) =
        relations_of(r#"<table id=x><caption id=c>Sales</caption><tr><td>1</td></tr></table>"#);
    assert_eq!(relations.labelled_by, ids(&dom, &["c"]));
    let (dom, relations) = relations_of(
        r#"<figure id=x><img alt=a src=a.png><figcaption id=f>Cap</figcaption></figure>"#,
    );
    assert_eq!(relations.labelled_by, ids(&dom, &["f"]));
    // A fieldset with no legend has no label.
    let (_, relations) = relations_of(r#"<fieldset id=x><p>a</p></fieldset>"#);
    assert!(relations.labelled_by.is_empty());
}

// ---------------------------------------------------------------------------------------------
// owns
// ---------------------------------------------------------------------------------------------

#[test]
fn relations_owns_lists_the_owned_nodes() {
    let html = r#"<div id=x aria-owns="b a"></div><p id=a>a</p><p id=b>b</p>"#;
    let (dom, relations) = relations_of(html);
    assert_eq!(relations.owns, ids(&dom, &["b", "a"]));
}

#[test]
fn relations_owns_drops_self_ancestors_and_unresolved_ids() {
    let html =
        r#"<section id=top><div id=x aria-owns="x top missing mid"><i id=mid></i></div></section>"#;
    let (dom, relations) = relations_of(html);
    // The node itself and its ancestor are not owned; a descendant may be.
    assert_eq!(relations.owns, ids(&dom, &["mid"]));
}

#[test]
fn relations_owns_a_node_claimed_twice_goes_to_the_first_owner() {
    let html = r#"
        <div id=first aria-owns="shared one"></div>
        <div id=second aria-owns="shared two"></div>
        <p id=shared>s</p><p id=one>1</p><p id=two>2</p>"#;
    let dom = TestDom::from_html(html);
    let first = dom.semantics_of_id("first").relations.owns;
    let second = dom.semantics_of_id("second").relations.owns;
    assert_eq!(first, ids(&dom, &["shared", "one"]));
    assert_eq!(second, ids(&dom, &["two"]));
}

#[test]
fn relations_owns_ancestor_cycles_between_two_owners() {
    // `outer` owns `inner`'s id list target `deep`, and `deep` contains `outer`'s owner: no loops
    // can come out: each owner keeps only what is not its ancestor.
    let html = r#"<div id=deep><div id=x aria-owns=deep></div></div>"#;
    let (_, relations) = relations_of(html);
    assert!(relations.owns.is_empty());
}

// ---------------------------------------------------------------------------------------------
// whole-node checks
// ---------------------------------------------------------------------------------------------

#[test]
fn relations_text_nodes_have_none() {
    let dom = TestDom::from_html(r#"<p id=x aria-describedby=d>hello</p><span id=d>d</span>"#);
    let ids_index = dom.id_index();
    let text = dom.by_id("x").children().next().expect("text");
    let semantics: NodeSemantics = mercury_a11y::compute(&dom, text, &ids_index);
    assert_eq!(semantics.relations, Relations::default());
}

#[test]
fn relations_do_not_change_the_name_or_the_other_properties() {
    let html = r#"<label for=x>Email</label><input id=x type=email aria-describedby=h value=a@b.example><p id=h>Help</p>"#;
    let semantics = TestDom::from_html(html).semantics_of_id("x");
    assert_eq!(semantics.node.value(), Some("a@b.example"));
    assert_eq!(semantics.node.description(), Some("Help"));
    assert_eq!(semantics.node.label(), Some("Email"));
}
