// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! Renders the site-security chrome with Slint's software renderer: the indicator before the
//! address, the site-information popover and the HTTPS-only interstitial, in each appearance.
//! Set `MERCURY_UI_RENDER_DIR` to keep the PNGs.

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

use mercury_ui::{
    BrowserTab, ChromePalette, ChromeTheme, MainWindow, SiteInfo, SitePermissionRow,
    SiteSecurityIcon, TabAudioState,
};
use slint::platform::software_renderer::{MinimalSoftwareWindow, RepaintBufferType};
use slint::platform::{Key, Platform, WindowAdapter, WindowEvent};
use slint::{ComponentHandle, Image, LogicalSize, ModelRc, SharedPixelBuffer, VecModel};

struct SoftwarePlatform;

impl Platform for SoftwarePlatform {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, slint::PlatformError> {
        Ok(MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer))
    }
}

const THEMES: [(ChromeTheme, &str); 3] = [
    (ChromeTheme::Light, "light"),
    (ChromeTheme::Dark, "dark"),
    (ChromeTheme::System, "system"),
];

fn save(pixels: &SharedPixelBuffer<slint::Rgba8Pixel>, name: &str) {
    let directory = std::env::var_os("MERCURY_UI_RENDER_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
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

fn pixel(pixels: &SharedPixelBuffer<slint::Rgba8Pixel>, x: u32, y: u32) -> [u8; 3] {
    let pixel = pixels.as_slice()[(y * pixels.width() + x) as usize];
    [pixel.r, pixel.g, pixel.b]
}

fn rgb(color: slint::Color) -> [u8; 3] {
    [color.red(), color.green(), color.blue()]
}

fn window(theme: ChromeTheme, address: &str) -> MainWindow {
    let window = MainWindow::new().expect("the main window can be created");
    window.global::<ChromePalette>().set_theme(theme);
    window.window().set_size(LogicalSize::new(900.0, 600.0));
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
    window.set_address(address.into());
    window
}

fn site_info(icon: SiteSecurityIcon, host: &str, heading: &str) -> SiteInfo {
    SiteInfo {
        host: host.into(),
        icon,
        heading: heading.into(),
        detail: "Information you send to this site cannot be read by others on the network.".into(),
        permissions: ModelRc::new(VecModel::from(vec![SitePermissionRow {
            name: "Store data permanently on this device".into(),
            state: "Allowed".into(),
        }])),
        site_data: "3 cookies, local storage".into(),
        can_clear_site_data: true,
        https_exception: false,
        certificate_exception: false,
    }
}

/// The popover under the indicator, with the indicator's request and the popover's buttons
/// wired up.
fn render_popover(theme: ChromeTheme, name: &str) {
    let window = window(theme, "https://münchen.example/path");
    window.set_security_icon(SiteSecurityIcon::Padlock);
    window.set_security_label("Connection is secure".into());
    window.set_site_info(site_info(
        SiteSecurityIcon::Padlock,
        "münchen.example",
        "Connection is secure",
    ));
    window.show().expect("the window can be shown");
    let pixels = window.window().take_snapshot().expect("the window renders");
    save(&pixels, &format!("site-security-padlock-{name}.png"));

    window.invoke_show_site_info();
    let pixels = window.window().take_snapshot().expect("the window renders");
    save(&pixels, &format!("site-info-{name}.png"));
    // The popover's panel starts under the indicator, inside the window's left half.
    let panel = rgb(window.global::<ChromePalette>().get_popup_background());
    assert_eq!(pixel(&pixels, 200, 160), panel, "{name}");
    // Escape closes it.
    window.window().dispatch_event(WindowEvent::KeyPressed {
        text: Key::Escape.into(),
    });
    window.window().dispatch_event(WindowEvent::KeyReleased {
        text: Key::Escape.into(),
    });
    let pixels = window.window().take_snapshot().expect("the window renders");
    assert_ne!(pixel(&pixels, 200, 160), panel, "{name}");
    window.hide().expect("the window can be hidden");
}

fn render_warning_popover(theme: ChromeTheme, name: &str) {
    let window = window(theme, "https://self-signed.test/");
    window.set_security_icon(SiteSecurityIcon::Warning);
    window.set_security_label("Not secure: you allowed an invalid certificate".into());
    let mut info = site_info(
        SiteSecurityIcon::Warning,
        "self-signed.test",
        "Not secure: you allowed an invalid certificate",
    );
    info.detail = "".into();
    info.permissions = ModelRc::new(VecModel::from(Vec::new()));
    info.site_data = "None".into();
    info.can_clear_site_data = false;
    info.certificate_exception = true;
    info.https_exception = true;
    window.set_site_info(info);
    window.show().expect("the window can be shown");
    window.invoke_show_site_info();
    let pixels = window.window().take_snapshot().expect("the window renders");
    save(&pixels, &format!("site-info-warning-{name}.png"));
    window.hide().expect("the window can be hidden");
}

fn render_interstitial(theme: ChromeTheme, name: &str) {
    let window = window(theme, "http://plain.test:8471/");
    window.set_security_icon(SiteSecurityIcon::Warning);
    window.set_web_view_visible(true);
    window.set_https_only_interstitial_shown(true);
    window.set_https_only_interstitial_host("plain.test".into());
    window.set_https_only_interstitial_tried_https(true);
    let answers = Rc::new(RefCell::new(Vec::new()));
    let back = Rc::clone(&answers);
    window.on_https_only_go_back(move || back.borrow_mut().push("back"));
    let onward = Rc::clone(&answers);
    window.on_https_only_continue(move || onward.borrow_mut().push("continue"));
    window.show().expect("the window can be shown");
    let pixels = window.window().take_snapshot().expect("the window renders");
    save(&pixels, &format!("https-only-interstitial-{name}.png"));
    // It covers the page area.
    let background = rgb(window.global::<ChromePalette>().get_content_background());
    assert_eq!(pixel(&pixels, 20, 580), background, "{name}");
    assert!(answers.borrow().is_empty());
    window.hide().expect("the window can be hidden");
}

#[test]
fn the_site_security_chrome_renders() {
    slint::platform::set_platform(Box::new(SoftwarePlatform)).expect("no platform is set yet");
    for (theme, name) in THEMES {
        render_popover(theme, name);
        render_warning_popover(theme, name);
        render_interstitial(theme, name);
    }
}
