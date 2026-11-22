// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! The user's global protection settings (MH13).

use serde::{Deserialize, Serialize};

/// What happens to a public page's request for loopback or the local network (R14c D6).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LnaPolicy {
    /// Refuse, and show the block in the panel with an "Allow on this site" button.
    #[default]
    Block,
    /// Ask the user (needs the dialogs lane's permission sheet).
    Ask,
}
