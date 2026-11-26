// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! Whether macOS lets Mercury Browser use the camera and the microphone at all.
//!
//! The answer comes from `+[AVCaptureDevice authorizationStatusForMediaType:]`, which reads the
//! privacy setting and neither prompts nor touches a device. AVFoundation is loaded only when a
//! page actually asks, and never in automated runs: those assume "allowed", or simulate a refusal
//! with `--fake-system-denies-capture`.

use std::cell::{Cell, RefCell};
use std::time::{Duration, Instant};

use talaria_core::PagePermission;

use crate::platform::system_settings::{open_pane, pane_for_permission};

/// What macOS allows for one kind of device.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SystemAccess {
    /// Allowed, or not decided yet (macOS asks the user itself when the page opens the device).
    Allowed,
    /// Refused in System Settings, or restricted by the administrator.
    Denied,
}

/// Where the answer comes from in this run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SystemAccessSource {
    /// Ask macOS (a normal, interactive run).
    Query,
    /// Automated runs: never ask macOS.
    AssumeAllowed,
    /// `--fake-system-denies-capture`.
    SimulateDenied,
}

thread_local! {
    static SOURCE: Cell<SystemAccessSource> = const { Cell::new(SystemAccessSource::AssumeAllowed) };
}

pub(crate) fn set_source(source: SystemAccessSource) {
    SOURCE.with(|current| current.set(source));
}

/// What macOS allows for `permission` (camera or microphone; anything else is "allowed").
pub(crate) fn system_access(permission: PagePermission) -> SystemAccess {
    match SOURCE.with(Cell::get) {
        SystemAccessSource::AssumeAllowed => SystemAccess::Allowed,
        SystemAccessSource::SimulateDenied => SystemAccess::Denied,
        SystemAccessSource::Query => cached_query(permission),
    }
}

/// How long one answer from macOS is reused: the prompt is rebuilt on every UI update, and the
/// setting changes only when the user goes to System Settings.
const QUERY_REUSE: Duration = Duration::from_secs(3);

thread_local! {
    static CACHE: RefCell<Vec<(PagePermission, SystemAccess, Instant)>> = const { RefCell::new(Vec::new()) };
}

fn cached_query(permission: PagePermission) -> SystemAccess {
    let now = Instant::now();
    CACHE.with(|cache| {
        let mut cache = cache.borrow_mut();
        cache.retain(|(_, _, at)| now.duration_since(*at) < QUERY_REUSE);
        if let Some((_, access, _)) = cache.iter().find(|(kind, _, _)| *kind == permission) {
            return *access;
        }
        let access = query(permission);
        cache.push((permission, access, now));
        access
    })
}

/// `AVAuthorizationStatus`: 0 not determined, 1 restricted, 2 denied, 3 authorized.
#[cfg(any(target_os = "macos", test))]
pub(crate) fn access_from_status(status: Option<isize>) -> SystemAccess {
    match status {
        Some(1 | 2) => SystemAccess::Denied,
        _ => SystemAccess::Allowed,
    }
}

#[cfg(target_os = "macos")]
fn query(permission: PagePermission) -> SystemAccess {
    let symbol = match permission {
        PagePermission::Camera => c"AVMediaTypeVideo",
        PagePermission::Microphone => c"AVMediaTypeAudio",
        _ => return SystemAccess::Allowed,
    };
    access_from_status(authorization_status(symbol))
}

#[cfg(target_os = "macos")]
fn authorization_status(media_type_symbol: &std::ffi::CStr) -> Option<isize> {
    use objc2::msg_send;
    use objc2::runtime::{AnyClass, AnyObject};

    const AV_FOUNDATION: &std::ffi::CStr =
        c"/System/Library/Frameworks/AVFoundation.framework/AVFoundation";
    // SAFETY: dlopen and dlsym with valid C strings; the framework stays loaded (never closed).
    // `AVMediaTypeVideo`/`AVMediaTypeAudio` are `AVMediaType const` (an `NSString *`) globals,
    // so the symbol's address points at an object pointer. The class method takes that string
    // and returns an `NSInteger`.
    unsafe {
        let handle = libc::dlopen(AV_FOUNDATION.as_ptr(), libc::RTLD_LAZY);
        if handle.is_null() {
            return None;
        }
        let symbol = libc::dlsym(handle, media_type_symbol.as_ptr());
        if symbol.is_null() {
            return None;
        }
        let media_type = *(symbol as *const *const AnyObject);
        if media_type.is_null() {
            return None;
        }
        let class = AnyClass::get(c"AVCaptureDevice")?;
        let status: isize = msg_send![class, authorizationStatusForMediaType: media_type];
        Some(status)
    }
}

#[cfg(not(target_os = "macos"))]
fn query(_permission: PagePermission) -> SystemAccess {
    SystemAccess::Allowed
}

/// Opens the Privacy & Security pane for `permission` in System Settings.
pub(crate) fn open_privacy_settings(permission: PagePermission) {
    if let Some(pane) = pane_for_permission(permission) {
        open_pane(pane);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_refusal_or_a_restriction_counts_as_denied() {
        assert_eq!(access_from_status(Some(0)), SystemAccess::Allowed);
        assert_eq!(access_from_status(Some(1)), SystemAccess::Denied);
        assert_eq!(access_from_status(Some(2)), SystemAccess::Denied);
        assert_eq!(access_from_status(Some(3)), SystemAccess::Allowed);
        assert_eq!(access_from_status(None), SystemAccess::Allowed);
    }

    #[test]
    fn automated_runs_never_ask_macos() {
        // The default source of every thread, including the tests'.
        assert_eq!(system_access(PagePermission::Camera), SystemAccess::Allowed);
        set_source(SystemAccessSource::SimulateDenied);
        assert_eq!(
            system_access(PagePermission::Microphone),
            SystemAccess::Denied
        );
        set_source(SystemAccessSource::AssumeAllowed);
    }
}
