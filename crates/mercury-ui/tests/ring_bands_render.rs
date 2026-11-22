// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
#![cfg(feature = "test-support")]
//! Pixel checks of the tab-group ring drawn by Slint's software renderer, from the real
//! `RingBandsData` (the same scenes as `examples/render_ring_bands.rs`). Set `MERCURY_UI_RENDER_DIR`
//! to also write the PNGs.

#[path = "ring_bands_scenes/mod.rs"]
mod scenes;

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

use mercury_ui::ring_bands::fixtures::{generated, mockup_groups, mockup_tabs};
use mercury_ui::ring_geometry::point_on_circle;
use mercury_ui::{ChromePalette, FlagKind};
use scenes::{CENTRE, Params, Scene, at, rgb, save};
use slint::platform::{PointerEventButton, WindowEvent};
use slint::{Color, ComponentHandle, LogicalPosition};

const READING_5: i32 = 114;
const TRAM: i32 = 108;

fn colour(c: Color) -> (u8, u8, u8) {
    (c.red(), c.green(), c.blue())
}

fn within(a: (u8, u8, u8), b: (u8, u8, u8), tolerance: u8) -> bool {
    a.0.abs_diff(b.0) <= tolerance
        && a.1.abs_diff(b.1) <= tolerance
        && a.2.abs_diff(b.2) <= tolerance
}

fn blend(over: Color, under: Color, alpha: f32) -> (u8, u8, u8) {
    let mix = |a: u8, b: u8| (f32::from(a) * alpha + f32::from(b) * (1.0 - alpha)).round() as u8;
    (
        mix(over.red(), under.red()),
        mix(over.green(), under.green()),
        mix(over.blue(), under.blue()),
    )
}

fn out(name: &str, scene: &Scene, pixels: &scenes::Pixels) {
    let _ = scene;
    if let Some(directory) = std::env::var_os("MERCURY_UI_RENDER_DIR").map(PathBuf::from) {
        save(pixels, &directory.join(format!("{name}.png")));
    }
}

/// A point on the tab's centre line at `radius`.
fn on_tab(scene: &Scene, tab_id: i32, radius: f32) -> (f32, f32) {
    let data = scene.data.borrow();
    let flat = data
        .tabs
        .iter()
        .position(|t| t.id == tab_id)
        .expect("a tab");
    let arc = data.model.layout.tabs[flat];
    let p = point_on_circle(CENTRE, radius, (arc.start_deg + arc.end_deg) / 2.0);
    (p.x, p.y)
}

#[test]
fn every_segment_of_the_rest_scene_is_its_shade_in_light_and_dark() {
    for dark in [false, true] {
        let scene = Scene::new(&mockup_groups(dark), &mockup_tabs(dark), Params::hub(dark));
        let pixels = scene.render();
        out(
            if dark { "rest-dark" } else { "rest-light" },
            &scene,
            &pixels,
        );
        for tab in scene.data.borrow().tabs.iter() {
            let (x, y) = scene.mid_of(tab.id);
            assert_eq!(
                rgb(at(&pixels, x, y)),
                colour(tab.shade),
                "dark {dark}: tab {} at ({x}, {y})",
                tab.id
            );
        }
    }
}

#[test]
fn a_hovered_segment_is_lifted_with_a_popup_coloured_outline() {
    for dark in [false, true] {
        let scene = Scene::new(&mockup_groups(dark), &mockup_tabs(dark), Params::hub(dark));
        let palette = scene.window.global::<ChromePalette>();
        let background = palette.get_window_background();
        let shade = colour(
            scene
                .data
                .borrow()
                .tabs
                .iter()
                .find(|t| t.id == READING_5)
                .unwrap()
                .shade,
        );
        let (ox, oy) = on_tab(&scene, READING_5, 195.0 + 5.0);
        let (sx, sy) = on_tab(&scene, READING_5, 195.0 + 8.5);
        let before = scene.render();
        let tint = colour(
            scene
                .data
                .borrow()
                .groups
                .iter()
                .find(|g| g.id == 3)
                .expect("Reading")
                .tint,
        );
        assert_eq!(
            rgb(at(&before, ox, oy)),
            tint,
            "at rest the label band is there, not the tab"
        );
        let _ = background;

        scene.window.set_hovered_tab(READING_5);
        let after = scene.render();
        out(
            if dark { "hover-dark" } else { "hover-light" },
            &scene,
            &after,
        );
        assert_eq!(
            rgb(at(&after, ox, oy)),
            shade,
            "dark {dark}: the lifted part has the shade"
        );
        assert!(
            within(
                rgb(at(&after, sx, sy)),
                colour(palette.get_popup_background()),
                3
            ),
            "dark {dark}: the outline is popup-background, got {:?}",
            rgb(at(&after, sx, sy))
        );
    }
}

#[test]
fn searching_fades_the_segments_that_do_not_match_to_18_percent() {
    let mut tabs = mockup_tabs(false);
    for tab in &mut tabs {
        tab.matches_query = tab.id == TRAM;
    }
    let scene = Scene::new(&mockup_groups(false), &tabs, Params::hub(false));
    scene.window.set_query_active(true);
    let pixels = scene.render();
    out("searching-light", &scene, &pixels);
    let background = scene
        .window
        .global::<ChromePalette>()
        .get_window_background();
    let data = scene.data.borrow();
    for tab in &data.tabs {
        let (x, y) = scene.mid_of(tab.id);
        let got = rgb(at(&pixels, x, y));
        if tab.id == TRAM {
            assert_eq!(got, colour(tab.shade), "the match stays lit");
        } else {
            assert!(
                within(got, blend(tab.shade, background, 0.18), 3),
                "tab {} faded: {got:?} vs {:?}",
                tab.id,
                blend(tab.shade, background, 0.18)
            );
        }
    }
}

