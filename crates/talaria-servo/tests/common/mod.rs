// SPDX-License-Identifier: MPL-2.0
/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

// Adapted from Servo's own test harness: https://github.com/servo/servo, tag v0.8.0, file
// `tests/common/mod.rs` (MPL-2.0; it is `third_party/servo/tests/common/mod.rs` in this repository).
// `ServoTest`, `WebViewDelegateImpl`, the JavaScript helper and the scene helper are Servo's, trimmed to
// what the accessibility tests use and made `pub` (Servo's are `pub(crate)` in a crate of their own).
// Mercury: a11y-1 item 3 (`core-states`). The accessibility harness built on it is
// `a11y_harness.rs`; the entry for THIRD-PARTY-NOTICES.md is in the item's notes.
//
// Servo's rule for these tests holds here: **all of them run serially on one thread** (`--test-threads=1`;
// `make test-a11y-pages` does it), because Servo keeps process-wide state.

#![allow(dead_code)] // Used by some tests and not others.

pub mod a11y_harness;

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use dpi::PhysicalSize;
use servo::{
    ConsoleLogLevel, EmbedderControl, EventLoopWaker, InputEventId, InputEventResult, JSValue,
    JavaScriptEvaluationError, LoadStatus, RenderingContext, Servo, ServoBuilder, SimpleDialog,
    SoftwareRenderingContext, WebView, WebViewDelegate,
};

pub struct ServoTest {
    pub servo: Servo,
    pub rendering_context: Rc<dyn RenderingContext>,
}

impl ServoTest {
    pub fn new_with_builder<F>(customize: F) -> Self
    where
        F: FnOnce(ServoBuilder) -> ServoBuilder,
    {
        let rendering_context = Rc::new(
            SoftwareRenderingContext::new(PhysicalSize {
                width: 500,
                height: 500,
            })
            .expect("Could not create SoftwareRenderingContext"),
        );
        assert!(rendering_context.make_current().is_ok());

        #[derive(Clone)]
        struct EventLoopWakerImpl(Arc<AtomicBool>);
        impl EventLoopWaker for EventLoopWakerImpl {
            fn clone_box(&self) -> Box<dyn EventLoopWaker> {
                Box::new(self.clone())
            }

            fn wake(&self) {
                self.0.store(true, Ordering::Relaxed);
            }
        }

        let user_event_triggered = Arc::new(AtomicBool::new(false));
        let builder = ServoBuilder::default()
            .event_loop_waker(Box::new(EventLoopWakerImpl(user_event_triggered)));
        let builder = customize(builder);
        Self {
            servo: builder.build(),
            rendering_context,
        }
    }

    pub fn servo(&self) -> &Servo {
        &self.servo
    }

    /// Spin the Servo event loop until the provided callback returns `false`.
    pub fn spin(&self, callback: impl Fn() -> bool) {
        while callback() {
            self.servo.spin_event_loop();
            std::thread::sleep(Duration::from_millis(1));
        }
    }
}

#[derive(Default)]
pub struct WebViewDelegateImpl {
    pub url_changed: Cell<bool>,
    pub cursor_changed: Cell<bool>,
    pub new_frame_ready: Cell<bool>,
    pub load_status_changed: Cell<bool>,
    pub controls_shown: RefCell<Vec<EmbedderControl>>,
    pub active_dialog: RefCell<Option<SimpleDialog>>,
    pub number_of_controls_shown: Cell<usize>,
    pub number_of_controls_hidden: Cell<usize>,
    /// The tree updates the page sent, in order, not yet taken by the harness.
    pub last_accesskit_tree_updates: RefCell<Vec<accesskit::TreeUpdate>>,
    pub console_messages: RefCell<Vec<(ConsoleLogLevel, String)>>,
    pub fullscreen: Cell<bool>,
    pub crashes: Cell<usize>,
    pub input_event_result: RefCell<Option<InputEventResult>>,
}

