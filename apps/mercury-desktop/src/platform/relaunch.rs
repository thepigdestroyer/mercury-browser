// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! Relaunch after a macOS restart (MM16): the application opts in explicitly. Whether the tabs
//! come back is the start-up setting's business.

use std::cell::Cell;

pub(crate) trait RelaunchOnLogin {
    fn enable(&self);
}

#[derive(Default)]
pub(crate) struct RecordingRelaunch {
    pub(crate) enabled: Cell<u32>,
}

impl RelaunchOnLogin for RecordingRelaunch {
    fn enable(&self) {
        self.enabled.set(self.enabled.get() + 1);
    }
}