#[test]
fn held_makes_the_label_bands_solid_and_dims_the_other_groups() {
    for dark in [false, true] {
        let scene = Scene::new(
            &mockup_groups(dark),
            &mockup_tabs(dark),
            Params {
                held: true,
                selected_group: Some(2),
                ..Params::hub(dark)
            },
        );
        scene.window.set_dim_others(true);
        let pixels = scene.render();
        out(
            if dark { "held-dark" } else { "held-light" },
            &scene,
            &pixels,
        );
        let background = scene
            .window
            .global::<ChromePalette>()
            .get_window_background();
        let data = scene.data.borrow();
        for (g, group) in data.groups.iter().enumerate() {
            let arc = data.model.layout.groups[g];
            let spec = data.model.layout.spec;
            let p = point_on_circle(
                CENTRE,
                spec.label_outer_radius() - 3.0,
                arc.label_start_deg + 1.5,
            );
            let got = rgb(at(&pixels, p.x, p.y));
            let want = if group.id == 2 {
                colour(group.colour)
            } else {
                blend(group.colour, background, 0.45)
            };
            assert!(
                within(got, want, 3),
                "dark {dark}: band of {} {got:?} vs {want:?}",
                group.name
            );
        }
    }
}

#[test]
fn a_single_tab_ring_is_filled_all_the_way_round() {
    let (groups, tabs) = generated(1, 1, false);
    let scene = Scene::new(&groups, &tabs, Params::hub(false));
    let pixels = scene.render();
    out("single-tab-light", &scene, &pixels);
    let data = scene.data.borrow();
    let arc = data.model.layout.tabs[0];
    assert!(
        arc.end_deg - arc.start_deg > 300.0,
        "the tab spans {:?}",
        arc
    );
    let shade = colour(data.tabs[0].shade);
    for deg in [arc.start_deg + 1.0, 90.0, 180.0, 270.0, arc.end_deg - 1.0] {
        let p = point_on_circle(CENTRE, 180.0, deg);
        assert_eq!(rgb(at(&pixels, p.x, p.y)), shade, "at {deg} degrees");
    }
    let gap = point_on_circle(CENTRE, 180.0, 0.0);
    assert_ne!(
        rgb(at(&pixels, gap.x, gap.y)),
        shade,
        "the reserve at 12 o'clock is empty"
    );
}

#[test]
fn sixty_tabs_and_64_groups_draw_every_segment() {
    for (n_groups, n_tabs, name) in [(5, 60, "sixty-tabs-light"), (64, 64, "crowded-64-light")] {
        let (groups, tabs) = generated(n_groups, n_tabs, false);
        let scene = Scene::new(&groups, &tabs, Params::hub(false));
        let pixels = scene.render();
        out(name, &scene, &pixels);
        let data = scene.data.borrow();
        assert_eq!(data.segments.len(), n_tabs);
        let background = scene
            .window
            .global::<ChromePalette>()
            .get_window_background();
        for tab in &data.tabs {
            let (x, y) = scene.mid_of(tab.id);
            let got = rgb(at(&pixels, x, y));
            assert_ne!(got, colour(background), "{name}: tab {} is drawn", tab.id);
        }
    }
}

#[test]
fn the_flag_colours_follow_the_scheme() {
    let mut accents = Vec::new();
    for dark in [false, true] {
        let scene = Scene::new(&mockup_groups(dark), &mockup_tabs(dark), Params::hub(dark));
        let pixels = scene.render();
        let accent = scene.window.global::<ChromePalette>().get_accent();
        let data = scene.data.borrow();
        let pin = data
            .flags
            .iter()
            .find(|p| p.kind == FlagKind::Unread)
            .expect("the mockup has unread tabs");
        assert_eq!(
            rgb(at(&pixels, pin.x, pin.y)),
            colour(accent),
            "dark {dark}: the unread dot"
        );
        accents.push(colour(accent));
    }
    assert_ne!(
        accents[0], accents[1],
        "the two schemes have different accents"
    );
}

#[test]
fn the_real_hit_test_drives_hover_click_and_background_press() {
    let scene = Scene::new(
        &mockup_groups(false),
        &mockup_tabs(false),
        Params::hub(false),
    );
    let log: Rc<RefCell<Vec<String>>> = Rc::default();
    let sink = Rc::clone(&log);
    scene
        .window
        .on_tab_activated(move |id| sink.borrow_mut().push(format!("activated {id}")));
    let sink = Rc::clone(&log);
    scene
        .window
        .on_background_pressed(move || sink.borrow_mut().push("background".into()));
    let window = scene.window.window();
    let (x, y) = scene.mid_of(READING_5);
    let position = LogicalPosition::new(x, y);
    window.dispatch_event(WindowEvent::PointerMoved { position });
    assert_eq!(scene.window.get_hovered_tab(), READING_5);
    window.dispatch_event(WindowEvent::PointerPressed {
        position,
        button: PointerEventButton::Left,
    });
    window.dispatch_event(WindowEvent::PointerReleased {
        position,
        button: PointerEventButton::Left,
    });
    assert_eq!(*log.borrow(), vec![format!("activated {READING_5}")]);

    log.borrow_mut().clear();
    let corner = LogicalPosition::new(4.0, 4.0);
    window.dispatch_event(WindowEvent::PointerMoved { position: corner });
    assert_eq!(scene.window.get_hovered_tab(), -1);
    window.dispatch_event(WindowEvent::PointerPressed {
        position: corner,
        button: PointerEventButton::Left,
    });
    window.dispatch_event(WindowEvent::PointerReleased {
        position: corner,
        button: PointerEventButton::Left,
    });
    assert_eq!(*log.borrow(), vec!["background".to_string()]);
}
