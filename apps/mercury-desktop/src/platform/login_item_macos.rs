// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! `SMAppService.mainApp` (ServiceManagement, loaded with `dlopen` at first use) and the
//! relaunch opt-in.

use super::login_item::{LoginItemService, LoginItemStatus};
use super::relaunch::RelaunchOnLogin;

pub(crate) struct SmAppService;

impl LoginItemService for SmAppService {
    fn status(&self) -> LoginItemStatus {
        LoginItemStatus::Unavailable("not implemented")
    }

    fn register(&self) -> Result<(), String> {
        Err("not implemented".to_owned())
    }

    fn unregister(&self) -> Result<(), String> {
        Err("not implemented".to_owned())
    }
}

pub(crate) struct NsRelaunch;

impl RelaunchOnLogin for NsRelaunch {
    fn enable(&self) {}
}
