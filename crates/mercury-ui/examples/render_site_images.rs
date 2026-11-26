// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! Renders the site-information popover with its "Images" row (images per site) to PNG files
//! without a display, through Slint's software renderer: images shown and blocked, in Light,
//! Dark and System, and once with the row unavailable (a page that is not a web page), where the
//! popover must be as it was before the row existed. Nothing opens a window or Metal.
//!
//! `cargo run --release -p mercury-ui --example render_site_images -- <output directory>`

use std::cell::RefCell;
use std::error::Error;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use mercury_ui::{
    BrowserTab, ChromePalette, ChromeTheme, MainWindow, SiteInfo, SitePermissionRow,
    SiteSecurityIcon, TabAudioState,
};
use slint::platform::software_renderer::{MinimalSoftwareWindow, RepaintBufferType};
use slint::platform::{Platform, PlatformError, WindowAdapter, WindowEvent};
use slint::{
    ComponentHandle, Image, ModelRc, PhysicalSize, Rgba8Pixel, SharedPixelBuffer, VecModel,
};

const SCALE: f32 = 2.0;
/// The window in logical pixels.
const WIDTH: u32 = 900;
const HEIGHT: u32 = 600;
/// The part of the window saved: the popover is under the indicator, at the left.
const CROP_WIDTH: u32 = 560;
const CROP_HEIGHT: u32 = 460;

struct HeadlessPlatform {
    windows: RefCell<Vec<Rc<MinimalSoftwareWindow>>>,
}

impl Platform for HeadlessPlatform {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, PlatformError> {
        let window = MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer);
        self.windows.borrow_mut().push(Rc::clone(&window));
        Ok(window)
    }
}

type Pixels = SharedPixelBuffer<Rgba8Pixel>;

fn render(window: &MainWindow) -> Result<Pixels, Box<dyn Error>> {
    let slint_window = window.window();
    slint_window.dispatch_event(WindowEvent::ScaleFactorChanged {
        scale_factor: SCALE,
    });
    slint_window.set_size(PhysicalSize::new(
        (WIDTH as f32 * SCALE) as u32,
        (HEIGHT as f32 * SCALE) as u32,
    ));
    for _ in 0..3 {
        slint::platform::update_timers_and_animations();
        slint_window.request_redraw();
        let _ = slint_window.take_snapshot();
    }
    Ok(slint_window.take_snapshot()?)
}

fn save_cropped(pixels: &Pixels, path: &Path) -> Result<(), Box<dyn Error>> {
    let width = ((CROP_WIDTH as f32 * SCALE) as u32).min(pixels.width());
    let height = ((CROP_HEIGHT as f32 * SCALE) as u32).min(pixels.height());
    let mut bytes = Vec::with_capacity((width * height * 3) as usize);
    for y in 0..height {
        for x in 0..width {
            let pixel = pixels.as_slice()[(y * pixels.width() + x) as usize];
            bytes.extend([pixel.r, pixel.g, pixel.b]);
        }
    }
    image::save_buffer(path, &bytes, width, height, image::ExtendedColorType::Rgb8)?;
    println!("wrote {}", path.display());
    Ok(())
}

fn rgb(pixels: &Pixels, x: u32, y: u32) -> [u8; 3] {
    let pixel = pixels.as_slice()[(y * pixels.width() + x) as usize];
    [pixel.r, pixel.g, pixel.b]
}

fn color(color: slint::Color) -> [u8; 3] {
    [color.red(), color.green(), color.blue()]
}

/// The height of the popover's panel in logical pixels, measured on a column in its left padding
/// (no text there): from the row where the panel colour starts, down to where it ends.
fn panel_height(pixels: &Pixels, panel: [u8; 3]) -> Result<f32, Box<dyn Error>> {
    // A point inside the panel, in the blank band between its heading and its text.
    let inside_y = (160.0 * SCALE) as u32;
    let mut left = (200.0 * SCALE) as u32;
    if rgb(pixels, left, inside_y) != panel {
        return Err("the point inside the panel is not the panel's colour".into());
    }
    while left > 0 && rgb(pixels, left - 1, inside_y) == panel {
        left -= 1;
    }
    // Six logical pixels in from the left edge: inside the 14 px of padding, so no text or icon.
    let column = left + (6.0 * SCALE) as u32;
    let mut top = inside_y;
    while top > 0 && rgb(pixels, column, top - 1) == panel {
        top -= 1;
    }
    let mut bottom = inside_y;
    while bottom + 1 < pixels.height() && rgb(pixels, column, bottom + 1) == panel {
        bottom += 1;
    }
    Ok((bottom - top + 1) as f32 / SCALE)
}

