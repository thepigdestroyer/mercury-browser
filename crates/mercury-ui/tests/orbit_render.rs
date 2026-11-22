// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
#![cfg(feature = "test-support")]
//! Pixel probes of the Orbit overview rendered with Slint's software renderer (1440 x 900 logical
//! px at 2x), every frame built through the real pipeline (`tests/orbit_scenes/mod.rs`: fixtures,
//! `OrbitMachine`, `OrbitData::build`, `OrbitModels::update`): the sun's colour (the same in both
//! schemes), the lit halo's accent stroke, a planet band equal to the tab's shade, the move frame's
//! accent bar and outlined source slot, a level-2 card's outline, the level-3 card's picture, the
//! placeholder, the search fade and the title bar flipping with the scheme. Set
//! `MERCURY_UI_RENDER_DIR` to a directory to get the PNGs. The probes avoid text (the renderer's
//! fonts differ between Macs).

#[allow(dead_code)]
#[path = "orbit_scenes/mod.rs"]
mod scenes;

use std::path::PathBuf;

use mercury_ui::orbit_view::fixtures::{skeleton_picture, tab_id};
use mercury_ui::{ChromePalette, OrbitPlanet};
use scenes::{Frame, Pixels, Scene, at, init_software, rgb, scene};
use slint::{Color, ComponentHandle, Rgba8Pixel};

/// The frame drawn, its scene and (when `MERCURY_UI_RENDER_DIR` is set) a PNG.
fn draw(frame: Frame, dark: bool) -> (Pixels, Scene) {
    init_software();
    let scene = scene(frame, dark);
    let pixels = scene.render();
    if let Some(directory) = std::env::var_os("MERCURY_UI_RENDER_DIR").map(PathBuf::from) {
        let name = format!(
            "{}-{}.png",
            scenes::file_name(frame),
            if dark { "dark" } else { "light" }
        );
        scenes::save(&pixels, &directory.join(name));
    }
    (pixels, scene)
}

fn accent(scene: &Scene) -> Color {
    scene.window.global::<ChromePalette>().get_accent()
}

fn is(pixel: Rgba8Pixel, colour: Color, tolerance: u8) -> bool {
    pixel.r.abs_diff(colour.red()) <= tolerance
        && pixel.g.abs_diff(colour.green()) <= tolerance
        && pixel.b.abs_diff(colour.blue()) <= tolerance
}

/// Some pixel of the short run of logical px starting at (x, y) going `dx`, `dy` is `colour`.
fn near_colour(pixels: &Pixels, (x, y): (f32, f32), (dx, dy): (f32, f32), colour: Color) -> bool {
    (0..8).any(|step| {
        let step = step as f32 * 0.5;
        is(at(pixels, x + dx * step, y + dy * step), colour, 16)
    })
}

/// The stage position of the middle of the segment of `tab` on a planet, and the tab's shade.
fn segment_of(scene: &Scene, group: i32, tab: i32) -> ((f32, f32), Color) {
    let planet = scene
        .data
        .planets
        .iter()
        .find(|p| p.row.group == group)
        .expect("the planet");
    let index = planet
        .ring
        .segments
        .iter()
        .position(|s| s.tab == tab)
        .expect("a segment of that tab");
    let seg = &planet.ring.segments[index];
    let row: &OrbitPlanet = &planet.row;
    (
        (
            row.x - row.ring_size / 2.0 + seg.mid_x,
            row.y - row.ring_size / 2.0 + seg.mid_y,
        ),
        planet.ring.tabs[index].shade,
    )
}

#[test]
fn the_sun_has_the_same_warm_colour_in_light_and_dark() {
    let (light, scene) = draw(Frame::Overview, false);
    let (dark, _) = draw(Frame::Overview, true);
    let sun = &scene.data.sun;
    // A point of the disc away from the title and the subtitle: 60 px right, 40 px above its centre.
    let (x, y) = (sun.x + 60.0, sun.y - 40.0);
    let l = at(&light, x, y);
    let d = at(&dark, x, y);
    assert!(l.r >= 235 && (90..230).contains(&l.g) && l.b < 140, "{l:?}");
    assert_eq!(rgb(l), rgb(d), "the sun is brand art, not themed");
    // The gradient: lighter near its focus, darker at the rim.
    let focus = at(&light, sun.x - 20.0, sun.y - 30.0);
    let rim = at(&light, sun.x + 86.0, sun.y);
    assert!(focus.g > l.g && l.g > rim.g, "{focus:?} {l:?} {rim:?}");
}

