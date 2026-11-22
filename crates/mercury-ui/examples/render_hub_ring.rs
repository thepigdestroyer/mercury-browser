// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! Renders the hub ring popover (SPEC §4.3) to PNG files without a display, through Slint's software
//! renderer: the four `ring-*` frames of the design (`ring-open`, `ring-search`, `ring-keys`,
//! `ring-status` and `ring-status-loading`) and the crowded and sparse rings (`pinned-8`,
//! `pinned-only`, `one-tab`, `three-groups`, `crowded-64`, `focus-ring`), each in light and dark, at
//! 2x on the page backdrop with the toolbar, the hub button and the tail
//! (`RingPopoverPreview { show-chrome: true }`, 1000 x 640 logical px).
//!
//! `cargo run --release -p mercury-ui --features test-support --example render_hub_ring -- <dir>`
//!
//! `tests/hub_ring_render.rs` builds the same scenes through this file's `scenes` module and checks
//! their pixels.

#[cfg(feature = "test-support")]
pub mod scenes {
    use std::error::Error;
    use std::path::Path;
    use std::rc::Rc;

    use mercury_ui::hub_ring::{HubRing, HubRingHost, HubRingInput, fixtures};
    use mercury_ui::{ChromePalette, ChromeTheme, RingPopoverPreview, ring_bands};
    use slint::platform::software_renderer::{MinimalSoftwareWindow, RepaintBufferType};
    use slint::platform::{Key, Platform, PlatformError, WindowAdapter, WindowEvent};
    use slint::{ComponentHandle, PhysicalSize, Rgba8Pixel, SharedPixelBuffer, SharedString};

    pub const SCALE: f32 = 2.0;
    /// The logical size of the window: the mockup's detail around the toolbar and the popover.
    pub const WIDTH: u32 = 1000;
    pub const HEIGHT: u32 = 640;
    /// The circle box of the popover in the window, and the tip of the tail.
    pub const RING_X: f32 = 80.0;
    pub const RING_Y: f32 = 84.0;
    pub const TAIL_TIP: (f32, f32) = (196.0, 40.0);

    pub const SCENES: [&str; 11] = [
        "ring-open",
        "ring-search",
        "ring-keys",
        "ring-status",
        "ring-status-loading",
        "pinned-8",
        "pinned-only",
        "one-tab",
        "three-groups",
        "crowded-64",
        "focus-ring",
    ];

    struct SoftwarePlatform;

