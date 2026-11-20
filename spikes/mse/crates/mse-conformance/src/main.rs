// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! The conformance scenarios C01..C15 of lane sp-mse: does GstMSE do what an MSE player needs.
//! OWNER: item 3 (see `.superpowers/sdd/lane-sp-mse-brief.md` §6.1 for the scenario table).
//!
//! Each scenario is a `fn(&Ctx) -> ScenarioResult` in its own module under `src/scenarios/` and is
//! listed in `SCENARIOS`; the command line, child processes, timeouts and the report are
//! `mse_rig::harness`. Shared helpers: `support` (recorder, event and colour helpers, feeding
//! segments), `play` (the C02 play-through and its checks), `probes` (seek measurements),
//! `initinfo` (what an init segment declares).
//!
//! Fail means "the scenario ran correctly and GstMSE did the wrong thing", with the exact events,
//! times and counts in `notes`; Error means the harness broke.

mod initinfo;
mod isolate;
mod play;
mod probes;
mod scenarios;
mod support;
#[cfg(test)]
mod tests;

use std::time::Duration;

use mse_rig::harness::{self, Priority, ScenarioDef};

use scenarios::{c01, c02, c03, c04, c05, c06, c07, c08, c09, c10, c11, c12, c13, c14, c15};

const fn def(
    id: &'static str,
    title: &'static str,
    priority: Priority,
    timeout_s: u64,
    run: harness::ScenarioFn,
) -> ScenarioDef {
    ScenarioDef {
        id,
        title,
        priority,
        timeout: Duration::from_secs(timeout_s),
        run,
    }
}

static SCENARIOS: &[ScenarioDef] = &[
    def("C01", c01::TITLE, Priority::P0, 60, c01::run),
    def("C02", c02::TITLE, Priority::P0, 150, c02::run),
    def("C03", c03::TITLE, Priority::P1, 60, c03::run),
    def("C04", c04::TITLE, Priority::P1, 120, c04::run),
    def("C05", c05::TITLE, Priority::P1, 120, c05::run),
    def("C06", c06::TITLE, Priority::P0, 90, c06::run),
    def("C07", c07::TITLE, Priority::P0, 120, c07::run),
    def("C08", c08::TITLE, Priority::P0, 90, c08::run),
    def("C09", c09::TITLE, Priority::P1, 60, c09::run),
    def("C10", c10::TITLE, Priority::P0, 120, c10::run),
    def("C11", c11::TITLE, Priority::P0, 120, c11::run),
    def("C12", c12::TITLE, Priority::P1, 90, c12::run),
    def("C13", c13::TITLE, Priority::P1, 300, c13::run),
    def("C14", c14::TITLE, Priority::P1, 90, c14::run),
    def("C15", c15::TITLE, Priority::P1, 240, c15::run),
];

fn main() -> std::process::ExitCode {
    harness::main_for("mse-conformance", SCENARIOS)
}