#[test]
fn the_halo_is_around_the_sun_and_fades_out() {
    let (light, scene) = draw(Frame::Overview, false);
    let sun = &scene.data.sun;
    let background = at(&light, 20.0, 480.0);
    let inner = at(&light, sun.x + 100.0, sun.y);
    let outer = at(&light, sun.x + 140.0, sun.y);
    assert_ne!(rgb(inner), rgb(background));
    assert_eq!(rgb(outer), rgb(background));
}

#[test]
fn the_lit_halo_has_an_accent_stroke_and_the_others_do_not() {
    let (pixels, scene) = draw(Frame::Overview, false);
    let research = scene.planet(1);
    assert!(research.lit, "the machine's hover lights Research");
    let top = (research.x, research.y - research.halo_radius - 1.0);
    assert!(
        near_colour(&pixels, top, (0.0, 1.0), accent(&scene)),
        "the lit halo's stroke at {top:?}: {:?}",
        at(&pixels, top.0, top.1)
    );
    let lisbon = scene.planet(2);
    assert!(!lisbon.lit);
    let rest = (lisbon.x, lisbon.y - lisbon.halo_radius - 1.0);
    assert!(
        !near_colour(&pixels, rest, (0.0, 1.0), accent(&scene)),
        "an unlit planet has no halo stroke"
    );
}

#[test]
fn a_planets_band_is_the_shade_of_its_tab() {
    let (pixels, scene) = draw(Frame::Overview, false);
    // Shopping carries no flag; its first tab's segment is drawn in the tab's shade.
    let (mid, shade) = segment_of(&scene, 4, tab_id(3, 0));
    let got = at(&pixels, mid.0, mid.1);
    assert!(is(got, shade, 6), "{got:?} vs {:?}", shade.to_argb_u8());
    // So is the last tab of Work (the lightest green).
    let (mid, shade) = segment_of(&scene, 5, tab_id(4, 5));
    let got = at(&pixels, mid.0, mid.1);
    assert!(is(got, shade, 6), "{got:?} vs {:?}", shade.to_argb_u8());
}

#[test]
fn the_move_frame_has_an_accent_bar_at_the_slot_an_outlined_source_slot_and_a_lit_target() {
    let (pixels, scene) = draw(Frame::MoveTab, false);
    let reading = scene.planet(3);
    assert!(reading.drop_lit && reading.has_drop_bar);
    // The 3 px accent ring just outside the label band, at radius 97.
    let ring = (reading.x, reading.y - 97.0 * reading.scale - 2.0);
    assert!(
        near_colour(&pixels, ring, (0.0, 1.0), accent(&scene)),
        "{:?}",
        at(&pixels, ring.0, ring.1)
    );
    // The accent bar across the band at the insertion slot.
    let origin = (
        reading.x - reading.ring_size / 2.0,
        reading.y - reading.ring_size / 2.0,
    );
    let bar = (
        origin.0 + reading.drop_bar.mid_x,
        origin.1 + reading.drop_bar.mid_y,
    );
    assert!(
        near_colour(
            &pixels,
            (bar.0 - 1.0, bar.1 - 1.0),
            (0.5, 0.5),
            accent(&scene)
        ),
        "the bar at {bar:?}: {:?}",
        at(&pixels, bar.0, bar.1)
    );
    // The slot of the dragged tab (Lisbon trip's second) is outlined, not filled with its shade.
    let dragged = tab_id(1, 1);
    let (slot, shade) = segment_of(&scene, 2, dragged);
    assert!(
        !is(at(&pixels, slot.0, slot.1), shade, 10),
        "the slot is not filled"
    );
    // The other segments of that ring are.
    let (other, shade) = segment_of(&scene, 2, tab_id(1, 4));
    assert!(is(at(&pixels, other.0, other.1), shade, 8));
    // The ghost pill follows the drag: its colour is the group's colour.
    let drag = &scene.data.drag;
    assert!(drag.active && drag.has_from);
    assert!(is(
        at(&pixels, drag.ghost_x - 30.0, drag.ghost_y - 8.0),
        drag.ghost_colour,
        20
    ));
}

