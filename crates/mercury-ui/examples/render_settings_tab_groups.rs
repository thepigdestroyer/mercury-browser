// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! Renders the settings window's Tab groups section to PNG files without a display, through
//! Slint's software renderer (no window, no Metal): every state of the accessibility goldens (the
//! design's mockup with a conflict and an accent key, no conflicts, an empty window, keys switched
//! off, the Always choice, a refused key, the note that the interface is not in yet) and a window
//! of 64 groups, plus a key cap with keyboard focus, in the light and the dark scheme, at the
//! narrowest window (560 px) and at 900 px. The section is filled through the real bridge from
//! `mercury_appearance::panels::key_panel`; group colours are `PRESETS` by position.
//!
//! `cargo run --release -p mercury-ui --example render_settings_tab_groups -- <output directory>`

use std::cell::RefCell;
use std::error::Error;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use mercury_appearance::PRESETS;
use mercury_appearance::keymap::LetterKey;
use mercury_appearance::panels::key_panel::{GroupInput, KeyPanel, KeyboardFacts, PanelView};
use mercury_settings::{GroupKeySettings, KeyCapture};
use mercury_ui::SettingsWindow;
use mercury_ui::settings_sections::section;
use mercury_ui::settings_sections::tab_groups;
use slint::language::ColorScheme;
use slint::platform::software_renderer::{MinimalSoftwareWindow, RepaintBufferType};
use slint::platform::{Platform, PlatformError, WindowAdapter, WindowEvent};
use slint::{ComponentHandle, PhysicalSize, Rgb8Pixel, SharedString};

const SCALE: f32 = 2.0;

/// The Tab stops before the first key cap: the sidebar's seven tabs and the "Show keys beside
/// names" switch, and one more to land on the cap.
const TABS_TO_FIRST_CAP: usize = 7 + 1 + 1;

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

fn group(id: u64, name: &str, colour: usize, tabs: usize, letter: Option<char>) -> GroupInput {
    GroupInput {
        id,
        name: name.to_owned(),
        colour: PRESETS[colour % PRESETS.len()].id.to_owned(),
        tabs,
        key: letter.and_then(LetterKey::from_char),
    }
}

/// The groups of the design's mockup; Shopping has no key until S is typed into it.
fn mockup_groups() -> Vec<GroupInput> {
    vec![
        group(1, "Research", 0, 5, Some('A')),
        group(2, "Lisbon trip", 1, 4, Some('S')),
        group(3, "Reading", 2, 6, Some('D')),
        group(4, "Shopping", 3, 3, None),
        group(5, "Work", 4, 2, Some('E')),
    ]
}

fn panel(
    groups: &[GroupInput],
    loose: usize,
    keys: &GroupKeySettings,
    interface_live: bool,
) -> KeyPanel {
    let mut panel = KeyPanel::new();
    panel.refresh(groups, loose, keys, &KeyboardFacts::us(), interface_live);
    panel
}

/// One state of the section: its name, what it shows, how tall the window is at 560 px and at
/// 900 px, and how many Tab presses move the keyboard focus to what it should show.
struct State {
    name: &'static str,
    view: PanelView,
    heights: (u32, u32),
    tabs: usize,
}

fn states() -> Vec<State> {
    let defaults = GroupKeySettings::default();

    let mut conflict = panel(&mockup_groups(), 2, &defaults, true);
    conflict.key_typed(4, "s");

    let mut refused = panel(&mockup_groups(), 0, &defaults, true);
    refused.key_typed(4, "7");

    let no_work: Vec<GroupInput> = mockup_groups()
        .into_iter()
        .filter(|group| group.name != "Work")
        .collect();

    let off = GroupKeySettings {
        enabled: false,
        ..GroupKeySettings::default()
    };
    let always = GroupKeySettings {
        capture: KeyCapture::Always,
        ..GroupKeySettings::default()
    };

    let many: Vec<GroupInput> = (1..=64_u64)
        .map(|id| {
            let at = usize::try_from(id).unwrap_or(0);
            group(
                id,
                &format!("Group number {id} with a rather long name"),
                at,
                at,
                None,
            )
        })
        .collect();

    let state = |name, view, heights, tabs| State {
        name,
        view,
        heights,
        tabs,
    };
    let normal = (1250, 1000);
    vec![
        state("mockup", conflict.view(), normal, 0),
        state(
            "plain",
            panel(&no_work, 0, &defaults, true).view(),
            normal,
            0,
        ),
        state(
            "empty",
            panel(&[], 0, &defaults, false).view(),
            (900, 760),
            0,
        ),
        state(
            "keys-off",
            panel(&mockup_groups(), 0, &off, true).view(),
            normal,
            0,
        ),
        state(
            "capture-always",
            panel(&mockup_groups(), 0, &always, true).view(),
            normal,
            0,
        ),
        state("refused", refused.view(), normal, 0),
        state(
            "note-shown",
            panel(&mockup_groups(), 2, &defaults, false).view(),
            (1350, 1050),
            0,
        ),
        state(
            "64-groups",
            panel(&many, 5, &defaults, true).view(),
            (4300, 3300),
            0,
        ),
        // The keyboard focus: on the first key cap.
        state(
            "focus-key-cap",
            panel(&no_work, 0, &defaults, true).view(),
            normal,
            TABS_TO_FIRST_CAP,
        ),
    ]
}

fn main() -> Result<(), Box<dyn Error>> {
    let output = PathBuf::from(
        std::env::args()
            .nth(1)
            .ok_or("usage: render_settings_tab_groups <output directory>")?,
    );
    std::fs::create_dir_all(&output)?;
    slint::platform::set_platform(Box::new(HeadlessPlatform {
        windows: RefCell::default(),
    }))
    .map_err(|error| format!("{error:?}"))?;

    for (scheme, scheme_name) in [(ColorScheme::Light, "light"), (ColorScheme::Dark, "dark")] {
        for State {
            name,
            view,
            heights,
            tabs,
        } in states()
        {
            for (width, height) in [(560_u32, heights.0), (900, heights.1)] {
                // The focus state only at the wide size: the ring looks the same at both.
                if tabs > 0 && width == 560 {
                    continue;
                }
                let window = SettingsWindow::new()?;
                window.set_forced_color_scheme(scheme);
                window.set_section(section::TAB_GROUPS);
                window.set_show_bookmarks_bar(true);
                tab_groups::fill(&window, &view);
                tab_groups::connect(&window, |_| {});
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
                    &output.join(format!("tab-groups-{scheme_name}-{name}-{width}.png")),
                )?;
                window.hide()?;
            }
        }
    }
    Ok(())
}
