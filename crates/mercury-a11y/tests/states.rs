// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! States, properties, values and actions of a node (EE4 and the advertised actions), checked through
//! `compute` on `TestDom`s written as HTML. Test names start with `states_` or `actions_`, so
//! `cargo test -p mercury-a11y states actions` runs them. Every table row is one case.

use a11y_dom::{ArenaNode, ComputedStyle, Document, Node, Rect, Rendering, ancestors};
use accesskit::{Action, AriaCurrent, AutoComplete, HasPopup, Invalid, Orientation, Role, Toggled};
use mercury_a11y::actions::SUPPORTED_ACTIONS;
use mercury_a11y::states::{MAX_ATTRIBUTE_BYTES, MAX_COUNT, MAX_VALUE_BYTES};
use mercury_a11y::testing::TestDom;
use mercury_a11y::{HostState, NativeFlags, NodeSemantics, RangeValue, compute};

/// The semantics of the element with this id in this fragment.
#[track_caller]
fn sem(html: &str, id: &str) -> NodeSemantics {
    TestDom::from_html(html).semantics_of_id(id)
}

// ---------------------------------------------------------------------------------------------
// toggled: checked, pressed, mixed
// ---------------------------------------------------------------------------------------------

#[test]
fn states_toggled_table() {
    use Toggled::{False, Mixed, True};
    let cases: &[(&str, Option<Toggled>)] = &[
        // Native controls: the host's flags.
        (r#"<input id=x type=checkbox>"#, Some(False)),
        (r#"<input id=x type=checkbox checked>"#, Some(True)),
        (
            r#"<input id=x type=checkbox data-test-indeterminate=true>"#,
            Some(Mixed),
        ),
        (
            r#"<input id=x type=checkbox checked data-test-indeterminate=true>"#,
            Some(Mixed),
        ),
        (r#"<input id=x type=radio>"#, Some(False)),
        (r#"<input id=x type=radio checked>"#, Some(True)),
        // Native state wins over a contradicting ARIA state.
        (
            r#"<input id=x type=checkbox aria-checked=true>"#,
            Some(False),
        ),
        (
            r#"<input id=x type=checkbox checked aria-checked=false>"#,
            Some(True),
        ),
        // ARIA widgets.
        (
            r#"<div id=x role=checkbox aria-checked=true></div>"#,
            Some(True),
        ),
        (
            r#"<div id=x role=checkbox aria-checked=false></div>"#,
            Some(False),
        ),
        (
            r#"<div id=x role=checkbox aria-checked=mixed></div>"#,
            Some(Mixed),
        ),
        (
            r#"<div id=x role=checkbox aria-checked=TRUE></div>"#,
            Some(True),
        ),
        // Required property: no aria-checked is unchecked.
        (r#"<div id=x role=checkbox></div>"#, Some(False)),
        (
            r#"<div id=x role=checkbox aria-checked=banana></div>"#,
            Some(False),
        ),
        (r#"<div id=x role=switch></div>"#, Some(False)),
        (
            r#"<div id=x role=switch aria-checked=true></div>"#,
            Some(True),
        ),
        (
            r#"<div id=x role=menuitemcheckbox aria-checked=mixed></div>"#,
            Some(Mixed),
        ),
        (
            r#"<div id=x role=menuitemradio aria-checked=true></div>"#,
            Some(True),
        ),
        // `mixed` is a checkbox value only.
        (
            r#"<div id=x role=radio aria-checked=mixed></div>"#,
            Some(False),
        ),
        (
            r#"<div id=x role=switch aria-checked=mixed></div>"#,
            Some(False),
        ),
        // Options and tree items take aria-checked but do not require it.
        (
            r#"<div id=x role=option aria-checked=true></div>"#,
            Some(True),
        ),
        (r#"<div id=x role=option></div>"#, None),
        (
            r#"<div id=x role=treeitem aria-checked=false></div>"#,
            Some(False),
        ),
        // A role that does not take aria-checked ignores it.
        (r#"<div id=x aria-checked=true></div>"#, None),
        (r#"<p id=x role=paragraph aria-checked=true>text</p>"#, None),
        // aria-pressed: buttons only.
        (
            r#"<button id=x aria-pressed=true>Bold</button>"#,
            Some(True),
        ),
        (
            r#"<button id=x aria-pressed=false>Bold</button>"#,
            Some(False),
        ),
        (
            r#"<button id=x aria-pressed=mixed>Bold</button>"#,
            Some(Mixed),
        ),
        (r#"<button id=x>Bold</button>"#, None),
        (
            r#"<div id=x role=button aria-pressed=true>Bold</div>"#,
            Some(True),
        ),
        (r#"<a id=x href=# aria-pressed=true>Bold</a>"#, None),
    ];
    for (html, expected) in cases {
        assert_eq!(sem(html, "x").node.toggled(), *expected, "{html}");
    }
}

// ---------------------------------------------------------------------------------------------
// expanded
// ---------------------------------------------------------------------------------------------

#[test]
fn states_expanded_table() {
    let cases: &[(&str, Option<bool>)] = &[
        (
            r#"<details id=x><summary>More</summary>text</details>"#,
            Some(false),
        ),
        (
            r#"<details id=x open><summary>More</summary>text</details>"#,
            Some(true),
        ),
        // The summary follows its details.
        (
            r#"<details open><summary id=x>More</summary>text</details>"#,
            Some(true),
        ),
        (
            r#"<details><summary id=x>More</summary>text</details>"#,
            Some(false),
        ),
        (
            r#"<button id=x aria-expanded=true>Menu</button>"#,
            Some(true),
        ),
        (
            r#"<button id=x aria-expanded=false>Menu</button>"#,
            Some(false),
        ),
        (r#"<button id=x>Menu</button>"#, None),
        (
            r#"<button id=x aria-expanded=undefined>Menu</button>"#,
            None,
        ),
        (r#"<a id=x href=# aria-expanded=true>Menu</a>"#, Some(true)),
        (
            r#"<div id=x role=treeitem aria-expanded=false></div>"#,
            Some(false),
        ),
        (
            r#"<div id=x role=tab aria-expanded=true></div>"#,
            Some(true),
        ),
        // A role that does not take aria-expanded ignores it.
        (r#"<p id=x aria-expanded=true>text</p>"#, None),
        (r#"<div id=x aria-expanded=true></div>"#, None),
        // A combobox requires aria-expanded: collapsed by default; a select's popup is the host's.
        (r#"<div id=x role=combobox></div>"#, Some(false)),
        (
            r#"<div id=x role=combobox aria-expanded=true></div>"#,
            Some(true),
        ),
        (r#"<select id=x><option>a</option></select>"#, Some(false)),
        (
            r#"<input id=x list=l><datalist id=l></datalist>"#,
            Some(false),
        ),
    ];
    for (html, expected) in cases {
        assert_eq!(sem(html, "x").node.is_expanded(), *expected, "{html}");
    }
}

// ---------------------------------------------------------------------------------------------
// selected
// ---------------------------------------------------------------------------------------------

#[test]
fn states_selected_table() {
    let cases: &[(&str, Option<bool>)] = &[
        // The first option of a single-choice select is selected unless another says so.
        (
            r#"<select><option id=x>a</option><option>b</option></select>"#,
            Some(true),
        ),
        (
            r#"<select><option>a</option><option id=x>b</option></select>"#,
            Some(false),
        ),
        (
            r#"<select><option>a</option><option id=x selected>b</option></select>"#,
            Some(true),
        ),
        (
            r#"<select multiple><option id=x selected>a</option></select>"#,
            Some(true),
        ),
        (
            r#"<div id=x role=tab aria-selected=true></div>"#,
            Some(true),
        ),
        (
            r#"<div id=x role=tab aria-selected=false></div>"#,
            Some(false),
        ),
        (r#"<div id=x role=tab></div>"#, None),
        (
            r#"<div id=x role=row aria-selected=true></div>"#,
            Some(true),
        ),
        (
            r#"<div id=x role=gridcell aria-selected=true></div>"#,
            Some(true),
        ),
        (
            r#"<div id=x role=treeitem aria-selected=false></div>"#,
            Some(false),
        ),
        // Required property: an option with no aria-selected is not selected.
        (r#"<div id=x role=option></div>"#, Some(false)),
        (
            r#"<div id=x role=option aria-selected=true></div>"#,
            Some(true),
        ),
        // Roles that do not take it.
        (r#"<div id=x aria-selected=true></div>"#, None),
        (r#"<button id=x aria-selected=true>b</button>"#, None),
    ];
    for (html, expected) in cases {
        assert_eq!(sem(html, "x").node.is_selected(), *expected, "{html}");
    }
}

// ---------------------------------------------------------------------------------------------
// disabled, read-only, required, invalid
// ---------------------------------------------------------------------------------------------

#[test]
fn states_disabled_read_only_required_table() {
    // (html, disabled, read_only, required)
    let cases: &[(&str, bool, bool, bool)] = &[
        (r#"<button id=x>b</button>"#, false, false, false),
        (r#"<button id=x disabled>b</button>"#, true, false, false),
        (r#"<input id=x disabled>"#, true, false, false),
        // A disabled fieldset disables the controls inside, except those in its first legend.
        (
            r#"<fieldset disabled><input id=x></fieldset>"#,
            true,
            false,
            false,
        ),
        (
            r#"<fieldset disabled><legend><input id=x></legend></fieldset>"#,
            false,
            false,
            false,
        ),
        (
            r#"<div id=x role=button aria-disabled=true></div>"#,
            true,
            false,
            false,
        ),
        (
            r#"<div id=x role=button aria-disabled=false></div>"#,
            false,
            false,
            false,
        ),
        // aria-disabled is inherited by widgets under the element.
        (
            r#"<div role=toolbar aria-disabled=true><div id=x role=button></div></div>"#,
            true,
            false,
            false,
        ),
        (
            r#"<div role=toolbar aria-disabled=true><p id=x>text</p></div>"#,
            false,
            false,
            false,
        ),
        (r#"<input id=x readonly>"#, false, true, false),
        (
            r#"<textarea id=x readonly>t</textarea>"#,
            false,
            true,
            false,
        ),
        (
            r#"<div id=x role=textbox aria-readonly=true></div>"#,
            false,
            true,
            false,
        ),
        (
            r#"<div id=x role=checkbox aria-readonly=true></div>"#,
            false,
            true,
            false,
        ),
        (r#"<p id=x aria-readonly=true>t</p>"#, false, false, false),
        (r#"<input id=x required>"#, false, false, true),
        (
            r#"<select id=x required><option>a</option></select>"#,
            false,
            false,
            true,
        ),
        (
            r#"<div id=x role=textbox aria-required=true></div>"#,
            false,
            false,
            true,
        ),
        (
            r#"<div id=x role=combobox aria-required=true></div>"#,
            false,
            false,
            true,
        ),
        (
            r#"<div id=x role=textbox aria-required=false></div>"#,
            false,
            false,
            false,
        ),
        (r#"<p id=x aria-required=true>t</p>"#, false, false, false),
        (
            r#"<input id=x type=password required readonly>"#,
            false,
            true,
            true,
        ),
    ];
    for (html, disabled, read_only, required) in cases {
        let node = sem(html, "x").node;
        assert_eq!(node.is_disabled(), *disabled, "disabled: {html}");
        assert_eq!(node.is_read_only(), *read_only, "read-only: {html}");
        assert_eq!(node.is_required(), *required, "required: {html}");
    }
}

#[test]
fn states_invalid_table() {
    let cases: &[(&str, Option<Invalid>)] = &[
        (r#"<input id=x>"#, None),
        (r#"<input id=x aria-invalid=true>"#, Some(Invalid::True)),
        (
            r#"<input id=x aria-invalid=grammar>"#,
            Some(Invalid::Grammar),
        ),
        (
            r#"<input id=x aria-invalid=spelling>"#,
            Some(Invalid::Spelling),
        ),
        (r#"<input id=x aria-invalid=false>"#, None),
        (r#"<input id=x aria-invalid="">"#, None),
        // A value ARIA does not list counts as true (Blink).
        (r#"<input id=x aria-invalid=banana>"#, Some(Invalid::True)),
        (
            r#"<div id=x role=textbox aria-invalid=true></div>"#,
            Some(Invalid::True),
        ),
        // Native constraint validation through the host.
        (
            r#"<input id=x data-test-invalid=true>"#,
            Some(Invalid::True),
        ),
        (r#"<input id=x required>"#, Some(Invalid::True)),
        (r#"<input id=x required value=filled>"#, None),
        // An explicit aria-invalid=false wins over the native state.
        (r#"<input id=x required aria-invalid=false>"#, None),
    ];
    for (html, expected) in cases {
        assert_eq!(sem(html, "x").node.invalid(), *expected, "{html}");
    }
}

// ---------------------------------------------------------------------------------------------
// busy, modal, multiselectable, has-popup, current, orientation, auto-complete
// ---------------------------------------------------------------------------------------------

#[test]
fn states_busy_modal_multiselectable_table() {
    // (html, busy, modal, multiselectable)
    let cases: &[(&str, bool, bool, bool)] = &[
        (r#"<div id=x aria-busy=true></div>"#, true, false, false),
        (r#"<div id=x aria-busy=false></div>"#, false, false, false),
        (
            r#"<div id=x role=dialog aria-modal=true></div>"#,
            false,
            true,
            false,
        ),
        (
            r#"<div id=x role=alertdialog aria-modal=true></div>"#,
            false,
            true,
            false,
        ),
        (r#"<div id=x role=dialog></div>"#, false, false, false),
        (r#"<div id=x aria-modal=true></div>"#, false, false, false),
        (
            r#"<div id=x role=listbox aria-multiselectable=true></div>"#,
            false,
            false,
            true,
        ),
        (r#"<div id=x role=listbox></div>"#, false, false, false),
        (
            r#"<div id=x role=tree aria-multiselectable=true></div>"#,
            false,
            false,
            true,
        ),
        (
            r#"<div id=x role=grid aria-multiselectable=true></div>"#,
            false,
            false,
            true,
        ),
        (
            r#"<select id=x multiple><option>a</option></select>"#,
            false,
            false,
            true,
        ),
        (
            r#"<select id=x size=3><option>a</option></select>"#,
            false,
            false,
            false,
        ),
        (
            r#"<div id=x aria-multiselectable=true></div>"#,
            false,
            false,
            false,
        ),
    ];
    for (html, busy, modal, multiselectable) in cases {
        let node = sem(html, "x").node;
        assert_eq!(node.is_busy(), *busy, "busy: {html}");
        assert_eq!(node.is_modal(), *modal, "modal: {html}");
        assert_eq!(node.is_multiselectable(), *multiselectable, "multi: {html}");
    }
}

#[test]
fn states_has_popup_and_current_table() {
    let popups: &[(&str, Option<HasPopup>)] = &[
        (
            r#"<button id=x aria-haspopup=true>b</button>"#,
            Some(HasPopup::Menu),
        ),
        (
            r#"<button id=x aria-haspopup=menu>b</button>"#,
            Some(HasPopup::Menu),
        ),
        (
            r#"<button id=x aria-haspopup=listbox>b</button>"#,
            Some(HasPopup::Listbox),
        ),
        (
            r#"<button id=x aria-haspopup=tree>b</button>"#,
            Some(HasPopup::Tree),
        ),
        (
            r#"<button id=x aria-haspopup=grid>b</button>"#,
            Some(HasPopup::Grid),
        ),
        (
            r#"<button id=x aria-haspopup=dialog>b</button>"#,
            Some(HasPopup::Dialog),
        ),
        (r#"<button id=x aria-haspopup=false>b</button>"#, None),
        (r#"<button id=x aria-haspopup="">b</button>"#, None),
        (
            r#"<button id=x aria-haspopup=whatever>b</button>"#,
            Some(HasPopup::Menu),
        ),
        (r#"<button id=x>b</button>"#, None),
    ];
    for (html, expected) in popups {
        assert_eq!(sem(html, "x").node.has_popup(), *expected, "{html}");
    }
    let currents: &[(&str, Option<AriaCurrent>)] = &[
        (
            r#"<a id=x href=# aria-current=page>a</a>"#,
            Some(AriaCurrent::Page),
        ),
        (
            r#"<a id=x href=# aria-current=true>a</a>"#,
            Some(AriaCurrent::True),
        ),
        (
            r#"<li id=x aria-current=step>a</li>"#,
            Some(AriaCurrent::Step),
        ),
        (
            r#"<li id=x aria-current=location>a</li>"#,
            Some(AriaCurrent::Location),
        ),
        (
            r#"<li id=x aria-current=date>a</li>"#,
            Some(AriaCurrent::Date),
        ),
        (
            r#"<li id=x aria-current=time>a</li>"#,
            Some(AriaCurrent::Time),
        ),
        (r#"<a id=x href=# aria-current=false>a</a>"#, None),
        (
            r#"<a id=x href=# aria-current=PAGE>a</a>"#,
            Some(AriaCurrent::Page),
        ),
        (
            r#"<a id=x href=# aria-current=nonsense>a</a>"#,
            Some(AriaCurrent::True),
        ),
        (r#"<a id=x href=#>a</a>"#, None),
    ];
    for (html, expected) in currents {
        assert_eq!(sem(html, "x").node.aria_current(), *expected, "{html}");
    }
}

#[test]
fn states_orientation_and_autocomplete_table() {
    let orientations: &[(&str, Option<Orientation>)] = &[
        (
            r#"<div id=x role=tablist></div>"#,
            Some(Orientation::Horizontal),
        ),
        (
            r#"<div id=x role=tablist aria-orientation=vertical></div>"#,
            Some(Orientation::Vertical),
        ),
        (
            r#"<div id=x role=toolbar></div>"#,
            Some(Orientation::Horizontal),
        ),
        (
            r#"<div id=x role=listbox></div>"#,
            Some(Orientation::Vertical),
        ),
        (
            r#"<div id=x role=listbox aria-orientation=horizontal></div>"#,
            Some(Orientation::Horizontal),
        ),
        (r#"<div id=x role=menu></div>"#, Some(Orientation::Vertical)),
        (
            r#"<div id=x role=menubar></div>"#,
            Some(Orientation::Horizontal),
        ),
        (
            r#"<div id=x role=scrollbar></div>"#,
            Some(Orientation::Vertical),
        ),
        (r#"<input id=x type=range>"#, Some(Orientation::Horizontal)),
        (r#"<hr id=x>"#, Some(Orientation::Horizontal)),
        (
            r#"<div id=x role=slider aria-orientation=sideways></div>"#,
            Some(Orientation::Horizontal),
        ),
        // A role with no orientation takes none.
        (r#"<div id=x aria-orientation=vertical></div>"#, None),
        (r#"<button id=x aria-orientation=vertical>b</button>"#, None),
    ];
    for (html, expected) in orientations {
        assert_eq!(sem(html, "x").node.orientation(), *expected, "{html}");
    }
    let completions: &[(&str, Option<AutoComplete>)] = &[
        (
            r#"<div id=x role=combobox aria-autocomplete=list></div>"#,
            Some(AutoComplete::List),
        ),
        (
            r#"<div id=x role=combobox aria-autocomplete=both></div>"#,
            Some(AutoComplete::Both),
        ),
        (
            r#"<div id=x role=textbox aria-autocomplete=inline></div>"#,
            Some(AutoComplete::Inline),
        ),
        (
            r#"<div id=x role=combobox aria-autocomplete=none></div>"#,
            None,
        ),
        (r#"<div id=x aria-autocomplete=list></div>"#, None),
    ];
    for (html, expected) in completions {
        assert_eq!(sem(html, "x").node.auto_complete(), *expected, "{html}");
    }
}

// ---------------------------------------------------------------------------------------------
// level, set size and position
// ---------------------------------------------------------------------------------------------

#[test]
fn states_level_table() {
    let cases: &[(&str, Option<usize>)] = &[
        (r#"<h1 id=x>t</h1>"#, Some(1)),
        (r#"<h2 id=x>t</h2>"#, Some(2)),
        (r#"<h3 id=x>t</h3>"#, Some(3)),
        (r#"<h4 id=x>t</h4>"#, Some(4)),
        (r#"<h5 id=x>t</h5>"#, Some(5)),
        (r#"<h6 id=x>t</h6>"#, Some(6)),
        // aria-level overrides the element's level, as browsers do.
        (r#"<h2 id=x aria-level=5>t</h2>"#, Some(5)),
        (r#"<div id=x role=heading aria-level=3>t</div>"#, Some(3)),
        // A heading with no usable level is level 2.
        (r#"<div id=x role=heading>t</div>"#, Some(2)),
        (r#"<div id=x role=heading aria-level=0>t</div>"#, Some(2)),
        (
            r#"<div id=x role=heading aria-level=banana>t</div>"#,
            Some(2),
        ),
        (r#"<div id=x role=heading aria-level=2.5>t</div>"#, Some(2)),
        (r#"<h4 id=x aria-level=-1>t</h4>"#, Some(4)),
        (
            r#"<h1 id=x aria-level=99999999999999999999>t</h1>"#,
            Some(1),
        ),
        (r#"<div id=x role=treeitem aria-level=4></div>"#, Some(4)),
        (r#"<li id=x aria-level=2>a</li>"#, Some(2)),
        (r#"<li id=x>a</li>"#, None),
        (r#"<p id=x aria-level=2>a</p>"#, None),
    ];
    for (html, expected) in cases {
        assert_eq!(sem(html, "x").node.level(), *expected, "{html}");
    }
}

#[test]
fn states_set_size_and_position_table() {
    // (html, position, size)
    let cases: &[(&str, Option<usize>, Option<usize>)] = &[
        // Native counts for list items, options and radio groups.
        (
            r#"<ul><li>a</li><li id=x>b</li><li>c</li><li>d</li></ul>"#,
            Some(2),
            Some(4),
        ),
        (
            r#"<select><option>a</option><option id=x>b</option></select>"#,
            Some(2),
            Some(2),
        ),
        (
            r#"<form><input type=radio name=g><input id=x type=radio name=g><input type=radio name=g></form>"#,
            Some(2),
            Some(3),
        ),
        // Explicit values replace the native ones, each on its own.
        (
            r#"<ul><li id=x aria-posinset=7 aria-setsize=40>a</li><li>b</li></ul>"#,
            Some(7),
            Some(40),
        ),
        (
            r#"<ul><li id=x aria-posinset=3>a</li><li>b</li></ul>"#,
            Some(3),
            None,
        ),
        (
            r#"<ul><li id=x aria-setsize=9>a</li><li>b</li></ul>"#,
            None,
            Some(9),
        ),
        // `aria-setsize=-1` says the size is unknown.
        (
            r#"<div id=x role=listitem aria-posinset=2 aria-setsize=-1></div>"#,
            Some(2),
            None,
        ),
        (
            r#"<div id=x role=article aria-posinset=1 aria-setsize=20></div>"#,
            Some(1),
            Some(20),
        ),
        (
            r#"<div id=x role=tab aria-posinset=1 aria-setsize=3></div>"#,
            Some(1),
            Some(3),
        ),
        // Invalid positions are dropped.
        (
            r#"<div id=x role=option aria-posinset=0></div>"#,
            None,
            None,
        ),
        (
            r#"<div id=x role=option aria-posinset=abc aria-setsize=xyz></div>"#,
            None,
            None,
        ),
        // A role without sets takes none.
        (
            r#"<p id=x aria-posinset=2 aria-setsize=4>t</p>"#,
            None,
            None,
        ),
        // Huge numbers are clamped.
        (
            r#"<div id=x role=listitem aria-posinset=999999999999 aria-setsize=999999999999></div>"#,
            Some(MAX_COUNT),
            Some(MAX_COUNT),
        ),
    ];
    for (html, position, size) in cases {
        let node = sem(html, "x").node;
        assert_eq!(node.position_in_set(), *position, "position: {html}");
        assert_eq!(node.size_of_set(), *size, "size: {html}");
    }
}

// ---------------------------------------------------------------------------------------------
// value
// ---------------------------------------------------------------------------------------------

#[test]
fn states_value_table() {
    let cases: &[(&str, Option<&str>)] = &[
        (r#"<input id=x value=Ada>"#, Some("Ada")),
        (
            r#"<input id=x value=Ada data-test-value=Grace>"#,
            Some("Grace"),
        ),
        (r#"<input id=x type=search value=cats>"#, Some("cats")),
        (
            r#"<input id=x type=email value=a@b.example>"#,
            Some("a@b.example"),
        ),
        (r#"<input id=x type=tel value=555>"#, Some("555")),
        (
            r#"<input id=x type=url value=http://x.example>"#,
            Some("http://x.example"),
        ),
        (
            r#"<input id=x type=date value=2026-11-20>"#,
            Some("2026-11-20"),
        ),
        (
            r##"<input id=x type=color value="#ff0000">"##,
            Some("#ff0000"),
        ),
        (
            r#"<textarea id=x>first line</textarea>"#,
            Some("first line"),
        ),
        (
            r#"<textarea id=x data-test-value=typed>old</textarea>"#,
            Some("typed"),
        ),
        (
            r#"<select id=x><option>One</option><option selected>Two</option></select>"#,
            Some("Two"),
        ),
        (r#"<output id=x>42</output>"#, Some("42")),
        // Not a text value: numbers, toggles, buttons.
        (r#"<input id=x type=range value=3>"#, None),
        (r#"<input id=x type=checkbox value=on>"#, None),
        (r#"<input id=x type=radio value=a>"#, None),
        (r#"<input id=x type=submit value=Send>"#, None),
        (r#"<input id=x type=button value=Go>"#, None),
        (r#"<input id=x type=hidden value=secret-token>"#, None),
        (r#"<input id=x type=file value=whatever>"#, None),
        (r#"<button id=x value=v>b</button>"#, None),
        (r#"<div id=x role=textbox>typed</div>"#, None),
    ];
    for (html, expected) in cases {
        assert_eq!(sem(html, "x").node.value(), *expected, "{html}");
    }
}

/// D7: a password field exposes no value of any kind, whatever the host answers and however the page
/// writes the type.
#[test]
fn states_password_exposes_no_value_anywhere() {
    let secret = "hunter2-SECRET";
    let cases = [
        format!(r#"<input id=x type=password value="{secret}">"#),
        format!(r#"<input id=x type=password data-test-value="{secret}">"#),
        format!(r#"<input id=x type=PASSWORD value="{secret}">"#),
        format!(r#"<input id=x type=" password " value="{secret}">"#),
        format!(
            r#"<input id=x type=password value="{secret}" aria-valuenow=7 aria-valuetext="{secret}" aria-valuemin=1 aria-valuemax=9>"#
        ),
        format!(r#"<input id=x type=password value="{secret}" role=slider aria-valuenow=7>"#),
        format!(
            r#"<input id=x type=password value="{secret}" role=spinbutton aria-valuetext="{secret}">"#
        ),
        format!(
            r#"<input id=x type=password data-test-value="{secret}" required placeholder="Password" aria-label="Secret field">"#
        ),
        format!(r#"<input id=x type=password value="{secret}" min=1 max=9 step=2>"#),
        format!(
            r#"<input id=x type=password value="{secret}" aria-describedby=d><p id=d>Enter it</p>"#
        ),
    ];
    for html in &cases {
        let semantics = sem(html, "x");
        let node = &semantics.node;
        assert_eq!(node.value(), None, "value: {html}");
        assert_eq!(node.numeric_value(), None, "numeric value: {html}");
        assert_eq!(node.min_numeric_value(), None, "min: {html}");
        assert_eq!(node.max_numeric_value(), None, "max: {html}");
        assert_eq!(node.numeric_value_step(), None, "step: {html}");
        assert!(
            !format!("{semantics:?}").contains(secret),
            "the password text reached a node property: {html}"
        );
    }
}

#[test]
fn states_password_keeps_state_and_placeholder() {
    let semantics = sem(
        r#"<input id=x type=password required readonly placeholder="Password" aria-label="Password">"#,
        "x",
    );
    let node = &semantics.node;
    assert_eq!(node.role(), Role::PasswordInput);
    assert!(node.is_required());
    assert!(node.is_read_only());
    assert_eq!(node.placeholder(), Some("Password"));
    assert_eq!(node.label(), Some("Password"));
}

/// A host that forgets the rule and answers a value for a password field leaks nothing: `states`
/// checks the input type and the role itself.
#[test]
fn states_password_with_a_careless_host_leaks_nothing() {
    struct Careless(TestDom);
    impl Document for Careless {
        type N<'a> = ArenaNode<'a>;
        fn root(&self) -> ArenaNode<'_> {
            self.0.root()
        }
    }
    impl Rendering for Careless {
        fn computed_style<'n>(&'n self, node: ArenaNode<'n>) -> Option<ComputedStyle> {
            self.0.computed_style(node)
        }
        fn bounds<'n>(&'n self, node: ArenaNode<'n>) -> Option<Rect> {
            self.0.bounds(node)
        }
    }
    impl HostState for Careless {
        fn native_flags<'n>(&'n self, node: ArenaNode<'n>) -> NativeFlags {
            self.0.native_flags(node)
        }
        fn control_value<'n>(&'n self, _node: ArenaNode<'n>) -> Option<String> {
            Some("LEAKED".to_string())
        }
        fn range<'n>(&'n self, _node: ArenaNode<'n>) -> Option<RangeValue> {
            Some(RangeValue {
                value: 31337.0,
                min: 0.0,
                max: 99999.0,
                step: Some(1.0),
            })
        }
    }
    let dom = Careless(TestDom::from_html(
        r#"<input id=x type=password value=secret><input id=y type=text role=slider><input id=z type=password role=slider>"#,
    ));
    let ids = dom.0.id_index();
    for id in ["x", "z"] {
        let semantics = compute(&dom, dom.0.by_id(id), &ids);
        assert_eq!(semantics.node.value(), None, "#{id}");
        assert_eq!(semantics.node.numeric_value(), None, "#{id}");
        assert!(!format!("{semantics:?}").contains("LEAKED"), "#{id}");
        assert!(!format!("{semantics:?}").contains("31337"), "#{id}");
    }
    // Control: the same careless host does give a text field its value and a slider its range.
    let control = compute(&dom, dom.0.by_id("y"), &ids);
    assert_eq!(control.node.numeric_value(), Some(31337.0));
}

// ---------------------------------------------------------------------------------------------
// numeric value, minimum, maximum, step
// ---------------------------------------------------------------------------------------------

#[test]
fn states_numeric_table() {
    // (html, now, min, max, step)
    type Row = (
        &'static str,
        Option<f64>,
        Option<f64>,
        Option<f64>,
        Option<f64>,
    );
    let cases: &[Row] = &[
        // Native ranges: the host's value, bounds and step.
        (
            r#"<input id=x type=range>"#,
            Some(50.0),
            Some(0.0),
            Some(100.0),
            Some(1.0),
        ),
        (
            r#"<input id=x type=range min=10 max=20 value=12 step=2>"#,
            Some(12.0),
            Some(10.0),
            Some(20.0),
            Some(2.0),
        ),
        (
            r#"<input id=x type=range step=any>"#,
            Some(50.0),
            Some(0.0),
            Some(100.0),
            None,
        ),
        (
            r#"<input id=x type=number min=0 max=10 value=4>"#,
            Some(4.0),
            Some(0.0),
            Some(10.0),
            Some(1.0),
        ),
        (
            r#"<progress id=x value=30 max=120></progress>"#,
            Some(30.0),
            Some(0.0),
            Some(120.0),
            None,
        ),
        (
            r#"<progress id=x value=0.5></progress>"#,
            Some(0.5),
            Some(0.0),
            Some(1.0),
            None,
        ),
        // Indeterminate: a progress bar with no value has no number.
        (r#"<progress id=x></progress>"#, None, None, None, None),
        (
            r#"<meter id=x value=6 min=0 max=10></meter>"#,
            Some(6.0),
            Some(0.0),
            Some(10.0),
            None,
        ),
        // A number input the host has no bounds for: what parses.
        (
            r#"<input id=x type=number value=7>"#,
            Some(7.0),
            None,
            None,
            None,
        ),
        (
            r#"<input id=x type=number value=7 min=1 step=0.5>"#,
            Some(7.0),
            Some(1.0),
            None,
            Some(0.5),
        ),
        (r#"<input id=x type=number>"#, None, None, None, None),
        // ARIA widgets.
        (
            r#"<div id=x role=slider aria-valuenow=30></div>"#,
            Some(30.0),
            Some(0.0),
            Some(100.0),
            None,
        ),
        (
            r#"<div id=x role=slider aria-valuenow=3 aria-valuemin=1 aria-valuemax=5></div>"#,
            Some(3.0),
            Some(1.0),
            Some(5.0),
            None,
        ),
        // A slider with no value is at the midpoint.
        (
            r#"<div id=x role=slider></div>"#,
            Some(50.0),
            Some(0.0),
            Some(100.0),
            None,
        ),
        (
            r#"<div id=x role=slider aria-valuemin=10 aria-valuemax=30></div>"#,
            Some(20.0),
            Some(10.0),
            Some(30.0),
            None,
        ),
        // A maximum below the minimum is raised to it.
        (
            r#"<div id=x role=slider aria-valuenow=5 aria-valuemin=10 aria-valuemax=2></div>"#,
            Some(5.0),
            Some(10.0),
            Some(10.0),
            None,
        ),
        // A spin button has no default bounds.
        (
            r#"<div id=x role=spinbutton aria-valuenow=3></div>"#,
            Some(3.0),
            None,
            None,
            None,
        ),
        (
            r#"<div id=x role=spinbutton></div>"#,
            None,
            None,
            None,
            None,
        ),
        // A progress bar with no aria-valuenow is indeterminate.
        (
            r#"<div id=x role=progressbar></div>"#,
            None,
            None,
            None,
            None,
        ),
        (
            r#"<div id=x role=progressbar aria-valuenow=40></div>"#,
            Some(40.0),
            Some(0.0),
            Some(100.0),
            None,
        ),
        (
            r#"<div id=x role=meter aria-valuenow=2 aria-valuemax=4></div>"#,
            Some(2.0),
            Some(0.0),
            Some(4.0),
            None,
        ),
        // Numbers that are not numbers.
        (
            r#"<div id=x role=spinbutton aria-valuenow=NaN></div>"#,
            None,
            None,
            None,
            None,
        ),
        (
            r#"<div id=x role=spinbutton aria-valuenow=inf></div>"#,
            None,
            None,
            None,
            None,
        ),
        (
            r#"<div id=x role=spinbutton aria-valuenow=abc></div>"#,
            None,
            None,
            None,
            None,
        ),
        // A separator is a number only when it can take focus (a splitter).
        (
            r#"<div id=x role=separator aria-valuenow=40></div>"#,
            None,
            None,
            None,
            None,
        ),
        (
            r#"<div id=x role=separator tabindex=0 aria-valuenow=40></div>"#,
            Some(40.0),
            Some(0.0),
            Some(100.0),
            None,
        ),
        // A role with no number ignores the attributes.
        (
            r#"<div id=x aria-valuenow=40></div>"#,
            None,
            None,
            None,
            None,
        ),
    ];
    for (html, now, min, max, step) in cases {
        let node = sem(html, "x").node;
        assert_eq!(node.numeric_value(), *now, "now: {html}");
        assert_eq!(node.min_numeric_value(), *min, "min: {html}");
        assert_eq!(node.max_numeric_value(), *max, "max: {html}");
        assert_eq!(node.numeric_value_step(), *step, "step: {html}");
    }
}

#[test]
fn states_value_text_of_a_range() {
    let node = sem(
        r#"<div id=x role=slider aria-valuenow=2 aria-valuemax=3 aria-valuetext="Medium"></div>"#,
        "x",
    )
    .node;
    assert_eq!(node.value(), Some("Medium"));
    assert_eq!(node.numeric_value(), Some(2.0));
    let node = sem(
        r#"<input id=x type=range value=4 aria-valuetext="four of ten">"#,
        "x",
    )
    .node;
    assert_eq!(node.value(), Some("four of ten"));
    // Empty text is no text.
    let node = sem(
        r#"<div id=x role=slider aria-valuenow=2 aria-valuetext="  "></div>"#,
        "x",
    )
    .node;
    assert_eq!(node.value(), None);
    // Only on a role with a number.
    let node = sem(r#"<div id=x aria-valuetext="x"></div>"#, "x").node;
    assert_eq!(node.value(), None);
}

// ---------------------------------------------------------------------------------------------
// placeholder, URL
// ---------------------------------------------------------------------------------------------

#[test]
fn states_placeholder_table() {
    let cases: &[(&str, Option<&str>)] = &[
        (r#"<input id=x placeholder="Your name">"#, Some("Your name")),
        (r#"<input id=x placeholder="  padded  ">"#, Some("padded")),
        (r#"<input id=x placeholder="">"#, None),
        (
            r#"<input id=x type=email placeholder="you@example.com">"#,
            Some("you@example.com"),
        ),
        (
            r#"<input id=x type=password placeholder="Password">"#,
            Some("Password"),
        ),
        (r#"<input id=x type=number placeholder="0">"#, Some("0")),
        // A type HTML does not define is a text field.
        (r#"<input id=x type=bogus placeholder="p">"#, Some("p")),
        (
            r#"<textarea id=x placeholder="Say something"></textarea>"#,
            Some("Say something"),
        ),
        (
            r#"<div id=x role=textbox aria-placeholder="Type here"></div>"#,
            Some("Type here"),
        ),
        (r#"<input id=x type=checkbox placeholder="no">"#, None),
        (r#"<input id=x type=date placeholder="no">"#, None),
        (r#"<button id=x placeholder="no">b</button>"#, None),
    ];
    for (html, expected) in cases {
        assert_eq!(sem(html, "x").node.placeholder(), *expected, "{html}");
    }
}

#[test]
fn states_url_table() {
    let cases: &[(&str, Option<&str>)] = &[
        (
            r#"<a id=x href="https://example.com/a?b=c#d">link</a>"#,
            Some("https://example.com/a?b=c#d"),
        ),
        (
            r#"<a id=x href="  /relative/path  ">link</a>"#,
            Some("/relative/path"),
        ),
        // No href: not a link, no URL.
        (r#"<a id=x>link</a>"#, None),
        // A link role on another element takes no URL: the attribute is the anchor's.
        (r#"<span id=x role=link>link</span>"#, None),
        (r#"<a id=x href="">link</a>"#, None),
    ];
    for (html, expected) in cases {
        assert_eq!(sem(html, "x").node.url(), *expected, "{html}");
    }
    // A URL as long as a page (a data: URL) is not exposed.
    let long = format!(
        r#"<a id=x href="data:text/html,{}">x</a>"#,
        "a".repeat(MAX_ATTRIBUTE_BYTES)
    );
    assert_eq!(sem(&long, "x").node.url(), None);
}

// ---------------------------------------------------------------------------------------------
// hidden
// ---------------------------------------------------------------------------------------------

#[test]
fn states_hidden_table() {
    let cases: &[(&str, bool)] = &[
        (r#"<p id=x>t</p>"#, false),
        (r#"<p id=x aria-hidden=true>t</p>"#, true),
        (r#"<p id=x aria-hidden=TRUE>t</p>"#, true),
        (r#"<p id=x aria-hidden=false>t</p>"#, false),
        (r#"<p id=x aria-hidden=undefined>t</p>"#, false),
        (r#"<p id=x inert>t</p>"#, true),
        (r#"<p id=x inert=false>t</p>"#, true),
        (r#"<p id=x hidden>t</p>"#, true),
        (r#"<p id=x hidden=until-found>t</p>"#, true),
        // The page's CSS may make a `hidden` element render, and then it is read.
        (r#"<p id=x hidden style="display:block">t</p>"#, false),
        (r#"<p id=x style="display:none">t</p>"#, true),
        (r#"<p id=x style="visibility:hidden">t</p>"#, true),
        (r#"<p id=x style="visibility:collapse">t</p>"#, true),
        (r#"<p id=x style="visibility:visible">t</p>"#, false),
        (
            r#"<div style="visibility:hidden"><p id=x>t</p></div>"#,
            true,
        ),
        (
            r#"<div style="visibility:hidden"><p id=x style="visibility:visible">t</p></div>"#,
            false,
        ),
        (r#"<input id=x type=hidden>"#, true),
        (r#"<input id=x type=text>"#, false),
        // Hidden controls are still computed (their state is real), and carry the flag.
        (r#"<button id=x disabled aria-hidden=true>b</button>"#, true),
    ];
    for (html, hidden) in cases {
        let semantics = sem(html, "x");
        assert_eq!(semantics.hidden, *hidden, "hidden: {html}");
        assert_eq!(semantics.node.is_hidden(), *hidden, "node flag: {html}");
    }
}

/// A DOM that says which element holds focus and which link it resolves, and whether a script
/// listens for click on an element.
struct HostDom {
    dom: TestDom,
    focus: Option<&'static str>,
    clickable: Option<&'static str>,
    link: Option<&'static str>,
}

impl HostDom {
    fn new(html: &str) -> Self {
        HostDom {
            dom: TestDom::from_html(html),
            focus: None,
            clickable: None,
            link: None,
        }
    }

    fn compute(&self, id: &str) -> NodeSemantics {
        let ids = self.dom.id_index();
        compute(self, self.dom.by_id(id), &ids)
    }
}

impl Document for HostDom {
    type N<'a> = ArenaNode<'a>;
    fn root(&self) -> ArenaNode<'_> {
        self.dom.root()
    }
}

impl Rendering for HostDom {
    fn computed_style<'n>(&'n self, node: ArenaNode<'n>) -> Option<ComputedStyle> {
        self.dom.computed_style(node)
    }
    fn bounds<'n>(&'n self, node: ArenaNode<'n>) -> Option<Rect> {
        self.dom.bounds(node)
    }
}

impl HostState for HostDom {
    fn native_flags<'n>(&'n self, node: ArenaNode<'n>) -> NativeFlags {
        self.dom.native_flags(node)
    }
    fn control_value<'n>(&'n self, node: ArenaNode<'n>) -> Option<String> {
        self.dom.control_value(node)
    }
    fn range<'n>(&'n self, node: ArenaNode<'n>) -> Option<RangeValue> {
        self.dom.range(node)
    }
    fn position_in_set<'n>(&'n self, node: ArenaNode<'n>) -> Option<(usize, usize)> {
        self.dom.position_in_set(node)
    }
    fn has_focus<'n>(&'n self, node: ArenaNode<'n>) -> bool {
        let Some(focus) = self.focus else {
            return false;
        };
        let focused = self.dom.by_id(focus);
        std::iter::once(focused)
            .chain(ancestors(focused))
            .any(|candidate| candidate == node)
    }
    fn has_click_handler<'n>(&'n self, node: ArenaNode<'n>) -> bool {
        self.clickable.is_some_and(|id| self.dom.by_id(id) == node)
    }
    fn link_url<'n>(&'n self, node: ArenaNode<'n>) -> Option<String> {
        node.attr("href")?;
        self.link.map(str::to_string)
    }
}

#[test]
fn states_aria_hidden_holding_focus_is_not_hidden() {
    let html = r#"<div id=wrap aria-hidden=true><button id=inner>b</button><p id=other>t</p></div><div id=gone aria-hidden=true><button id=lost>b</button></div>"#;
    let mut dom = HostDom::new(html);
    // No focus anywhere: everything under aria-hidden is hidden.
    assert!(dom.compute("wrap").hidden);
    assert!(dom.compute("gone").hidden);
    // Focus inside the first one: the wrapper and the focused button are exposed; the other
    // aria-hidden element stays hidden.
    dom.focus = Some("inner");
    let wrap = dom.compute("wrap");
    assert!(!wrap.hidden && !wrap.node.is_hidden());
    assert!(!dom.compute("inner").hidden);
    assert!(dom.compute("gone").hidden);
    // aria-hidden on the focused element itself.
    let mut dom = HostDom::new(r#"<button id=x aria-hidden=true>b</button>"#);
    assert!(dom.compute("x").hidden);
    dom.focus = Some("x");
    assert!(!dom.compute("x").hidden);
    // Focus does not undo inert, the hidden attribute or display none.
    let mut dom = HostDom::new(r#"<button id=x inert>b</button>"#);
    dom.focus = Some("x");
    assert!(dom.compute("x").hidden);
}

#[test]
fn states_link_url_is_the_hosts_when_it_resolves() {
    let mut dom = HostDom::new(r#"<a id=x href="../up">link</a>"#);
    assert_eq!(dom.compute("x").node.url(), Some("../up"));
    dom.link = Some("https://example.com/up");
    assert_eq!(dom.compute("x").node.url(), Some("https://example.com/up"));
}

// ---------------------------------------------------------------------------------------------
// caps and hostile input
// ---------------------------------------------------------------------------------------------

#[test]
fn states_strings_from_the_page_are_capped_at_a_character_boundary() {
    // A multi-byte character straddling the cap: never cut inside it.
    let long = "€".repeat(MAX_ATTRIBUTE_BYTES);
    let node = sem(&format!(r#"<input id=x placeholder="{long}">"#), "x").node;
    let placeholder = node.placeholder().expect("a placeholder");
    assert!(placeholder.len() <= MAX_ATTRIBUTE_BYTES);
    assert!(placeholder.len() > MAX_ATTRIBUTE_BYTES - 3);
    assert!(placeholder.chars().all(|c| c == '€'));

    let big = "€".repeat(MAX_VALUE_BYTES);
    let node = sem(
        &format!(r#"<textarea id=x data-test-value="{big}"></textarea>"#),
        "x",
    )
    .node;
    let value = node.value().expect("a value");
    assert!(value.len() <= MAX_VALUE_BYTES);
    assert!(value.len() > MAX_VALUE_BYTES - 3);
    assert!(value.chars().all(|c| c == '€'));

    let node = sem(
        &format!(r#"<div id=x role=slider aria-valuenow=1 aria-valuetext="{long}"></div>"#),
        "x",
    )
    .node;
    assert!(node.value().expect("a value").len() <= MAX_ATTRIBUTE_BYTES);
}

#[test]
fn states_hostile_attribute_values_give_defaults_and_never_panic() {
    let mut nasty: Vec<String> = [
        "",
        " ",
        "\u{0}",
        "\u{feff}",
        "1e999",
        "-1e999",
        "NaN",
        "inf",
        "-0",
        "0x10",
        "٣",
        "true false",
        "TRUE",
        "True ",
        "mixed;",
        "<script>",
        "'quoted'",
    ]
    .iter()
    .map(|text| text.to_string())
    .collect();
    nasty.push("9".repeat(400));
    let attributes = [
        "aria-checked",
        "aria-pressed",
        "aria-expanded",
        "aria-selected",
        "aria-disabled",
        "aria-readonly",
        "aria-required",
        "aria-invalid",
        "aria-busy",
        "aria-modal",
        "aria-haspopup",
        "aria-current",
        "aria-orientation",
        "aria-level",
        "aria-posinset",
        "aria-setsize",
        "aria-valuenow",
        "aria-valuemin",
        "aria-valuemax",
        "aria-valuetext",
        "aria-hidden",
        "aria-autocomplete",
        "aria-placeholder",
        "aria-multiselectable",
    ];
    let roles = [
        "checkbox",
        "radio",
        "switch",
        "button",
        "combobox",
        "listbox",
        "option",
        "tab",
        "treeitem",
        "slider",
        "spinbutton",
        "progressbar",
        "meter",
        "scrollbar",
        "separator",
        "textbox",
        "heading",
        "listitem",
        "row",
        "gridcell",
        "dialog",
        "tablist",
        "generic",
    ];
    for role in roles {
        for attribute in attributes {
            for value in &nasty {
                let html =
                    format!(r#"<div id=x role="{role}" tabindex=0 {attribute}="{value}"></div>"#);
                let semantics = sem(&html, "x");
                // Whatever came out, a node that says "numeric value" has a finite one.
                if let Some(now) = semantics.node.numeric_value() {
                    assert!(now.is_finite(), "{html}");
                }
                for number in [
                    semantics.node.min_numeric_value(),
                    semantics.node.max_numeric_value(),
                ]
                .into_iter()
                .flatten()
                {
                    assert!(number.is_finite(), "{html}");
                }
                if let Some(level) = semantics.node.level() {
                    assert!((1..=MAX_COUNT).contains(&level), "{html}");
                }
            }
        }
    }
}

#[test]
fn states_text_nodes_and_unknown_elements_are_left_alone() {
    let dom = TestDom::from_html("<p id=p>hello</p>");
    let ids = dom.id_index();
    let text = dom
        .by_id("p")
        .children()
        .next()
        .expect("the paragraph has a text child");
    let semantics = compute(&dom, text, &ids);
    assert_eq!(semantics.node.role(), Role::TextRun);
    assert!(!semantics.hidden);
    assert_eq!(semantics.node.toggled(), None);
    assert_eq!(semantics.node.is_expanded(), None);
    assert_eq!(semantics.node.level(), None);
    let custom = sem(
        r#"<my-widget id=x aria-checked=true aria-expanded=true></my-widget>"#,
        "x",
    );
    assert_eq!(custom.node.toggled(), None);
    assert_eq!(custom.node.is_expanded(), None);
}

// ---------------------------------------------------------------------------------------------
// actions
// ---------------------------------------------------------------------------------------------

/// The actions the node advertises, in `SUPPORTED_ACTIONS` order.
fn actions_of(semantics: &NodeSemantics) -> Vec<Action> {
    SUPPORTED_ACTIONS
        .iter()
        .copied()
        .filter(|action| semantics.node.supports_action(*action))
        .collect()
}

#[test]
fn actions_supported_list_is_pinned() {
    // Item 5's `accessibility_actions.rs` implements exactly this list: changing it changes both.
    assert_eq!(
        SUPPORTED_ACTIONS,
        &[
            Action::Focus,
            Action::Blur,
            Action::Click,
            Action::SetValue,
            Action::ReplaceSelectedText,
            Action::ScrollIntoView,
            Action::ScrollToPoint,
            Action::ScrollUp,
            Action::ScrollDown,
            Action::ScrollLeft,
            Action::ScrollRight,
            Action::Increment,
            Action::Decrement,
            Action::Expand,
            Action::Collapse,
            Action::ShowContextMenu,
        ]
    );
    // No duplicates.
    for (index, action) in SUPPORTED_ACTIONS.iter().enumerate() {
        assert!(
            !SUPPORTED_ACTIONS[..index].contains(action),
            "{action:?} is listed twice"
        );
    }
}

/// The actions of several groups, in one list.
fn join(parts: &[&[Action]]) -> Vec<Action> {
    parts.concat()
}

#[test]
fn actions_table() {
    use Action::{
        Blur, Click, Collapse, Decrement, Expand, Focus, Increment, ReplaceSelectedText,
        ScrollDown, ScrollIntoView, ScrollLeft, ScrollRight, ScrollToPoint, ScrollUp, SetValue,
        ShowContextMenu,
    };
    // Every element scrolls into view; the rest depends on the role and the state.
    let base = [ScrollIntoView, ScrollToPoint];
    let focus = [Focus, Blur, ShowContextMenu];
    let mut cases: Vec<(&str, Vec<Action>)> = vec![
        // Static content.
        (r#"<p id=x>t</p>"#, base.to_vec()),
        (r#"<h1 id=x>t</h1>"#, base.to_vec()),
        (r#"<img id=x alt=a src=a.png>"#, base.to_vec()),
        (r#"<ul id=x><li>a</li></ul>"#, base.to_vec()),
        // Controls with a default action.
        (
            r#"<button id=x>b</button>"#,
            join(&[&focus, &[Click], &base]),
        ),
        (r#"<a id=x href=#>l</a>"#, join(&[&focus, &[Click], &base])),
        (
            r#"<input id=x type=checkbox>"#,
            join(&[&focus, &[Click], &base]),
        ),
        (
            r#"<input id=x type=radio>"#,
            join(&[&focus, &[Click], &base]),
        ),
        (
            r#"<input id=x type=submit value=Go>"#,
            join(&[&focus, &[Click], &base]),
        ),
        (
            r#"<div id=x role=switch tabindex=0></div>"#,
            join(&[&focus, &[Click], &base]),
        ),
        (
            r#"<div id=x role=tab tabindex=0 aria-selected=true></div>"#,
            join(&[&focus, &[Click], &base]),
        ),
        // Disabled: nothing but scrolling.
        (r#"<button id=x disabled>b</button>"#, base.to_vec()),
        (r#"<input id=x type=checkbox disabled>"#, base.to_vec()),
        (
            r#"<div id=x role=button tabindex=0 aria-disabled=true></div>"#,
            base.to_vec(),
        ),
        // Text fields: focus and edit, no click, no step.
        (
            r#"<input id=x type=text>"#,
            join(&[&focus, &[SetValue, ReplaceSelectedText], &base]),
        ),
        (
            r#"<textarea id=x></textarea>"#,
            join(&[&focus, &[SetValue, ReplaceSelectedText], &base]),
        ),
        (
            r#"<input id=x type=password>"#,
            join(&[&focus, &[SetValue, ReplaceSelectedText], &base]),
        ),
        (
            r#"<div id=x contenteditable>t</div>"#,
            join(&[&focus, &[SetValue, ReplaceSelectedText], &base]),
        ),
        // Read-only text can take focus and be read, not edited.
        (r#"<input id=x readonly>"#, join(&[&focus, &base])),
        (
            r#"<div id=x role=textbox tabindex=0 aria-readonly=true></div>"#,
            join(&[&focus, &base]),
        ),
        // Steppers.
        (
            r#"<input id=x type=range>"#,
            join(&[&focus, &[Increment, Decrement], &base]),
        ),
        (
            r#"<input id=x type=number>"#,
            join(&[
                &focus,
                &[SetValue, ReplaceSelectedText, Increment, Decrement],
                &base,
            ]),
        ),
        (
            r#"<div id=x role=slider tabindex=0 aria-valuenow=3></div>"#,
            join(&[&focus, &[Increment, Decrement], &base]),
        ),
        // A slider that cannot change takes no step.
        (r#"<input id=x type=range disabled>"#, base.to_vec()),
        (
            r#"<div id=x role=slider tabindex=0 aria-readonly=true></div>"#,
            join(&[&focus, &base]),
        ),
        // An ARIA spin button with no number has nothing to step from.
        (
            r#"<div id=x role=spinbutton tabindex=0></div>"#,
            join(&[&focus, &base]),
        ),
        // Expand or collapse, whichever changes the state.
        (
            r#"<button id=x aria-expanded=false>m</button>"#,
            join(&[&focus, &[Click, Expand], &base]),
        ),
        (
            r#"<button id=x aria-expanded=true>m</button>"#,
            join(&[&focus, &[Click, Collapse], &base]),
        ),
        (
            r#"<details id=x><summary>s</summary>t</details>"#,
            join(&[&[Expand], &base]),
        ),
        (
            r#"<details open><summary id=x>s</summary>t</details>"#,
            join(&[&focus, &[Click, Collapse], &base]),
        ),
        // Scroll containers scroll in four directions.
        (
            r#"<div id=x style="overflow:auto">t</div>"#,
            join(&[&base, &[ScrollUp, ScrollDown, ScrollLeft, ScrollRight]]),
        ),
        // A focusable generic element is clickable; a focusable text-like region is not.
        (
            r#"<div id=x tabindex=0>t</div>"#,
            join(&[&focus, &[Click], &base]),
        ),
        (
            r#"<div id=x tabindex=-1>t</div>"#,
            join(&[&focus, &[Click], &base]),
        ),
        (r#"<div id=x tabindex=abc>t</div>"#, base.to_vec()),
        // Selectable items are clickable.
        (
            r#"<div id=x role=row aria-selected=false></div>"#,
            join(&[&[Click], &base]),
        ),
        // The select pop-up button.
        (
            r#"<select id=x><option>a</option></select>"#,
            join(&[&focus, &[Click, Expand], &base]),
        ),
    ];
    // Hidden nodes advertise nothing.
    cases.push((r#"<button id=x aria-hidden=true>b</button>"#, Vec::new()));
    for (html, expected) in cases {
        let semantics = sem(html, "x");
        let actual: std::collections::BTreeSet<_> = actions_of(&semantics)
            .into_iter()
            .map(|a| format!("{a:?}"))
            .collect();
        let expected: std::collections::BTreeSet<_> =
            expected.into_iter().map(|a| format!("{a:?}")).collect();
        assert_eq!(actual, expected, "{html}");
    }
}

#[test]
fn actions_text_nodes_only_scroll_into_view() {
    let dom = TestDom::from_html("<p id=p>hello</p>");
    let ids = dom.id_index();
    let text = dom.by_id("p").children().next().expect("a text node");
    let semantics = compute(&dom, text, &ids);
    assert_eq!(actions_of(&semantics), vec![Action::ScrollIntoView]);
}

#[test]
fn actions_click_handler_makes_a_div_clickable_but_not_a_text_field() {
    let mut dom = HostDom::new(r#"<div id=x>t</div><input id=y>"#);
    dom.clickable = Some("x");
    let actions = actions_of(&dom.compute("x"));
    assert!(actions.contains(&Action::Click));
    // The handler is on x only.
    assert!(!actions_of(&dom.compute("y")).contains(&Action::Click));
    // And not on an editable text field, even if the host says so.
    dom.clickable = Some("y");
    assert!(!actions_of(&dom.compute("y")).contains(&Action::Click));
}

#[test]
fn actions_only_supported_actions_are_ever_advertised() {
    let html = r#"
        <h1 id=a>t</h1><a id=b href=#>l</a><button id=c aria-expanded=true aria-pressed=true>b</button>
        <input id=d type=text><input id=e type=range><input id=f type=number><input id=g type=checkbox>
        <textarea id=h></textarea><select id=i><option id=j>a</option></select>
        <details id=k><summary id=l>s</summary></details><div id=m style="overflow:scroll" tabindex=0>t</div>
        <div id=n role=slider tabindex=0></div><div id=o role=menuitem tabindex=0></div>
        <div id=p role=treeitem aria-expanded=false aria-selected=false></div><progress id=q></progress>
        <input id=r type=password><div id=s contenteditable>t</div><input id=t type=color>
    "#;
    let dom = TestDom::from_html(html);
    let ids = dom.id_index();
    for id in "abcdefghijklmnopqrst".chars() {
        let semantics = compute(&dom, dom.by_id(&id.to_string()), &ids);
        for action in [
            Action::Click,
            Action::Focus,
            Action::Blur,
            Action::Collapse,
            Action::Expand,
            Action::CustomAction,
            Action::Decrement,
            Action::Increment,
            Action::HideTooltip,
            Action::ShowTooltip,
            Action::ReplaceSelectedText,
            Action::ScrollDown,
            Action::ScrollLeft,
            Action::ScrollRight,
            Action::ScrollUp,
            Action::ScrollIntoView,
            Action::ScrollToPoint,
            Action::SetScrollOffset,
            Action::SetTextSelection,
            Action::SetSequentialFocusNavigationStartingPoint,
            Action::SetValue,
            Action::ShowContextMenu,
        ] {
            if semantics.node.supports_action(action) {
                assert!(
                    SUPPORTED_ACTIONS.contains(&action),
                    "#{id} advertises {action:?}, which the engine does not perform"
                );
            }
        }
    }
}
