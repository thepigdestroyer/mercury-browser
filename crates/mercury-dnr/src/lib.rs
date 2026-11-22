// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! The declarativeNetRequest evaluator for browser extensions (EE28).
//!
//! Rule parsing and validation, URL filters, the evaluator and the rewrite it returns. No engine, no
//! network and no clock: the request hook in `talaria-servo` (a later lane) calls it.

#![warn(missing_docs)]
#![cfg_attr(
    not(test),
    deny(clippy::unwrap_used, clippy::expect_used, clippy::panic)
)]

pub mod evaluator;
pub mod index;
pub mod limits;
pub mod request;
pub mod rule;
pub mod url_filter;
pub mod verdict;
