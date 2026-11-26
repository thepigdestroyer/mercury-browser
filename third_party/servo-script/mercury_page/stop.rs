/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

// Mercury: the page lane's stop. Stops loading the document of a pipeline, as `window.stop()` does.

use js::context::JSContext;
use log::warn;
use servo_base::id::PipelineId;

use crate::dom::bindings::codegen::Bindings::WindowBinding::WindowMethods;
use crate::event_loop::script_thread::ScriptThread;

/// Stop loading the document of `pipeline_id`. A pipeline that has gone is warned about and ignored.
pub(crate) fn handle(cx: &mut JSContext, pipeline_id: PipelineId) {
    let Some(window) = ScriptThread::find_window(pipeline_id) else {
        warn!("page lane: stop for a pipeline that is gone");
        return;
    };
    window.Stop(cx);
}
