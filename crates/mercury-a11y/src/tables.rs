// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! Table structure for AccessKit: the table kind, row and column indices, spans and headers.

use std::collections::HashMap;

use a11y_dom::{Node, Rendering};

use crate::host::HostState;

/// What kind of table this is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TableKind {
    Data,
    Layout,
    Grid,
    TreeGrid,
}

/// Which header a cell belongs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HeaderKind {
    None,
    Column,
    Row,
}

/// Position and header of one cell.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CellInfo {
    pub row_index: usize,
    pub column_index: usize,
    pub row_span: usize,
    pub column_span: usize,
    pub header: HeaderKind,
    pub row_index_text: Option<String>,
    pub column_index_text: Option<String>,
}

/// One row of a table.
pub struct RowInfo<N> {
    pub node: N,
    pub row_index: usize,
    pub row_index_text: Option<String>,
}

/// The whole table: its kind, its size and every row and cell.
pub struct TableAnalysis<N> {
    pub kind: TableKind,
    pub row_count: usize,
    pub column_count: usize,
    pub rows: Vec<RowInfo<N>>,
    pub cells: Vec<(N, CellInfo)>,
    pub column_header_of: Vec<(N, Vec<N>)>,
    /// Cells past the caps (10,000 rows, 1,000 columns, 100,000 cells) that are not in `cells`.
    pub cells_dropped: usize,
}

const MAX_ROWS: usize = 10_000;
const MAX_COLUMNS: usize = 1_000;
const MAX_CELLS: usize = 100_000;

/// The table model of `table`, or `None` for a table with no rows.
pub fn analyze_table<'n, D: Rendering + HostState>(
    _doc: &'n D,
    table: D::N<'n>,
) -> Option<TableAnalysis<D::N<'n>>> {
    let rows = collect_rows(table);
    if rows.is_empty() {
        return None;
    }
    let kind = table_kind(table);
    let aria = matches!(kind, TableKind::Grid | TableKind::TreeGrid);
    let row_total = rows.len();

    let mut occupied: Vec<Vec<bool>> = Vec::new();
    let mut row_infos = Vec::with_capacity(row_total);
    let mut cells: Vec<(D::N<'n>, CellInfo)> = Vec::new();
    let mut cells_dropped = 0usize;
    let mut column_count = 0usize;
    let mut row_count = 0usize;

    for (position, row) in rows.into_iter().enumerate() {
        let row_index = if aria {
            number(row, "aria-rowindex")
                .filter(|v| *v >= 1)
                .map_or(position, |v| v - 1)
        } else {
            position
        };
        row_infos.push(RowInfo {
            node: row,
            row_index,
            row_index_text: row.attr("aria-rowindex").map(str::to_owned),
        });
        row_count = row_count.max(row_index.saturating_add(1));

        let (col_attr, row_attr) = if aria {
            ("aria-colspan", "aria-rowspan")
        } else {
            ("colspan", "rowspan")
        };
        let mut cursor = 0usize;
        for child in row.children().filter(|c| is_cell(*c)) {
            let column_index = if aria {
                number(child, "aria-colindex")
                    .filter(|v| *v >= 1)
                    .map_or(cursor, |v| v - 1)
            } else {
                while is_occupied(&occupied, row_index, cursor) {
                    cursor += 1;
                }
                cursor
            };
            let column_span = span_of(child, col_attr);
            let row_span = if aria {
                span_of(child, row_attr)
            } else {
                span_of(child, row_attr).min(row_total - position)
            };
            let end = column_index.saturating_add(column_span);
            cursor = end;

            let info = CellInfo {
                row_index,
                column_index,
                row_span,
                column_span,
                header: header_of(child),
                row_index_text: child.attr("aria-rowindex").map(str::to_owned),
                column_index_text: child.attr("aria-colindex").map(str::to_owned),
            };
            if end > MAX_COLUMNS || cells.len() >= MAX_CELLS {
                cells_dropped += 1;
                continue;
            }
            column_count = column_count.max(end);
            if !aria {
                mark(
                    &mut occupied,
                    row_index,
                    column_index,
                    row_span,
                    column_span,
                );
            }
            cells.push((child, info));
        }
    }

    let column_header_of = column_headers(&cells);
    if let Some(count) = number(table, "aria-rowcount") {
        row_count = count;
    }
    if let Some(count) = number(table, "aria-colcount") {
        column_count = count;
    }

    Some(TableAnalysis {
        kind,
        row_count,
        column_count,
        rows: row_infos,
        cells,
        column_header_of,
        cells_dropped,
    })
}

