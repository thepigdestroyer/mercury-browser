// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! Renders the real main window's tab strip with `capture-tabs` set (media-capture, ported onto
//! the tabs-plus strip in merge train 21): the red camera or microphone mark shows on the tabs
//! whose pages may capture, on a pinned tab as well as on the others, and on no other tab. Set
//! `MERCURY_UI_RENDER_DIR` to a directory to get the PNGs.
//!
//! Geometry (from `tab_strip.rs`): the strip starts at x = 10; a pinned tab is 40 px wide on a
//! 46 px stride; with room to spare the other tabs are 184 px wide on a 190 px stride. Tabs are
//! 38 px high, from y = 7.

use std::path::PathBuf;
use std::rc::Rc;

use mercury_ui::{BrowserTab, CaptureView, ChromePalette, ChromeTheme, MainWindow};
use slint::platform::software_renderer::{MinimalSoftwareWindow, RepaintBufferType};
use slint::platform::{Platform, WindowAdapter};
use slint::{ComponentHandle, LogicalSize, ModelRc, SharedPixelBuffer, VecModel};

struct SoftwarePlatform;

impl Platform for SoftwarePlatform {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, slint::PlatformError> {
        Ok(MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer))
    }
}

type Pixels = SharedPixelBuffer<slint::Rgba8Pixel>;

const STRIP_X: u32 = 10;
const PINNED_STRIDE: u32 = 46;
const PINNED_WIDTH: u32 = 40;
const STRIDE: u32 = 190;
const TAB_WIDTH: u32 = 184;
const PINNED_COUNT: u32 = 2;
const TAB_Y: (u32, u32) = (14, 90);

fn save(pixels: &Pixels, name: &str) {
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

fn tab(index: i32, title: &str, active: bool, pinned: bool) -> BrowserTab {
    BrowserTab {
        title: title.into(),
        active,
        id: index + 1,
        pinned,
        url: format!("https://site{}.test/", index + 1).into(),
        ..Default::default()
    }
}

fn capture(camera: bool, microphone: bool) -> CaptureView {
    CaptureView {
        camera,
        microphone,
        heading: if camera || microphone {
            "example.test can use your camera".into()
        } else {
            "".into()
        },
        note: "".into(),
    }
}

/// Pixels in a box that are close to `colour`.
fn near(
    pixels: &Pixels,
    (x0, x1): (u32, u32),
    (y0, y1): (u32, u32),
    colour: slint::Color,
) -> usize {
    let close = |a: u8, b: u8| a.abs_diff(b) <= 24;
    (y0..y1)
        .flat_map(|y| (x0..x1).map(move |x| (x, y)))
        .filter(|(x, y)| {
            let pixel = pixels.as_slice()[(y * pixels.width() + x) as usize];
            close(pixel.r, colour.red())
                && close(pixel.g, colour.green())
                && close(pixel.b, colour.blue())
        })
        .count()
}

fn pinned_box(index: u32) -> (u32, u32) {
    let left = STRIP_X + index * PINNED_STRIDE;
    (left, left + PINNED_WIDTH)
}

fn tab_box(index: u32) -> (u32, u32) {
    let left = STRIP_X + PINNED_COUNT * PINNED_STRIDE + index * STRIDE;
    (left, left + TAB_WIDTH)
}

fn render(theme: ChromeTheme, name: &str) {
    let window = MainWindow::new().expect("the main window can be created");
    window.global::<ChromePalette>().set_theme(theme);
    window.window().set_size(LogicalSize::new(1000.0, 120.0));
    // Two pinned tabs (camera; none), then three others (none; camera and microphone, active;
    // microphone only).
    window.set_tabs(ModelRc::new(VecModel::from(vec![
        tab(0, "Pinned call", false, true),
        tab(1, "Pinned quiet", false, true),
        tab(2, "Quiet page", false, false),
        tab(3, "Video call", true, false),
        tab(4, "Recorder", false, false),
    ])));
    window.set_capture_tabs(ModelRc::new(VecModel::from(vec![
        capture(true, false),
        capture(false, false),
        capture(false, false),
        capture(true, true),
        capture(false, true),
    ])));
    window.set_pinned_tab_count(PINNED_COUNT as i32);
    window.set_active_tab_index(3);
    window.set_is_new_tab(false);
    window.show().expect("the main window can be shown");

    let pixels = window.window().take_snapshot().expect("the window renders");
    save(&pixels, &format!("tab-capture-{name}.png"));
    let mark = window.global::<ChromePalette>().get_capture_indicator();

    let marks = |range| near(&pixels, range, TAB_Y, mark);
    assert!(
        marks(pinned_box(0)) > 30,
        "{name}: a pinned tab whose page may use the camera shows the mark"
    );
    assert_eq!(
        marks(pinned_box(1)),
        0,
        "{name}: a pinned tab with no access shows none"
    );
    assert_eq!(
        marks(tab_box(0)),
        0,
        "{name}: a tab with no access shows none"
    );
    assert!(
        marks(tab_box(1)) > 30,
        "{name}: the active tab whose page may use both shows the mark"
    );
    assert!(
        marks(tab_box(2)) > 30,
        "{name}: a tab whose page may use the microphone shows the mark"
    );
}

#[test]
fn the_capture_mark_shows_on_the_tabs_that_may_capture() {
    slint::platform::set_platform(Box::new(SoftwarePlatform)).expect("the platform can be set");
    render(ChromeTheme::Light, "light");
    render(ChromeTheme::Dark, "dark");
}
