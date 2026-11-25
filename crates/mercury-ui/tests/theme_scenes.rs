// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! The theme layer in pictures, drawn by Slint's software renderer (no window, no GPU).
//!
//! - **G1, a push is the built-in look:** for each scene in Light and in Dark, the pixels of a
//!   window nobody pushed to equal, bit for bit, the pixels after `apply_look(Look::hermes_roomy())`.
//! - **G3, B2 reaches the pixels:** with `mercury-b2` pushed, known points of the main window
//!   (the window frame, the toolbar, the address field, the active tab) have the colour the B2
//!   table gives that role in each scheme, and the picture differs from the built-in one.
//!
//! The settings window is mostly drawn by Slint's standard widgets: only its accents (selection,
//! focus ring, danger, notice) come from the chrome palette.
//!
//! Set `MERCURY_UI_RENDER_DIR` to also save the pictures (`scene-<scene>-<scheme>-<look>.png`).

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
    let Some(directory) = std::env::var_os("MERCURY_UI_RENDER_DIR").map(PathBuf::from) else {
        return;
    };
    let scheme = if matches!(theme, ChromeTheme::Dark) {
        "dark"
    } else {
        "light"
    };
    let name = format!("scene-{scene:?}-{scheme}-{}.png", lit.name()).to_lowercase();
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
fn g1_pushing_hermes_and_roomy_draws_what_the_built_in_look_draws() {
    init_platform();
    let mut differing = Vec::new();
    for scene in SCENES {
        for (theme, scheme) in THEMES {
            let built_in = render(scene, theme, Lit::BuiltIn);
            let pushed = render(scene, theme, Lit::Pushed("hermes"));
            assert_eq!(
                (built_in.width(), built_in.height()),
                (pushed.width(), pushed.height())
            );
            if built_in.as_bytes() != pushed.as_bytes() {
                differing.push(format!("{scene:?} in {scheme:?}"));
            }
        }
    }
    assert!(differing.is_empty(), "pictures that differ: {differing:?}");
}

fn rgb(pixels: &Pixels, x: u32, y: u32) -> [u8; 3] {
    let pixel = pixels.as_slice()[(y * pixels.width() + x) as usize];
    [pixel.r, pixel.g, pixel.b]
}

fn role_colour(id: &str, scheme: Scheme, role: Role) -> [u8; 3] {
    let colour = palettes::find(id)
        .expect("a palette this build has")
        .colour(scheme, role);
    [colour.r, colour.g, colour.b]
}

#[test]
fn g3_b2_reaches_the_pixels_of_the_main_window() {
    init_platform();
    // The window frame is the tab row's padding; the toolbar's background is left of its first
    // button; the address field is empty to the right of the typed address; the active tab is
    // the second one (after the pinned "Mail"), above its title.
    let points = [
        ("the window frame", 3, 3, Role::WindowBackground),
        ("the toolbar", 3, 53, Role::ToolbarBackground),
        ("the address field", 600, 78, Role::FieldBackground),
        ("the active tab", 100, 12, Role::TabActive),
    ];
    for (theme, scheme) in THEMES {
        let hermes = render(Scene::Tabs, theme, Lit::Pushed("hermes"));
        let b2 = render(Scene::Tabs, theme, Lit::Pushed("mercury-b2"));
        for (what, x, y, role) in points {
            assert_eq!(
                rgb(&b2, x, y),
                role_colour("mercury-b2", scheme, role),
                "{what} in {scheme:?}, B2 pushed"
            );
            assert_eq!(
                rgb(&hermes, x, y),
                role_colour("hermes", scheme, role),
                "{what} in {scheme:?}, Hermes pushed"
            );
        }
        assert_ne!(
            b2.as_bytes(),
            hermes.as_bytes(),
            "B2 is drawn in {scheme:?}"
        );
    }
}

#[test]
fn g3_b2_reaches_a_hovered_tab() {
    init_platform();
    for (theme, scheme) in THEMES {
        let still = render(Scene::Tabs, theme, Lit::Pushed("mercury-b2"));
        let hovered = render(Scene::TabsHover, theme, Lit::Pushed("mercury-b2"));
        // Above the title of the third tab: the strip's own colour, and with the pointer on it
        // the tab's hover fill.
        assert_eq!(
            rgb(&still, 300, 12),
            role_colour("mercury-b2", scheme, Role::WindowBackground),
            "an unhovered tab in {scheme:?}"
        );
        assert_eq!(
            rgb(&hovered, 300, 12),
            role_colour("mercury-b2", scheme, Role::TabHover),
            "a hovered tab in {scheme:?}"
        );
    }
}

/// How many pixels of the picture are exactly `colour`.
fn count(pixels: &Pixels, colour: [u8; 3]) -> usize {
    pixels
        .as_slice()
        .iter()
        .filter(|pixel| [pixel.r, pixel.g, pixel.b] == colour)
        .count()
}

#[test]
fn g3_b2_reaches_the_about_window() {
    init_platform();
    for (theme, scheme) in THEMES {
        let built_in = render(Scene::About, theme, Lit::BuiltIn);
        let b2 = render(Scene::About, theme, Lit::Pushed("mercury-b2"));
        let colour = |id, role| role_colour(id, scheme, role);
        // The window's own background fills its corner, and the heading is drawn in `text`.
        assert_eq!(
            rgb(&b2, 3, 3),
            colour("mercury-b2", Role::WindowBackground),
            "the About window's background in {scheme:?}"
        );
        assert_eq!(
            rgb(&built_in, 3, 3),
            colour("hermes", Role::WindowBackground)
        );
        assert!(
            count(&b2, colour("mercury-b2", Role::Text)) > 50,
            "the About window has B2's text colour in {scheme:?}"
        );
        assert_eq!(
            count(&b2, colour("hermes", Role::Text)),
            0,
            "and none of Hermes's in {scheme:?}"
        );
    }
}

/// With `MERCURY_UI_RENDER_DIR` set this writes the pictures of every scene in B2, the ones
/// the owner looks at; without it, it checks that B2 changes every scene (the settings window takes only its accents from
/// the palette: selection, focus ring, danger and notice; its body is drawn by Slint's standard widgets).
#[test]
fn b2_is_drawn_in_every_scene() {
    init_platform();
    for scene in SCENES {
        for (theme, scheme) in THEMES {
            let built_in = render(scene, theme, Lit::BuiltIn);
            let b2 = render(scene, theme, Lit::Pushed("mercury-b2"));
            assert_ne!(
                built_in.as_bytes(),
                b2.as_bytes(),
                "{scene:?} in {scheme:?}"
            );
        }
    }
}
