// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! Keeps a covered window rendering in automated runs (orbit-view brief D13): a window that is
//! covered renders only for `take_snapshot`, so a `--background` run never presents the frames that
//! the tab pictures (`tab_snapshots.rs`) are made from. With `MERCURY_AUTOMATED_RENDER_MS` set (50 to
//! 5000) a repeating timer takes a snapshot of the window that often. It counts in automated runs
//! only (`--screenshot`, `--background`, import); anywhere else it is ignored with a warning, and
//! without the variable nothing is created.
//!
//! STUB in the scaffold commit; item 5 `desktop` of lane orbit-view writes it.

use mercury_ui::MainWindow;

/// Starts the driver if `MERCURY_AUTOMATED_RENDER_MS` asks for it in an automated run.
#[allow(dead_code)] // wired by the stub's hook line in main.rs until item 5 fills it in
pub(crate) fn start(_window: &MainWindow, _automated: bool) -> Option<slint::Timer> {
    None
}
