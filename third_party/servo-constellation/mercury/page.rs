/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

// Mercury: this file belongs to the page lane. Implement handlers here only.

use embedder_traits::OpenHints;
use layout_api::ScriptThreadFactory;
use log::warn;
use servo_base::id::{PipelineId, WebViewId};
use servo_constellation_traits::{PageEmbedderMsg, PageScriptMsg, ServiceWorkerManagerFactory};

use crate::constellation::Constellation;

impl<STF, SWF> Constellation<STF, SWF>
where
    STF: ScriptThreadFactory,
    SWF: ServiceWorkerManagerFactory,
{
    /// Handle a message of the `page` lane from the embedding layer. Handlers are added with the
    /// lane's tasks.
    pub(super) fn handle_page_embedder_message(&mut self, message: PageEmbedderMsg) {
        warn!("page lane: {message:?} not handled yet");
    }

    /// Handle a message of the `page` lane from the script thread of `source_pipeline_id` in
    /// `webview_id`. Handlers are added with the lane's tasks.
    pub(super) fn handle_page_script_message(
        &mut self,
        _webview_id: WebViewId,
        _source_pipeline_id: PipelineId,
        message: PageScriptMsg,
    ) {
        warn!("page lane: {message:?} not handled yet");
    }

    /// Forward the open hints of an auxiliary webview request to the embedder. Stub: nothing yet.
    pub(crate) fn mercury_send_open_hints(&mut self, _opener: WebViewId, _hints: OpenHints) {}

    /// Count a screen wake lock of `pipeline_id`. Stub: nothing yet.
    pub(crate) fn mercury_wake_lock_acquire(&mut self, _pipeline_id: PipelineId) {}

    /// Count the release of a screen wake lock of `pipeline_id`. Stub: nothing yet.
    pub(crate) fn mercury_wake_lock_release(&mut self, _pipeline_id: PipelineId) {}

    /// Drop the wake locks of an exited pipeline. Stub: nothing yet.
    pub(crate) fn mercury_wake_lock_pipeline_gone(&mut self, _pipeline_id: PipelineId) {}

    /// Report a refused navigation of `pipeline_id`. Stub: nothing yet.
    pub(crate) fn mercury_navigation_refused(&mut self, _pipeline_id: PipelineId, _webview_id: WebViewId) {}
}
