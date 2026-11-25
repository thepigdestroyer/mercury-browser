// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! Renders the group dock (SPEC §4.4) to PNG files without a display, through Slint's software
//! renderer: the dock open, shrunk (the capsule), magnified (the pointer on the third pill, with
//! the tooltip of a sliver) and with ⌥ held (key badges), each in light and dark, plus the
//! crowded dock (64 groups) and the capsule with 200 tabs in light. The window is 1440 x 900 like
//! the mockups (`design/ui-final/bar/bar-*.png`), at 2x.
//!
//! `cargo run --release -p mercury-ui --features test-support --example render_group_dock -- <output directory>`

use std::cell::RefCell;
use std::error::Error;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use mercury_ui::group_dock::{DockData, DockModels, DockOptions};
use mercury_ui::ring_bands::fixtures::{generated, mockup_groups, mockup_tabs};
use mercury_ui::ring_bands::metrics;
use mercury_ui::{ChromePalette, ChromeTheme, GroupDockPreview, GroupRow, TabRow};
use slint::platform::software_renderer::{MinimalSoftwareWindow, RepaintBufferType};
use slint::platform::{Platform, PlatformError, WindowAdapter, WindowEvent};
use slint::{ComponentHandle, Model, PhysicalSize};

const SCALE: f32 = 2.0;
const WIDTH: f32 = 1440.0;
const HEIGHT: f32 = 900.0;

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

fn render(window: &slint::Window, path: &Path) -> Result<(), Box<dyn Error>> {
    window.dispatch_event(WindowEvent::ScaleFactorChanged {
        scale_factor: SCALE,
    });
    window.set_size(PhysicalSize::new(
        (WIDTH * SCALE) as u32,
        (HEIGHT * SCALE) as u32,
    ));
    for _ in 0..3 {
        slint::platform::update_timers_and_animations();
        window.request_redraw();
        let _ = window.take_snapshot();
    }
    let snapshot = window.take_snapshot()?;
    let bytes = snapshot
        .as_slice()
        .iter()
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

/// The y of the middle of the pills at rest, in the window (the bar's bottom is 12 px above it).
fn mid_y() -> f32 {
    HEIGHT - 12.0 - 26.0
}

/// What the picture shows.
struct Scene<'a> {
    name: &'a str,
    dark: bool,
    groups: Vec<GroupRow>,
    tabs: Vec<TabRow>,
    options: DockOptions,
    /// The pill the pointer stands on, if any (with the tooltip of its second sliver or chip).
    pointer_on: Option<usize>,
    shrunk: bool,
    /// Every state marker on the chips of the current group (one shape each).
    states: bool,
}

fn draw(scene: Scene<'_>, output: &Path) -> Result<(), Box<dyn Error>> {
    let mut tabs = scene.tabs.clone();
    if scene.states {
        // Research's five chips: muted + loading on the active one, then unread, crashed, camera,
        // playing sound.
        tabs[0].muted = true;
        tabs[0].loading = true;
        tabs[1].unread = true;
        tabs[2].crashed = true;
        tabs[3].capture = true;
        tabs[4].audible = true;
    }
    let metrics = metrics();
    let data = DockData::new(&scene.groups, &tabs, &scene.options, metrics.as_ref());
    println!("{}: fit tier {}", scene.name, data.tier());
    let models = DockModels::new();
    let window = GroupDockPreview::new()?;
    window.global::<ChromePalette>().set_theme(if scene.dark {
        ChromeTheme::Dark
    } else {
        ChromeTheme::Light
    });
    window.set_reduce_motion(true);
    window.set_held(scene.options.held);
    window.set_pills(models.pills());
    window.set_slivers(models.slivers());

    let mut frame = data.frame(None);
    let mut tooltip = None;
    if let Some(index) = scene.pointer_on {
        // The pointer stands on the pill; the second move sees the magnified layout.
        for _ in 0..2 {
            let pill = &frame.pills[index];
            frame = data.frame(Some(pill.x + pill.width / 2.0));
        }
        let pill = &frame.pills[index];
        let target = pill
            .slivers
            .row_data(1)
            .map(|s| pill.x + s.x + s.width / 2.0)
            .or_else(|| pill.chips.row_data(1).map(|c| pill.x + c.x + c.width / 2.0));
        if let Some(x) = target {
            let local_y = mid_y() - (HEIGHT - DockData::container_height());
            tooltip = data.tooltip_for(&frame, x, local_y);
        }
    }
    let capsule = data.capsule();
    models.update(&frame, &capsule);
    window.set_count_text(capsule.count_text.clone());
    window.set_capsule_label(capsule.label.clone());
    window.set_dock_x(frame.dock_x);
    window.set_dock_width(frame.dock_width);
    window.set_dock_height(frame.dock_height);
    window.set_new_tab_x(frame.new_tab_x);
    window.set_shrink_x(frame.shrink_x);
    window.set_hovered_tab(frame.hovered_tab.unwrap_or(-1));
    if let Some((text, x)) = tooltip {
        window.set_tooltip(text);
        window.set_tooltip_x(x);
    }
    window.set_shrunk(scene.shrunk);
    window.show()?;
    render(window.window(), &output.join(format!("{}.png", scene.name)))?;
    window.hide()?;
    Ok(())
}

fn main() -> Result<(), Box<dyn Error>> {
    let output = PathBuf::from(
        std::env::args()
            .nth(1)
            .ok_or("usage: render_group_dock <output directory>")?,
    );
    std::fs::create_dir_all(&output)?;
    slint::platform::set_platform(Box::new(HeadlessPlatform {
        windows: RefCell::new(Vec::new()),
    }))
    .map_err(|error| format!("{error:?}"))?;

    for (dark, tag) in [(false, "light"), (true, "dark")] {
        let rows = |dark| (mockup_groups(dark), mockup_tabs(dark));
        let plain = DockOptions::default();
        let held = DockOptions {
            held: true,
            ..DockOptions::default()
        };
        let scenes = [
            ("open", plain, None, false),
            ("shrunk", plain, None, true),
            ("magnified", plain, Some(2), false),
            ("held-keys", held, None, false),
        ];
        for (name, options, pointer_on, shrunk) in scenes {
            let (groups, tabs) = rows(dark);
            draw(
                Scene {
                    name: &format!("{name}-{tag}"),
                    dark,
                    groups,
                    tabs,
                    options,
                    pointer_on,
                    shrunk,
                    states: false,
                },
                &output,
            )?;
        }
    }

    for (dark, tag) in [(false, "light"), (true, "dark")] {
        draw(
            Scene {
                name: &format!("states-{tag}"),
                dark,
                groups: mockup_groups(dark),
                tabs: mockup_tabs(dark),
                options: DockOptions::default(),
                pointer_on: None,
                shrunk: false,
                states: true,
            },
            &output,
        )?;
    }
    let (groups, tabs) = generated(64, 192, false);
    draw(
        Scene {
            name: "crowded-64-light",
            dark: false,
            groups,
            tabs,
            options: DockOptions::default(),
            pointer_on: Some(30),
            shrunk: false,
            states: false,
        },
        &output,
    )?;
    let (groups, tabs) = generated(8, 200, false);
    draw(
        Scene {
            name: "capsule-200-light",
            dark: false,
            groups,
            tabs,
            options: DockOptions::default(),
            pointer_on: None,
            shrunk: true,
            states: false,
        },
        &output,
    )?;
    Ok(())
}
