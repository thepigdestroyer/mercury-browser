// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! A tab without a page icon shows the Mercury mark, and it is the branding lane's small mark
//! (`mercury-browser-small.svg`, a heavier M that survives at 16 px), in the pinned layout and
//! in the other layout. The test renders the strip with Slint's software renderer and compares a
//! tab without an icon to the same tab given each of the two marks as its page icon: the
//! placeholder must be pixel for pixel the small mark, and the full mark must look different, so
//! the comparison can tell them apart. The same holds for a row of the all-tabs list. Set
//! `MERCURY_UI_RENDER_DIR` to a directory to get the PNGs.
//!
//! Geometry (from `tab_strip.rs`): the strip starts at x = 10; a pinned tab is 40 px wide on a
//! 46 px stride; with room to spare the other tabs are 184 px wide on a 190 px stride. Tabs are
//! 38 px high, from y = 7.

use std::path::{Path, PathBuf};
use std::rc::Rc;

use mercury_ui::{BrowserTab, ChromePalette, ChromeTheme, MainWindow, TabAudioState, TabSearchRow};
use slint::platform::software_renderer::{MinimalSoftwareWindow, RepaintBufferType};
use slint::platform::{Platform, WindowAdapter};
use slint::{ComponentHandle, Image, LogicalSize, ModelRc, SharedPixelBuffer, VecModel};

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
const WIDTH: u32 = 184;
const PINNED_COUNT: u32 = 3;
const TAB_Y: (u32, u32) = (7, 45);

fn mark(name: &str) -> Image {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../apps/mercury-desktop/resources")
        .join(name);
    Image::load_from_path(&path).unwrap_or_else(|error| panic!("{} loads: {error:?}", name))
}

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

/// An inactive, silent tab; `icon` is its page icon, or none.
fn tab(index: i32, pinned: bool, icon: Option<&Image>) -> BrowserTab {
    BrowserTab {
        title: "Same title".into(),
        id: index + 1,
        pinned,
        audio: TabAudioState::Silent,
        url: format!("https://site{}.test/", index + 1).into(),
        has_icon: icon.is_some(),
        icon: icon.cloned().unwrap_or_default(),
        ..Default::default()
    }
}

fn pinned_left(index: u32) -> u32 {
    STRIP_X + index * PINNED_STRIDE
}

fn tab_left(index: u32) -> u32 {
    STRIP_X + PINNED_COUNT * PINNED_STRIDE + index * STRIDE
}

fn at(pixels: &Pixels, x: u32, y: u32) -> slint::Rgba8Pixel {
    pixels.as_slice()[(y * pixels.width() + x) as usize]
}

/// Pixels of a `width` px wide tab (from its left edge) that differ between the tabs whose left
/// edges are `from` and `other`.
fn differing(pixels: &Pixels, from: u32, other: u32, width: u32) -> usize {
    (TAB_Y.0..TAB_Y.1)
        .flat_map(|y| (0..width).map(move |x| (x, y)))
        .filter(|(x, y)| at(pixels, from + x, *y) != at(pixels, other + x, *y))
        .count()
}

/// Pixels that differ between two renderings of the same window.
fn whole_difference(a: &Pixels, b: &Pixels) -> usize {
    assert_eq!((a.width(), a.height()), (b.width(), b.height()));
    a.as_slice()
        .iter()
        .zip(b.as_slice())
        .filter(|(left, right)| left != right)
        .count()
}

fn check(theme: ChromeTheme, name: &str) {
    let small = mark("mercury-browser-small.svg");
    let full = mark("mercury-browser.svg");
    // Pinned: no icon, the small mark, the full mark. Others: the same three.
    let tabs = vec![
        tab(0, true, None),
        tab(1, true, Some(&small)),
        tab(2, true, Some(&full)),
        tab(3, false, None),
        tab(4, false, Some(&small)),
        tab(5, false, Some(&full)),
        BrowserTab {
            active: true,
            ..tab(6, false, None)
        },
    ];
    let window = MainWindow::new().expect("the main window can be created");
    window.global::<ChromePalette>().set_theme(theme);
    window.window().set_size(LogicalSize::new(1400.0, 120.0));
    window.set_tabs(ModelRc::new(VecModel::from(tabs)));
    window.set_pinned_tab_count(PINNED_COUNT as i32);
    window.set_active_tab_index(6);
    window.set_is_new_tab(false);
    window.show().expect("the main window can be shown");
    let pixels = window.window().take_snapshot().expect("the window renders");
    save(&pixels, &format!("tab-placeholder-{name}.png"));

    // Pinned layout.
    assert_eq!(
        differing(&pixels, pinned_left(0), pinned_left(1), PINNED_WIDTH),
        0,
        "{name}: a pinned tab without an icon shows the small mark"
    );
    assert!(
        differing(&pixels, pinned_left(0), pinned_left(2), PINNED_WIDTH) > 10,
        "{name}: the full mark looks different from the small one in a pinned tab"
    );
    // Other layout.
    assert_eq!(
        differing(&pixels, tab_left(0), tab_left(1), WIDTH),
        0,
        "{name}: a tab without an icon shows the small mark"
    );
    assert!(
        differing(&pixels, tab_left(0), tab_left(2), WIDTH) > 10,
        "{name}: the full mark looks different from the small one in a tab"
    );

    // The all-tabs list (tab search) shows the same placeholder for a row without an icon: the
    // whole window is the same with the small mark as the row's icon, and not with the full mark.
    let row = |icon: Option<&Image>| {
        ModelRc::new(VecModel::from(vec![TabSearchRow {
            id: 1,
            title: "Same title".into(),
            url: "https://site.test/".into(),
            has_icon: icon.is_some(),
            icon: icon.cloned().unwrap_or_default(),
            ..Default::default()
        }]))
    };
    window.invoke_show_tab_search();
    window.set_tab_search_rows(row(None));
    let placeholder = window.window().take_snapshot().expect("the window renders");
    save(&placeholder, &format!("tab-placeholder-search-{name}.png"));
    window.set_tab_search_rows(row(Some(&small)));
    let with_small = window.window().take_snapshot().expect("the window renders");
    window.set_tab_search_rows(row(Some(&full)));
    let with_full = window.window().take_snapshot().expect("the window renders");
    assert_eq!(
        whole_difference(&placeholder, &with_small),
        0,
        "{name}: a row of the all-tabs list without an icon shows the small mark"
    );
    assert!(
        whole_difference(&placeholder, &with_full) > 10,
        "{name}: the full mark looks different from the small one in the all-tabs list"
    );
    window.hide().expect("the main window can be hidden");
}

#[test]
fn a_tab_without_an_icon_shows_the_small_mercury_mark() {
    slint::platform::set_platform(Box::new(SoftwarePlatform)).expect("no platform is set yet");
    for (theme, name) in [(ChromeTheme::Light, "light"), (ChromeTheme::Dark, "dark")] {
        check(theme, name);
    }
}
