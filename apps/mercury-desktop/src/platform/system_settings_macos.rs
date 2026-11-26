// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! Opens System Settings panes through `NSWorkspace`. Installed by `platform::attach` in
//! interactive runs only.

use objc2::rc::Retained;
use objc2::runtime::{AnyObject, Bool};
use objc2::{class, msg_send};
use objc2_foundation::NSString;

use super::system_settings::{SettingsOpener, SettingsPane, pane_url};

pub(crate) struct WorkspaceOpener;

impl SettingsOpener for WorkspaceOpener {
    fn open(&self, pane: SettingsPane) -> bool {
        let string = NSString::from_str(pane_url(pane));
        // SAFETY: `URLWithString:` takes an `NSString` and returns an `NSURL`, or nil; `openURL:`
        // takes an `NSURL` and returns a `BOOL`. The URL is one of the fixed pane addresses.
        unsafe {
            let url: Option<Retained<AnyObject>> =
                msg_send![class!(NSURL), URLWithString: &*string];
            let Some(url) = url else {
                return false;
            };
            let workspace: Retained<AnyObject> = msg_send![class!(NSWorkspace), sharedWorkspace];
            let opened: Bool = msg_send![&workspace, openURL: &*url];
            opened.as_bool()
        }
    }
}
