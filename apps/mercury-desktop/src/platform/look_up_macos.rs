// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! The dictionary popover (`showDefinitionForAttributedString:atPoint:`).

use super::look_up::Dictionary;

pub(crate) struct NsDictionary;

impl Dictionary for NsDictionary {
    fn show(&self, text: &str) -> bool {
        let _ = text;
        false
    }
}
