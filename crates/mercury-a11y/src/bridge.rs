// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! Which tab's page tree reaches the window: per-tab routing, active tab only (OD-4). A pure state
//! machine over tab numbers and AccessKit types; the shell executes the commands it returns.
//!
//! Owner: item 1 (`graft`).
