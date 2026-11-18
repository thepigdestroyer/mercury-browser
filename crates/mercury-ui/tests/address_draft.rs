// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! The address field remembers what the application last wrote into it (`synced-address`), so
//! the application can tell the user's own text from the page's address: typing makes a draft,
//! Return accepts it and Escape restores the committed address.

use std::cell::RefCell;
use std::rc::Rc;

use mercury_ui::MainWindow;
use slint::platform::software_renderer::{MinimalSoftwareWindow, RepaintBufferType};
use slint::platform::{Key, Platform, WindowAdapter, WindowEvent};
use slint::{ComponentHandle, LogicalSize};

struct SoftwarePlatform;

impl Platform for SoftwarePlatform {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, slint::PlatformError> {
        Ok(MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer))
    }
}

fn key(window: &slint::Window, text: impl Into<slint::SharedString> + Clone) {
    window.dispatch_event(WindowEvent::KeyPressed {
        text: text.clone().into(),
    });
    window.dispatch_event(WindowEvent::KeyReleased { text: text.into() });
}

#[test]
fn typing_makes_a_draft_that_return_accepts_and_escape_drops() {
    slint::platform::set_platform(Box::new(SoftwarePlatform)).expect("no platform is set yet");
    let window = MainWindow::new().expect("the main window can be created");
    window.window().set_size(LogicalSize::new(900.0, 400.0));
    window.show().expect("the main window can be shown");
    let navigated = Rc::new(RefCell::new(Vec::<String>::new()));
    let recorded = Rc::clone(&navigated);
    window.on_navigate(move |text| recorded.borrow_mut().push(text.into()));

    // The application writes the page's address.
    window.set_committed_address("https://a.test/".into());
    window.invoke_reset_address("https://a.test/".into());
    assert_eq!(window.get_synced_address(), "https://a.test/");

    // The user types: the field differs from what was synced.
    window.invoke_focus_address_bar();
    key(window.window(), "x");
    assert_eq!(window.get_address(), "x");
    assert_eq!(window.get_synced_address(), "https://a.test/");

    // Escape restores the committed address and the draft is gone.
    window.set_committed_address("https://b.test/".into());
    key(window.window(), Key::Escape);
    assert_eq!(window.get_address(), "https://b.test/");
    assert_eq!(window.get_synced_address(), "https://b.test/");

    // Return accepts the typed text: it is no longer a draft.
    window.invoke_focus_address_bar();
    key(window.window(), "y");
    key(window.window(), Key::Return);
    assert_eq!(*navigated.borrow(), ["y"]);
    assert_eq!(window.get_address(), "y");
    assert_eq!(window.get_synced_address(), "y");

    accepting_the_draft_from_elsewhere_lets_the_page_address_back_in(&window);
    window.hide().expect("the window can be hidden");
}

// tabs-plus review I1: a navigation that does not come from the field's Return.
fn accepting_the_draft_from_elsewhere_lets_the_page_address_back_in(window: &MainWindow) {
    window.invoke_reset_address("https://a.test/".into());
    window.set_address("exa".into());
    assert_eq!(window.get_synced_address(), "https://a.test/");

    // A suggestion, the history window, Home, Back… navigated: the draft is no longer one.
    window.invoke_accept_address_draft();

    assert_eq!(window.get_synced_address(), "exa");
}
