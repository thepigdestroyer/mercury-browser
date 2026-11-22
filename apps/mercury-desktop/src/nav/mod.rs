// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! Navigation basics (lane nav-basics, R13 W1-6): the back and forward list popup, ⌘-click and
//! middle-click on links, Open File and file drops, the quit warning. The replaceable engine
//! stand-ins (Stop, reload bypassing the cache) are in `talaria-servo/src/nav_interim.rs`.
//!
//! Each submodule has one `install`, called from here, so the shared files only get one line.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use mercury_ui::MainWindow;
use talaria_core::BrowserController;

mod click_open;
mod file_open;
mod history_popup;
mod modifiers;
mod quit_warning;

/// What every submodule's `install` may need.
pub(crate) struct Context<'a> {
    pub(crate) window: &'a MainWindow,
    pub(crate) controller: &'a Rc<RefCell<BrowserController>>,
    pub(crate) settings: &'a Rc<crate::settings::SettingsHandle>,
    pub(crate) web_view_focused: &'a Rc<Cell<bool>>,
    /// `--exit-after`, `--screenshot` and other diagnostics: no prompts, no panels.
    pub(crate) automated: bool,
}

pub(crate) fn install(cx: &Context<'_>) {
    history_popup::install(cx);
    click_open::install(cx);
    file_open::install(cx);
    quit_warning::install(cx);
}
