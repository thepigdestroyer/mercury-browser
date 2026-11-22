// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! A tab without a page icon shows the Mercury mark, and the mark takes the theme's accent: its
//! disc is the palette's fill accent (`primary-button-background`) in the scheme that is drawn,
//! in the pinned layout, in the other layout and in the all-tabs list. A tab that has an icon
//! shows its icon and none of the mark. The test renders the strip with Slint's software renderer
//! for Hermes (what a window nobody pushes to shows) and for the four palettes a user can choose,
//! in Light and in Dark, and counts the pixels that are exactly the palette's accent. Set
//! `MERCURY_UI_RENDER_DIR` to a directory to get the PNGs.
//!
//! Geometry (from `tab_strip.rs`): the strip starts at x = 10; a pinned tab is 40 px wide on a
//! 46 px stride; with room to spare the other tabs are 184 px wide on a 190 px stride. Tabs are
//! 38 px high, from y = 7.

use std::path::PathBuf;
use std::rc::Rc;

use mercury_appearance::palette::{Role, Scheme};
use mercury_appearance::{Metrics, Palette, palettes};
use mercury_ui::{
    BrowserTab, ChromePalette, ChromeTheme, Look, MainWindow, TabAudioState, TabSearchRow,
    apply_look,
};
use slint::platform::software_renderer::{MinimalSoftwareWindow, RepaintBufferType};
use slint::platform::{Platform, WindowAdapter};
use slint::{
    ComponentHandle, Image, LogicalSize, ModelRc, Rgba8Pixel, SharedPixelBuffer, VecModel,
};

struct SoftwarePlatform;

impl Platform for SoftwarePlatform {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, slint::PlatformError> {
        Ok(MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer))
    }
}

type Pixels = SharedPixelBuffer<Rgba8Pixel>;

const STRIP_X: u32 = 10;
const PINNED_STRIDE: u32 = 46;
const PINNED_WIDTH: u32 = 40;
const STRIDE: u32 = 190;
const WIDTH: u32 = 184;
const PINNED_COUNT: u32 = 3;
const TAB_Y: (u32, u32) = (7, 45);

/// The least pixels of exactly the accent a mark leaves (its disc is about 250 px at 18 px).
const MARK_MIN: usize = 100;

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

