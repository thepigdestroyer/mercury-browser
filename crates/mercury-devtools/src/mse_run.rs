// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! `mse-run`: the Media Source Extensions checks that need a window, made short, mechanical and
//! repeatable (the MED-1b brief, D10 to D13). The browser is started once per test, a page
//! reports to the in-process test server (`serve`), and the run is judged here:
//!
//! - `model`, `journal`: the result of a test and the journal that lets `--resume` continue;
//! - `reports`: what a page posts, and how a finished page is judged;
//! - `expect`: `tests/wpt/media-source.expect.toml` and the comparison of a WPT file with it;
//! - `lock`: the check for a locked screen; `watchdog`: SIGTERM, SIGKILL, and state `U`;
//! - `phases`, `runner`, `summary`, `cli`: the plan, the run, the report and the command line;
//! - `verdict`: after the window run, whether the flip conditions F1 to F5 hold (`mse-run verdict`).
//!
//! Nothing here needs a browser to be tested; the tests use a stub browser (a script that posts
//! canned reports) and a fake `ioreg`. How to run it for real: `docs/developer-guide/mse-testing.md`.

pub mod cli;
pub mod expect;
pub mod journal;
pub mod lock;
pub mod model;
pub mod phases;
pub mod reports;
pub mod runner;
pub mod summary;
pub mod verdict;
pub mod watchdog;
