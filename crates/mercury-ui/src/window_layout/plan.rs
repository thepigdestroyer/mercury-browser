// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! The plan (task 2 of the layout-switch plan): what a window shows for the stored settings, the
//! window's session values, whether the interface is live and the platform.

use mercury_settings::{AppearanceSettings, DockStart};

use super::{ChromePlan, Layout, Platform, WindowChrome};
use crate::corner_dial::effective_corner;
use crate::group_dock::initial_shrunk;

/// What a window shows for these settings and session values. With `interface_live` false the
/// window is today's window.
pub fn plan(
    appearance: &AppearanceSettings,
    session: &WindowChrome,
    interface_live: bool,
    platform: Platform,
) -> ChromePlan {
    if !interface_live {
        return ChromePlan::todays_window();
    }
    let groups = &appearance.tab_groups;
    let layout = Layout::from_id(&appearance.layout);
    let show_dock = match layout {
        Layout::Mercury => groups.dock,
        Layout::Classic => groups.dock_in_classic,
    };
    let show_hub = groups.ring;
    ChromePlan {
        layout,
        show_tab_row: layout == Layout::Classic || (!show_dock && !show_hub),
        show_hub,
        show_dock,
        dock_starts_shrunk: initial_shrunk(
            session.dock_shrunk,
            groups.dock_starts == DockStart::Shrunk,
        ),
        show_dial: groups.dial,
        dial_corner: effective_corner(session.dial_corner.as_deref(), groups.dial_corner),
        orbit_enabled: groups.orbit,
        orbit_pinch: groups.orbit_pinch && groups.orbit,
        integrated_title_bar: platform == Platform::MacOs,
    }
}
