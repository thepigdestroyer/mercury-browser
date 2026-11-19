// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! The conformance scenarios C01..C15 of lane sp-mse: does GstMSE do what an MSE player needs.
//! OWNER: item 3 (see `.superpowers/sdd/lane-sp-mse-brief.md` §6.1 for the scenario table).
//!
//! Write each scenario as a `fn(&Ctx) -> ScenarioResult` in its own module under `src/scenarios/`
//! and list it in `SCENARIOS`; the command line, child processes, timeouts and the report are
//! `mse_rig::harness`.

use mse_rig::harness::{self, ScenarioDef};

/// Filled in by item 3.
static SCENARIOS: &[ScenarioDef] = &[];

fn main() -> std::process::ExitCode {
    harness::main_for("mse-conformance", SCENARIOS)
}