    impl Platform for SoftwarePlatform {
        fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, PlatformError> {
            Ok(MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer))
        }
    }

    /// Slint's platform belongs to the thread; a second call on the same thread is harmless.
    pub fn init_platform() {
        let _ = slint::platform::set_platform(Box::new(SoftwarePlatform));
    }

    /// A host that does nothing: the scenes only look.
    struct LookOnly;

    impl HubRingHost for LookOnly {
        fn activate_tab(&self, _tab: i32) {}
        fn close_tab(&self, _tab: i32) {}
        fn toggle_mute(&self, _tab: i32) {}
        fn toggle_pause(&self, _tab: i32) {}
        fn move_tab(&self, _tab: i32, _group: i32, _index: usize) {}
        fn open_orbit(&self) {}
        fn search_web(&self, _query: &str) {}
    }

    /// One scene: the window, its controller (keep it alive while the window is drawn) and the rows.
    pub struct Scene {
        pub window: RingPopoverPreview,
        pub ring: HubRing,
        pub input: HubRingInput,
    }

    fn extra_pinned(input: &mut HubRingInput, count: usize) {
        let template = fixtures::pinned_tabs();
        for extra in 0..count {
            let mut tab = template[extra % template.len()].clone();
            tab.id = 300 + extra as i32;
            tab.title = format!("Pinned extra {extra}").into();
            input.tabs.push(tab);
        }
    }

    /// The rows of a scene.
    pub fn input_for(name: &str, dark: bool) -> HubRingInput {
        let mut input = fixtures::mockup_input(dark);
        input.reduce_motion = true;
        match name {
            "pinned-8" => extra_pinned(&mut input, 5),
            "pinned-only" => {
                input.groups.clear();
                input.tabs.retain(|tab| tab.pinned);
                input.media.clear();
            }
            "one-tab" => {
                input.groups.truncate(1);
                input.groups[0].count = 1;
                input.tabs.retain(|tab| tab.id == 101 || tab.pinned);
                input.media.clear();
            }
            "three-groups" => {
                input.groups.truncate(3);
                input.tabs.retain(|tab| tab.group <= 3 || tab.pinned);
            }
            "crowded-64" => {
                let (groups, mut tabs) = ring_bands::fixtures::generated(64, 200, dark);
                tabs.extend(fixtures::pinned_tabs());
                input.groups = groups;
                input.tabs = tabs;
                input.media.clear();
            }
            _ => {}
        }
        input
    }

    fn hover(scene: &Scene, tab: i32) {
        let bridge = scene.window.global::<mercury_ui::HubRingBridge>();
        bridge.set_hovered_tab(tab);
        bridge.invoke_tab_hovered(tab);
    }

    fn press(scene: &Scene, key: &str, alt: bool) {
        let bridge = scene.window.global::<mercury_ui::HubRingBridge>();
        bridge.invoke_key_pressed(SharedString::from(key), alt, false, false, false);
    }

    /// Builds the scene `name` (one of [`SCENES`]) in light or dark, shown and in its state.
    pub fn scene(name: &str, dark: bool) -> Result<Scene, Box<dyn Error>> {
        let window = RingPopoverPreview::new()?;
        window.set_show_chrome(true);
        window.global::<ChromePalette>().set_theme(if dark {
            ChromeTheme::Dark
        } else {
            ChromeTheme::Light
        });
        window.show()?;
        let input = input_for(name, dark);
        let ring = HubRing::install(&window, Rc::new(LookOnly), ring_bands::metrics());
        ring.set_input(input.clone());
        ring.opened();
        let scene = Scene {
            window,
            ring,
            input,
        };
        let bridge = scene.window.global::<mercury_ui::HubRingBridge>();
        match name {
            // The pointer is on "Essay on maps".
            "ring-open" => hover(&scene, 111),
            "ring-search" => {
                bridge.set_query("tram".into());
                bridge.invoke_query_edited("tram".into());
            }
            // ⌥ S: Lisbon trip, digits 1 to 4 on its tabs.
            "ring-keys" => {
                scene.ring.set_held(true);
                press(&scene, "s", true);
            }
            // The audio tab is hovered: Pause and Mute.
            "ring-status" => hover(&scene, 115),
            // The loading tab is hovered: its tooltip.
            "ring-status-loading" => hover(&scene, 102),
            // Keyboard focus on the third tab.
            "focus-ring" => {
                let right = char::from(Key::RightArrow).to_string();
                press(&scene, &right, false);
                press(&scene, &right, false);
            }
            _ => {}
        }
        Ok(scene)
    }

    /// The window drawn at [`SCALE`]: `WIDTH * 2` by `HEIGHT * 2` pixels.
    pub fn pixels(window: &slint::Window) -> Result<SharedPixelBuffer<Rgba8Pixel>, Box<dyn Error>> {
        window.dispatch_event(WindowEvent::ScaleFactorChanged {
            scale_factor: SCALE,
        });
        window.set_size(PhysicalSize::new(
            (WIDTH as f32 * SCALE) as u32,
            (HEIGHT as f32 * SCALE) as u32,
        ));
        for _ in 0..3 {
            slint::platform::update_timers_and_animations();
            window.request_redraw();
            let _ = window.take_snapshot();
        }
        Ok(window.take_snapshot()?)
    }

    pub fn save(pixels: &SharedPixelBuffer<Rgba8Pixel>, path: &Path) -> Result<(), Box<dyn Error>> {
        let bytes = pixels
            .as_slice()
            .iter()
            .flat_map(|pixel| [pixel.r, pixel.g, pixel.b])
            .collect::<Vec<_>>();
        image::save_buffer(
            path,
            &bytes,
            pixels.width(),
            pixels.height(),
            image::ExtendedColorType::Rgb8,
        )?;
        Ok(())
    }
}

#[cfg(feature = "test-support")]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    use slint::ComponentHandle;

    let output = std::path::PathBuf::from(
        std::env::args()
            .nth(1)
            .ok_or("usage: render_hub_ring <output directory>")?,
    );
    std::fs::create_dir_all(&output)?;
    scenes::init_platform();
    for name in scenes::SCENES {
        for (dark, scheme) in [(false, "light"), (true, "dark")] {
            let scene = scenes::scene(name, dark)?;
            let pixels = scenes::pixels(scene.window.window())?;
            let path = output.join(format!("{name}-{scheme}.png"));
            scenes::save(&pixels, &path)?;
            println!("wrote {}", path.display());
            scene.window.hide()?;
        }
    }
    Ok(())
}

#[cfg(not(feature = "test-support"))]
fn main() {
    eprintln!("render_hub_ring needs `--features test-support` (the hub ring's test scenes)");
}
