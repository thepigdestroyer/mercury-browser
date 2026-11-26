// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! Coalescing of tree updates: at most one send per interval, with layout-only damage waiting for
//! the next slot.

/// How much of the tree changed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Damage {
    Urgent,
    LayoutOnly,
}

/// What to do with an offered update.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Decision {
    Nothing,
    Send,
    WaitMs(u64),
}

/// Decides when the next tree update may be sent.
pub struct UpdateGate {
    _min_interval_ms: u64,
}

impl UpdateGate {
    // stub
    pub fn new(min_interval_ms: u64) -> Self {
        Self {
            _min_interval_ms: min_interval_ms,
        }
    }

    // stub
    pub fn offer(&mut self, _now_ms: u64, _damage: Option<Damage>) -> Decision {
        Decision::Nothing
    }

    // stub
    pub fn sent(&mut self, _now_ms: u64) {}
}
