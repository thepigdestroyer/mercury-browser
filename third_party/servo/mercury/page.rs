/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

// Mercury: this file belongs to the page lane's embedder half. Stub: the methods are filled by the
// lane's tasks; they do nothing until then.

use crate::webview::WebView;

impl WebView {
    /// Stop loading the document of this webview. Stub: does nothing.
    pub fn stop_loading(&self) {}

    /// Answer an unload question of this webview. Stub: does nothing.
    pub fn permit_unload(&self, _request_id: u64, _allowed: bool) {}

    /// Reload this webview, bypassing the cache. Stub: does nothing.
    pub fn reload_bypassing_cache(&self) {}
}
