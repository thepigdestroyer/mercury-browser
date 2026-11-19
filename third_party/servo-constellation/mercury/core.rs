/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

// Mercury: this file belongs to the core lane (the scaffold itself). Implement handlers here only.

use layout_api::ScriptThreadFactory;
use log::trace;
use servo_base::id::{PipelineId, WebViewId};
use servo_constellation_traits::{CoreEmbedderMsg, CoreScriptMsg, ServiceWorkerManagerFactory};

use crate::constellation::Constellation;

impl<STF, SWF> Constellation<STF, SWF>
where
    STF: ScriptThreadFactory,
    SWF: ServiceWorkerManagerFactory,
{
    /// Handle a message of the `core` lane from the embedding layer.
    pub(super) fn handle_core_embedder_message(&mut self, message: CoreEmbedderMsg) {
        match message {
            CoreEmbedderMsg::Probe(value) => {
                trace!("Mercury core probe {value} from the embedder");
            },
        }
    }

    /// Handle a message of the `core` lane from the script thread of `source_pipeline_id`.
    pub(super) fn handle_core_script_message(
        &mut self,
        _webview_id: WebViewId,
        source_pipeline_id: PipelineId,
        message: CoreScriptMsg,
    ) {
        match message {
            CoreScriptMsg::Probe(value) => {
                trace!("Mercury core probe {value} from {source_pipeline_id}");
            },
        }
    }
}