impl WebViewDelegateImpl {
    pub fn reset(&self) {
        self.url_changed.set(false);
        self.cursor_changed.set(false);
        self.new_frame_ready.set(false);
        self.load_status_changed.set(false);
        self.controls_shown.borrow_mut().clear();
        *self.active_dialog.borrow_mut() = None;
        self.number_of_controls_shown.set(0);
        self.number_of_controls_hidden.set(0);
        self.last_accesskit_tree_updates.borrow_mut().clear();
        self.console_messages.borrow_mut().clear();
        self.fullscreen.set(false);
        self.crashes.set(0);
        *self.input_event_result.borrow_mut() = None;
    }
}

impl WebViewDelegate for WebViewDelegateImpl {
    fn notify_url_changed(&self, _webview: servo::WebView, _url: url::Url) {
        self.url_changed.set(true);
    }

    fn notify_cursor_changed(&self, _webview: WebView, _: servo::Cursor) {
        self.cursor_changed.set(true);
    }

    fn notify_new_frame_ready(&self, webview: WebView) {
        self.new_frame_ready.set(true);
        webview.paint();
    }

    fn notify_load_status_changed(&self, _webview: WebView, status: LoadStatus) {
        if matches!(status, LoadStatus::Complete | LoadStatus::Stopped) {
            self.load_status_changed.set(true);
        }
    }

    fn show_embedder_control(&self, _: WebView, embedder_control: EmbedderControl) {
        if let EmbedderControl::SimpleDialog(simple_dialog) = embedder_control {
            // A page's `alert`: keep the dialog (a dropped one answers by itself) and let the test read it.
            self.active_dialog.borrow_mut().replace(simple_dialog);
            return;
        }
        // Even if not used, controls must be stored so that they do not automatically reply
        // when dropped.
        self.controls_shown.borrow_mut().push(embedder_control);

        self.number_of_controls_shown
            .set(self.number_of_controls_shown.get() + 1);
    }

    fn hide_embedder_control(&self, _webview: WebView, _control_id: servo::EmbedderControlId) {
        self.number_of_controls_hidden
            .set(self.number_of_controls_hidden.get() + 1);
    }

    fn notify_accessibility_tree_update(
        &self,
        _webview: WebView,
        tree_update: accesskit::TreeUpdate,
    ) {
        self.last_accesskit_tree_updates
            .borrow_mut()
            .push(tree_update);
    }

    fn show_console_message(&self, _webview: WebView, level: ConsoleLogLevel, message: String) {
        self.console_messages.borrow_mut().push((level, message));
    }

    fn notify_fullscreen_state_changed(&self, _webview: WebView, fullscreen: bool) {
        self.fullscreen.set(fullscreen);
    }

    fn notify_crashed(&self, _webview: WebView, _reason: String, _backtrace: Option<String>) {
        self.crashes.set(self.crashes.get() + 1);
    }

    fn notify_input_event_handled(
        &self,
        _webview: WebView,
        _event_id: InputEventId,
        result: InputEventResult,
    ) {
        *self.input_event_result.borrow_mut() = Some(result);
    }
}

/// Run a script in the page once it has loaded and wait for its value.
pub fn evaluate_javascript(
    servo_test: &ServoTest,
    webview: WebView,
    script: impl ToString,
) -> Result<JSValue, JavaScriptEvaluationError> {
    servo_test.spin(|| {
        !matches!(
            webview.load_status(),
            LoadStatus::Complete | LoadStatus::Stopped,
        )
    });

    let saved_result = Rc::new(RefCell::new(None));
    let callback_result = saved_result.clone();
    webview.evaluate_javascript(script, move |result| {
        *callback_result.borrow_mut() = Some(result)
    });

    servo_test.spin(|| saved_result.borrow().is_none());

    (*saved_result.borrow())
        .clone()
        .expect("Should have waited until value available")
}

/// Wait for the WebRender scene to reflect the current state of the WebView by triggering a
/// screenshot, waiting for it to be ready, and then throwing away the results.
pub fn wait_for_webview_scene_to_be_up_to_date(servo_test: &ServoTest, webview: &WebView) {
    let waiting = Rc::new(Cell::new(true));
    let callback_waiting = waiting.clone();
    webview.take_screenshot(None, move |result| {
        assert!(result.is_ok());
        callback_waiting.set(false);
    });
    servo_test.spin(move || waiting.get());
}
