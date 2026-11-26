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
    match pane {
        SettingsPane::Camera => {
            "x-apple.systempreferences:com.apple.preference.security?Privacy_Camera"
        }
        SettingsPane::Microphone => {
            "x-apple.systempreferences:com.apple.preference.security?Privacy_Microphone"
        }
        SettingsPane::Location => {
            "x-apple.systempreferences:com.apple.preference.security?Privacy_LocationServices"
        }
        SettingsPane::Notifications => {
            "x-apple.systempreferences:com.apple.Notifications-Settings.extension"
        }
        SettingsPane::LoginItems => {
            "x-apple.systempreferences:com.apple.LoginItems-Settings.extension"
        }
    }
}

/// The pane that lets the user change `permission` for the application, if there is one.
pub(crate) fn pane_for_permission(permission: PagePermission) -> Option<SettingsPane> {
    match permission {
        PagePermission::Camera => Some(SettingsPane::Camera),
        PagePermission::Microphone => Some(SettingsPane::Microphone),
        PagePermission::Geolocation => Some(SettingsPane::Location),
        PagePermission::Notifications => Some(SettingsPane::Notifications),
        _ => None,
    }
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

/// The default in every thread: logs what it would open and opens nothing.
pub(crate) struct RefusingOpener;

impl SettingsOpener for RefusingOpener {
    fn open(&self, pane: SettingsPane) -> bool {
        log::debug!("system settings: not opening {pane:?} (no opener installed)");
        false
    }
}

thread_local! {
    static OPENER: RefCell<Rc<dyn SettingsOpener>> = RefCell::new(Rc::new(RefusingOpener));
}

pub(crate) fn set_opener(opener: Rc<dyn SettingsOpener>) {
    OPENER.with(|current| *current.borrow_mut() = opener);
}

/// Opens `pane` through the installed opener (refused until `set_opener` is called).
pub(crate) fn open_pane(pane: SettingsPane) {
    let opener = OPENER.with(|current| current.borrow().clone());
    opener.open(pane);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pane_urls_are_the_system_settings_ones() {
        assert_eq!(
            pane_url(SettingsPane::Camera),
            "x-apple.systempreferences:com.apple.preference.security?Privacy_Camera"
        );
        assert_eq!(
            pane_url(SettingsPane::Microphone),
            "x-apple.systempreferences:com.apple.preference.security?Privacy_Microphone"
        );
        assert_eq!(
            pane_url(SettingsPane::Location),
            "x-apple.systempreferences:com.apple.preference.security?Privacy_LocationServices"
        );
        assert_eq!(
            pane_url(SettingsPane::Notifications),
            "x-apple.systempreferences:com.apple.Notifications-Settings.extension"
        );
        assert_eq!(
            pane_url(SettingsPane::LoginItems),
            "x-apple.systempreferences:com.apple.LoginItems-Settings.extension"
        );
    }

    /// Every `PagePermission`: a new variant fails to compile in `expected_pane` until it is mapped.
    const ALL_PERMISSIONS: [PagePermission; 14] = [
        PagePermission::Geolocation,
        PagePermission::Notifications,
        PagePermission::Push,
        PagePermission::Midi,
        PagePermission::Camera,
        PagePermission::Microphone,
        PagePermission::Speaker,
        PagePermission::DeviceInfo,
        PagePermission::BackgroundSync,
        PagePermission::Bluetooth,
        PagePermission::PersistentStorage,
        PagePermission::ScreenWakeLock,
        PagePermission::Gamepad,
        PagePermission::ClipboardRead,
    ];

    fn expected_pane(permission: PagePermission) -> Option<SettingsPane> {
        match permission {
            PagePermission::Camera => Some(SettingsPane::Camera),
            PagePermission::Microphone => Some(SettingsPane::Microphone),
            PagePermission::Geolocation => Some(SettingsPane::Location),
            PagePermission::Notifications => Some(SettingsPane::Notifications),
            PagePermission::Push
            | PagePermission::Midi
            | PagePermission::Speaker
            | PagePermission::DeviceInfo
            | PagePermission::BackgroundSync
            | PagePermission::Bluetooth
            | PagePermission::PersistentStorage
            | PagePermission::ScreenWakeLock
            | PagePermission::Gamepad
            | PagePermission::ClipboardRead => None,
        }
    }

    #[test]
    fn permissions_map_to_panes() {
        for permission in ALL_PERMISSIONS {
            assert_eq!(
                pane_for_permission(permission),
                expected_pane(permission),
                "{permission:?}"
            );
        }
    }

    #[test]
    fn open_pane_goes_through_the_installed_opener() {
        let opener = Rc::new(RecordingSettingsOpener::default());
        set_opener(opener.clone());
        open_pane(SettingsPane::Microphone);
        assert_eq!(*opener.opened.borrow(), [SettingsPane::Microphone]);
    }

    #[test]
    fn the_default_opener_refuses() {
        std::thread::spawn(|| {
            let default = OPENER.with(|current| current.borrow().clone());
            assert!(!default.open(SettingsPane::Camera));
        })
        .join()
        .expect("the default opener test thread panicked");
    }

    #[test]
    fn camera_and_microphone_buttons_use_the_pane_module() {
        let opener = Rc::new(RecordingSettingsOpener::default());
        set_opener(opener.clone());
        crate::media_capture::open_privacy_settings(PagePermission::Microphone);
        assert_eq!(*opener.opened.borrow(), [SettingsPane::Microphone]);
    }
}
