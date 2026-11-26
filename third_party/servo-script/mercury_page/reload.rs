/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

// Mercury: the page lane's reload. Stub: does nothing until the lane's tasks fill it.

use std::cell::Cell;

use js::context::JSContext;
use log::warn;
use servo_base::id::PipelineId;

use crate::dom::bindings::codegen::Bindings::WindowBinding::WindowMethods;
use crate::event_loop::script_thread::ScriptThread;

thread_local! {
    static BYPASS: Cell<bool> = const { Cell::new(false) };
}

/// Reload the document of `pipeline_id`, bypassing the cache. A pipeline that has gone is warned
/// about and ignored.
pub(crate) fn handle(cx: &mut JSContext, pipeline_id: PipelineId) {
    let Some(window) = ScriptThread::find_window(pipeline_id) else {
        warn!("page lane: reload for a pipeline that is gone");
        return;
    };
    BYPASS.with(|bypass| bypass.set(true));
    window.Location(cx).reload_without_origin_check(cx, &window);
}

/// Whether the next fetch must bypass the cache. Clears the flag; `false` in the stub.
pub(crate) fn take_bypass() -> bool {
    BYPASS.with(|bypass| bypass.replace(false))
}

#[cfg(test)]
mod tests {
    use super::{BYPASS, take_bypass};

    #[test]
    fn bypass_is_taken_once() {
        BYPASS.with(|bypass| bypass.set(true));
        assert!(take_bypass());
        assert!(!take_bypass());
    }

    #[test]
    fn bypass_is_off_by_default() {
        assert!(!take_bypass());
    }
}
