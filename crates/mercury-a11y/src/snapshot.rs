// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! The stable text form of an AccessKit tree, for tests (EE12).
//!
//! Owner: item 3 (`core-states`). The skeleton only fixes the entry point that the headless-Servo
//! harness calls; `dump` returns an empty string until item 3 writes the format.

/// What `dump` prints beyond role, name, states and value.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DumpOptions {
    /// Print each node's bounds, rounded to one pixel. Off by default: geometry makes a snapshot brittle.
    pub bounds: bool,
}

/// The tree as stable text, one line per node, indented by depth.
pub fn dump(_tree: &accesskit_consumer::Tree, _options: &DumpOptions) -> String {
    String::new()
}
