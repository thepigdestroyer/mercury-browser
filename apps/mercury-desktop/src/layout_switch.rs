// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! The layout switch, desktop side (lane layout-switch, plan tasks 7 to 9): pushes the plan into
//! the window, feeds the dock and the dial, runs the dock's settle timer, saves the per-window
//! choices and answers the View menu.
//!
//! **SCAFFOLD:** the four functions the menus call are no-ops until task 8 fills them.
#![allow(dead_code)] // layout-switch: filled in by tasks 7 to 9

use std::cell::RefCell;

use mercury_ui::MainWindow;
use talaria_core::BrowserController;

/// View > Show Tab Ring.
pub(crate) fn show_tab_ring(_window: &MainWindow, _controller: &RefCell<BrowserController>) {}

/// View > Shrink Dock / Show Dock (⌃⌘B).
pub(crate) fn toggle_dock(_window: &MainWindow, _controller: &RefCell<BrowserController>) {}

/// View > Show Tab Dial (⌘⇧E).
pub(crate) fn toggle_dial(_window: &MainWindow, _controller: &RefCell<BrowserController>) {}
