//! Camera and microphone access of a tab's page (media-capture lane).
//!
//! The engine never says when a page starts or stops capturing, so the browser shows what the
//! page may use: a kind becomes "accessible" when the user (or a remembered decision) allows it
//! for the tab's current document, and stops being so when a new document starts, the page
//! crashes, the tab's web view goes away, or the user stops it. The engine releases the devices
//! when the document goes away (Mercury's vendored servo-script).

use crate::PagePermission;

/// Which capture devices the tab's current page has been allowed to use.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CaptureAccess {
    pub camera: bool,
    pub microphone: bool,
}

impl CaptureAccess {
    pub const fn is_empty(self) -> bool {
        !self.camera && !self.microphone
    }

    pub(crate) fn grant(&mut self, permission: PagePermission) {
        match permission {
            PagePermission::Camera => self.camera = true,
            PagePermission::Microphone => self.microphone = true,
            _ => {}
        }
    }
}

/// Camera and microphone: the permissions that are shown together and remembered per kind.
pub(crate) const fn is_capture_permission(permission: PagePermission) -> bool {
    matches!(
        permission,
        PagePermission::Camera | PagePermission::Microphone
    )
}

/// The other capture kind: camera for microphone and the reverse.
pub(crate) const fn paired_capture_permission(
    permission: PagePermission,
) -> Option<PagePermission> {
    match permission {
        PagePermission::Camera => Some(PagePermission::Microphone),
        PagePermission::Microphone => Some(PagePermission::Camera),
        _ => None,
    }
}
