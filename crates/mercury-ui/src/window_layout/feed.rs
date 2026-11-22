// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! The feed (task 4 of the layout-switch plan). SCAFFOLD: every method is inert until filled in.

use slint::{Color, Image, SharedString};

use super::ChromePlan;
use crate::ring_geometry::GlyphMetrics;
use crate::{GroupRow, MainWindow, TabRow};

/// The dial's rest face: the active tab's page and group.
#[derive(Clone, Debug, Default)]
pub struct DialFace {
    pub active_colour: Color,
    pub favicon: Option<Image>,
    pub fallback_letter: SharedString,
    pub loading: bool,
    /// 0..=1; -1 = unknown.
    pub progress: f32,
    pub label: SharedString,
}

/// Everything the overlays are built from.
#[derive(Clone, Debug, Default)]
pub struct WindowRows {
    pub groups: Vec<GroupRow>,
    pub tabs: Vec<TabRow>,
    pub dark: bool,
    /// The window's content width (the dock is centred in it).
    pub container_width: f32,
    /// The page area (the dial's overlay), logical px.
    pub area: (f32, f32),
    pub held: bool,
    pub show_keys: bool,
    pub reduce_motion: bool,
    pub dial: DialFace,
}

/// Fills the dock and dial bridges once and only on change.
pub struct WindowFeed {
    _metrics: Box<dyn GlyphMetrics>,
}

impl WindowFeed {
    /// Sets every model and handler of the bridges once.
    pub fn install(_window: &MainWindow, metrics: Box<dyn GlyphMetrics>) -> WindowFeed {
        WindowFeed { _metrics: metrics }
    }

    /// Applies a plan (the dial's corner, the dock's first state).
    pub fn set_plan(&self, _window: &MainWindow, _plan: &ChromePlan) {}

    /// Rebuilds what the rows changed; writes nothing when nothing changed.
    pub fn set_rows(&self, _window: &MainWindow, _rows: &WindowRows) {}

    /// The dock was shrunk or expanded (by the user or the desktop).
    pub fn dock_state_changed(&self, _window: &MainWindow, _shrunk: bool) {}
}
