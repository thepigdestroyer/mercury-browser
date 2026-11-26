// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! The protocol-handler table as loaded from the store.

use std::collections::BTreeMap;

use mercury_store::ProtocolHandlerRow;

use crate::MAX_HANDLERS;

/// The protocol handlers the model holds, keyed by scheme and origin.
#[derive(Debug, Default)]
pub(crate) struct HandlerTable {
    handlers: BTreeMap<(String, String), ProtocolHandlerRow>,
}

impl HandlerTable {
    /// Builds the table from stored rows. A repeated scheme and origin, and rows beyond
    /// [`MAX_HANDLERS`], are not kept; the number not kept is returned.
    pub(crate) fn from_rows(rows: Vec<ProtocolHandlerRow>) -> (Self, usize) {
        let mut table = Self::default();
        let mut ignored = 0;
        for row in rows {
            let key = (row.scheme.clone(), row.origin.clone());
            if table.handlers.contains_key(&key) || table.handlers.len() >= MAX_HANDLERS {
                ignored += 1;
            } else {
                table.handlers.insert(key, row);
            }
        }
        (table, ignored)
    }

    pub(crate) fn len(&self) -> usize {
        self.handlers.len()
    }
}