#[test]
fn the_current_and_the_lit_cards_have_an_accent_outline_the_others_do_not() {
    let (pixels, scene) = draw(Frame::Group, false);
    let left_edge =
        |row: &mercury_ui::OrbitPreviewCard| (row.x - row.width * row.scale / 2.0, row.y);
    let cards: Vec<_> = scene.data.previews.iter().map(|p| p.row.clone()).collect();
    assert_eq!(cards.len(), 10);
    let current = cards.iter().find(|c| c.current).expect("the current tab");
    let lit = cards
        .iter()
        .find(|c| c.scale > 1.0)
        .expect("the hovered card is lifted");
    assert!(lit.lit);
    for card in [current, lit] {
        let (x, y) = left_edge(card);
        assert!(
            near_colour(&pixels, (x, y), (0.5, 0.0), accent(&scene)),
            "{}: {:?}",
            card.title,
            at(&pixels, x, y)
        );
    }
    let ordinary = cards
        .iter()
        .find(|c| !c.current && !c.lit)
        .expect("an ordinary card");
    let (x, y) = left_edge(ordinary);
    assert!(
        !near_colour(&pixels, (x, y), (0.5, 0.0), accent(&scene)),
        "an ordinary card has no accent outline"
    );
}

#[test]
fn the_level_three_card_shows_the_supplied_picture() {
    for dark in [false, true] {
        let (pixels, scene) = draw(Frame::Tab, dark);
        let card = &scene.data.card;
        assert!(card.visible && card.has_picture);
        // The 480 x 300 picture is scaled to cover the card's picture area (the part below the 40 px
        // header) and cropped at the sides.
        let scale = (card.width / 480.0).max((card.height - 40.0) / 300.0);
        let crop = (480.0 * scale - card.width) / 2.0;
        let supplied = skeleton_picture(dark)
            .to_rgba8()
            .expect("the picture has pixels");
        // A flat part of the picture's big block, away from the mountain.
        let (ix, iy) = (40usize, 150usize);
        let expected = supplied.as_slice()[iy * 480 + ix];
        let x = card.x + (ix as f32 + 0.5) * scale - crop;
        let y = card.y + 40.0 + (iy as f32 + 0.5) * scale;
        let got = at(&pixels, x, y);
        assert!(
            got.r.abs_diff(expected.r) <= 3
                && got.g.abs_diff(expected.g) <= 3
                && got.b.abs_diff(expected.b) <= 3,
            "dark={dark}: {got:?} vs {expected:?} at ({x}, {y})"
        );
    }
}

#[test]
fn level_three_draws_the_big_ring_behind_and_two_half_neighbours() {
    let (pixels, scene) = draw(Frame::Tab, false);
    let centre = &scene
        .data
        .centre
        .as_ref()
        .expect("the ring behind the card")
        .row;
    assert!(centre.ring_size > 1000.0, "{}", centre.ring_size);
    // The ring is drawn: a pixel of one of its segments is not the window colour.
    let seg = &scene.data.centre.as_ref().unwrap().ring.segments[0];
    let (x, y) = (
        centre.x - centre.ring_size / 2.0 + seg.mid_x,
        centre.y - centre.ring_size / 2.0 + seg.mid_y,
    );
    let background = at(&pixels, 20.0, 700.0);
    if y > 60.0 && x > 0.0 && x < 1440.0 {
        assert_ne!(rgb(at(&pixels, x, y)), rgb(background));
    }
    let near = &scene.data.neighbours;
    assert_eq!(near.len(), 2);
    assert!(
        near.iter().any(|n| n.x < 0.0),
        "the left neighbour is half off the stage"
    );
    assert!(
        near.iter().any(|n| n.x + n.width > 1440.0),
        "the right neighbour is half off the stage"
    );
}

