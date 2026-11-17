// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! A click on a suggestion row reaches `suggestion-chosen`, even though pressing the row may
//! take focus from the address field, and losing focus closes the list (as the application's
//! `address-blurred` handler does).

use std::cell::RefCell;
use std::rc::Rc;

use mercury_ui::{MainWindow, SuggestionRow};
use slint::platform::software_renderer::{MinimalSoftwareWindow, RepaintBufferType};
use slint::platform::{Platform, PointerEventButton, WindowAdapter, WindowEvent};
use slint::{ComponentHandle, LogicalPosition, ModelRc, PhysicalSize, VecModel};

struct Headless {
    window: Rc<MinimalSoftwareWindow>,
}

impl Platform for Headless {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, slint::PlatformError> {
        Ok(self.window.clone())
    }
}

fn row(title: &str) -> SuggestionRow {
    SuggestionRow {
        title_before: title.into(),
        ..SuggestionRow::default()
    }
}

#[test]
fn clicking_a_suggestion_row_chooses_it() {
    let platform_window = MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer);
    platform_window.set_size(PhysicalSize::new(1180, 760));
    slint::platform::set_platform(Box::new(Headless {
        window: platform_window.clone(),
    }))
    .unwrap();
    let window = MainWindow::new().unwrap();
    window.show().unwrap();

    let chosen = Rc::new(RefCell::new(Vec::new()));
    let record = Rc::clone(&chosen);
    window.on_suggestion_chosen(move |index| record.borrow_mut().push(index));
    let weak = window.as_weak();
    window.on_address_blurred(move || {
        // What the application does: losing focus closes the list.
        if let Some(window) = weak.upgrade() {
            window.set_suggestions_open(false);
        }
    });
    window.set_suggestions(ModelRc::new(VecModel::from(vec![
        row("Search for exa"),
        row("Example Domain"),
        row("Example docs"),
    ])));
    window.invoke_focus_address_bar();
    window.set_address("exa".into());
    window.set_suggestions_open(true);
    assert!(window.get_address_focused());

    // The list starts 6 px under the address field (y 59, height 38) and has 6 px of padding;
    // rows are 46 px high. The second row's middle:
    let position = LogicalPosition::new(590.0, 59.0 + 38.0 + 6.0 + 6.0 + 46.0 + 23.0);
    platform_window.dispatch_event(WindowEvent::PointerMoved { position });
    platform_window.dispatch_event(WindowEvent::PointerPressed {
        position,
        button: PointerEventButton::Left,
    });
    platform_window.dispatch_event(WindowEvent::PointerReleased {
        position,
        button: PointerEventButton::Left,
    });

    assert_eq!(*chosen.borrow(), vec![1]);
    // Pressing a row does not take focus from the field (a `TouchArea` is not focusable), so
    // the blur-close cannot beat the click. Should that change, the row would be gone before
    // the release and the assertion above would fail.
    assert!(window.get_address_focused());
}
