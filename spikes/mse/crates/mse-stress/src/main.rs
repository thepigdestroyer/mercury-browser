// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! The stress, performance and decoder scenarios T01..T12 of lane sp-mse: how GstMSE behaves under
//! load and abuse. OWNER: item 4 (see `.superpowers/sdd/lane-sp-mse-brief.md` §6.2 for the table).
//!
//! Write each scenario as a `fn(&Ctx) -> ScenarioResult` in its own module under `src/scenarios/`
//! and list it in `SCENARIOS`; the command line (including `--iterations` and `--seed`), child
//! processes, timeouts and the report are `mse_rig::harness`.

use mse_rig::harness::{self, ScenarioDef};

/// Filled in by item 4.
static SCENARIOS: &[ScenarioDef] = &[];

fn main() -> std::process::ExitCode {
    harness::main_for("mse-stress", SCENARIOS)
}
