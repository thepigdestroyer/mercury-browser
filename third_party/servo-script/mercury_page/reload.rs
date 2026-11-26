/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

// Mercury: the page lane's reload. Stub: does nothing until the lane's tasks fill it.

use std::cell::Cell;

use js::context::JSContext;
use servo_base::id::PipelineId;

thread_local! {
    static BYPASS: Cell<bool> = const { Cell::new(false) };
}

/// Reload the document of `pipeline_id`, bypassing the cache. Stub: does nothing.
pub(crate) fn handle(_cx: &mut JSContext, _pipeline_id: PipelineId) {}

/// Whether the next fetch must bypass the cache. Clears the flag; `false` in the stub.
pub(crate) fn take_bypass() -> bool {
    BYPASS.with(|bypass| bypass.replace(false))
}
