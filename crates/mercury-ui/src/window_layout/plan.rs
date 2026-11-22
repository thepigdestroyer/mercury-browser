// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! The plan (task 2 of the layout-switch plan). SCAFFOLD: returns today's window until filled in.

use mercury_settings::AppearanceSettings;

use super::{ChromePlan, Platform, WindowChrome};

/// What a window shows for these settings and session values. With `interface_live` false the
/// window is today's window.
pub fn plan(
    _appearance: &AppearanceSettings,
    _session: &WindowChrome,
    _interface_live: bool,
    _platform: Platform,
) -> ChromePlan {
    ChromePlan::todays_window()
}
