/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

// Mercury: this file belongs to the media lane. Implement handlers here only.

use layout_api::ScriptThreadFactory;
use servo_base::id::{PipelineId, WebViewId};
use servo_constellation_traits::{MediaEmbedderMsg, MediaScriptMsg, ServiceWorkerManagerFactory};

use crate::constellation::Constellation;

impl<STF, SWF> Constellation<STF, SWF>
where
    STF: ScriptThreadFactory,
    SWF: ServiceWorkerManagerFactory,
{
    /// Handle a message of the `media` lane from the embedding layer. The enum is still empty.
    pub(super) fn handle_media_embedder_message(&mut self, message: MediaEmbedderMsg) {
        match message {}
    }

    /// Handle a message of the `media` lane from the script thread of `source_pipeline_id` in
    /// `webview_id`. The enum is still empty.
    pub(super) fn handle_media_script_message(
        &mut self,
        _webview_id: WebViewId,
        _source_pipeline_id: PipelineId,
        message: MediaScriptMsg,
    ) {
        match message {}
    }
}
