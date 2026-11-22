// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! Renders the settings window's Appearance section to PNG files without a display, through
//! Slint's software renderer (no window, no Metal): every state of the accessibility goldens
//! (Mercury defaults, Classic chosen, Orbit and the dial off, the note that the interface is not in
//! yet), plus the cards and a switch with keyboard focus, in the light and the dark scheme, at the
//! narrowest window (560 px) and at 900 px. The section is filled through the real bridge from
//! `mercury_appearance::panels::layout_panel`; all data is the defaults or a change to them.
//!
//! `cargo run --release -p mercury-ui --example render_settings_appearance -- <output directory>`

use std::cell::RefCell;
use std::error::Error;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use mercury_appearance::panels::layout_panel::{
    Choice, Control, layout_panel, set_choice, set_switch,
};
use mercury_settings::AppearanceSettings;
use mercury_ui::SettingsWindow;
use mercury_ui::settings_sections::appearance;
use mercury_ui::settings_sections::section;
use slint::language::ColorScheme;
use slint::platform::software_renderer::{MinimalSoftwareWindow, RepaintBufferType};
use slint::platform::{Platform, PlatformError, WindowAdapter, WindowEvent};
use slint::{ComponentHandle, PhysicalSize, Rgb8Pixel, SharedString};

const SCALE: f32 = 2.0;

/// The Tab stops before the first card: the sidebar's seven tabs, the Theme list and the two
/// check boxes above the section, and one more to land on the first card.
const TABS_TO_FIRST_CARD: usize = 7 + 1 + 2 + 1;

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

/// Renders the window at `logical` pixels (twice that many physical) to `path`.
fn render(window: &slint::Window, logical: (u32, u32), path: &Path) -> Result<(), Box<dyn Error>> {
    window.dispatch_event(WindowEvent::ScaleFactorChanged {
        scale_factor: SCALE,
    });
    window.set_size(PhysicalSize::new(logical.0 * 2, logical.1 * 2));
    // Let layouts, bindings and any animation settle.
    for _ in 0..3 {
        slint::platform::update_timers_and_animations();
        window.request_redraw();
        let _ = window.take_snapshot();
    }
    let snapshot = window.take_snapshot()?;
    let bytes = snapshot
        .as_slice()
        .iter()
        .map(|pixel| Rgb8Pixel::new(pixel.r, pixel.g, pixel.b))
        .flat_map(|pixel| [pixel.r, pixel.g, pixel.b])
        .collect::<Vec<_>>();
    image::save_buffer(
        path,
        &bytes,
        snapshot.width(),
        snapshot.height(),
        image::ExtendedColorType::Rgb8,
    )?;
    println!("wrote {}", path.display());
    Ok(())
}

/// Presses and releases Tab `count` times, so that the focus moves on as it does for the user.
fn tab(window: &SettingsWindow, count: usize) {
    let text = SharedString::from("\t");
    for _ in 0..count {
        window
            .window()
            .dispatch_event(WindowEvent::KeyPressed { text: text.clone() });
        window
            .window()
            .dispatch_event(WindowEvent::KeyReleased { text: text.clone() });
    }
}

/// One state of the section: its name, the settings it shows, whether the interface counts as
/// live (no note), and how many Tab presses move the keyboard focus to what it should show.
struct State {
    name: &'static str,
    settings: AppearanceSettings,
    interface_live: bool,
    tabs: usize,
}

fn states() -> Vec<State> {
    let mut classic = AppearanceSettings::default();
    set_choice(&mut classic, Choice::Layout, 1);

    let mut off = AppearanceSettings::default();
    set_choice(&mut off, Choice::DialCorner, 0);
    set_switch(&mut off, Control::Orbit, false);
    set_switch(&mut off, Control::Dial, false);

    // A hand-edited memory value, a shrunk dock and a Classic dock that was turned on.
    let mut odd = classic.clone();
    odd.tab_groups.snapshot_cap_mb = 200;
    set_switch(&mut odd, Control::Dock, true);
    set_choice(&mut odd, Choice::DockStarts, 1);

    let state = |name, settings: &AppearanceSettings, interface_live, tabs| State {
        name,
        settings: settings.clone(),
        interface_live,
        tabs,
    };
    let defaults = AppearanceSettings::default();
    vec![
        state("mercury-defaults", &defaults, true, 0),
        state("classic", &classic, true, 0),
        state("orbit-and-dial-off", &off, true, 0),
        state("note-shown", &defaults, false, 0),
        state("classic-dock-on-odd-memory", &odd, true, 0),
        // The keyboard focus: on the chosen card, on the other card, on the dock switch.
        state("focus-card", &defaults, true, TABS_TO_FIRST_CARD),
        state(
            "focus-classic-card",
            &defaults,
            true,
            TABS_TO_FIRST_CARD + 1,
        ),
        state("focus-switch", &defaults, true, TABS_TO_FIRST_CARD + 2),
    ]
}

fn main() -> Result<(), Box<dyn Error>> {
    let output = PathBuf::from(
        std::env::args()
            .nth(1)
            .ok_or("usage: render_settings_appearance <output directory>")?,
    );
    std::fs::create_dir_all(&output)?;
    slint::platform::set_platform(Box::new(HeadlessPlatform {
        windows: RefCell::default(),
    }))
    .map_err(|error| format!("{error:?}"))?;

    for (scheme, scheme_name) in [(ColorScheme::Light, "light"), (ColorScheme::Dark, "dark")] {
        for State {
            name,
            settings,
            interface_live,
            tabs,
        } in states()
        {
            for (width, height) in [(560_u32, 1700_u32), (900, 1400)] {
                // The focus states only at the wide size: the rings look the same at both.
                if tabs > 0 && width == 560 {
                    continue;
                }
                let window = SettingsWindow::new()?;
                window.set_forced_color_scheme(scheme);
                window.set_section(section::APPEARANCE);
                window.set_show_bookmarks_bar(true);
                appearance::fill(&window, &layout_panel(&settings, interface_live));
                window.show()?;
                // Lay the window out at its size first, so that Tab walks the real controls.
                window
                    .window()
                    .dispatch_event(WindowEvent::ScaleFactorChanged {
                        scale_factor: SCALE,
                    });
                window
                    .window()
                    .set_size(PhysicalSize::new(width * 2, height * 2));
                tab(&window, tabs);
                render(
                    window.window(),
                    (width, height),
                    &output.join(format!("appearance-{scheme_name}-{name}-{width}.png")),
                )?;
                window.hide()?;
            }
        }
    }
    Ok(())
}
