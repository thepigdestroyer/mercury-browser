// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! One module per scenario of the brief's table (§6.2); `main.rs` lists them with id, title,
//! priority and timeout.

pub mod t01_threads;
pub mod t02_reentrancy;
pub mod t03_concurrency;
pub mod t04_lifecycle;
pub mod t05_throughput;
pub mod t06_memory;
pub mod t07_decoders;
pub mod t08_protection;
pub mod t09_live;
pub mod t10_many;
pub mod t11_sync;
pub mod t12_fuzz;
