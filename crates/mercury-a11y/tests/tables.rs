// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! The table model (EE9) on the markup: the slot grid, spans, headers, the ARIA indices and the
//! caps. Item 3 (`a11y-2`).

use a11y_dom::{ArenaNode, Node};
use mercury_a11y::tables::{CellInfo, HeaderKind, TableAnalysis, TableKind, analyze_table};
use mercury_a11y::testing::TestDom;

fn analysis(dom: &TestDom) -> TableAnalysis<ArenaNode<'_>> {
    analyze_table(dom, dom.first("table")).expect("the table has rows")
}

fn cell(analysis: &TableAnalysis<ArenaNode<'_>>, id: &str) -> CellInfo {
    analysis
        .cells
        .iter()
        .find(|(node, _)| node.attr("id") == Some(id))
        .map(|(_, info)| info.clone())
        .unwrap_or_else(|| panic!("no cell with id {id:?}"))
}

/// A table of `rows` by `cols` empty cells, as markup.
fn grid_html(rows: usize, cols: usize) -> String {
    let mut html = String::from("<table>");
    for _ in 0..rows {
        html.push_str("<tr>");
        for _ in 0..cols {
            html.push_str("<td></td>");
        }
        html.push_str("</tr>");
    }
    html.push_str("</table>");
    html
}

#[test]
fn a_2x3_table_counts() {
    let dom = TestDom::from_html(
        "<table><tr><td>a</td><td>b</td><td>c</td></tr><tr><td>d</td><td>e</td><td>f</td></tr></table>",
    );
    let a = analysis(&dom);
    assert_eq!((a.row_count, a.column_count), (2, 3));
}

#[test]
fn colspan_shifts_the_next_cell() {
    let dom =
        TestDom::from_html("<table><tr><td id=a colspan=2>x</td><td id=b>y</td></tr></table>");
    let a = analysis(&dom);
    let first = cell(&a, "a");
    assert_eq!((first.column_index, first.column_span), (0, 2));
    let second = cell(&a, "b");
    assert_eq!((second.row_index, second.column_index), (0, 2));
    assert_eq!(a.column_count, 3);
}

#[test]
fn rowspan_pushes_the_cell_below_right() {
    let dom = TestDom::from_html(
        "<table><tr><td id=a rowspan=2>x</td><td id=b>y</td></tr><tr><td id=c>z</td></tr></table>",
    );
    let a = analysis(&dom);
    assert_eq!(cell(&a, "a").row_span, 2);
    assert_eq!(cell(&a, "b").column_index, 1);
    let below = cell(&a, "c");
    assert_eq!((below.row_index, below.column_index), (1, 1));
    assert_eq!(a.column_count, 2);
}

#[test]
fn a_ragged_row_does_not_extend_the_count_beyond_the_widest_row() {
    let dom = TestDom::from_html(
        "<table><tr><td>a</td><td>b</td><td>c</td></tr><tr><td>d</td><td>e</td></tr></table>",
    );
    let a = analysis(&dom);
    assert_eq!((a.row_count, a.column_count), (2, 3));
}

#[test]
fn colspan_zero_is_one() {
    let dom =
        TestDom::from_html("<table><tr><td id=a colspan=0>x</td><td id=b>y</td></tr></table>");
    let a = analysis(&dom);
    assert_eq!(cell(&a, "a").column_span, 1);
    assert_eq!(cell(&a, "b").column_index, 1);
    assert_eq!(a.column_count, 2);
}

#[test]
fn rowspan_past_the_last_row_is_clamped() {
    let dom = TestDom::from_html(
        "<table><tr><td id=a rowspan=5>x</td></tr><tr><td id=b>y</td></tr></table>",
    );
    let a = analysis(&dom);
    assert_eq!(cell(&a, "a").row_span, 2);
    assert_eq!(cell(&a, "b").column_index, 1);
    assert_eq!(a.column_count, 2);
}

#[test]
fn thead_tbody_tfoot_rows_are_numbered_in_document_order() {
    let dom = TestDom::from_html(
        "<table><thead><tr id=h><th>H</th></tr></thead>\
         <tbody><tr id=b1><td>1</td></tr><tr id=b2><td>2</td></tr></tbody>\
         <tfoot><tr id=f><td>F</td></tr></tfoot></table>",
    );
    let a = analysis(&dom);
    let rows: Vec<(Option<&str>, usize)> = a
        .rows
        .iter()
        .map(|row| (row.node.attr("id"), row.row_index))
        .collect();
    assert_eq!(
        rows,
        vec![
            (Some("h"), 0),
            (Some("b1"), 1),
            (Some("b2"), 2),
            (Some("f"), 3)
        ]
    );
    assert_eq!(a.row_count, 4);
}

