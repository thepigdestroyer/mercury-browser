// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! The scenario runner both binaries share (item 1), so that items 3 and 4 write scenarios only.
//!
//! `main_for` parses the command line (`--list`, `--only C01,C04`, `--shape bare|playbin`,
//! `--plugin-mode system|servo`, `--glib-main-loop`, `--report FILE`, `--scratch DIR`,
//! `--iterations N`, `--seed N`), and runs **every scenario in a child process of the same
//! executable** (`<exe> --child <ID> <the same options>`), so that a crash, an abort or a hang in
//! GStreamer costs one scenario and not the report. The child initialises GStreamer
//! (`env::init`), calls the scenario function, catches panics, and prints one JSON line
//! (`ScenarioResult`) as the last line of its standard output. The parent enforces
//! `ScenarioDef::timeout` (kills the child), turns a missing result, a signal or a timeout into
//! `Status::Error` with the exit status in `notes`, builds the `Report`, prints `Report::summary`,
//! writes `--report`, and returns exit status 0 unless some scenario is `Error` (2 for usage errors).
//!
//! The parent never initialises GStreamer itself.

use std::path::PathBuf;
use std::time::Duration;

use crate::env::{GstInfo, PluginMode};
use crate::report::ScenarioResult;
use crate::rig::{RigConfig, Shape};

/// P0 must be done and reported; P1 next; P2 if time remains (the brief's §6).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Priority {
    P0,
    P1,
    P2,
}

/// What a scenario function gets: how the run was configured, and a scratch directory of its own.
#[derive(Debug, Clone)]
pub struct Ctx {
    pub shape: Shape,
    pub plugin_mode: PluginMode,
    pub glib_main_loop: bool,
    /// Created before the scenario runs, removed after it, private to it.
    pub scratch: PathBuf,
    /// `--iterations` (default 1; the repeated scenarios choose their own default when it is 1).
    pub iterations: usize,
    /// `--seed` (default: derived from the time and printed in `notes` by the scenario that uses it).
    pub seed: u64,
    pub info: GstInfo,
}

impl Ctx {
    /// A `RigConfig` with this run's shape and everything else at its default.
    pub fn rig_config(&self) -> RigConfig {
        RigConfig {
            shape: self.shape,
            ..RigConfig::default()
        }
    }
}

/// A scenario: it returns its result (and sets `status`); it must not call `process::exit`.
pub type ScenarioFn = fn(&Ctx) -> ScenarioResult;

#[derive(Debug, Clone, Copy)]
pub struct ScenarioDef {
    pub id: &'static str,
    pub title: &'static str,
    pub priority: Priority,
    /// Wall-clock limit for the child process; 60 s unless the scenario is long.
    pub timeout: Duration,
    pub run: ScenarioFn,
}

/// The whole `main` of a scenario binary: `fn main() -> ExitCode { harness::main_for("mse-conformance", SCENARIOS) }`.
pub fn main_for(_tool: &'static str, _scenarios: &'static [ScenarioDef]) -> std::process::ExitCode {
    todo!("item 1")
}
