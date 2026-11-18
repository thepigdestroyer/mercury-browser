// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! The general pasteboard, for copying a user name or password from the settings window. Only
//! normal interactive runs use it; tests and automated runs use an in-memory fake.

use mercury_password_manager::Pasteboard;
use objc2_app_kit::{NSPasteboard, NSPasteboardContentsOptions, NSPasteboardTypeString};
use objc2_foundation::{NSArray, NSString};

/// Markers that clipboard managers following nspasteboard.org honour: a concealed item is kept
/// out of their history, a transient one is not kept at all.
const CONCEALED_TYPE: &str = "org.nspasteboard.ConcealedType";
const TRANSIENT_TYPE: &str = "org.nspasteboard.TransientType";

pub(crate) struct SystemPasteboard;

impl Pasteboard for SystemPasteboard {
    fn write_text(&self, text: &str, concealed: bool) -> Option<i64> {
        let pasteboard = NSPasteboard::generalPasteboard();
        // SAFETY: an AppKit constant, valid for the life of the process.
        let string_type = unsafe { NSPasteboardTypeString };
        if concealed {
            // This Mac only: never handed to Universal Clipboard.
            pasteboard
                .prepareForNewContentsWithOptions(NSPasteboardContentsOptions::CurrentHostOnly);
            // Every type is declared, markers included, before any data is set, so a clipboard
            // manager that looks in between already sees the markers.
            let types = NSArray::from_retained_slice(&[
                NSString::from_str(&string_type.to_string()),
                NSString::from_str(CONCEALED_TYPE),
                NSString::from_str(TRANSIENT_TYPE),
            ]);
            // SAFETY: no owner object.
            unsafe { pasteboard.declareTypes_owner(&types, None) };
            let _concealed = pasteboard
                .setString_forType(&NSString::from_str(""), &NSString::from_str(CONCEALED_TYPE));
            let _transient = pasteboard
                .setString_forType(&NSString::from_str(""), &NSString::from_str(TRANSIENT_TYPE));
        } else {
            pasteboard.clearContents();
        }
        if !pasteboard.setString_forType(&NSString::from_str(text), string_type) {
            return None;
        }
        Some(i64::try_from(pasteboard.changeCount()).unwrap_or_default())
    }

    fn change_count(&self) -> i64 {
        i64::try_from(NSPasteboard::generalPasteboard().changeCount()).unwrap_or_default()
    }

    fn clear(&self) {
        NSPasteboard::generalPasteboard().clearContents();
    }
}
