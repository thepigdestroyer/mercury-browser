/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

// Mercury: the page lane's open hints. Stub: records nothing until the lane's tasks fill it.

use embedder_traits::OpenHints;

use crate::dom::window::Window;

/// Records that a link was followed, from the modifier keys. Stub: does nothing.
pub(crate) fn capture_link(_window: &Window, _modifier_new_tab: bool) {}

/// Records that a middle click opened a new webview. Stub: does nothing.
pub(crate) fn capture_middle_click(_window: &Window) {}

/// Records the features of a `window.open`. Stub: does nothing.
pub(crate) fn capture_window_open(_window: &Window, _features: &str) {}

/// Takes the hints recorded for the next opening, and resets them. Stub: always the default.
pub(crate) fn take() -> OpenHints {
    OpenHints::default()
}
