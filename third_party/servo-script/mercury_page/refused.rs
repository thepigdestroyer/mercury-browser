/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

// Mercury: the page lane's refused navigation. Stub until the lane's tasks fill it.

use js::context::JSContext;
use servo_base::id::PipelineId;
use servo_url::ServoUrl;

/// Whether `url` has a scheme that does not fetch: anything except `http`, `https`, `file`, `data`,
/// `blob`, `about`, `javascript`, `resource`, `urlinfo` and `servo`.
pub(crate) fn is_non_fetch_scheme(url: &ServoUrl) -> bool {
    !matches!(
        url.scheme(),
        "http" | "https" | "file" | "data" | "blob" | "about" | "javascript" | "resource" | "urlinfo" |
            "servo"
    )
}

/// The navigation of `pipeline_id` was refused. Stub: does nothing.
pub(crate) fn handle(_cx: &mut JSContext, _pipeline_id: PipelineId) {}
