/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

// Mercury: the page lane's open hints, recorded by the opening that follows and taken once.

use std::cell::Cell;

use embedder_traits::{OpenDisposition, OpenHints};

use crate::dom::window::Window;

thread_local! {
    static CONTEXT: Cell<Option<OpenHints>> = const { Cell::new(None) };
}

/// Records the hints of the opening that is about to happen. A later capture in the same task
/// replaces the earlier one.
fn set(hints: OpenHints) {
    CONTEXT.with(|context| context.set(Some(hints)));
}

/// Records that a link was followed, from the modifier keys. Stub: does nothing.
pub(crate) fn capture_link(window: &Window, modifier_new_tab: bool) {
    let disposition = if modifier_new_tab {
        OpenDisposition::BackgroundTab
    } else {
        OpenDisposition::ForegroundTab
    };
    set(OpenHints {
        user_activation: window.has_sticky_activation(),
        disposition,
        features: None,
    });
}

/// Records that a middle click opened a new webview.
pub(crate) fn capture_middle_click(window: &Window) {
    set(OpenHints {
        user_activation: window.has_sticky_activation(),
        disposition: OpenDisposition::BackgroundTab,
        features: None,
    });
}

/// Records the features of a `window.open`.
pub(crate) fn capture_window_open(window: &Window, features: &str) {
    set(OpenHints {
        user_activation: window.has_transient_activation(),
        disposition: OpenDisposition::from_features(features),
        features: Some(features.to_owned()),
    });
}

/// Takes the hints recorded for the next opening, and resets them.
pub(crate) fn take() -> OpenHints {
    CONTEXT.with(|context| context.take()).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use embedder_traits::{OpenDisposition, OpenHints};

    use super::{set, take};

    #[test]
    fn open_take_resets() {
        set(OpenHints {
            user_activation: true,
            disposition: OpenDisposition::Popup,
            features: Some("popup".to_owned()),
        });
        let first = take();
        assert_eq!(first.disposition, OpenDisposition::Popup);
        assert!(first.user_activation);
        let second = take();
        assert_eq!(second.disposition, OpenDisposition::ForegroundTab);
        assert!(!second.user_activation);
        assert_eq!(second.features, None);
    }

    #[test]
    fn take_without_capture_is_default() {
        let hints = take();
        assert_eq!(hints.disposition, OpenDisposition::ForegroundTab);
        assert!(!hints.user_activation);
    }

    #[test]
    fn middle_click_is_background_tab() {
        set(OpenHints {
            user_activation: true,
            disposition: OpenDisposition::BackgroundTab,
            features: None,
        });
        assert_eq!(take().disposition, OpenDisposition::BackgroundTab);
    }

    #[test]
    fn link_modifier_is_background_tab() {
        set(OpenHints {
            user_activation: false,
            disposition: OpenDisposition::BackgroundTab,
            features: None,
        });
        let hints = take();
        assert_eq!(hints.disposition, OpenDisposition::BackgroundTab);
        assert_eq!(hints.features, None);
    }

    #[test]
    fn window_open_features_are_kept() {
        set(OpenHints {
            user_activation: false,
            disposition: OpenDisposition::from_features("width=400"),
            features: Some("width=400".to_owned()),
        });
        let hints = take();
        assert_eq!(hints.disposition, OpenDisposition::Popup);
        assert_eq!(hints.features.as_deref(), Some("width=400"));
    }
}