struct Case {
    name: &'static str,
    theme: ChromeTheme,
    /// `None`: the row is unavailable.
    images: Option<bool>,
}

fn window_for(case: &Case) -> Result<MainWindow, Box<dyn Error>> {
    let window = MainWindow::new()?;
    window.global::<ChromePalette>().set_theme(case.theme);
    window
        .window()
        .set_size(slint::LogicalSize::new(WIDTH as f32, HEIGHT as f32));
    window.set_tabs(ModelRc::new(VecModel::from(vec![BrowserTab {
        title: "Example".into(),
        icon: Image::default(),
        has_icon: false,
        active: true,
        loading: false,
        id: 1,
        pinned: false,
        url: "https://example.com/".into(),
        audio: TabAudioState::Silent,
        chip_before: 0.0,
    }])));
    window.set_is_new_tab(false);
    window.set_address("https://example.com/article".into());
    window.set_security_icon(SiteSecurityIcon::Padlock);
    window.set_security_label("Connection is secure".into());
    window.set_site_info(SiteInfo {
        host: "example.com".into(),
        icon: SiteSecurityIcon::Padlock,
        heading: "Connection is secure".into(),
        detail:
            "Information you send to this site cannot be read or changed by others on the network."
                .into(),
        permissions: ModelRc::new(VecModel::from(vec![SitePermissionRow {
            name: "Store data permanently on this device".into(),
            state: "Allowed".into(),
        }])),
        site_data: "3 cookies, local storage".into(),
        can_clear_site_data: true,
        https_exception: false,
        certificate_exception: false,
    });
    window.set_site_images_available(case.images.is_some());
    window.set_site_images_blocked(case.images == Some(true));
    Ok(window)
}

fn main() -> Result<(), Box<dyn Error>> {
    let output = PathBuf::from(
        std::env::args()
            .nth(1)
            .ok_or("usage: render_site_images <output directory>")?,
    );
    std::fs::create_dir_all(&output)?;
    slint::platform::set_platform(Box::new(HeadlessPlatform {
        windows: RefCell::new(Vec::new()),
    }))
    .map_err(|error| format!("{error:?}"))?;

    let cases = [
        Case {
            name: "light-shown",
            theme: ChromeTheme::Light,
            images: Some(false),
        },
        Case {
            name: "light-blocked",
            theme: ChromeTheme::Light,
            images: Some(true),
        },
        Case {
            name: "light-unavailable",
            theme: ChromeTheme::Light,
            images: None,
        },
        Case {
            name: "dark-shown",
            theme: ChromeTheme::Dark,
            images: Some(false),
        },
        Case {
            name: "dark-blocked",
            theme: ChromeTheme::Dark,
            images: Some(true),
        },
        Case {
            name: "dark-unavailable",
            theme: ChromeTheme::Dark,
            images: None,
        },
        Case {
            name: "system-shown",
            theme: ChromeTheme::System,
            images: Some(false),
        },
        Case {
            name: "system-blocked",
            theme: ChromeTheme::System,
            images: Some(true),
        },
    ];
    let mut unavailable_height = [0.0_f32; 2];
    for case in &cases {
        let window = window_for(case)?;
        window.show()?;
        window.invoke_show_site_info();
        let pixels = render(&window)?;
        let panel = color(window.global::<ChromePalette>().get_popup_background());
        let height = panel_height(&pixels, panel)?;
        println!("{}: the panel is {height} px high", case.name);
        match (case.theme, case.images) {
            (ChromeTheme::Light, None) => unavailable_height[0] = height,
            (ChromeTheme::Dark, None) => unavailable_height[1] = height,
            _ => {}
        }
        save_cropped(
            &pixels,
            &output.join(format!("site-images-{}.png", case.name)),
        )?;
        window.hide()?;
        // The row adds a separator, a title and a line with a button: about 80 px.
        let reference = match case.theme {
            ChromeTheme::Dark => unavailable_height[1],
            _ => unavailable_height[0],
        };
        match case.images {
            Some(_) if reference > 0.0 => {
                assert!(
                    height > reference + 50.0,
                    "{}: the row did not add height ({height} against {reference})",
                    case.name
                );
            }
            _ => {}
        }
    }
    Ok(())
}
