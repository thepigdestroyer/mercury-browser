// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! What the main window shows and how the tab-group overlays are fed (lane layout-switch,
//! `.superpowers/sdd/plans/layout-switch-plan.md`).
//!
//! - [`plan`] turns the stored settings and the window's session values into a [`ChromePlan`]:
//!   the layout, whether the tab row, the hub button, the dock and the corner dial show.
//! - [`WindowFeed`] fills the dock and dial bridge globals (`ui/layout/bridges.slint`) once and
//!   only when something changed. The desktop app and the headless renders use the same feed.
//!
//! **SCAFFOLD:** the types and signatures are frozen; the bodies are inert until tasks 2 and 4 of
//! the plan fill them.

mod feed;
mod plan;

pub use feed::{DialFace, WindowFeed, WindowRows};
pub use plan::plan;

use mercury_settings::DialCorner;

use crate::WindowLayout;

/// The two layouts (`appearance.layout`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Layout {
    /// The toolbar on top, the group dock at the bottom, no tab strip.
    #[default]
    Mercury,
    /// The tab strip and the toolbar, with group labels.
    Classic,
}

impl Layout {
    /// The layout a stored id names; anything unknown is [`Layout::Mercury`].
    pub fn from_id(id: &str) -> Layout {
        match id {
            "classic" => Layout::Classic,
            _ => Layout::Mercury,
        }
    }

    /// The id stored in `settings.json`.
    pub fn id(self) -> &'static str {
        match self {
            Layout::Mercury => "mercury",
            Layout::Classic => "classic",
        }
    }

    /// The Slint enum of `MainWindow.layout`.
    pub fn to_slint(self) -> WindowLayout {
        match self {
            Layout::Mercury => WindowLayout::Mercury,
            Layout::Classic => WindowLayout::Classic,
        }
    }
}

/// The platform, for the integrated title bar.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Platform {
    MacOs,
    Other,
}

/// The window's own values from `session.json`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WindowChrome {
    /// `WindowSnapshot.dock_shrunk`: `None` follows `appearance.tab_groups.dock_starts`.
    pub dock_shrunk: Option<bool>,
    /// `WindowSnapshot.dial_corner` ("top_left" ... "bottom_right"): `None` follows the setting.
    pub dial_corner: Option<String>,
}

/// What a window shows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChromePlan {
    pub layout: Layout,
    /// The strip row above the toolbar (Classic, or Mercury with neither dock nor hub).
    pub show_tab_row: bool,
    pub show_hub: bool,
    pub show_dock: bool,
    pub dock_starts_shrunk: bool,
    pub show_dial: bool,
    pub dial_corner: DialCorner,
    pub orbit_enabled: bool,
    pub orbit_pinch: bool,
    /// The traffic lights sit inside the first chrome row.
    pub integrated_title_bar: bool,
}

impl ChromePlan {
    /// Today's window: Classic, the tab row, nothing of the tab-group interface.
    pub fn todays_window() -> ChromePlan {
        ChromePlan {
            layout: Layout::Classic,
            show_tab_row: true,
            show_hub: false,
            show_dock: false,
            dock_starts_shrunk: false,
            show_dial: false,
            dial_corner: DialCorner::default(),
            orbit_enabled: false,
            orbit_pinch: false,
            integrated_title_bar: false,
        }
    }
}
