// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! The stress, performance and decoder scenarios T01..T12 of lane sp-mse: how GstMSE behaves under
//! load and abuse (see `.superpowers/sdd/lane-sp-mse-brief.md` §6.2 for the table).
//!
//! Each scenario is a `fn(&Ctx) -> ScenarioResult` in its own module under `src/scenarios/`, listed
//! in `SCENARIOS`; the command line (including `--iterations` and `--seed`), child processes,
//! timeouts and the report are `mse_rig::harness`. Scenarios that must survive a crash or a deadlock
//! of what they test (T02, T04, T12) start nested runs of this same executable (`sub`).

mod common;
mod plain;
mod proc;
mod rng;
mod scenarios;
mod stats;
mod sub;
mod synth;
mod vt;
mod watchdog;

use std::time::Duration;

use mse_rig::harness::{self, Priority, ScenarioDef};

const fn secs(n: u64) -> Duration {
    Duration::from_secs(n)
}

static SCENARIOS: &[ScenarioDef] = &[
    ScenarioDef {
        id: "T01",
        title: "Threads and the main loop",
        priority: Priority::P0,
        timeout: secs(90),
        run: scenarios::t01_threads::run,
    },
    ScenarioDef {
        id: "T02",
        title: "Re-entrancy from inside callbacks",
        priority: Priority::P0,
        timeout: secs(280),
        run: scenarios::t02_reentrancy::run,
    },
    ScenarioDef {
        id: "T03",
        title: "Concurrent callers",
        priority: Priority::P1,
        timeout: secs(150),
        run: scenarios::t03_concurrency::run,
    },
    ScenarioDef {
        id: "T04",
        title: "Lifecycle and teardown",
        priority: Priority::P0,
        timeout: secs(300),
        run: scenarios::t04_lifecycle::run,
    },
    ScenarioDef {
        id: "T05",
        title: "Throughput and latency",
        priority: Priority::P0,
        timeout: secs(280),
        run: scenarios::t05_throughput::run,
    },
    ScenarioDef {
        id: "T06",
        title: "Memory, eviction and quota",
        priority: Priority::P1,
        timeout: secs(290),
        run: scenarios::t06_memory::run,
    },
    ScenarioDef {
        id: "T07",
        title: "Decoder choice and speed",
        priority: Priority::P1,
        timeout: secs(240),
        run: scenarios::t07_decoders::run,
    },
    ScenarioDef {
        id: "T08",
        title: "Protection events (EME)",
        priority: Priority::P1,
        timeout: secs(90),
        run: scenarios::t08_protection::run,
    },
    ScenarioDef {
        id: "T09",
        title: "Live-style stream",
        priority: Priority::P2,
        timeout: secs(150),
        run: scenarios::t09_live::run,
    },
    ScenarioDef {
        id: "T10",
        title: "Many rigs at once",
        priority: Priority::P1,
        timeout: secs(240),
        run: scenarios::t10_many::run,
    },
    ScenarioDef {
        id: "T11",
        title: "Clock, A/V sync and drops",
        priority: Priority::P1,
        timeout: secs(180),
        run: scenarios::t11_sync::run,
    },
    ScenarioDef {
        id: "T12",
        title: "Fuzz",
        priority: Priority::P2,
        timeout: secs(300),
        run: scenarios::t12_fuzz::run,
    },
];

fn main() -> std::process::ExitCode {
    harness::main_for("mse-stress", SCENARIOS)
}
