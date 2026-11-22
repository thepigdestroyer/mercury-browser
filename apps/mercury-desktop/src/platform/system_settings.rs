// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! Links that open System Settings panes (MM12): where macOS keeps the camera, microphone,
//! location and notification permissions, and the Login Items list.
//!
//! The opener is a trait. The thread-local default refuses (and logs), so a test or an automated
//! run never opens System Settings; `platform::attach` installs the real one for interactive runs.

use std::cell::RefCell;
use std::rc::Rc;

use talaria_core::PagePermission;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SettingsPane {
    Camera,
    Microphone,
    Location,
    Notifications,
    LoginItems,
}

/// The `x-apple.systempreferences:` URL of `pane`.
pub(crate) fn pane_url(pane: SettingsPane) -> &'static str {
    let _ = pane;
    ""
}

/// The pane that lets the user change `permission` for the application, if there is one.
pub(crate) fn pane_for_permission(permission: PagePermission) -> Option<SettingsPane> {
    let _ = permission;
    None
}

pub(crate) trait SettingsOpener {
    /// True if the pane was opened.
    fn open(&self, pane: SettingsPane) -> bool;
}

/// Remembers what it was asked to open and opens nothing.
#[derive(Default)]
pub(crate) struct RecordingSettingsOpener {
    pub(crate) opened: RefCell<Vec<SettingsPane>>,
}

impl SettingsOpener for RecordingSettingsOpener {
    fn open(&self, pane: SettingsPane) -> bool {
        self.opened.borrow_mut().push(pane);
        false
    }
}

thread_local! {
    static OPENER: RefCell<Option<Rc<dyn SettingsOpener>>> = const { RefCell::new(None) };
}

pub(crate) fn set_opener(opener: Rc<dyn SettingsOpener>) {
    OPENER.with(|current| *current.borrow_mut() = Some(opener));
}

/// Opens `pane` through the installed opener; without one, nothing is opened.
pub(crate) fn open_pane(pane: SettingsPane) {
    let opener = OPENER.with(|current| current.borrow().clone());
    match opener {
        Some(opener) => {
            opener.open(pane);
        }
        None => log::debug!("system settings: no opener installed; not opening {pane:?}"),
    }
}
