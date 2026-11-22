// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! The Dock menu (MM07): the entries, built from the controller, and what choosing one does.

use std::cell::RefCell;

use talaria_core::{BrowserController, TabId};

pub(crate) const MAX_DOCK_TABS: usize = 10;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum DockEntry {
    NewTab,
    Separator,
    Tab {
        id: TabId,
        title: String,
        active: bool,
    },
}

/// The Dock menu for `tabs` (id, title, URL), the active one being `active`.
pub(crate) fn dock_entries(tabs: &[(TabId, String, String)], active: TabId) -> Vec<DockEntry> {
    let _ = (tabs, active);
    vec![DockEntry::NewTab]
}

/// The entries for the controller as it is now; only "New Tab" while it is borrowed.
pub(crate) fn entries_for(controller: &RefCell<BrowserController>) -> Vec<DockEntry> {
    let _ = controller;
    vec![DockEntry::NewTab]
}

pub(crate) trait DockActions {
    fn new_tab(&self);
    fn select_tab(&self, id: TabId);
}

#[derive(Default)]
pub(crate) struct RecordingDockActions {
    pub(crate) calls: RefCell<Vec<String>>,
}

impl DockActions for RecordingDockActions {
    fn new_tab(&self) {
        self.calls.borrow_mut().push("new_tab".to_owned());
    }

    fn select_tab(&self, id: TabId) {
        self.calls
            .borrow_mut()
            .push(format!("select_tab {}", id.value()));
    }
}
