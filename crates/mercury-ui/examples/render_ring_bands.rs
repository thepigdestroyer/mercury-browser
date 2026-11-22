// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! Renders the tab-group ring (`RingBands`) to PNG files without a display, through Slint's software
//! renderer: the mockup's 22 tabs at rest, with a tab hovered, searching, with ⌥ held, with the
//! keyboard focus, being dragged, and the 3 tab, 60 tab, single tab and 64 group rings; light and
//! dark. The ring is drawn alone on the window background (the popover and the centre disc are the
//! hub ring's).
//!
//! `cargo run --release -p mercury-ui --features test-support --example render_ring_bands -- <output directory>`

#[path = "../tests/ring_bands_scenes/mod.rs"]
mod scenes;

use std::error::Error;
use std::path::PathBuf;

use mercury_ui::FlagKind;
use mercury_ui::ring_bands::fixtures::{generated, mockup_groups, mockup_tabs};
use scenes::{Params, Scene, save};
use slint::ComponentHandle;
use slint::LogicalPosition;
use slint::platform::{PointerEventButton, WindowEvent};

/// The tab "Reading: tab 5" of the mockup (the hovered tab of `ring-open`).
const READING_5: i32 = 101 + 5 + 4 + 4;
/// "Tram 28 timetable", the one match of the search frame.
const TRAM: i32 = 101 + 5 + 2;
const LISBON: i32 = 2;

fn point(x: f32, y: f32) -> LogicalPosition {
    LogicalPosition::new(x, y)
}

fn mockup(dark: bool, params: Params) -> Scene {
    Scene::new(&mockup_groups(dark), &mockup_tabs(dark), params)
}

fn main() -> Result<(), Box<dyn Error>> {
    let output = PathBuf::from(
        std::env::args()
            .nth(1)
            .ok_or("usage: render_ring_bands <output directory>")?,
    );
    std::fs::create_dir_all(&output)?;

    for dark in [false, true] {
        let name = if dark { "dark" } else { "light" };
        let file = |scene: &str| output.join(format!("{scene}-{name}.png"));

        save(&mockup(dark, Params::hub(dark)).render(), &file("rest"));

        let hover = mockup(dark, Params::hub(dark));
        hover.window.set_hovered_tab(READING_5);
        save(&hover.render(), &file("hover"));

        let mut tabs = mockup_tabs(dark);
        for tab in &mut tabs {
            tab.matches_query = tab.id == TRAM;
        }
        let search = Scene::new(&mockup_groups(dark), &tabs, Params::hub(dark));
        search.window.set_query_active(true);
        save(&search.render(), &file("searching"));

        let held = mockup(
            dark,
            Params {
                held: true,
                selected_group: Some(LISBON),
                ..Params::hub(dark)
            },
        );
        held.window.set_dim_others(true);
        save(&held.render(), &file("held"));

        let focus = mockup(dark, Params::hub(dark));
        focus.window.set_focused_tab(READING_5);
        save(&focus.render(), &file("focused"));

        let (groups, tabs) = generated(2, 3, dark);
        save(
            &Scene::new(&groups, &tabs, Params::hub(dark)).render(),
            &file("three-tabs"),
        );
        let (groups, tabs) = generated(5, 60, dark);
        save(
            &Scene::new(&groups, &tabs, Params::hub(dark)).render(),
            &file("sixty-tabs"),
        );
    }

    let light = |scene: &str| output.join(format!("{scene}-light.png"));

    let (groups, tabs) = generated(1, 1, false);
    save(
        &Scene::new(&groups, &tabs, Params::hub(false)).render(),
        &light("single-tab"),
    );
    let (groups, tabs) = generated(64, 64, false);
    save(
        &Scene::new(&groups, &tabs, Params::hub(false)).render(),
        &light("crowded-64"),
    );

    let mut tabs = mockup_tabs(false);
    for (index, kind) in [FlagKind::Muted, FlagKind::Crashed, FlagKind::Capture]
        .into_iter()
        .enumerate()
    {
        let tab = &mut tabs[9 + index];
        match kind {
            FlagKind::Muted => (tab.audible, tab.muted) = (true, true),
            FlagKind::Crashed => tab.crashed = true,
            _ => tab.capture = true,
        }
    }
    save(
        &Scene::new(&mockup_groups(false), &tabs, Params::hub(false)).render(),
        &light("flags-all"),
    );

    let drag = mockup(false, Params::hub(false));
    let (x, y) = drag.mid_of(READING_5);
    let window = drag.window.window();
    window.dispatch_event(WindowEvent::PointerMoved {
        position: point(x, y),
    });
    window.dispatch_event(WindowEvent::PointerPressed {
        position: point(x, y),
        button: PointerEventButton::Left,
    });
    let (to_x, to_y) = drag.mid_of(101 + 5 + 1);
    for step in 1..=4 {
        let t = step as f32 / 4.0;
        window.dispatch_event(WindowEvent::PointerMoved {
            position: point(x + (to_x - x) * t, y + (to_y - y) * t),
        });
    }
    save(&drag.render(), &light("dragging"));
    window.dispatch_event(WindowEvent::PointerReleased {
        position: point(to_x, to_y),
        button: PointerEventButton::Left,
    });
    Ok(())
}
