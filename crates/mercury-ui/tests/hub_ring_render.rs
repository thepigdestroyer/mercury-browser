// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! The hub ring in pictures, drawn by Slint's software renderer (no window, no GPU): the scenes
//! of `examples/render_hub_ring.rs` (the four `ring-*` frames, the crowded and sparse rings), each in
//! light and dark at 2x, and probes of their pixels (brief §7 item 4):
//!
//! - the popover's fill in the surround of the disc, the disc's fill and the tail's tip are the
//!   palette's `popup-background`, `window-background` and `popup-background` again on the page's
//!   `content-background`;
//! - a segment has its tab's shade and a segment that the query fades is the 18 % blend of it over
//!   the popover fill (within 2 levels);
//! - the search field's border is the accent while a query is typed;
//! - light and dark differ at every probe.
//!
//! Set `MERCURY_UI_RENDER_DIR` to also save the pictures (`<scene>-<scheme>.png`). Slint's software
//! renderer draws no drop shadow, so the popover's shadow is not in these pictures.
#![cfg(feature = "test-support")]

#[allow(dead_code)]
#[path = "../examples/render_hub_ring.rs"]
mod render_hub_ring;

use std::path::PathBuf;

use mercury_ui::{ChromePalette, HubRingBridge};
use render_hub_ring::scenes::{self, Scene};
use slint::{ComponentHandle, Model, Rgba8Pixel, SharedPixelBuffer};

type Pixels = SharedPixelBuffer<Rgba8Pixel>;

/// The probe's colour as `[r, g, b]` at the logical point (`x`, `y`) of the window.
fn pixel(pixels: &Pixels, x: f32, y: f32) -> [u8; 3] {
    let px = (x * scenes::SCALE) as u32;
    let py = (y * scenes::SCALE) as u32;
    let pixel = pixels.as_slice()[(py * pixels.width() + px) as usize];
    [pixel.r, pixel.g, pixel.b]
}

fn rgb(color: slint::Color) -> [u8; 3] {
    [color.red(), color.green(), color.blue()]
}

fn within(actual: [u8; 3], expected: [u8; 3], levels: i32) -> bool {
    actual
        .iter()
        .zip(expected)
        .all(|(&a, e)| (i32::from(a) - i32::from(e)).abs() <= levels)
}

/// `shade` at `alpha` over `under`, per channel.
fn blend(shade: [u8; 3], under: [u8; 3], alpha: f32) -> [u8; 3] {
    let mix = |s: u8, u: u8| (f32::from(s) * alpha + f32::from(u) * (1.0 - alpha)).round() as u8;
    [
        mix(shade[0], under[0]),
        mix(shade[1], under[1]),
        mix(shade[2], under[2]),
    ]
}

fn render(name: &str, dark: bool) -> (Scene, Pixels) {
    scenes::init_platform();
    let scene = scenes::scene(name, dark).expect("the scene can be built");
    let pixels = scenes::pixels(scene.window.window()).expect("the scene can be drawn");
    if let Some(directory) = std::env::var_os("MERCURY_UI_RENDER_DIR") {
        let directory = PathBuf::from(directory);
        std::fs::create_dir_all(&directory).expect("the render directory can be made");
        let scheme = if dark { "dark" } else { "light" };
        scenes::save(&pixels, &directory.join(format!("{name}-{scheme}.png")))
            .expect("the picture can be saved");
    }
    (scene, pixels)
}

/// A point of the ring at `radius` and `angle_deg` (clockwise from 12 o'clock), in window pixels.
fn on_ring(radius: f32, angle_deg: f32) -> (f32, f32) {
    let angle = angle_deg.to_radians();
    (
        scenes::RING_X + 258.0 + radius * angle.sin(),
        scenes::RING_Y + 258.0 - radius * angle.cos(),
    )
}

/// What the colours of the probes are in `scene`'s palette, and what was drawn there.
struct Probes {
    popover_fill: ([u8; 3], [u8; 3]),
    disc_fill: ([u8; 3], [u8; 3]),
    tail_tip: ([u8; 3], [u8; 3]),
    beside_tail: ([u8; 3], [u8; 3]),
}

fn probes(scene: &Scene, pixels: &Pixels) -> Probes {
    let palette = scene.window.global::<ChromePalette>();
    // Between the label band and the circle's edge (radius 240 of 258): only the popover's fill.
    let (fx, fy) = on_ring(240.0, 200.0);
    // Inside the disc, left of the preview and above the field (radius about 126 of 136).
    let (dx, dy) = (
        scenes::RING_X + 258.0 - 120.0,
        scenes::RING_Y + 258.0 - 40.0,
    );
    // Just under the tail's tip: two pixels wide here, above the circle box.
    let (tx, ty) = (scenes::TAIL_TIP.0 + 1.4, scenes::TAIL_TIP.1 + 3.0);
    Probes {
        popover_fill: (pixel(pixels, fx, fy), rgb(palette.get_popup_background())),
        disc_fill: (pixel(pixels, dx, dy), rgb(palette.get_window_background())),
        tail_tip: (pixel(pixels, tx, ty), rgb(palette.get_popup_background())),
        beside_tail: (
            pixel(pixels, scenes::TAIL_TIP.0 - 16.0, ty + 17.0),
            rgb(palette.get_content_background()),
        ),
    }
}

