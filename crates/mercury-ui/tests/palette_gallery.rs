// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! Palette gallery: the 7 scenes of `theme_scenes.rs` drawn by Slint's software renderer for every
//! palette a user can choose (`palettes::selectable`), in Light and in Dark, with the Mercury mark
//! in each palette's accent.
//!
//! Set `MERCURY_PALETTE_DIR` to save them as `<dir>/<palette>/<n>-<scene>-<scheme>.png`, the file
//! names of the theme-plumbing owner set.

use std::cell::Cell;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Duration;

use mercury_appearance::{Metrics, Role, Scheme, palettes};
use mercury_ui::{
    AboutWindow, BrowserMenuCommand, BrowserMenuEntry, BrowserTab, ChromeMetrics, ChromePalette,
    ChromeTheme, Look, MainWindow, SettingsWindow, TabAudioState, apply_look,
};
use slint::platform::software_renderer::{MinimalSoftwareWindow, RepaintBufferType};
use slint::platform::{Platform, WindowAdapter, WindowEvent};
use slint::{
    ComponentHandle, Global, LogicalPosition, LogicalSize, ModelRc, Rgba8Pixel, SharedPixelBuffer,
    VecModel,
};

struct SoftwarePlatform;

impl Platform for SoftwarePlatform {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, slint::PlatformError> {
        Ok(MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer))
    }

    fn duration_since_start(&self) -> Duration {
        CLOCK.with(Cell::get)
    }
}

thread_local! {
    /// The platform's clock: each drawing step moves it by a fixed amount, so that an animation
    /// (a tab's hover fade) is at the same frame whenever the picture is taken.
    static CLOCK: Cell<Duration> = const { Cell::new(Duration::ZERO) };
}

fn advance_clock() {
    CLOCK.with(|clock| clock.set(clock.get() + Duration::from_millis(100)));
}

fn init_platform() {
    let _ = slint::platform::set_platform(Box::new(SoftwarePlatform));
}

type Pixels = SharedPixelBuffer<Rgba8Pixel>;

#[derive(Clone, Copy, Debug)]
enum Scene {
    NewTab,
    Tabs,
    TabsHover,
    PopUpBlocked,
    BrowserMenu,
    About,
    Settings,
}

const SCENES: [Scene; 7] = [
    Scene::NewTab,
    Scene::Tabs,
    Scene::TabsHover,
    Scene::PopUpBlocked,
    Scene::BrowserMenu,
    Scene::About,
    Scene::Settings,
];

/// Where the pointer rests in the hovered-tab scene: the middle of the third tab, "Example Domain".
const HOVERED_TAB: (f32, f32) = (300.0, 26.0);

const WIDTH: u32 = 900;
const HEIGHT: u32 = 560;

/// How a scene is lit: nothing pushed, or a look pushed.
#[derive(Clone, Copy)]
enum Lit {
    #[allow(dead_code)]
    BuiltIn,
    Pushed(&'static str),
}

impl Lit {
    fn name(self) -> &'static str {
        match self {
            Self::BuiltIn => "builtin",
            Self::Pushed(id) => id,
        }
    }
}

fn look(id: &str) -> Look {
    let palette = palettes::find(id).expect("a palette this build has");
    Look::new(palette, palette, Metrics::ROOMY)
}

fn tab(title: &str, active: bool, pinned: bool, audio: TabAudioState) -> BrowserTab {
    BrowserTab {
        title: title.into(),
        active,
        pinned,
        audio,
        id: title.len() as i32,
        ..Default::default()
    }
}

fn entry(title: &str, shortcut: &str, command: BrowserMenuCommand, line: bool) -> BrowserMenuEntry {
    BrowserMenuEntry {
        title: title.into(),
        shortcut: shortcut.into(),
        enabled: true,
        command,
        separator_before: line,
    }
}

/// Gives the window its look and theme, shows it, lets animations settle and takes the picture.
fn picture<C>(
    window: &C,
    theme: ChromeTheme,
    lit: Lit,
    size: (u32, u32),
    hover: Option<(f32, f32)>,
) -> Pixels
where
    C: ComponentHandle,
    for<'a> ChromePalette<'a>: Global<'a, C>,
    for<'a> ChromeMetrics<'a>: Global<'a, C>,
{
    if let Lit::Pushed(id) = lit {
        apply_look(window, &look(id));
    }
    window.global::<ChromePalette>().set_theme(theme);
    window
        .window()
        .set_size(LogicalSize::new(size.0 as f32, size.1 as f32));
    window.show().expect("the window can be shown");
    if let Some((x, y)) = hover {
        window.window().dispatch_event(WindowEvent::PointerMoved {
            position: LogicalPosition::new(x, y),
        });
    }
    for _ in 0..3 {
        advance_clock();
        slint::platform::update_timers_and_animations();
        window.window().request_redraw();
        let _ = window.window().take_snapshot();
    }
    window.window().take_snapshot().expect("the window renders")
}

