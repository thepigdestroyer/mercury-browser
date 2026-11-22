// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! Share the page (MD23): what may be shared, and the share sheet behind a trait.

use std::cell::RefCell;

use talaria_core::BrowserController;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ShareItem {
    pub(crate) url: String,
    pub(crate) title: String,
}

/// The item to share for a page, or `None` for a page that is never shared.
pub(crate) fn share_item(url: &str, title: &str) -> Option<ShareItem> {
    let _ = (url, title);
    None
}

pub(crate) trait Sharer {
    /// True if the share sheet was shown.
    fn share(&self, item: &ShareItem) -> bool;
}

#[derive(Default)]
pub(crate) struct RecordingSharer {
    pub(crate) shared: RefCell<Vec<ShareItem>>,
}

impl Sharer for RecordingSharer {
    fn share(&self, item: &ShareItem) -> bool {
        self.shared.borrow_mut().push(item.clone());
        false
    }
}

/// Shares the active tab's page; false if it cannot be shared.
pub(crate) fn share_active_page(controller: &BrowserController, sharer: &dyn Sharer) -> bool {
    let _ = (controller, sharer);
    false
}
