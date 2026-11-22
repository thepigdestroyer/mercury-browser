// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! The calculator's parser and evaluator (stub: the omnibox-core lane's calculator task fills it).

/// The value of `input` when it is arithmetic worth an answer row: it holds at least one
/// operator, function call or percent sign (a bare number or constant is not an answer).
pub(crate) fn evaluate(_input: &str) -> Option<f64> {
    None
}

/// The value of `input` as any expression, including a bare number; for the unit converter's
/// quantity (`2 * 3` in `2 * 3 km to m`).
pub(crate) fn value_of(_input: &str) -> Option<f64> {
    None
}
