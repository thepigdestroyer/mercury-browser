// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! The radii and font sizes of `ChromeMetrics` reach the pixels they name. They have no geometry
//! an assistive technology reports (`theme_follow.rs` measures those), so each is changed through
//! the generated setter and the pictures before and after are compared: the pixels that change
//! must be inside the area the property draws, and there must be some. A setter wired to another
//! property with the same value (the toolbar's spacing and a tab's radius are both 8 px) moves
//! pixels elsewhere and fails.

use std::rc::Rc;
use std::time::Duration;

use mercury_ui::{BrowserTab, ChromeMetrics, MainWindow};
use slint::platform::software_renderer::{MinimalSoftwareWindow, RepaintBufferType};
use slint::platform::{Platform, WindowAdapter, WindowEvent};
use slint::{
    ComponentHandle, LogicalPosition, LogicalSize, ModelRc, Rgba8Pixel, SharedPixelBuffer, VecModel,
};

struct SoftwarePlatform;

impl Platform for SoftwarePlatform {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, slint::PlatformError> {
        Ok(MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer))
    }

    /// A clock that stands still: no animation moves between the two pictures of a test.
    fn duration_since_start(&self) -> Duration {
        Duration::from_secs(1)
    }
}

fn init_platform() {
    let _ = slint::platform::set_platform(Box::new(SoftwarePlatform));
}

type Pixels = SharedPixelBuffer<Rgba8Pixel>;

const WIDTH: u32 = 900;
const HEIGHT: u32 = 560;

/// An area in window pixels: left, top, right (not included), bottom (not included).
type Area = (u32, u32, u32, u32);

/// The back, forward and reload buttons: 36 x 34 px from x = 14, 8 px apart, from y = 59.
const TOOLBAR_BUTTONS: Area = (14, 59, 138, 93);
/// The menu button at the right end of the toolbar.
const MENU_BUTTON: Area = (850, 59, 886, 93);
/// The "+" button at the right end of the tab row.
const NEW_TAB_BUTTON: Area = (852, 7, 886, 41);
/// The back button alone.
const BACK_BUTTON: Area = (14, 59, 50, 93);
/// The address field: after the three buttons, up to the menu button.
const ADDRESS_FIELD: Area = (146, 59, 842, 97);
/// The tab row's tabs (the strip starts 10 px in; both tabs are 184 px wide with 6 px between).
const FIRST_TAB: Area = (10, 7, 194, 45);
const TABS: Area = (10, 7, 384, 45);

fn window() -> MainWindow {
    init_platform();
    let window = MainWindow::new().expect("the main window can be created");
    let tab = |id: i32, title: &str, active: bool| BrowserTab {
        title: title.into(),
        id,
        active,
        ..Default::default()
    };
    window.set_tabs(ModelRc::new(VecModel::from(vec![
        tab(1, "Opener", true),
        tab(2, "Documentation", false),
    ])));
    window.set_active_tab_index(0);
    window.set_address("https://news.example/article".into());
    window.set_is_new_tab(false);
    window.set_can_go_back(true);
    window.set_can_go_forward(true);
    window
        .window()
        .set_size(LogicalSize::new(WIDTH as f32, HEIGHT as f32));
    window.show().expect("the window can be shown");
    window
}

fn picture(window: &MainWindow) -> Pixels {
    for _ in 0..3 {
        slint::platform::update_timers_and_animations();
        window.window().request_redraw();
        let _ = window.window().take_snapshot();
    }
    window.window().take_snapshot().expect("the window renders")
}

fn hover(window: &MainWindow, x: f32, y: f32) {
    window.window().dispatch_event(WindowEvent::PointerMoved {
        position: LogicalPosition::new(x, y),
    });
}

/// The pixels that differ between two pictures of the same size.
fn changed(before: &Pixels, after: &Pixels) -> Vec<(u32, u32)> {
    assert_eq!(
        (before.width(), before.height()),
        (after.width(), after.height())
    );
    let width = before.width();
    before
        .as_slice()
        .iter()
        .zip(after.as_slice())
        .enumerate()
        .filter(|(_, (old, new))| (old.r, old.g, old.b) != (new.r, new.g, new.b))
        .map(|(index, _)| (index as u32 % width, index as u32 / width))
        .collect()
}

fn inside(point: (u32, u32), area: Area) -> bool {
    (area.0..area.2).contains(&point.0) && (area.1..area.3).contains(&point.1)
}

/// Sets one property and checks that the picture changed, and only inside `areas`.
fn follows(
    what: &str,
    hovered: Option<(f32, f32)>,
    set: impl Fn(&ChromeMetrics<'_>),
    areas: &[Area],
) {
    let window = window();
    if let Some((x, y)) = hovered {
        hover(&window, x, y);
    }
    let before = picture(&window);
    set(&window.global::<ChromeMetrics>());
    let after = picture(&window);
    let moved = changed(&before, &after);
    assert!(!moved.is_empty(), "{what}: nothing was redrawn");
    let outside: Vec<_> = moved
        .iter()
        .filter(|point| !areas.iter().any(|area| inside(**point, *area)))
        .take(5)
        .collect();
    assert!(
        outside.is_empty(),
        "{what}: pixels changed outside {areas:?}, for example at {outside:?}"
    );
}

#[test]
fn button_radius_rounds_the_buttons() {
    // The pointer on "back" fills it, so the corners show.
    follows(
        "button-radius",
        Some((32.0, 76.0)),
        |sizes| sizes.set_button_radius(0.0),
        &[BACK_BUTTON],
    );
}

#[test]
fn button_font_size_sizes_the_glyphs_on_the_buttons() {
    follows(
        "button-font-size",
        None,
        |sizes| sizes.set_button_font_size(30.0),
        &[TOOLBAR_BUTTONS, MENU_BUTTON, NEW_TAB_BUTTON],
    );
}

#[test]
fn address_radius_rounds_the_address_field() {
    follows(
        "address-radius",
        None,
        |sizes| sizes.set_address_radius(0.0),
        &[ADDRESS_FIELD],
    );
}

#[test]
fn address_font_size_sizes_the_typed_text() {
    follows(
        "address-font-size",
        None,
        |sizes| sizes.set_address_font_size(24.0),
        &[ADDRESS_FIELD],
    );
}

#[test]
fn tab_radius_rounds_the_tabs() {
    // Only the selected tab has a fill, so only its corners show.
    follows(
        "tab-radius",
        None,
        |sizes| sizes.set_tab_radius(0.0),
        &[FIRST_TAB],
    );
}

#[test]
fn tab_title_font_size_sizes_the_titles() {
    follows(
        "tab-title-font-size",
        None,
        |sizes| sizes.set_tab_title_font_size(20.0),
        &[TABS],
    );
}
