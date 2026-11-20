// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! The ⌥ group keys, desktop side (SPEC design/ui-final/SPEC.md §5.1; lane brief
//! `lane-key-switcher-brief.md` §6 item 5).
//!
//! The state machine is the crate `mercury-option-keys`: pure, with injected time, tested without
//! Servo. This module is the thin glue the `input-hooks` lane will call from `keyboard.rs`:
//! winit's plain enums to the crate's `Key`, `Mods` and `Event`, the clock, the hold timer's
//! delay, and the machine itself in a thread-local. Nothing calls it yet.
//!
//! GROUNDWORK STUB. Item 5 `desktop-glue` writes the functions listed in the brief. Remove this
//! paragraph when you do.

#![allow(dead_code)] // used by the input-hooks lane
