// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! Where the viewer's Copy puts the selected text.
//!
//! The viewer writes to a [`ClipboardSink`], never to the clipboard directly: the real one is the
//! system clipboard, an automated run (`--screenshot`, `--background`) gets one in memory so that
//! it never replaces what the user copied, and tests use the memory one too. Nothing in a test
//! touches the user's clipboard.

use std::cell::RefCell;

/// A place to put copied text.
pub(super) trait ClipboardSink {
    fn write_text(&self, text: &str);
}

/// A clipboard shared by several owners (tests keep a handle to read it back).
impl<T: ClipboardSink + ?Sized> ClipboardSink for std::rc::Rc<T> {
    fn write_text(&self, text: &str) {
        (**self).write_text(text);
    }
}

/// The system clipboard (`NSPasteboard.generalPasteboard` on macOS), opened at its first use.
#[derive(Default)]
pub(super) struct SystemClipboard {
    clipboard: RefCell<Option<arboard::Clipboard>>,
}

impl ClipboardSink for SystemClipboard {
    fn write_text(&self, text: &str) {
        let mut clipboard = self.clipboard.borrow_mut();
        if clipboard.is_none() {
            match arboard::Clipboard::new() {
                Ok(opened) => *clipboard = Some(opened),
                Err(error) => {
                    log::warn!("the system clipboard is not available: {error}");
                    return;
                }
            }
        }
        if let Some(clipboard) = clipboard.as_mut()
            && let Err(error) = clipboard.set_text(text)
        {
            log::warn!("the PDF viewer could not copy to the clipboard: {error}");
        }
    }
}

/// A clipboard in memory: for automated runs and for tests.
#[derive(Default)]
pub(super) struct MemoryClipboard {
    text: RefCell<Option<String>>,
}

impl MemoryClipboard {
    /// The last text written, if any.
    #[cfg(test)]
    pub(super) fn text(&self) -> Option<String> {
        self.text.borrow().clone()
    }
}

impl ClipboardSink for MemoryClipboard {
    fn write_text(&self, text: &str) {
        *self.text.borrow_mut() = Some(text.to_owned());
    }
}

/// The clipboard for this run: the system's, except in an automated run.
pub(super) fn clipboard_for_run(automated: bool) -> Box<dyn ClipboardSink> {
    if automated {
        Box::new(MemoryClipboard::default())
    } else {
        Box::new(SystemClipboard::default())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_memory_clipboard_keeps_the_last_text() {
        let clipboard = MemoryClipboard::default();
        assert_eq!(clipboard.text(), None);
        clipboard.write_text("one");
        clipboard.write_text("two\nlines");
        assert_eq!(clipboard.text().as_deref(), Some("two\nlines"));
    }
}