#[test]
fn th_scope_col_is_a_column_header() {
    let dom = TestDom::from_html(
        "<table><tr><th id=a scope=col>x</th><th id=b>y</th></tr><tr><td id=c>z</td><td>w</td></tr></table>",
    );
    let a = analysis(&dom);
    assert_eq!(cell(&a, "a").header, HeaderKind::Column);
    assert_eq!(cell(&a, "b").header, HeaderKind::Column);
    assert_eq!(cell(&a, "c").header, HeaderKind::None);
}

#[test]
fn th_in_a_row_with_scope_row_is_a_row_header() {
    let dom = TestDom::from_html(
        "<table><tr><td id=a>x</td><td id=b>y</td></tr><tr><th id=r scope=row>z</th><td>w</td></tr></table>",
    );
    let a = analysis(&dom);
    assert_eq!(cell(&a, "r").header, HeaderKind::Row);
    assert_eq!(cell(&a, "a").header, HeaderKind::None);
}

#[test]
fn headers_attribute_associates_a_cell_with_its_headers() {
    let dom = TestDom::from_html(
        "<table><tr><th id=h1>A</th><th id=h2>B</th></tr>\
         <tr><td id=c headers=\"h1\">1</td><td id=d headers=\"h2 h1\">2</td></tr></table>",
    );
    let a = analysis(&dom);
    let ids_of = |id: &str| -> Vec<String> {
        a.column_header_of
            .iter()
            .find(|(node, _)| node.attr("id") == Some(id))
            .map(|(_, headers)| {
                headers
                    .iter()
                    .filter_map(|h| h.attr("id").map(str::to_owned))
                    .collect()
            })
            .unwrap_or_default()
    };
    assert_eq!(ids_of("c"), vec!["h1".to_owned()]);
    assert_eq!(ids_of("d"), vec!["h2".to_owned(), "h1".to_owned()]);
}

#[test]
fn aria_grid_uses_aria_rowindex_and_colindex() {
    let dom = TestDom::from_html(
        "<div role=grid id=g><div role=row id=r aria-rowindex=2>\
         <div role=gridcell id=c aria-colindex=3>x</div></div></div>",
    );
    let a = analyze_table(&dom, dom.by_id("g")).expect("the grid has rows");
    assert_eq!(a.kind, TableKind::Grid);
    assert_eq!(a.rows[0].row_index, 1);
    let c = cell(&a, "c");
    assert_eq!((c.row_index, c.column_index), (1, 2));
    assert_eq!(c.header, HeaderKind::None);
}

#[test]
fn aria_rowcount_wins_over_counted_rows() {
    let dom = TestDom::from_html("<table aria-rowcount=10><tr><td>x</td></tr></table>");
    let a = analysis(&dom);
    assert_eq!(a.rows.len(), 1);
    assert_eq!(a.row_count, 10);
}

#[test]
fn a_nested_table_is_analysed_on_its_own() {
    let dom = TestDom::from_html(
        "<table id=outer><tr><td><table id=inner><tr><td>a</td><td>b</td></tr>\
         <tr><td>c</td><td>d</td></tr></table></td></tr></table>",
    );
    let outer = analyze_table(&dom, dom.by_id("outer")).expect("the outer table has rows");
    assert_eq!((outer.row_count, outer.column_count), (1, 1));
    assert_eq!(outer.cells.len(), 1);
    let inner = analyze_table(&dom, dom.by_id("inner")).expect("the inner table has rows");
    assert_eq!((inner.row_count, inner.column_count), (2, 2));
    assert_eq!(inner.cells.len(), 4);
}

#[test]
fn a_100_by_100_table_completes_and_drops_nothing() {
    let dom = TestDom::from_html(&grid_html(100, 100));
    let a = analysis(&dom);
    assert_eq!((a.row_count, a.column_count), (100, 100));
    assert_eq!(a.cells.len(), 10_000);
    assert_eq!(a.cells_dropped, 0);
}

#[test]
fn a_table_over_100000_cells_is_capped() {
    let dom = TestDom::from_html(&grid_html(320, 320));
    let a = analysis(&dom);
    assert_eq!(a.cells.len(), 100_000);
    assert_eq!(a.cells_dropped, 102_400 - 100_000);
}

#[test]
fn more_than_1000_columns_are_dropped() {
    let row: String = "<td></td>".repeat(1002);
    let dom = TestDom::from_html(&format!("<table><tr>{row}</tr></table>"));
    let a = analysis(&dom);
    assert_eq!(a.column_count, 1000);
    assert_eq!(a.cells.len(), 1000);
    assert_eq!(a.cells_dropped, 2);
}

#[test]
fn no_rows_gives_none() {
    let dom = TestDom::from_html("<table><caption>x</caption></table>");
    assert!(analyze_table(&dom, dom.first("table")).is_none());
}
