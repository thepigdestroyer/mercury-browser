// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//
// Adapted from Firefox's accessible/html/HTMLTableAccessible.cpp, `IsProbablyLayoutTable`
// (Mozilla Public License 2.0, https://mozilla.org/MPL/2.0/). The conditions and their order are
// Firefox's; the layout-dependent ones use the host's computed style and the table grid.

//! The layout-table heuristic: whether a `<table>` is a page layout rather than data.

use a11y_dom::{Node, NodeKind, Rendering};

use crate::host::HostState;
use crate::tables::{TableAnalysis, build_grid};

/// A table with more rows than this is data, unless it is bordered or a layout by an earlier rule.
const MAX_LAYOUT_ROWS: usize = 20;
/// A table with at most this many cells is layout (Firefox: "2-4 columns, 10 cells or less").
const MAX_SMALL_CELLS: usize = 10;
/// A table with at least this many columns is data.
const MIN_DATA_COLUMNS: usize = 5;

/// Whether `table` is a layout table, by Firefox's `IsProbablyLayoutTable`.
pub fn is_probably_layout_table<'n, D: Rendering + HostState>(doc: &'n D, table: D::N<'n>) -> bool {
    if let Some(role) = table.attr("role") {
        return role == "presentation" || role == "none";
    }
    if table.attr("datatable") == Some("0") {
        return true;
    }
    if table.has_attr("summary") || has_caption(table) {
        return false;
    }
    for child in elements(table) {
        if matches!(child.local_name(), "col" | "colgroup" | "tfoot" | "thead") {
            return false;
        }
        if child.is_element("tbody")
            && let Some(answer) = rows_answer(child)
        {
            return answer;
        }
    }
    if let Some(answer) = rows_answer(table) {
        return answer;
    }

    let Some(analysis) = build_grid(table) else {
        return false;
    };
    let rows = analysis.row_count;
    let cols = analysis.column_count;
    if cols <= 1 {
        return true;
    }
    if rows <= 1 {
        return true;
    }
    if cols >= MIN_DATA_COLUMNS {
        return false;
    }
    if has_border(doc, table, &analysis) {
        return false;
    }
    if descendants(table).any(|c| c.is_element("table")) {
        return true;
    }
    if rows > MAX_LAYOUT_ROWS {
        return false;
    }
    if rows * cols <= MAX_SMALL_CELLS {
        return true;
    }
    descendants(table).any(|c| matches!(c.local_name(), "embed" | "object" | "iframe"))
}

/// The answer the `tr` children of a table or `tbody` give, or `None` when they decide nothing.
fn rows_answer<'n, N: Node<'n>>(parent: N) -> Option<bool> {
    for row in elements(parent).filter(|c| c.is_element("tr")) {
        if row.attr("role").is_some_and(|role| role != "row") {
            return Some(true);
        }
        for cell in elements(row) {
            if cell.is_element("th")
                || cell.has_attr("headers")
                || cell.has_attr("scope")
                || cell.has_attr("abbr")
            {
                return Some(false);
            }
            if cell.attr("role").is_some_and(|role| {
                !matches!(role, "cell" | "columnheader" | "rowheader" | "gridcell")
            }) {
                return Some(true);
            }
            if has_only_abbr(cell) {
                return Some(false);
            }
        }
    }
    None
}

/// Whether the table's first cell has a border, which marks a data table.
fn has_border<'n, D: Rendering + HostState>(
    doc: &'n D,
    table: D::N<'n>,
    analysis: &TableAnalysis<D::N<'n>>,
) -> bool {
    let cell_border = analysis
        .cells
        .iter()
        .find(|(_, info)| info.row_index == 0 && info.column_index == 0)
        .and_then(|(node, _)| doc.computed_style(*node))
        .and_then(|style| style.border_top_width_px)
        .is_some_and(|width| width > 0.0);
    cell_border
        || table
            .attr("border")
            .is_some_and(|b| b.trim() != "0" && !b.trim().is_empty())
}

fn has_caption<'n, N: Node<'n>>(table: N) -> bool {
    elements(table).any(|c| c.is_element("caption") && c.children().next().is_some())
}

fn has_only_abbr<'n, N: Node<'n>>(cell: N) -> bool {
    let mut children = elements(cell);
    matches!((children.next(), children.next()), (Some(only), None) if only.is_element("abbr"))
}

fn elements<'n, N: Node<'n>>(node: N) -> impl Iterator<Item = N> + 'n {
    node.children().filter(|c| c.kind() == NodeKind::Element)
}

fn descendants<'n, N: Node<'n>>(node: N) -> impl Iterator<Item = N> + 'n {
    let mut stack: Vec<N> = node.children().collect();
    std::iter::from_fn(move || {
        let next = stack.pop()?;
        stack.extend(next.children());
        Some(next)
    })
}
