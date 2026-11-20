/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

// Mercury: this file belongs to the page lane. Implement handlers here only.

use layout_api::ScriptThreadFactory;
use servo_base::id::{PipelineId, WebViewId};
use servo_constellation_traits::{PageEmbedderMsg, PageScriptMsg, ServiceWorkerManagerFactory};

use crate::constellation::Constellation;

impl<STF, SWF> Constellation<STF, SWF>
where
    STF: ScriptThreadFactory,
    SWF: ServiceWorkerManagerFactory,
{
    /// Handle a message of the `page` lane from the embedding layer. The enum is still empty.
    pub(super) fn handle_page_embedder_message(&mut self, message: PageEmbedderMsg) {
        match message {}
    }

    /// Handle a message of the `page` lane from the script thread of `source_pipeline_id` in
    /// `webview_id`. The enum is still empty.
    pub(super) fn handle_page_script_message(
        &mut self,
        _webview_id: WebViewId,
        _source_pipeline_id: PipelineId,
        message: PageScriptMsg,
    ) {
        match message {}
    }
}
