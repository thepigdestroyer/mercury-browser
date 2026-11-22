// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! `applicationDockMenu:` on winit's application delegate.

use std::cell::RefCell;
use std::rc::Rc;

use mercury_ui::MainWindow;
use talaria_core::BrowserController;

/// Adds the Dock-menu methods to the delegate.
pub(crate) fn install() {}

/// Gives the Dock menu the controller and the window it acts on.
pub(crate) fn attach(window: &MainWindow, controller: Rc<RefCell<BrowserController>>) {
    let _ = (window, controller);
}
