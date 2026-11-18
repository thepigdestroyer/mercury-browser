// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! The general pasteboard, for copying a user name or password from the settings window. Only
//! normal interactive runs use it; tests and automated runs use an in-memory fake.

use mercury_password_manager::Pasteboard;
use objc2_app_kit::{NSPasteboard, NSPasteboardTypeString};
use objc2_foundation::NSString;

/// The marker clipboard managers that follow nspasteboard.org skip and do not keep in their
/// history.
const CONCEALED_TYPE: &str = "org.nspasteboard.ConcealedType";

pub(crate) struct SystemPasteboard;

impl Pasteboard for SystemPasteboard {
    fn write_text(&self, text: &str, concealed: bool) -> Option<i64> {
        let pasteboard = NSPasteboard::generalPasteboard();
        pasteboard.clearContents();
        // SAFETY: an AppKit constant, valid for the life of the process.
        let string_type = unsafe { NSPasteboardTypeString };
        if !pasteboard.setString_forType(&NSString::from_str(text), string_type) {
            return None;
        }
        if concealed {
            let _marked = pasteboard
                .setString_forType(&NSString::from_str(""), &NSString::from_str(CONCEALED_TYPE));
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
