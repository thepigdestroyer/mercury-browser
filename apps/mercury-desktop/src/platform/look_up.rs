// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! Look Up (MD41): the dictionary popover for the selected text.

use std::cell::RefCell;
use std::rc::Rc;

use mercury_ui::MainWindow;
use talaria_core::BrowserController;

/// The text to look up for a selection, or `None` if there is nothing sensible to look up.
pub(crate) fn look_up_text(selection: &str) -> Option<String> {
    let _ = selection;
    None
}

pub(crate) trait Dictionary {
    /// True if the popover was shown.
    fn show(&self, text: &str) -> bool;
}

#[derive(Default)]
pub(crate) struct RecordingDictionary {
    pub(crate) shown: RefCell<Vec<String>>,
}

impl Dictionary for RecordingDictionary {
    fn show(&self, text: &str) -> bool {
        self.shown.borrow_mut().push(text.to_owned());
        false
    }
}

/// Looks up the selection of the PDF viewer or of the page.
pub(crate) fn look_up_selection(
    window: &MainWindow,
    controller: Rc<RefCell<BrowserController>>,
    dictionary: Rc<dyn Dictionary>,
) {
    let _ = (window, controller, dictionary);
}