fn main_window(scene: Scene) -> MainWindow {
    let window = MainWindow::new().expect("the main window can be created");
    match scene {
        Scene::NewTab => {}
        Scene::Tabs | Scene::TabsHover | Scene::PopUpBlocked => {
            window.set_tabs(ModelRc::new(VecModel::from(vec![
                tab("Mail", false, true, TabAudioState::Silent),
                tab("Opener", true, false, TabAudioState::Silent),
                tab("Example Domain", false, false, TabAudioState::Playing),
            ])));
            window.set_pinned_tab_count(1);
            window.set_active_tab_index(1);
            window.set_address("https://news.example/article".into());
            window.set_is_new_tab(false);
            if matches!(scene, Scene::PopUpBlocked) {
                window.set_pop_up_blocked_origin("https://news.example".into());
            }
        }
        Scene::BrowserMenu => {
            window.set_browser_menu_top_entries(ModelRc::new(VecModel::from(vec![entry(
                "New Tab",
                "⌘T",
                BrowserMenuCommand::NewTab,
                false,
            )])));
            window.set_browser_menu_bottom_entries(ModelRc::new(VecModel::from(vec![
                entry("Find…", "⌘F", BrowserMenuCommand::Find, false),
                entry("Settings…", "⌘,", BrowserMenuCommand::Settings, true),
                entry("Quit Mercury Browser", "⌘Q", BrowserMenuCommand::Quit, true),
            ])));
        }
        Scene::About | Scene::Settings => unreachable!("not a main-window scene"),
    }
    window
}

fn render(scene: Scene, theme: ChromeTheme, lit: Lit) -> Pixels {
    let pixels = match scene {
        Scene::About => {
            let about = AboutWindow::new().expect("the About window can be created");
            about.set_version_line("Version 0.2.0".into());
            picture(&about, theme, lit, (520, 480), None)
        }
        Scene::Settings => {
            let settings = SettingsWindow::new().expect("the settings window can be created");
            picture(&settings, theme, lit, (760, 640), None)
        }
        _ => {
            let window = main_window(scene);
            let size = (WIDTH, HEIGHT);
            if matches!(scene, Scene::BrowserMenu) {
                window
                    .window()
                    .set_size(LogicalSize::new(WIDTH as f32, HEIGHT as f32));
                window.show().expect("the window can be shown");
                window.invoke_show_browser_menu();
            }
            let hover = matches!(scene, Scene::TabsHover).then_some(HOVERED_TAB);
            picture(&window, theme, lit, size, hover)
        }
    };
    save(&pixels, scene, theme, lit);
    pixels
}

fn save(pixels: &Pixels, scene: Scene, theme: ChromeTheme, lit: Lit) {
    let Some(directory) = std::env::var_os("MERCURY_PALETTE_DIR").map(PathBuf::from) else {
        return;
    };
    let directory = directory.join(lit.name());
    std::fs::create_dir_all(&directory).expect("the folder can be made");
    let scheme = if matches!(theme, ChromeTheme::Dark) {
        "dark"
    } else {
        "light"
    };
    let scene = match scene {
        Scene::NewTab => "1-main-window-new-tab",
        Scene::Tabs => "2-main-window-tabs-active",
        Scene::TabsHover => "3-main-window-tabs-hovered",
        Scene::PopUpBlocked => "4-main-window-popup-bar",
        Scene::BrowserMenu => "5-main-window-browser-menu",
        Scene::About => "6-about-window",
        Scene::Settings => "7-settings-window",
    };
    let name = format!("{scene}-{scheme}.png");
    image::save_buffer_with_format(
        directory.join(name),
        pixels.as_bytes(),
        pixels.width(),
        pixels.height(),
        image::ExtendedColorType::Rgba8,
        image::ImageFormat::Png,
    )
    .expect("the picture can be saved");
}

const THEMES: [(ChromeTheme, Scheme); 2] = [
    (ChromeTheme::Light, Scheme::Light),
    (ChromeTheme::Dark, Scheme::Dark),
];

#[test]
fn every_chosen_palette_is_drawn_in_every_scene_with_the_mark_in_its_accent() {
    init_platform();
    for palette in palettes::selectable() {
        for scene in SCENES {
            for (theme, scheme) in THEMES {
                let pixels = render(scene, theme, Lit::Pushed(palette.id));
                let corner = rgb(&pixels, 3, 3);
                if matches!(scene, Scene::Tabs) {
                    let frame = palette.colour(scheme, Role::WindowBackground);
                    assert_eq!(
                        corner,
                        [frame.r, frame.g, frame.b],
                        "{} {scheme:?}",
                        palette.id
                    );
                }
                // The new-tab page and the About window draw the mark large: its disc is the
                // palette's fill accent, thousands of pixels of exactly that colour.
                if matches!(scene, Scene::NewTab | Scene::About) {
                    let fill = palette.colour(scheme, Role::PrimaryButtonBackground);
                    let disc = pixels
                        .as_slice()
                        .iter()
                        .filter(|p| [p.r, p.g, p.b] == [fill.r, fill.g, fill.b])
                        .count();
                    assert!(
                        disc > 2000,
                        "{scene:?} {} {scheme:?}: the mark's disc has {disc} px of {}",
                        palette.id,
                        fill.to_hex()
                    );
                }
            }
        }
    }
}

fn rgb(pixels: &Pixels, x: u32, y: u32) -> [u8; 3] {
    let pixel = pixels.as_slice()[(y * pixels.width() + x) as usize];
    [pixel.r, pixel.g, pixel.b]
}
