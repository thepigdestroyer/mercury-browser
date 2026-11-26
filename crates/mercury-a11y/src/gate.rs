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
    min_interval_ms: u64,
    last_sent_ms: Option<u64>,
    pending_layout: bool,
}

impl UpdateGate {
    pub fn new(min_interval_ms: u64) -> Self {
        Self {
            min_interval_ms,
            last_sent_ms: None,
            pending_layout: false,
        }
    }

    /// What to do at `now_ms` with `damage`, which may be `None` when a timer fires with nothing
    /// new. Urgent damage is always sent. Layout-only damage waits until one interval has passed
    /// since the last send, and a `WaitMs` answer means the caller should offer again then.
    pub fn offer(&mut self, now_ms: u64, damage: Option<Damage>) -> Decision {
        if damage == Some(Damage::Urgent) {
            return Decision::Send;
        }
        self.pending_layout |= damage == Some(Damage::LayoutOnly);
        if !self.pending_layout {
            return Decision::Nothing;
        }
        let due = self
            .last_sent_ms
            .map_or(now_ms, |sent| sent.saturating_add(self.min_interval_ms));
        if now_ms >= due {
            Decision::Send
        } else {
            Decision::WaitMs(due - now_ms)
        }
    }

    /// Records that an update was sent at `now_ms`.
    pub fn sent(&mut self, now_ms: u64) {
        self.last_sent_ms = Some(now_ms);
        self.pending_layout = false;
    }
}
