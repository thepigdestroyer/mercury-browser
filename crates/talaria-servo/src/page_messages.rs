// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

// Page lane: routes the engine's `Page` messages to the page lane. Stub: nothing is recorded yet.

use servo::PageMsg;
use talaria_core::TabId;

/// Routes one `Page` message of `tab`, from the Page arm of `mercury_messages::handle`. Stub:
/// ignores it.
pub(crate) fn handle(_tab: Option<TabId>, _message: PageMsg) {}
