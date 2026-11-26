// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! The layout-table heuristic (EE9), cases from Firefox's `IsProbablyLayoutTable`. Item 3
//! (`a11y-2`).

use mercury_a11y::layout_table::is_probably_layout_table;
use mercury_a11y::tables::{TableKind, analyze_table};
use mercury_a11y::testing::TestDom;

fn is_layout(html: &str) -> bool {
    let dom = TestDom::from_html(html);
    is_probably_layout_table(&dom, dom.first("table"))
}

#[test]
fn a_table_with_th_is_data() {
    assert!(!is_layout(
        "<table><tr><th>H</th></tr><tr><td>1</td></tr></table>"
    ));
}

#[test]
fn a_table_with_caption_or_summary_is_data() {
    assert!(!is_layout(
        "<table><caption>Sales</caption><tr><td>1</td><td>2</td></tr></table>"
    ));
    assert!(!is_layout(
        "<table summary=\"sales\"><tr><td>1</td><td>2</td></tr></table>"
    ));
}

#[test]
fn role_presentation_is_layout() {
    assert!(is_layout(
        "<table role=presentation><tr><td>1</td><td>2</td></tr></table>"
    ));
}

#[test]
fn a_single_column_table_is_layout() {
    assert!(is_layout(
        "<table><tr><td>1</td></tr><tr><td>2</td></tr></table>"
    ));
}

#[test]
fn a_table_with_nested_table_is_layout() {
    assert!(is_layout(
        "<table><tr><td><table><tr><td>a</td><td>b</td></tr></table></td><td>c</td></tr>\
         <tr><td>d</td><td>e</td></tr></table>"
    ));
}

#[test]
fn a_table_with_many_cells_and_borders_is_data() {
    let rows: String = "<tr><td>1</td><td>2</td><td>3</td><td>4</td><td>5</td></tr>".repeat(30);
    assert!(!is_layout(&format!("<table border=1>{rows}</table>")));
}

#[test]
fn a_two_by_two_with_no_markup_is_layout() {
    assert!(is_layout(
        "<table><tr><td>1</td><td>2</td></tr><tr><td>3</td><td>4</td></tr></table>"
    ));
}

#[test]
fn analyze_table_reports_layout_kind() {
    let dom = TestDom::from_html("<table role=presentation><tr><td>1</td></tr></table>");
    let analysis = analyze_table(&dom, dom.first("table")).expect("the table has rows");
    assert_eq!(analysis.kind, TableKind::Layout);
}
