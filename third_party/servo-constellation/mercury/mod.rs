/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

// Mercury: this whole directory is Mercury's own (K0 engine base, R14 section 3.4). The
// constellation hands every `Mercury` message to the router below, which hands it to one function
// per lane, so a lane implements its handlers in its own file here and never edits
// `constellation.rs`. Nothing in upstream Servo refers to it except the two match arms and the
// `mod` line in `constellation.rs`.

//! Routing of Mercury's lane messages inside the `Constellation`.
//!
//! This module is declared inside `constellation.rs` (with `#[path]`), so it is a child of that
//! module and its lane files can read and change the private state of [`Constellation`]. The
//! handlers are `impl` blocks of `Constellation` in the lane files:
//!
//! | file | script messages | embedder messages |
//! |---|---|---|
//! | `core.rs` | `handle_core_script_message` | `handle_core_embedder_message` |
//! | `a11y.rs` | `handle_a11y_script_message` | `handle_a11y_embedder_message` |
//! | `media.rs` | `handle_media_script_message` | `handle_media_embedder_message` |
//! | `page.rs` | `handle_page_script_message` | `handle_page_embedder_message` |
//! | `text.rs` | `handle_text_script_message` | `handle_text_embedder_message` |
//!
//! The network lane has no constellation messages.
//!
//! Handlers must not log a message's payload: what a page sent must not reach a log.

mod a11y;
mod core;
mod media;
mod page;
mod text;

use layout_api::ScriptThreadFactory;
use servo_base::id::{PipelineId, WebViewId};
use servo_constellation_traits::{
    MercuryEmbedderToConstellation, MercuryScriptToConstellation, ServiceWorkerManagerFactory,
};

use crate::constellation::Constellation;

impl<STF, SWF> Constellation<STF, SWF>
where
    STF: ScriptThreadFactory,
    SWF: ServiceWorkerManagerFactory,
{
    /// Handle a [`MercuryEmbedderToConstellation`] message by passing it to its lane.
    pub(super) fn handle_mercury_embedder_message(
        &mut self,
        message: MercuryEmbedderToConstellation,
    ) {
        match message {
            MercuryEmbedderToConstellation::Core(message) => {
                self.handle_core_embedder_message(message);
            },
            // One arm per lane, each separated from the next by a comment line.
            MercuryEmbedderToConstellation::A11y(message) => {
                self.handle_a11y_embedder_message(message);
            },
            // The media lane.
            MercuryEmbedderToConstellation::Media(message) => {
                self.handle_media_embedder_message(message);
            },
            // The page lane.
            MercuryEmbedderToConstellation::Page(message) => {
                self.handle_page_embedder_message(message);
            },
            // The text lane.
            MercuryEmbedderToConstellation::Text(message) => {
                self.handle_text_embedder_message(message);
            },
        }
    }

    /// Handle a [`MercuryScriptToConstellation`] message that the script thread of
    /// `source_pipeline_id` (in `webview_id`) sent, by passing it to its lane.
    pub(super) fn handle_mercury_script_message(
        &mut self,
        webview_id: WebViewId,
        source_pipeline_id: PipelineId,
        message: MercuryScriptToConstellation,
    ) {
        match message {
            MercuryScriptToConstellation::Core(message) => {
                self.handle_core_script_message(webview_id, source_pipeline_id, message);
            },
            // One arm per lane, each separated from the next by a comment line.
            MercuryScriptToConstellation::A11y(message) => {
                self.handle_a11y_script_message(webview_id, source_pipeline_id, message);
            },
            // The media lane.
            MercuryScriptToConstellation::Media(message) => {
                self.handle_media_script_message(webview_id, source_pipeline_id, message);
            },
            // The page lane.
            MercuryScriptToConstellation::Page(message) => {
                self.handle_page_script_message(webview_id, source_pipeline_id, message);
            },
            // The text lane.
            MercuryScriptToConstellation::Text(message) => {
                self.handle_text_script_message(webview_id, source_pipeline_id, message);
            },
        }
    }
}
