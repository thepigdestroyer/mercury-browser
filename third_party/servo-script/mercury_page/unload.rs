/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

// Mercury: the page lane's unload question. The `beforeunload` prompt needs sticky activation.

use js::context::JSContext;
use log::warn;
use servo_base::id::PipelineId;
use servo_constellation_traits::{
    MercuryScriptToConstellation, PageScriptMsg, ScriptToConstellationMessage,
};

use crate::dom::window::Window;
use crate::event_loop::script_thread::ScriptThread;

/// Whether the unload prompt applies to `window`: only with sticky activation.
pub(crate) fn may_prompt(window: &Window) -> bool {
    window.has_sticky_activation()
}

/// Answer the unload question `request_id` of `pipeline_id`. The answer is `UnloadChecked`, sent
/// to the constellation, with `allowed` set from the `beforeunload` check. A pipeline that has
/// gone is warned about and ignored: the constellation times the question out at 30 s.
pub(crate) fn handle(cx: &mut JSContext, pipeline_id: PipelineId, request_id: u64) {
    let Some(document) = ScriptThread::find_document(pipeline_id) else {
        warn!("page lane: unload check {request_id} for a pipeline that is gone");
        return;
    };
    let allowed = document.check_if_unloading_is_cancelled(cx, false);
    document
        .window()
        .send_to_constellation(ScriptToConstellationMessage::Mercury(
            MercuryScriptToConstellation::Page(PageScriptMsg::UnloadChecked {
                request_id,
                allowed,
            }),
        ));
}
