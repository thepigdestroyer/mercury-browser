// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! Table structure for AccessKit: the table kind, row and column indices, spans and headers.

use a11y_dom::Rendering;

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
}

// stub
pub fn analyze_table<'n, D: Rendering + HostState>(
    _doc: &'n D,
    _table: D::N<'n>,
) -> Option<TableAnalysis<D::N<'n>>> {
    None
}
