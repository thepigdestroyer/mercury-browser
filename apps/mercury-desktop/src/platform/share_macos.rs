// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! The system share sheet (`NSSharingServicePicker`).

use super::share::{ShareItem, Sharer};

pub(crate) struct NsSharingPicker;

impl Sharer for NsSharingPicker {
    fn share(&self, item: &ShareItem) -> bool {
        let _ = item;
        false
    }
}
