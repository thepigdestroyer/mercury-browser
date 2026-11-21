// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! The Orbit overview's glue (SPEC §4.6, §6.1; orbit-view brief §6 item 5): opening takes a fresh
//! picture of the active tab, keeps the pictures as `slint::Image`s while Orbit is open, runs the
//! machine (`mercury_ui::orbit_view`), turns its effects into controller group commands and
//! closes. The pure parts (state machine, layout, drag resolver) are in `mercury-ui`.
//!
//! STUB in the scaffold commit; item 5 `desktop` of lane orbit-view writes it.
#![allow(dead_code)] // used by layout-switch and input-hooks
