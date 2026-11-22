// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! The login item (MM15): "Open Mercury Browser at login", off until the user ticks the box.
//! The system holds the state; nothing is stored in the settings.

use std::cell::{Cell, RefCell};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum LoginItemStatus {
    NotRegistered,
    Enabled,
    RequiresApproval,
    NotFound,
    Unavailable(&'static str),
}

pub(crate) trait LoginItemService {
    fn status(&self) -> LoginItemStatus;
    fn register(&self) -> Result<(), String>;
    fn unregister(&self) -> Result<(), String>;
}

/// A service that is never available: automated runs and unsupported platforms.
pub(crate) struct RefusingLoginItem {
    reason: &'static str,
}

impl RefusingLoginItem {
    pub(crate) const fn new(reason: &'static str) -> Self {
        Self { reason }
    }
}

impl LoginItemService for RefusingLoginItem {
    fn status(&self) -> LoginItemStatus {
        LoginItemStatus::Unavailable(self.reason)
    }

    fn register(&self) -> Result<(), String> {
        Err(self.reason.to_owned())
    }

    fn unregister(&self) -> Result<(), String> {
        Err(self.reason.to_owned())
    }
}

/// A login item that records its calls and keeps its status in memory.
pub(crate) struct FakeLoginItem {
    status: RefCell<LoginItemStatus>,
    calls: RefCell<Vec<&'static str>>,
    approval_needed: Cell<bool>,
    failure: RefCell<Option<String>>,
}

impl FakeLoginItem {
    pub(crate) fn new(status: LoginItemStatus) -> Self {
        Self {
            status: RefCell::new(status),
            calls: RefCell::new(Vec::new()),
            approval_needed: Cell::new(false),
            failure: RefCell::new(None),
        }
    }

    /// `register` leaves `RequiresApproval` instead of `Enabled`.
    pub(crate) fn requires_approval(&self) {
        self.approval_needed.set(true);
    }

    /// `register` and `unregister` fail with `message`.
    pub(crate) fn fail_with(&self, message: &str) {
        *self.failure.borrow_mut() = Some(message.to_owned());
    }

    /// `"register"` and `"unregister"`, in order.
    pub(crate) fn calls(&self) -> Vec<&'static str> {
        self.calls.borrow().clone()
    }
}

impl LoginItemService for FakeLoginItem {
    fn status(&self) -> LoginItemStatus {
        self.status.borrow().clone()
    }

    fn register(&self) -> Result<(), String> {
        self.calls.borrow_mut().push("register");
        if let Some(message) = self.failure.borrow().clone() {
            return Err(message);
        }
        *self.status.borrow_mut() = if self.approval_needed.get() {
            LoginItemStatus::RequiresApproval
        } else {
            LoginItemStatus::Enabled
        };
        Ok(())
    }

    fn unregister(&self) -> Result<(), String> {
        self.calls.borrow_mut().push("unregister");
        if let Some(message) = self.failure.borrow().clone() {
            return Err(message);
        }
        *self.status.borrow_mut() = LoginItemStatus::NotRegistered;
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum LoginItemOutcome {
    Enabled,
    Disabled,
    NeedsApproval,
    Failed(String),
}

/// What the Settings row shows for a status.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct LoginItemRow {
    pub(crate) checked: bool,
    pub(crate) enabled: bool,
    pub(crate) note: Option<String>,
    pub(crate) needs_approval_link: bool,
}

pub(crate) fn describe(status: &LoginItemStatus) -> LoginItemRow {
    let _ = status;
    LoginItemRow {
        checked: false,
        enabled: false,
        note: None,
        needs_approval_link: false,
    }
}

/// Turns the login item on or off and says what happened.
pub(crate) fn set_login_item(service: &dyn LoginItemService, on: bool) -> LoginItemOutcome {
    let _ = (service, on);
    LoginItemOutcome::Disabled
}

/// The `.app` directory that holds `executable`, if it runs from an application bundle.
pub(crate) fn app_bundle_path(executable: &std::path::Path) -> Option<std::path::PathBuf> {
    let _ = executable;
    None
}
