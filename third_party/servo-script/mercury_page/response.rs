/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

// Mercury: the page lane's main-document response. Stub: returns defaults until the lane's tasks fill it.

use embedder_traits::MainDocumentResponse;
use http::{HeaderMap, StatusCode};
use net_traits::Metadata;
use servo_base::id::PipelineId;
use servo_url::ServoUrl;

use crate::dom::document::Document;

/// The response of the main document. Stub: returns an empty response.
pub(crate) fn build(
    pipeline_id: PipelineId,
    _url: &ServoUrl,
    _status: &StatusCode,
    _headers: Option<&HeaderMap>,
) -> MainDocumentResponse {
    MainDocumentResponse {
        pipeline_id,
        url: String::new(),
        status: 0,
        status_text: String::new(),
        content_type: None,
        headers: Vec::new(),
    }
}

/// Sends the main-document response of `document` to the embedder. Stub: does nothing.
pub(crate) fn announce(_document: &Document, _metadata: &Metadata) {}