/// A 16 px page icon of one flat colour that no palette uses.
fn page_icon() -> Image {
    let mut buffer = SharedPixelBuffer::<Rgba8Pixel>::new(16, 16);
    for pixel in buffer.make_mut_slice() {
        *pixel = Rgba8Pixel::new(30, 136, 229, 255);
    }
    Image::from_rgba8(buffer)
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

fn at(pixels: &Pixels, x: u32, y: u32) -> Rgba8Pixel {
    pixels.as_slice()[(y * pixels.width() + x) as usize]
}

/// The pixels of the tab whose left edge is `from` and that are exactly `colour`.
fn in_tab(pixels: &Pixels, from: u32, width: u32, colour: (u8, u8, u8)) -> usize {
    (TAB_Y.0..TAB_Y.1)
        .flat_map(|y| (0..width).map(move |x| (x, y)))
        .filter(|(x, y)| {
            let pixel = at(pixels, from + x, *y);
            (pixel.r, pixel.g, pixel.b) == colour
        })
        .count()
}

/// The pixels of the whole window that are exactly `colour`.
fn in_window(pixels: &Pixels, colour: (u8, u8, u8)) -> usize {
    pixels
        .as_slice()
        .iter()
        .filter(|pixel| (pixel.r, pixel.g, pixel.b) == colour)
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

fn rgb(palette: &Palette, scheme: Scheme, role: Role) -> (u8, u8, u8) {
    let colour = palette.colour(scheme, role);
    (colour.r, colour.g, colour.b)
}

fn render(
    palette: Option<&'static Palette>,
    theme: ChromeTheme,
    name: &str,
) -> (MainWindow, Pixels) {
    let icon = page_icon();
    // Pinned: no icon, an icon. Others: the same two, and an active tab.
    let tabs = vec![
        tab(0, true, None),
        tab(1, true, Some(&icon)),
        tab(2, true, None),
        tab(3, false, None),
        tab(4, false, Some(&icon)),
        BrowserTab {
            active: true,
            ..tab(5, false, Some(&icon))
        },
    ];
    let window = MainWindow::new().expect("the main window can be created");
    if let Some(palette) = palette {
        apply_look(&window, &Look::new(palette, palette, Metrics::ROOMY));
    }
    window.global::<ChromePalette>().set_theme(theme);
    window.window().set_size(LogicalSize::new(1400.0, 120.0));
    window.set_tabs(ModelRc::new(VecModel::from(tabs)));
    window.set_pinned_tab_count(PINNED_COUNT as i32);
    window.set_active_tab_index(5);
    window.set_is_new_tab(false);
    window.show().expect("the main window can be shown");
    let pixels = window.window().take_snapshot().expect("the window renders");
    save(&pixels, &format!("tab-placeholder-{name}.png"));
    (window, pixels)
}

fn check(palette: Option<&'static Palette>, theme: ChromeTheme, scheme: Scheme, name: &str) {
    // `None` is a window nobody pushes to: the built-in tables, which are Hermes.
    let owner = palette.unwrap_or_else(|| palettes::find("hermes").expect("hermes exists"));
    let accent = rgb(owner, scheme, Role::PrimaryButtonBackground);
    let (window, pixels) = render(palette, theme, name);

    // Pinned layout: the two tabs without an icon show the mark in the accent; the one with an
    // icon shows the icon (flat blue) and none of the accent.
    for index in [0, 2] {
        assert!(
            in_tab(&pixels, pinned_left(index), PINNED_WIDTH, accent) > MARK_MIN,
            "{name}: pinned tab {index} without an icon shows the mark in the accent {accent:?}"
        );
    }
    assert_eq!(
        in_tab(&pixels, pinned_left(1), PINNED_WIDTH, accent),
        0,
        "{name}: a pinned tab with an icon does not show the mark"
    );
    assert!(
        in_tab(&pixels, pinned_left(1), PINNED_WIDTH, (30, 136, 229)) > 100,
        "{name}: a pinned tab with an icon shows it"
    );
    // The two placeholders are the same picture.
    let same = |a: u32, b: u32, width: u32| {
        (TAB_Y.0..TAB_Y.1)
            .flat_map(|y| (0..width).map(move |x| (x, y)))
            .all(|(x, y)| at(&pixels, a + x, y) == at(&pixels, b + x, y))
    };
    assert!(
        same(pinned_left(0), pinned_left(2), PINNED_WIDTH),
        "{name}: every pinned tab without an icon shows the same mark"
    );

    // Other layout.
    assert!(
        in_tab(&pixels, tab_left(0), WIDTH, accent) > MARK_MIN,
        "{name}: a tab without an icon shows the mark in the accent"
    );
    assert_eq!(
        in_tab(&pixels, tab_left(1), WIDTH, accent),
        0,
        "{name}: a tab with an icon does not show the mark"
    );

    // The all-tabs list (tab search): a row without an icon shows the mark, a row with an icon
    // does not.
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
    window.set_tab_search_rows(row(Some(&page_icon())));
    let with_icon = window.window().take_snapshot().expect("the window renders");
    assert!(
        in_window(&placeholder, accent) >= in_window(&with_icon, accent) + MARK_MIN,
        "{name}: a row of the all-tabs list without an icon shows the mark in the accent"
    );
    assert!(
        whole_difference(&placeholder, &with_icon) > 10,
        "{name}: the row with an icon looks different"
    );
    window.hide().expect("the main window can be hidden");
}

#[test]
fn a_tab_without_an_icon_shows_the_mercury_mark_in_the_themes_accent() {
    slint::platform::set_platform(Box::new(SoftwarePlatform)).expect("no platform is set yet");
    let schemes = [
        (ChromeTheme::Light, Scheme::Light, "light"),
        (ChromeTheme::Dark, Scheme::Dark, "dark"),
    ];
    for (theme, scheme, scheme_name) in schemes {
        check(None, theme, scheme, &format!("hermes-{scheme_name}"));
        for palette in palettes::selectable() {
            check(
                Some(palette),
                theme,
                scheme,
                &format!("{}-{scheme_name}", palette.id),
            );
        }
    }
}

#[test]
fn the_mark_changes_with_the_palette_and_no_two_chosen_palettes_share_it() {
    slint::platform::set_platform(Box::new(SoftwarePlatform)).ok();
    for (theme, scheme) in [
        (ChromeTheme::Light, Scheme::Light),
        (ChromeTheme::Dark, Scheme::Dark),
    ] {
        let accents: Vec<(u8, u8, u8)> = palettes::selectable()
            .map(|palette| rgb(palette, scheme, Role::PrimaryButtonBackground))
            .collect();
        for (index, accent) in accents.iter().enumerate() {
            assert!(!accents[index + 1..].contains(accent), "{accents:?}");
        }
        let renders: Vec<Pixels> = palettes::selectable()
            .map(|palette| render(Some(palette), theme, "compare").1)
            .collect();
        for (index, a) in renders.iter().enumerate() {
            for b in &renders[index + 1..] {
                let area = |pixels: &Pixels| {
                    (TAB_Y.0..TAB_Y.1)
                        .flat_map(|y| (0..PINNED_WIDTH).map(move |x| (x, y)))
                        .map(|(x, y)| at(pixels, pinned_left(0) + x, y))
                        .collect::<Vec<_>>()
                };
                assert_ne!(
                    area(a),
                    area(b),
                    "the pinned placeholder follows the palette"
                );
            }
        }
    }
}
