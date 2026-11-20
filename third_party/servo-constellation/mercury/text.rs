/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

// Mercury: this file belongs to the text lane. Implement handlers here only.

use layout_api::ScriptThreadFactory;
use servo_base::id::{PipelineId, WebViewId};
use servo_constellation_traits::{ServiceWorkerManagerFactory, TextEmbedderMsg, TextScriptMsg};

use crate::constellation::Constellation;

impl<STF, SWF> Constellation<STF, SWF>
where
    STF: ScriptThreadFactory,
    SWF: ServiceWorkerManagerFactory,
{
    /// Handle a message of the `text` lane from the embedding layer. The enum is still empty.
    pub(super) fn handle_text_embedder_message(&mut self, message: TextEmbedderMsg) {
        match message {}
    }

    /// Handle a message of the `text` lane from the script thread of `source_pipeline_id` in
    /// `webview_id`. The enum is still empty.
    pub(super) fn handle_text_script_message(
        &mut self,
        _webview_id: WebViewId,
        _source_pipeline_id: PipelineId,
        message: TextScriptMsg,
    ) {
        match message {}
    }
}
