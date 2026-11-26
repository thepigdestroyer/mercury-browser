// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
#![cfg(feature = "test-support")]
//! The palette picker's drawings, in pixels (lane polish-1). Each card of the Appearance section
//! draws its palette twice, in light and in dark. For every selectable palette, the window area of
//! each drawing is that palette's `window` colour in its own scheme, and the accent button of each
//! drawing is the palette's accent. The section is drawn by Slint's software renderer, the same way
//! `palette_gallery.rs` draws the browser's windows.
//!
//! `cargo test --release -p mercury-ui --all-features --test settings_palette_picker`

use std::cell::Cell;
use std::rc::Rc;
use std::time::Duration;

use mercury_appearance::panels::palette_panel::{PaletteSwatch, palette_panel};
use mercury_appearance::palettes::selectable;
use mercury_appearance::{Colour, Scheme};
use mercury_settings::AppearanceSettings;
use mercury_ui::SettingsWindow;
use mercury_ui::settings_sections::{appearance, section};
use slint::ComponentHandle;
use slint::platform::software_renderer::{MinimalSoftwareWindow, RepaintBufferType};
use slint::platform::{Platform, WindowAdapter};

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
    static CLOCK: Cell<Duration> = const { Cell::new(Duration::ZERO) };
}

fn init_platform() {
    let _ = slint::platform::set_platform(Box::new(SoftwarePlatform));
}

const WIDTH: u32 = 900;
const HEIGHT: u32 = 2600;

type Pixels = slint::SharedPixelBuffer<slint::Rgba8Pixel>;

fn picture(window: &SettingsWindow) -> Pixels {
    window
        .window()
        .set_size(slint::LogicalSize::new(WIDTH as f32, HEIGHT as f32));
    window.show().expect("the window can be shown");
    for _ in 0..3 {
        CLOCK.with(|clock| clock.set(clock.get() + Duration::from_millis(100)));
        slint::platform::update_timers_and_animations();
        window.window().request_redraw();
        let _ = window.window().take_snapshot();
    }
    window.window().take_snapshot().expect("the window renders")
}

fn is(colour: Colour, rgb: [u8; 3]) -> bool {
    [colour.r, colour.g, colour.b] == rgb
}

/// How many pixels of the picture are the swatch's window colour.
fn window_area(pixels: &Pixels, swatch: &PaletteSwatch) -> usize {
    let target = swatch.window;
    pixels
        .as_slice()
        .iter()
        .filter(|p| is(target, [p.r, p.g, p.b]))
        .count()
}

#[test]
fn every_selectable_palette_draws_its_own_light_and_dark_window_colour() {
    init_platform();
    for (index, palette) in selectable().enumerate() {
        let mut settings = AppearanceSettings::default();
        mercury_appearance::panels::palette_panel::set_palette(&mut settings, index);
        let window = SettingsWindow::new().expect("a window");
        window.set_section(section::APPEARANCE);
        window.set_theme_index(0);
        window.set_show_bookmarks_bar(true);
        appearance::fill(
            &window,
            &mercury_appearance::panels::layout_panel::layout_panel(&settings, true),
        );
        appearance::fill_palettes(&window, &palette_panel(&settings));
        let pixels = picture(&window);

        let panel = palette_panel(&settings);
        let card = &panel.cards[index];
        let light = PaletteSwatch::of(palette, Scheme::Light);
        let dark = PaletteSwatch::of(palette, Scheme::Dark);
        assert_eq!(card.light.window, light.window, "{}", palette.id);
        assert_eq!(card.dark.window, dark.window, "{}", palette.id);

        let light_area = window_area(&pixels, &light);
        let dark_area = window_area(&pixels, &dark);
        assert!(
            light_area > 1000,
            "{}: the light window colour covers {light_area} px",
            palette.id
        );
        assert!(
            dark_area > 1000,
            "{}: the dark window colour covers {dark_area} px",
            palette.id
        );
    }
}

#[test]
fn every_selectable_palette_draws_its_accent_on_the_card_buttons() {
    init_platform();
    for (index, palette) in selectable().enumerate() {
        let mut settings = AppearanceSettings::default();
        mercury_appearance::panels::palette_panel::set_palette(&mut settings, index);
        let window = SettingsWindow::new().expect("a window");
        window.set_section(section::APPEARANCE);
        window.set_theme_index(0);
        window.set_show_bookmarks_bar(true);
        appearance::fill(
            &window,
            &mercury_appearance::panels::layout_panel::layout_panel(&settings, true),
        );
        appearance::fill_palettes(&window, &palette_panel(&settings));
        let pixels = picture(&window);

        let light = PaletteSwatch::of(palette, Scheme::Light);
        let accent = light.accent;
        let count = pixels
            .as_slice()
            .iter()
            .filter(|p| is(accent, [p.r, p.g, p.b]))
            .count();
        assert!(
            count > 20,
            "{}: the accent {} covers {count} px",
            palette.id,
            accent.to_hex()
        );
    }
}
