// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! Renders the browser menu and the About window with Slint's software renderer, so they can be
//! seen without opening them by hand. Set `MERCURY_UI_RENDER_DIR` to keep the PNGs.

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

use mercury_ui::{
    AboutLink, AboutWindow, BrowserMenuCommand, BrowserMenuEntry, ChromePalette, ChromeTheme,
    MainWindow,
};
use slint::platform::software_renderer::{MinimalSoftwareWindow, RepaintBufferType};
use slint::platform::{Key, Platform, PointerEventButton, WindowAdapter, WindowEvent};
use slint::{
    ComponentHandle, LogicalPosition, LogicalSize, ModelRc, PhysicalSize, SharedPixelBuffer,
    VecModel,
};

struct SoftwarePlatform;

impl Platform for SoftwarePlatform {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, slint::PlatformError> {
        Ok(MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer))
    }
}

const THEMES: [(ChromeTheme, &str); 3] = [
    (ChromeTheme::Light, "light"),
    (ChromeTheme::Dark, "dark"),
    // Headless, the system's scheme is unknown, and the chrome then stays dark.
    (ChromeTheme::System, "system"),
];

fn output_path(name: &str) -> PathBuf {
    let directory = std::env::var_os("MERCURY_UI_RENDER_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    directory.join(name)
}

fn save(pixels: &SharedPixelBuffer<slint::Rgba8Pixel>, name: &str) {
    let path = output_path(name);
    image::save_buffer_with_format(
        &path,
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

fn entry(
    title: &str,
    shortcut: &str,
    enabled: bool,
    command: BrowserMenuCommand,
    separator_before: bool,
) -> BrowserMenuEntry {
    BrowserMenuEntry {
        title: title.into(),
        shortcut: shortcut.into(),
        enabled,
        command,
        separator_before,
    }
}

/// The rows `apps/mercury-desktop/src/menus` builds on macOS.
fn rows() -> (ModelRc<BrowserMenuEntry>, ModelRc<BrowserMenuEntry>) {
    (
        ModelRc::new(VecModel::from(vec![entry(
            "New Tab",
            "⌘T",
            true,
            BrowserMenuCommand::NewTab,
            false,
        )])),
        ModelRc::new(VecModel::from(vec![
            entry("Find…", "⌘F", true, BrowserMenuCommand::Find, false),
            entry("Settings…", "⌘,", true, BrowserMenuCommand::Settings, true),
            entry(
                "About Mercury Browser",
                "",
                true,
                BrowserMenuCommand::About,
                false,
            ),
            entry(
                "Mercury Browser Help",
                "",
                true,
                BrowserMenuCommand::Help,
                false,
            ),
            entry(
                "Quit Mercury Browser",
                "⌘Q",
                true,
                BrowserMenuCommand::Quit,
                true,
            ),
        ])),
    )
}

/// Where the browser menu's panel starts: the "⋮" button ends 14 px before the window's right
/// edge, and the popup is 264 px wide (`BrowserMenu` in `browser-menu.slint`).
const MENU_LEFT: u32 = 900 - 14 - 264;
/// A row's height (`BrowserMenuRow`).
const MENU_ROW_HEIGHT: u32 = 30;

/// The top of the row the arrow keys chose. A column 7 px inside the row's left edge (past its
/// rounded corner, before its title and the separators) shows the selection colour for exactly
/// that row, so a menu with no chosen row, or with two, fails here.
fn chosen_row_top(pixels: &SharedPixelBuffer<slint::Rgba8Pixel>, selected: [u8; 3]) -> u32 {
    // The panel's padding (6 px), then the 7 px.
    let column = MENU_LEFT + 6 + 7;
    let rows: Vec<u32> = (0..pixels.height())
        .filter(|&y| pixel(pixels, column, y) == selected)
        .collect();
    let (Some(&first), Some(&last)) = (rows.first(), rows.last()) else {
        panic!("no row of the menu is chosen");
    };
    assert_eq!(
        (rows.len() as u32, last - first + 1),
        (MENU_ROW_HEIGHT, MENU_ROW_HEIGHT),
        "one row, {MENU_ROW_HEIGHT} px tall, is chosen: {rows:?}"
    );
    first
}

fn press(window: &slint::Window, key: impl Into<slint::SharedString> + Clone) {
    window.dispatch_event(WindowEvent::KeyPressed {
        text: key.clone().into(),
    });
    window.dispatch_event(WindowEvent::KeyReleased { text: key.into() });
}

/// The menu open in the browser window, anchored under the "⋮" button; then chosen with the
/// keyboard.
fn render_browser_menu_in_window(theme: ChromeTheme, name: &str) {
    let window = MainWindow::new().expect("the main window can be created");
    window.global::<ChromePalette>().set_theme(theme);
    window.window().set_size(LogicalSize::new(900.0, 560.0));
    let (top, bottom) = rows();
    window.set_browser_menu_top_entries(top);
    window.set_browser_menu_bottom_entries(bottom);
    window.set_zoom_percent(110);
    let chosen = Rc::new(RefCell::new(Vec::new()));
    let recorded = Rc::clone(&chosen);
    window.on_browser_menu_activated(move |command| recorded.borrow_mut().push(command));
    window.show().expect("the main window can be shown");
    window.invoke_show_browser_menu();

    let pixels = window.window().take_snapshot().expect("the window renders");
    save(&pixels, &format!("browser-menu-{name}.png"));
    assert_eq!((pixels.width(), pixels.height()), (900, 560));
    // The "⋮" button ends 14 px before the window's right edge (the toolbar's padding); the
    // panel's right edge lines up with it.
    let panel = rgb(window.global::<ChromePalette>().get_popup_background());
    assert_eq!(pixel(&pixels, 900 - 14 - 130, 140), panel);
    assert_eq!(pixel(&pixels, 900 - 14 - 4, 140), panel);
    assert_ne!(pixel(&pixels, 900 - 14 + 4, 140), panel);

    // The arrow keys walk the enabled rows in order: New Tab, Find…, Settings…, then "About
    // Mercury Browser". Each stop is lower than the one before it, and Find… is a stop.
    let selected = rgb(window.global::<ChromePalette>().get_item_selected());
    let mut tops = Vec::new();
    for stop in 0..4 {
        press(window.window(), Key::DownArrow);
        let pixels = window.window().take_snapshot().expect("the window renders");
        tops.push(chosen_row_top(&pixels, selected));
        if stop == 3 {
            save(&pixels, &format!("browser-menu-keyboard-{name}.png"));
        }
    }
    assert!(
        tops.windows(2).all(|stops| stops[0] < stops[1]),
        "{name}: {tops:?}"
    );
    // Settings… has a separator above it, so it sits more than a row below Find….
    assert!(tops[2] - tops[1] > MENU_ROW_HEIGHT, "{name}: {tops:?}");
    press(window.window(), Key::Return);
    assert_eq!(*chosen.borrow(), [BrowserMenuCommand::About]);
    window.hide().expect("the main window can be hidden");
}

fn click(window: &slint::Window, x: f32, y: f32) {
    let position = LogicalPosition::new(x, y);
    window.dispatch_event(WindowEvent::PointerMoved { position });
    window.dispatch_event(WindowEvent::PointerPressed {
        position,
        button: PointerEventButton::Left,
    });
    window.dispatch_event(WindowEvent::PointerReleased {
        position,
        button: PointerEventButton::Left,
    });
}

/// Where the About window's links are: the rows they fill and, left to right, the first and last
/// column of each link.
#[derive(Debug)]
struct LinkRow {
    top: u32,
    bottom: u32,
    runs: Vec<(u32, u32)>,
}

/// Finds the links by their colour. They are the first text in the accent colour below the icon
/// (which has orange of its own), and the Slint badge below the separator has none; the end of
/// the search is where the separator is. Antialiased edges are left out by asking for a colour
/// close to the accent. Ink less than `LINK_GAP` apart is one link (the gap in a link is a
/// space, between links it is the layout's 18px).
fn link_row(pixels: &SharedPixelBuffer<slint::Rgba8Pixel>, accent: [u8; 3]) -> LinkRow {
    const BELOW_THE_ICON: u32 = 110;
    const ABOVE_THE_SEPARATOR: u32 = 285;
    const LINK_GAP: u32 = 12;
    let is_accent = |[r, g, b]: [u8; 3]| {
        r.abs_diff(accent[0]) <= 48 && g.abs_diff(accent[1]) <= 48 && b.abs_diff(accent[2]) <= 48
    };
    let mut rows = (BELOW_THE_ICON..ABOVE_THE_SEPARATOR)
        .filter(|&y| (0..pixels.width()).any(|x| is_accent(pixel(pixels, x, y))));
    let top = rows
        .next()
        .expect("the links have rows of the accent colour");
    let mut bottom = top;
    for y in rows {
        if y > bottom + 1 {
            break;
        }
        bottom = y;
    }
    let mut runs: Vec<(u32, u32)> = Vec::new();
    for x in (0..pixels.width()).filter(|&x| (top..=bottom).any(|y| is_accent(pixel(pixels, x, y))))
    {
        match runs.last_mut() {
            Some(run) if x - run.1 <= LINK_GAP => run.1 = x,
            _ => runs.push((x, x)),
        }
    }
    LinkRow { top, bottom, runs }
}

fn render_about(theme: ChromeTheme, name: &str) -> ([u8; 3], [u8; 3]) {
    let about = AboutWindow::new().expect("the About window can be created");
    about.global::<ChromePalette>().set_theme(theme);
    about.set_version_line("Version 0.2.0".into());
    about.set_engine_line("Servo 0.8.0 web engine".into());
    about.set_media_line("GStreamer 1.28.9 media framework".into());
    about.set_copyright_line(
        "Copyright © 2026 Mrinal Joshi and the Mercury Browser contributors. Portions © 2026 Kayk Caputo (Hermes Browser)."
            .into(),
    );
    about.set_licence_line(
        "Source-available: free for non-commercial use (PolyForm Noncommercial 1.0.0; MPL-2.0 \
         for parts from Hermes and Servo)."
            .into(),
    );
    about.set_credit_line("Began as a fork of Hermes Browser by Kayk Caputo.".into());
    let clicked = Rc::new(RefCell::new(Vec::new()));
    about.on_open_link({
        let clicked = Rc::clone(&clicked);
        move |link| clicked.borrow_mut().push(link)
    });
    // A software window has no size until it is given one; this is the size the window declares.
    about.window().set_size(LogicalSize::new(380.0, 420.0));
    about.show().expect("the About window can be shown");
    let pixels = about
        .window()
        .take_snapshot()
        .expect("the About window renders");
    save(&pixels, &format!("about-{name}.png"));
    assert_eq!(about.window().size(), PhysicalSize::new(380, 420));

    // Licence, Third-party notices and Source code: three links in the accent colour on one
    // line, inside the window's padding, and each one opens its own page.
    let accent = rgb(about.global::<ChromePalette>().get_accent());
    let links = link_row(&pixels, accent);
    assert_eq!(links.runs.len(), 3, "{name}: {links:?}");
    assert!(links.bottom - links.top < 20, "{name}: one line: {links:?}");
    let (first, last) = (links.runs[0], links.runs[2]);
    assert!(first.0 >= 24, "{name}: inside the padding: {links:?}");
    assert!(last.1 < 380 - 24, "{name}: inside the padding: {links:?}");
    assert!(
        first.0.abs_diff(379 - last.1) <= 6,
        "{name}: the links are centred: {links:?}"
    );
    let middle = links.top + (links.bottom - links.top) / 2;
    for run in &links.runs {
        let x = (run.0 + run.1) as f32 / 2.0;
        click(about.window(), x, middle as f32);
    }
    assert_eq!(
        *clicked.borrow(),
        [
            AboutLink::Licence,
            AboutLink::ThirdPartyNotices,
            AboutLink::SourceCode
        ],
        "{name}"
    );
    about.hide().expect("the About window can be hidden");
    (
        pixel(&pixels, 4, 4),
        rgb(about.global::<ChromePalette>().get_window_background()),
    )
}

fn rgb(color: slint::Color) -> [u8; 3] {
    [color.red(), color.green(), color.blue()]
}

#[test]
fn the_browser_menu_and_the_about_window_render() {
    slint::platform::set_platform(Box::new(SoftwarePlatform)).expect("no platform is set yet");

    let brightness = |[r, g, b]: [u8; 3]| u32::from(r) + u32::from(g) + u32::from(b);
    for (theme, name) in THEMES {
        render_browser_menu_in_window(theme, name);
        let (background, expected) = render_about(theme, name);
        assert_eq!(background, expected, "{name}");
        match theme {
            ChromeTheme::Light => assert!(brightness(background) > 600, "{name}: {background:?}"),
            _ => assert!(brightness(background) < 150, "{name}: {background:?}"),
        }
    }
}