/// The rows of a table in document order: its direct `tr` children and those in `thead`, `tbody`
/// and `tfoot`, capped at [`MAX_ROWS`].
fn collect_rows<'n, N: Node<'n>>(table: N) -> Vec<N> {
    let mut rows = Vec::new();
    for child in table.children() {
        if is_row(child) {
            rows.push(child);
        } else if is_row_group(child) {
            rows.extend(child.children().filter(|c| is_row(*c)));
        }
    }
    rows.truncate(MAX_ROWS);
    rows
}

fn is_row<'n, N: Node<'n>>(node: N) -> bool {
    node.is_element("tr") || node.attr("role") == Some("row")
}

fn is_row_group<'n, N: Node<'n>>(node: N) -> bool {
    matches!(node.local_name(), "thead" | "tbody" | "tfoot")
        || node.attr("role") == Some("rowgroup")
}

fn table_kind<'n, N: Node<'n>>(table: N) -> TableKind {
    match table.attr("role").and_then(|r| r.split_whitespace().next()) {
        Some("grid") => TableKind::Grid,
        Some("treegrid") => TableKind::TreeGrid,
        _ => TableKind::Data,
    }
}

fn is_cell<'n, N: Node<'n>>(node: N) -> bool {
    node.is_element("td")
        || node.is_element("th")
        || matches!(
            node.attr("role"),
            Some("cell" | "gridcell" | "columnheader" | "rowheader")
        )
}

/// A non-negative integer attribute, `None` when absent or not a number.
fn number<'n, N: Node<'n>>(node: N, name: &str) -> Option<usize> {
    node.attr(name)?.trim().parse::<usize>().ok()
}

/// The `colspan` or `rowspan` of a cell: 0 and garbage are 1.
fn span_of<'n, N: Node<'n>>(cell: N, name: &str) -> usize {
    number(cell, name).filter(|v| *v >= 1).unwrap_or(1)
}

/// The header kind of a cell: an ARIA `columnheader` or `rowheader` role, or a `th` by its
/// `scope` (a `th` with no scope is a column header).
fn header_of<'n, N: Node<'n>>(cell: N) -> HeaderKind {
    match cell.attr("role") {
        Some("columnheader") => return HeaderKind::Column,
        Some("rowheader") => return HeaderKind::Row,
        _ => {}
    }
    if !cell.is_element("th") {
        return HeaderKind::None;
    }
    match cell.attr("scope") {
        Some("row" | "rowgroup") => HeaderKind::Row,
        _ => HeaderKind::Column,
    }
}

fn is_occupied(occupied: &[Vec<bool>], row: usize, column: usize) -> bool {
    occupied
        .get(row)
        .and_then(|line| line.get(column))
        .copied()
        .unwrap_or(false)
}

/// Marks the slots a cell covers, so a later cell in the same row moves right past it.
fn mark(
    occupied: &mut Vec<Vec<bool>>,
    row: usize,
    column: usize,
    row_span: usize,
    column_span: usize,
) {
    for r in row..row + row_span {
        if occupied.len() <= r {
            occupied.resize(r + 1, Vec::new());
        }
        let line = &mut occupied[r];
        if line.len() < column + column_span {
            line.resize(column + column_span, false);
        }
        line[column..column + column_span].fill(true);
    }
}

/// The header cells named by each cell's `headers` attribute, in the order written.
fn column_headers<'n, N: Node<'n>>(cells: &[(N, CellInfo)]) -> Vec<(N, Vec<N>)> {
    let by_id: HashMap<&str, N> = cells
        .iter()
        .filter_map(|(node, _)| node.attr("id").map(|id| (id, *node)))
        .collect();
    cells
        .iter()
        .filter_map(|(node, _)| {
            let headers = node.attr("headers")?;
            let resolved = headers
                .split_whitespace()
                .filter_map(|id| by_id.get(id).copied())
                .collect();
            Some((*node, resolved))
        })
        .collect()
}
