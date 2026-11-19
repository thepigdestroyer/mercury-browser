// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! The general pasteboard, for copying a user name or password from the settings window. Only
//! normal interactive runs use it; tests and automated runs use an in-memory fake.

use mercury_password_manager::Pasteboard;
use objc2::runtime::ProtocolObject;
use objc2_app_kit::{
    NSPasteboard, NSPasteboardContentsOptions, NSPasteboardItem, NSPasteboardTypeString,
    NSPasteboardWriting,
};
use objc2_foundation::{NSArray, NSString};

/// Markers that clipboard managers following nspasteboard.org honour: a concealed item is kept
/// out of their history, a transient one is not kept at all.
const CONCEALED_TYPE: &str = "org.nspasteboard.ConcealedType";
const TRANSIENT_TYPE: &str = "org.nspasteboard.TransientType";

pub(crate) struct SystemPasteboard;

impl Pasteboard for SystemPasteboard {
    fn write_text(&self, text: &str, concealed: bool) -> Option<i64> {
        write_to(&NSPasteboard::generalPasteboard(), text, concealed)
    }

    fn change_count(&self) -> i64 {
        i64::try_from(NSPasteboard::generalPasteboard().changeCount()).unwrap_or_default()
    }

    fn clear(&self) {
        NSPasteboard::generalPasteboard().clearContents();
    }
}

/// Replaces the contents of `pasteboard` with `text` and returns its change count, or `None` if
/// AppKit refused the write. Split out of `write_text` so that a check can aim it at a private
/// pasteboard instead of the user's general one.
fn write_to(pasteboard: &NSPasteboard, text: &str, concealed: bool) -> Option<i64> {
    // SAFETY: an AppKit constant, valid for the life of the process.
    let string_type = unsafe { NSPasteboardTypeString };
    if concealed {
        // This Mac only: never handed to Universal Clipboard. This also empties the
        // pasteboard, and nothing after it resets the option: the data goes in with one
        // `writeObjects:`, not `declareTypes:owner:`.
        pasteboard.prepareForNewContentsWithOptions(NSPasteboardContentsOptions::CurrentHostOnly);
        // One item carries the string and both markers, written at once: a clipboard
        // manager never sees the string without them.
        let item = NSPasteboardItem::new();
        let empty = NSString::from_str("");
        if !item.setString_forType(&NSString::from_str(text), string_type)
            || !item.setString_forType(&empty, &NSString::from_str(CONCEALED_TYPE))
            || !item.setString_forType(&empty, &NSString::from_str(TRANSIENT_TYPE))
        {
            return None;
        }
        let items = NSArray::from_retained_slice(&[
            ProtocolObject::<dyn NSPasteboardWriting>::from_retained(item),
        ]);
        if !pasteboard.writeObjects(&items) {
            return None;
        }
    } else {
        pasteboard.clearContents();
        if !pasteboard.setString_forType(&NSString::from_str(text), string_type) {
            return None;
        }
    }
    Some(i64::try_from(pasteboard.changeCount()).unwrap_or_default())
}
