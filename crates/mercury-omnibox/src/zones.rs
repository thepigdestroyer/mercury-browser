// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! Time-zone questions (stub: the time-zone task fills it).

use crate::{Answer, Context};

/// The answer to a time or time-zone question (`query` is already trimmed and collapsed).
pub(crate) fn answer(_query: &str, _context: &Context) -> Option<Answer> {
    None
}
