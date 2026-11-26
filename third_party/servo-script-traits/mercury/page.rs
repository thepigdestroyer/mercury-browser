/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

// Mercury: this file belongs to the page lane. Add variants here only, never in another lane's file.

use std::fmt;

use serde::{Deserialize, Serialize};
use servo_base::id::PipelineId;
use strum::IntoStaticStr;

/// Messages from the constellation to a script thread for the `page` lane. `Debug` prints only the
/// variant name.
#[derive(Deserialize, IntoStaticStr, Serialize)]
pub enum PageThreadMsg {
    /// Ask the page whether it may unload; answered with `UnloadChecked`.
    PermitUnload {
        /// The document asked.
        pipeline_id: PipelineId,
        /// The question's id, echoed in the answer.
        request_id: u64,
    },
    /// Stop loading the document.
    StopLoading(PipelineId),
    /// Reload the document, bypassing the cache.
    ReloadBypassingCache(PipelineId),
    /// The navigation of the document was refused.
    NavigationRefused(PipelineId),
}

impl PageThreadMsg {
    /// The document the message is about.
    pub fn pipeline_id(&self) -> PipelineId {
        match self {
            Self::PermitUnload { pipeline_id, .. } => *pipeline_id,
            Self::StopLoading(pipeline_id) |
            Self::ReloadBypassingCache(pipeline_id) |
            Self::NavigationRefused(pipeline_id) => *pipeline_id,
        }
    }
}

impl fmt::Debug for PageThreadMsg {
    fn fmt(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        let variant: &'static str = self.into();
        write!(formatter, "{variant}")
    }
}
