// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! Renders the tab strip's sound indicator with Slint's software renderer and clicks it.
//! Set `MERCURY_UI_RENDER_DIR` to a directory to get the PNG.

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

use mercury_ui::{BrowserTab, MainWindow, TabAudioState};
use slint::platform::software_renderer::{MinimalSoftwareWindow, RepaintBufferType};
use slint::platform::{Platform, PointerEventButton, WindowAdapter, WindowEvent};
use slint::{
    ComponentHandle, Image, LogicalPosition, LogicalSize, ModelRc, SharedPixelBuffer, VecModel,
};

struct SoftwarePlatform;

impl Platform for SoftwarePlatform {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, slint::PlatformError> {
        Ok(MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer))
    }
}

/// Writes the PNG only when `MERCURY_UI_RENDER_DIR` names a directory.
fn save(pixels: &SharedPixelBuffer<slint::Rgba8Pixel>, name: &str) {
    let Some(directory) = std::env::var_os("MERCURY_UI_RENDER_DIR").map(PathBuf::from) else {
        return;
    };
    image::save_buffer_with_format(
        directory.join(name),
        pixels.as_bytes(),
        pixels.width(),
        pixels.height(),
        image::ExtendedColorType::Rgba8,
        image::ImageFormat::Png,
    )
    .expect("the rendering can be saved");
}

fn tab(title: &str, active: bool, audio: TabAudioState) -> BrowserTab {
    BrowserTab {
        title: title.into(),
        icon: Image::default(),
        has_icon: false,
        active,
        loading: false,
        audio,
        ..Default::default()
    }
}

/// Pixels in a box that differ from the tab's background: the icon's ink.
fn ink(pixels: &SharedPixelBuffer<slint::Rgba8Pixel>, x0: u32, x1: u32, y0: u32, y1: u32) -> usize {
    let background = pixels.as_slice()[(y0 * pixels.width() + x0) as usize];
    (y0..y1)
        .flat_map(|y| (x0..x1).map(move |x| (x, y)))
        .filter(|(x, y)| pixels.as_slice()[(y * pixels.width() + x) as usize] != background)
        .count()
}

fn click(window: &MainWindow, x: f32, y: f32) {
    let position = LogicalPosition::new(x, y);
    window
        .window()
        .dispatch_event(WindowEvent::PointerMoved { position });
    window.window().dispatch_event(WindowEvent::PointerPressed {
        position,
        button: PointerEventButton::Left,
    });
    window
        .window()
        .dispatch_event(WindowEvent::PointerReleased {
            position,
            button: PointerEventButton::Left,
        });
}

#[test]
fn the_sound_indicator_renders_and_toggles_mute_without_selecting_the_tab() {
    slint::platform::set_platform(Box::new(SoftwarePlatform)).expect("no platform is set yet");
    let window = MainWindow::new().expect("the main window can be created");
    window.window().set_size(LogicalSize::new(800.0, 120.0));
    window.set_tabs(ModelRc::new(VecModel::from(vec![
        tab("Quiet page", false, TabAudioState::Silent),
        tab("Song page", true, TabAudioState::Playing),
        tab("Muted page", false, TabAudioState::Muted),
    ])));
    window.set_active_tab_index(1);
    let toggled = Rc::new(RefCell::new(Vec::new()));
    let selected = Rc::new(RefCell::new(Vec::new()));
    let toggled_log = Rc::clone(&toggled);
    window.on_toggle_tab_muted(move |index| toggled_log.borrow_mut().push(index));
    let selected_log = Rc::clone(&selected);
    window.on_select_tab(move |index| selected_log.borrow_mut().push(index));
    window.show().expect("the main window can be shown");

    let pixels = window.window().take_snapshot().expect("the window renders");
    save(&pixels, "tab-audio.png");

    // Tabs are 184 px wide with 6 px gaps, from x = 10. The icon sits left of the close button
    // (22 px, 8 px padding), in the 22 px before it, separated by 8 px of spacing.
    let icon_box = |index: u32| {
        let right = 10 + index * 190 + 184 - 8 - 22 - 8;
        (right - 22, right)
    };
    let (x0, x1) = icon_box(0);
    assert_eq!(ink(&pixels, x0, x1, 12, 36), 0, "a silent tab has no icon");
    let (x0, x1) = icon_box(1);
    assert!(
        ink(&pixels, x0, x1, 12, 36) > 20,
        "a playing tab shows the speaker"
    );
    let (x0, x1) = icon_box(2);
    assert!(
        ink(&pixels, x0, x1, 12, 36) > 20,
        "a muted tab shows the muted speaker"
    );

    for index in [1, 2] {
        let (x0, x1) = icon_box(index);
        click(&window, (x0 + x1) as f32 / 2.0, 7.0 + 17.0);
    }
    assert_eq!(*toggled.borrow(), [1, 2]);
    assert!(selected.borrow().is_empty(), "{:?}", selected.borrow());

    // Clicking the tab elsewhere still selects it.
    click(&window, 60.0, 24.0);
    assert_eq!(*selected.borrow(), [0]);
    window.hide().expect("the main window can be hidden");
}
