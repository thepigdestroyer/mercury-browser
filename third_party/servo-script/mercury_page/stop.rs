/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

// Mercury: the page lane's stop. Stub: does nothing until the lane's tasks fill it.

use js::context::JSContext;
use servo_base::id::PipelineId;

/// Stop loading the document of `pipeline_id`. Stub: does nothing.
pub(crate) fn handle(_cx: &mut JSContext, _pipeline_id: PipelineId) {}
