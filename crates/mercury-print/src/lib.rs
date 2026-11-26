// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! Printing for Mercury: page setup, page ranges, pagination, the full-page capture state machine,
//! the PDF writer and the print backends. Everything here is plain data and pure functions, so the
//! tests never open a print panel, a save panel or a printer.

pub mod assembler;
pub mod backend;
pub mod breaks;
pub mod capture;
pub mod cups;
pub mod flow;
pub mod names;
pub mod paginator;
pub mod paper;
pub mod pdf;
pub mod png_sink;
pub mod ranges;
pub mod raster;
pub mod setup;
