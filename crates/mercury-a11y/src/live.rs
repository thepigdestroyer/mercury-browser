// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! Live regions: which nodes are live, what text was added, and what is spoken, with a per-second
//! budget.

use a11y_dom::Rendering;

use crate::host::HostState;

/// How urgently a live region is spoken.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Politeness {
    Off,
    Polite,
    Assertive,
}

/// The live-region attributes of one node.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LiveState {
    pub politeness: Politeness,
    pub atomic: bool,
    pub busy: bool,
}

// stub
pub fn live_state<'n, D: Rendering + HostState>(_doc: &'n D, _node: D::N<'n>) -> Option<LiveState> {
    None
}

// stub
pub fn diff_added(_old: &str, _new: &str, _limit: usize) -> Option<String> {
    None
}

/// Text to speak, and whether it interrupts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Spoken {
    pub text: String,
    pub assertive: bool,
}

/// Remembers the last text of each live region and how much is spoken per second.
pub struct Announcer {
    _max_per_second: u32,
}

impl Announcer {
    // stub
    pub fn new(max_per_second: u32) -> Self {
        Self {
            _max_per_second: max_per_second,
        }
    }

    // stub
    pub fn observe(
        &mut self,
        _now_ms: u64,
        _key: u64,
        _state: &LiveState,
        _text: &str,
        _initial: bool,
    ) -> Option<Spoken> {
        None
    }

    // stub
    pub fn forget(&mut self, _key: u64) {}
}
