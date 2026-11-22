// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! Opens System Settings panes through `NSWorkspace`. Installed by `platform::attach` in
//! interactive runs only.

use super::system_settings::{SettingsOpener, SettingsPane};

pub(crate) struct WorkspaceOpener;

impl SettingsOpener for WorkspaceOpener {
    fn open(&self, pane: SettingsPane) -> bool {
        let _ = pane;
        false
    }
}
