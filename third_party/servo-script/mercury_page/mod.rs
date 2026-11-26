/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

// Mercury: this directory is the page lane's script half. One file per concern; Mercury-only.

pub(crate) mod identity;
pub(crate) mod open;
pub(crate) mod refused;
pub(crate) mod reload;
pub(crate) mod response;
pub(crate) mod stop;
pub(crate) mod unload;

use js::context::JSContext;
use log::warn;
use net_traits::Metadata;
use script_traits::MercuryScriptThreadMsg;

use crate::dom::window::Window;

/// Handle a message from the constellation that belongs to the page lane. Stub: warns only.
pub(crate) fn handle_thread_message(_cx: &mut JSContext, message: MercuryScriptThreadMsg) {
    warn!("page lane: {message:?} not handled yet");
}

/// Called once a document is committed to its window. Stub: does nothing.
pub(crate) fn document_committed(_window: &Window, _metadata: Option<&Metadata>) {}