#[test]
fn every_scene_draws_in_light_and_dark_and_the_two_differ() {
    for name in scenes::SCENES {
        let (_light_scene, light) = render(name, false);
        let (_dark_scene, dark) = render(name, true);
        assert_eq!(
            (light.width(), light.height()),
            (scenes::WIDTH * 2, scenes::HEIGHT * 2),
            "{name}: the picture is the window at 2x"
        );
        assert_ne!(
            light.as_bytes(),
            dark.as_bytes(),
            "{name}: dark must not draw the light picture"
        );
    }
}

#[test]
fn the_popover_the_disc_and_the_tail_have_the_palette_colours_in_both_schemes() {
    let mut seen = Vec::new();
    for dark in [false, true] {
        let (scene, pixels) = render("ring-open", dark);
        let probes = probes(&scene, &pixels);
        let scheme = if dark { "dark" } else { "light" };
        for (what, (actual, expected)) in [
            ("popover fill", probes.popover_fill),
            ("disc fill", probes.disc_fill),
            ("tail tip", probes.tail_tip),
            ("page beside the tail", probes.beside_tail),
        ] {
            assert!(
                within(actual, expected, 1),
                "{scheme} {what}: drew {actual:?}, the palette says {expected:?}"
            );
        }
        seen.push([
            probes.popover_fill.0,
            probes.disc_fill.0,
            probes.tail_tip.0,
            probes.beside_tail.0,
        ]);
    }
    for (index, what) in ["popover fill", "disc fill", "tail tip", "page"]
        .into_iter()
        .enumerate()
    {
        assert_ne!(
            seen[0][index], seen[1][index],
            "dark differs from light: {what}"
        );
    }
}

#[test]
fn the_tail_stands_above_the_circle_box_and_the_toolbar_is_above_the_page() {
    let (scene, pixels) = render("ring-open", false);
    let palette = scene.window.global::<ChromePalette>();
    // The tail's tip is 44 px above the box's top edge, under the hub button.
    assert!(scenes::TAIL_TIP.1 < scenes::RING_Y - 40.0);
    assert!(within(
        pixel(&pixels, 400.0, 20.0),
        rgb(palette.get_toolbar_background()),
        1
    ));
    assert!(within(
        pixel(&pixels, 700.0, 300.0),
        rgb(palette.get_content_background()),
        1
    ));
}

// The segments are drawn by RingBands.
#[test]
fn a_segment_has_its_tabs_shade() {
    for dark in [false, true] {
        let (scene, pixels) = render("ring-open", dark);
        // "Slint rendering" (id 103), not hovered, no flag: the middle of its segment.
        let segments = scene.window.global::<HubRingBridge>().get_segments();
        let segment = segments
            .iter()
            .find(|segment| segment.tab == 103)
            .expect("the ring has a segment for tab 103");
        let shade = rgb(scene
            .input
            .tabs
            .iter()
            .find(|tab| tab.id == 103)
            .expect("the scene has tab 103")
            .shade);
        let at = pixel(
            &pixels,
            scenes::RING_X + segment.mid_x,
            scenes::RING_Y + segment.mid_y,
        );
        assert!(within(at, shade, 2), "drew {at:?}, the shade is {shade:?}");
    }
}

// The fade is RingBands' own.
#[test]
fn a_segment_that_the_query_fades_is_the_18_percent_blend_over_the_popover() {
    for dark in [false, true] {
        let (scene, pixels) = render("ring-search", dark);
        let segments = scene.window.global::<HubRingBridge>().get_segments();
        let segment = segments
            .iter()
            .find(|segment| segment.tab == 103)
            .expect("the ring has a segment for tab 103");
        let shade = rgb(scene
            .input
            .tabs
            .iter()
            .find(|tab| tab.id == 103)
            .expect("the scene has tab 103")
            .shade);
        let under = rgb(scene
            .window
            .global::<ChromePalette>()
            .get_popup_background());
        let at = pixel(
            &pixels,
            scenes::RING_X + segment.mid_x,
            scenes::RING_Y + segment.mid_y,
        );
        let want = blend(shade, under, 0.18);
        assert!(
            within(at, want, 2),
            "drew {at:?}, 18 % of {shade:?} over {under:?} is {want:?}"
        );
    }
}

/// The accent shows in a thin column on the left edge of the search field while a query is typed
/// (216 x 32 at (-108, -76) from the centre; at rest it is 200 x 30 at (-100, +46)): scanned, so a
/// pixel of drift in the field's own layout does not matter.
fn accent_on_field_edge(scene: &Scene, pixels: &Pixels, searching: bool) -> bool {
    let accent = rgb(scene.window.global::<ChromePalette>().get_accent());
    let (left, middle) = if searching {
        (
            scenes::RING_X + 258.0 - 108.0,
            scenes::RING_Y + 258.0 - 76.0 + 16.0,
        )
    } else {
        (
            scenes::RING_X + 258.0 - 100.0,
            scenes::RING_Y + 258.0 + 46.0 + 15.0,
        )
    };
    (-4..=4)
        .map(|step| left + step as f32 * 0.5)
        .any(|x| within(pixel(pixels, x, middle), accent, 8))
}

#[test]
fn the_search_field_border_is_the_accent_while_a_query_is_typed_and_not_at_rest() {
    for dark in [false, true] {
        let (searching, pixels) = render("ring-search", dark);
        assert!(
            accent_on_field_edge(&searching, &pixels, true),
            "dark={dark}: searching draws the accent border"
        );
        let (rest, pixels) = render("ring-open", dark);
        assert!(
            !accent_on_field_edge(&rest, &pixels, false),
            "dark={dark}: at rest the border is not the accent"
        );
    }
}