#[test]
fn a_tab_without_a_picture_shows_the_placeholder_not_a_blank_card() {
    let (pixels, scene) = draw(Frame::Empty, true);
    let cards: Vec<_> = scene.data.previews.iter().map(|p| p.row.clone()).collect();
    assert_eq!(cards.len(), 10);
    assert!(cards.iter().all(|c| !c.has_picture));
    let card = &cards[3];
    // The favicon chip (the tab's shade, about 20 x 20 px) is drawn in the picture area.
    let mut chip_pixels = 0;
    let mut y = card.y - card.height / 2.0;
    while y < card.y + card.height / 2.0 - 28.0 {
        let mut x = card.x - card.width / 2.0;
        while x < card.x + card.width / 2.0 {
            if is(at(&pixels, x, y), card.shade, 6) {
                chip_pixels += 1;
            }
            x += 0.5;
        }
        y += 0.5;
    }
    assert!(
        chip_pixels > 600,
        "the placeholder's chip: {chip_pixels} pixels of {:?}",
        card.shade.to_argb_u8()
    );
}

#[test]
fn twelve_groups_shrink_and_twenty_tabs_make_two_rings_that_do_not_hide_each_other() {
    let (_, many) = draw(Frame::ManyGroups, false);
    assert_eq!(many.data.planets.len(), 12);
    assert!(
        many.data.planets.iter().all(|p| p.row.scale < 0.95),
        "the planets shrink with many groups"
    );
    let (_, two) = draw(Frame::TwoRings, false);
    let cards: Vec<_> = two.data.previews.iter().map(|p| p.row.clone()).collect();
    assert_eq!(cards.len(), 20);
    // The inner ring is drawn first (it comes first in the model).
    let inner = &two.data.layout.previews;
    assert_eq!(inner[0].ring, 1);
    assert_eq!(inner[inner.len() - 1].ring, 0);
    for (i, a) in cards.iter().enumerate() {
        for b in &cards[i + 1..] {
            let overlap_x = (a.x + a.width / 2.0).min(b.x + b.width / 2.0)
                - (a.x - a.width / 2.0).max(b.x - b.width / 2.0);
            let overlap_y = (a.y + a.height / 2.0).min(b.y + b.height / 2.0)
                - (a.y - a.height / 2.0).max(b.y - b.height / 2.0);
            assert!(
                overlap_x < 5.0 || overlap_y < 5.0,
                "{} hides {}",
                a.title,
                b.title
            );
        }
    }
}

#[test]
fn a_search_fades_the_segments_that_do_not_match() {
    let (pixels, scene) = draw(Frame::Searching, false);
    // "Essay on maps" (Reading) matches "map"; "Running shoes" (Shopping) does not.
    let (mid, shade) = segment_of(&scene, 3, tab_id(2, 1));
    assert!(is(at(&pixels, mid.0, mid.1), shade, 8), "a match stays lit");
    let (mid, shade) = segment_of(&scene, 4, tab_id(3, 1));
    let got = at(&pixels, mid.0, mid.1);
    assert!(
        !is(got, shade, 30),
        "a non-match fades: {got:?} vs {:?}",
        shade.to_argb_u8()
    );
}

#[test]
fn the_title_bar_follows_the_scheme() {
    let (light, scene_light) = draw(Frame::Overview, false);
    let (dark, scene_dark) = draw(Frame::Overview, true);
    let toolbar = |scene: &Scene| {
        scene
            .window
            .global::<ChromePalette>()
            .get_toolbar_background()
    };
    let window = |scene: &Scene| {
        scene
            .window
            .global::<ChromePalette>()
            .get_window_background()
    };
    // Right of the breadcrumb, left of the search field: nothing is drawn there.
    let (x, y) = (400.0, 12.0);
    assert!(is(at(&light, x, y), toolbar(&scene_light), 2));
    assert!(is(at(&dark, x, y), toolbar(&scene_dark), 2));
    assert_ne!(rgb(at(&light, x, y)), rgb(at(&dark, x, y)));
    // Below the bar the stage is the window colour.
    assert!(is(at(&light, 30.0, 300.0), window(&scene_light), 2));
    assert!(is(at(&dark, 30.0, 300.0), window(&scene_dark), 2));
}
