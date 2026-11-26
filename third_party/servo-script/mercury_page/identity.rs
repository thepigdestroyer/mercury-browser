/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

// Mercury: the page lane's document identity. Stub: returns defaults until the lane's tasks fill it.

use embedder_traits::{MAX_ORIGIN_BYTES, RequesterInfo};
use servo_base::id::{BrowsingContextId, PipelineId};
use servo_url::ImmutableOrigin;

use crate::dom::document::Document;

/// The ASCII serialisation of `origin`, or `"null"` when it is opaque or longer than
/// [`MAX_ORIGIN_BYTES`].
pub(crate) fn origin_string(origin: &ImmutableOrigin) -> String {
    let serialised = origin.ascii_serialization();
    if serialised.len() > MAX_ORIGIN_BYTES {
        return "null".to_owned();
    }
    serialised.into_owned()
}

/// The identity of `document`. Stub: returns an empty identity.
pub(crate) fn requester_info(_document: &Document) -> RequesterInfo {
    RequesterInfo {
        pipeline_id: PipelineId::new(),
        browsing_context_id: BrowsingContextId::new(),
        origin: "null".to_owned(),
        top_level_origin: "null".to_owned(),
        is_top_level: false,
    }
}

/// Sends the identity of `document` to the embedder. Stub: does nothing.
pub(crate) fn announce(_document: &Document) {}
