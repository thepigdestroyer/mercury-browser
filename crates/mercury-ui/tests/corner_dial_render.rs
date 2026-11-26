// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
#![cfg(feature = "test-support")]
//! Pixel checks of the corner dial drawn by Slint's software renderer, from the real component, the
//! real `corner_dial` functions and the real `RingBandsData`: the same scenes as
//! `examples/render_corner_dial.rs`, which this file includes as a module. Set
//! `MERCURY_UI_RENDER_DIR` to also write the PNGs.
//!
//! The software renderer draws no transforms, so the fan's opening scale is not seen here (it is 1
//! when the fan is open) and the curved labels are upright; neither is looked at.

#[allow(dead_code)]
#[path = "../examples/render_corner_dial.rs"]
mod example;

use std::collections::HashSet;
use std::path::PathBuf;

use example::{CORNERS, ESSAY, HEIGHT, Params, Pixels, Scene, WIDTH, at, rgb, save, slug};
use mercury_settings::DialCorner;
use mercury_ui::ChromePalette;
use mercury_ui::ring_bands::fixtures::{mockup_groups, mockup_tabs};
use slint::{Color, ComponentHandle};

type Rgb = (u8, u8, u8);

fn colour(c: Color) -> Rgb {
    (c.red(), c.green(), c.blue())
}

fn within(a: Rgb, b: Rgb, tolerance: u8) -> bool {
    a.0.abs_diff(b.0) <= tolerance
        && a.1.abs_diff(b.1) <= tolerance
        && a.2.abs_diff(b.2) <= tolerance
}

/// `over` (with its own alpha) laid on the opaque `under`.
fn blend(over: Color, under: Rgb) -> Rgb {
    let alpha = f32::from(over.alpha()) / 255.0;
    let mix = |a: u8, b: u8| (f32::from(a) * alpha + f32::from(b) * (1.0 - alpha)).round() as u8;
    (
        mix(over.red(), under.0),
        mix(over.green(), under.1),
        mix(over.blue(), under.2),
    )
}

fn theme(dark: bool) -> &'static str {
    if dark { "dark" } else { "light" }
}

fn out(name: &str, pixels: &Pixels) {
    if let Some(directory) = std::env::var_os("MERCURY_UI_RENDER_DIR").map(PathBuf::from) {
        save(pixels, &directory.join(format!("{name}.png"))).unwrap();
    }
}

fn mockup(params: &Params) -> Scene {
    Scene::new(
        &mockup_groups(params.dark),
        &mockup_tabs(params.dark),
        params,
    )
    .unwrap()
}

/// The point at `radius` and `deg` clockwise from 12 o'clock around (`cx`, `cy`).
fn on_circle(cx: f32, cy: f32, radius: f32, deg: f32) -> (f32, f32) {
    let rad = deg.to_radians();
    (cx + radius * rad.sin(), cy - radius * rad.cos())
}

fn track(scene: &Scene) -> Rgb {
    colour(scene.window.global::<ChromePalette>().get_separator())
}

fn accent(scene: &Scene) -> Rgb {
    colour(scene.window.global::<ChromePalette>().get_accent())
}

/// The page stand-in's own colour, with no dial near.
fn page(scene: &Scene) -> Rgb {
    colour(
        scene
            .window
            .global::<ChromePalette>()
            .get_content_background(),
    )
}

/// Angles (degrees clockwise from 12 o'clock) all round the rim, none within 4 degrees of a
/// diagonal: Slint's software renderer ignores the rounded corners of a `clip`, so the planet is
/// drawn as a square there and its corners cover the rim at the diagonals (not so in the browser's
/// own renderer).
const RIM: [f32; 8] = [0.0, 20.0, 90.0, 110.0, 180.0, 200.0, 270.0, 290.0];

/// Equal up to the edge blending of a stroked path and the gradient's rounding.
fn near(got: Rgb, want: Rgb) -> bool {
    within(got, want, 6)
}

/// The colour of the ring (radius 36, 5 px wide) at `deg` clockwise from 12 o'clock.
fn ring_at(pixels: &Pixels, scene: &Scene, deg: f32) -> Rgb {
    ring_at_radius(pixels, scene, 36.0, deg)
}

fn ring_at_radius(pixels: &Pixels, scene: &Scene, radius: f32, deg: f32) -> Rgb {
    let centre = scene.dial_centre();
    let (x, y) = on_circle(centre.x, centre.y, radius, deg);
    rgb(at(pixels, x, y))
}

/// Where the load tests look at the ring: 35 px from the centre, inside the track (33.5 to 38.5)
/// and inside the progress arc as it is drawn today, 31 to 36 (see `the_progress_arc_lies_on_the_track`).
const ARC_PROBE: f32 = 35.0;

#[test]
fn the_dial_sits_on_the_page_in_every_corner_and_shows_the_planet() {
    // The planet is the group's colour inside a darker 4 px rim, with a lighter patch centred at
    // 35 % / 30 % of the 54 px box, offset (-8.1, -10.8) from the dial's centre, 10.8 px across:
    // (16.2, -10.8) is plain colour and (-8.1, -14.0) is in the patch.
    const PLAIN: (f32, f32) = (16.2, -10.8);
    const LIT: (f32, f32) = (-8.1, -14.0);
    for dark in [false, true] {
        for corner in CORNERS {
            let scene = mockup(&Params::rest(dark, corner));
            let pixels = scene.render().unwrap();
            out(&format!("rest-{}-{}", slug(corner), theme(dark)), &pixels);
            let centre = scene.dial_centre();
            let palette = scene.window.global::<ChromePalette>();
            let active = colour(mockup_groups(dark)[0].colour);
            let what = format!("{} {}", slug(corner), theme(dark));

            assert_ne!(
                rgb(at(&pixels, centre.x, centre.y)),
                page(&scene),
                "{what}: the centre is the dial, not the page"
            );
            let inward = if centre.x < WIDTH as f32 / 2.0 {
                60.0
            } else {
                -60.0
            };
            assert_eq!(
                rgb(at(&pixels, centre.x + inward, centre.y)),
                page(&scene),
                "{what}: beside the dial is the page"
            );
            let plain = rgb(at(&pixels, centre.x + PLAIN.0, centre.y + PLAIN.1));
            assert!(
                within(plain, active, 6),
                "{what}: the planet is the group's colour {active:?}, got {plain:?}"
            );
            let lit = rgb(at(&pixels, centre.x + LIT.0, centre.y + LIT.1));
            let sum = |c: Rgb| u32::from(c.0) + u32::from(c.1) + u32::from(c.2);
            assert!(
                sum(lit) > sum(active),
                "{what}: lit from the upper left: {lit:?} is not lighter than {active:?}"
            );
            let disc = rgb(at(&pixels, centre.x + 30.5, centre.y + 0.5));
            assert_eq!(
                disc,
                colour(palette.get_popup_background()),
                "{what}: between the planet and the ring is the raised disc"
            );
        }
    }
}

#[test]
fn at_rest_the_ring_is_the_track_all_the_way_round() {
    for dark in [false, true] {
        let scene = mockup(&Params::rest(dark, DialCorner::BottomRight));
        let pixels = scene.render().unwrap();
        for deg in RIM {
            let got = ring_at(&pixels, &scene, deg);
            assert!(
                near(got, track(&scene)),
                "{}: the rim at {deg} degrees is the track, got {got:?}",
                theme(dark)
            );
        }
    }
}

#[test]
fn the_load_ring_fills_clockwise_from_twelve_oclock() {
    for dark in [false, true] {
        let name = theme(dark);
        for (progress, filled, empty) in [
            (
                0.25,
                vec![4.0, 30.0, 86.0],
                vec![100.0, 180.0, 270.0, 330.0, 356.0],
            ),
            (
                0.65,
                vec![4.0, 90.0, 180.0, 215.0],
                vec![240.0, 270.0, 300.0, 356.0],
            ),
        ] {
            let params = Params {
                loading: true,
                progress,
                ..Params::rest(dark, DialCorner::BottomRight)
            };
            let scene = mockup(&params);
            let pixels = scene.render().unwrap();
            out(
                &format!("loading-{}-{name}", (progress * 100.0) as u32),
                &pixels,
            );
            for deg in filled {
                let got = ring_at_radius(&pixels, &scene, ARC_PROBE, deg);
                assert!(
                    near(got, accent(&scene)),
                    "{name}, {progress}: accent at {deg} degrees, got {got:?}"
                );
            }
            for deg in empty {
                let got = ring_at_radius(&pixels, &scene, ARC_PROBE, deg);
                assert!(
                    near(got, track(&scene)),
                    "{name}, {progress}: track at {deg} degrees, got {got:?}"
                );
            }
        }

        let params = Params {
            loading: true,
            progress: 1.0,
            ..Params::rest(dark, DialCorner::BottomRight)
        };
        let scene = mockup(&params);
        let pixels = scene.render().unwrap();
        for deg in RIM {
            let got = ring_at(&pixels, &scene, deg);
            assert!(
                near(got, accent(&scene)),
                "{name}: at 100 % the whole ring is accent ({deg} degrees), got {got:?}"
            );
        }
    }
}

/// Slint draws a `Path` inside its box less the stroke width, so the arc's viewbox is the box minus
/// the stroke width wide (72 px): the arc lies on the track (radius 36 px, as the mockup has it)
/// and a probe at 37.5 px (the outer half of the track) sees the arc.
#[test]
fn the_progress_arc_lies_on_the_track() {
    let params = Params {
        loading: true,
        progress: 0.25,
        ..Params::rest(false, DialCorner::BottomRight)
    };
    let scene = mockup(&params);
    let pixels = scene.render().unwrap();
    for deg in [30.0, 60.0, 86.0] {
        let got = ring_at_radius(&pixels, &scene, 37.5, deg);
        assert!(
            near(got, accent(&scene)),
            "the arc covers the outer half of the track at {deg} degrees, got {got:?}"
        );
    }
}

#[test]
fn the_ring_ends_whole_and_then_fades_back_to_the_track() {
    let params = Params {
        loading: true,
        progress: 0.9,
        reduce_motion: false,
        ..Params::rest(false, DialCorner::BottomRight)
    };
    let scene = mockup(&params);
    scene.render().unwrap();
    example::advance_clock(200);
    scene.frame().unwrap();
    assert_eq!(
        ring_at(&scene.frame().unwrap(), &scene, 340.0),
        track(&scene)
    );

    scene.set_load(false, 0.9);
    let pixels = scene.frame().unwrap();
    out("fading-light", &pixels);
    let distance = |a: Rgb, b: Rgb| {
        i32::from(a.0.abs_diff(b.0)) + i32::from(a.1.abs_diff(b.1)) + i32::from(a.2.abs_diff(b.2))
    };
    for deg in [340.0, 180.0] {
        let seen = ring_at(&pixels, &scene, deg);
        assert!(
            distance(seen, accent(&scene)) < distance(seen, track(&scene)),
            "the load just ended: the whole ring is still accent at {deg} degrees, got {seen:?}"
        );
    }

    // Half way through the fade (mocked clock: exactly 200 of 400 ms) the ring is neither.
    example::advance_clock(200);
    let half = ring_at(&scene.frame().unwrap(), &scene, 340.0);
    assert!(
        half != accent(&scene) && half != track(&scene),
        "200 ms into the fade the ring is between accent and track, got {half:?}"
    );
    assert!(
        distance(half, accent(&scene)) < distance(accent(&scene), track(&scene))
            && distance(half, track(&scene)) < distance(accent(&scene), track(&scene)),
        "and closer to each than they are to one another: {half:?}"
    );

    example::advance_clock(400);
    let pixels = scene.frame().unwrap();
    for deg in [340.0, 180.0] {
        assert_eq!(
            ring_at(&pixels, &scene, deg),
            track(&scene),
            "400 ms later the ring is the track again ({deg} degrees)"
        );
    }
}

/// The shades of the 22 tabs (what `ArcSeg` is filled with) as bytes.
fn shades(scene: &Scene) -> HashSet<Rgb> {
    scene
        .fan
        .borrow()
        .data
        .tabs
        .iter()
        .map(|tab| colour(tab.shade))
        .collect()
}

#[test]
fn the_fan_opens_away_from_every_corner_with_every_segment_in_its_shade() {
    for dark in [false, true] {
        for corner in CORNERS {
            let scene = mockup(&Params::fanned(dark, corner));
            let pixels = scene.render().unwrap();
            out(&format!("fanned-{}-{}", slug(corner), theme(dark)), &pixels);
            let what = format!("{} {}", slug(corner), theme(dark));
            let centre = scene.dial_centre();
            // From the corner toward the middle of the window: the way the fan opens.
            let toward = (
                WIDTH as f32 / 2.0 - centre.x,
                HEIGHT as f32 / 2.0 - centre.y,
            );
            let side = |x: f32, y: f32| (x - centre.x) * toward.0 + (y - centre.y) * toward.1;

            let fan = scene.fan.borrow();
            assert_eq!(fan.data.segments.len(), 22, "{what}");
            for (segment, tab) in fan.data.segments.iter().zip(&fan.data.tabs) {
                let (x, y) = (fan.origin.x + segment.mid_x, fan.origin.y + segment.mid_y);
                assert!(
                    x >= 0.0 && y >= 0.0 && x < WIDTH as f32 && y < HEIGHT as f32,
                    "{what}: tab {} is on the screen",
                    tab.id
                );
                assert!(
                    side(x, y) > 0.0,
                    "{what}: tab {} lies on the screen side of the dial",
                    tab.id
                );
                assert_eq!(
                    rgb(at(&pixels, x, y)),
                    colour(tab.shade),
                    "{what}: tab {} at ({x}, {y})",
                    tab.id
                );
            }
            drop(fan);

            // Every pixel behind the dial (outside the dial itself) is not a segment's; the front has all.
            let all = shades(&scene);
            let (mut behind, mut front) = (0, HashSet::new());
            for y in 0..HEIGHT {
                for x in 0..WIDTH {
                    let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
                    let pixel = rgb(at(&pixels, px, py));
                    if !all.contains(&pixel) {
                        continue;
                    }
                    if side(px, py) < 0.0 && (px - centre.x).hypot(py - centre.y) > 45.0 {
                        behind += 1;
                    } else if side(px, py) > 0.0 {
                        front.insert(pixel);
                    }
                }
            }
            assert_eq!(behind, 0, "{what}: segment pixels behind the dial");
            assert_eq!(front, all, "{what}: every shade is drawn in front of it");
        }
    }
}

#[test]
fn the_dial_stays_over_the_fan_as_its_hub() {
    for dark in [false, true] {
        let scene = mockup(&Params::fanned(dark, DialCorner::BottomRight));
        let pixels = scene.render().unwrap();
        let centre = scene.dial_centre();
        let palette = scene.window.global::<ChromePalette>();
        let active = colour(mockup_groups(dark)[0].colour);
        assert!(
            within(
                rgb(at(&pixels, centre.x + 16.2, centre.y - 10.8)),
                active,
                6
            ),
            "{}: the planet is not under the fan",
            theme(dark)
        );
        assert_eq!(
            rgb(at(&pixels, centre.x + 30.5, centre.y + 0.5)),
            colour(palette.get_popup_background()),
            "{}: the disc is not under the fan",
            theme(dark)
        );
        assert_eq!(
            ring_at(&pixels, &scene, 270.0),
            track(&scene),
            "{}: the rim is not under the fan",
            theme(dark)
        );
    }
}

#[test]
fn the_scrim_dims_the_page_by_the_palette_scrim_at_60_percent() {
    for dark in [false, true] {
        let rest = mockup(&Params::rest(dark, DialCorner::BottomRight));
        let before = rest.render().unwrap();
        let fanned = mockup(&Params::fanned(dark, DialCorner::BottomRight));
        let after = fanned.render().unwrap();
        let scrim = fanned
            .window
            .global::<ChromePalette>()
            .get_scrim()
            .transparentize(0.4);
        assert!(scrim.alpha() > 0, "{}: the scrim has a colour", theme(dark));
        let separator = track(&fanned);
        // Far from the fan: the empty page, and the stand-in's picture block.
        for (x, y, under) in [(100.0, 400.0, page(&rest)), (500.0, 300.0, separator)] {
            assert_eq!(
                rgb(at(&before, x, y)),
                under,
                "{}: the page at rest at ({x}, {y})",
                theme(dark)
            );
            let want = blend(scrim, under);
            let got = rgb(at(&after, x, y));
            assert!(
                within(got, want, 2),
                "{}: dimmed at ({x}, {y}): {got:?}, expected {want:?}",
                theme(dark)
            );
            if want != under {
                assert_ne!(got, under, "{}: it is dimmed", theme(dark));
            }
        }
    }
}

#[test]
fn the_caption_is_a_popup_box_next_to_the_hovered_tab() {
    for dark in [false, true] {
        let params = Params {
            hovered: Some(ESSAY),
            ..Params::fanned(dark, DialCorner::BottomRight)
        };
        let scene = mockup(&params);
        let pixels = scene.render().unwrap();
        out(&format!("caption-{}", theme(dark)), &pixels);
        let (caption, origin) = {
            let fan = scene.fan.borrow();
            (fan.caption(ESSAY).unwrap(), fan.origin)
        };
        assert_eq!(caption.text.as_str(), "Reading · Essay on maps");
        let palette = scene.window.global::<ChromePalette>();
        let (x, y) = (origin.x + caption.x, origin.y + caption.y);
        assert!(x >= 0.0 && y >= 0.0 && x + 186.0 <= WIDTH as f32 && y + 30.0 <= HEIGHT as f32);
        // Left of the text, inside the 12 px padding.
        assert_eq!(
            rgb(at(&pixels, x + 6.0, y + 15.0)),
            colour(palette.get_popup_background()),
            "{}: the caption's box",
            theme(dark)
        );
        // Without a hovered tab there is no caption box at that place.
        let plain = mockup(&Params::fanned(dark, DialCorner::BottomRight));
        let pixels = plain.render().unwrap();
        assert_ne!(
            rgb(at(&pixels, x + 6.0, y + 15.0)),
            colour(palette.get_popup_background()),
            "{}: no caption without a hovered tab",
            theme(dark)
        );
    }
}

#[test]
fn the_scheme_changes_the_discs_colour() {
    let mut discs = Vec::new();
    for dark in [false, true] {
        let scene = mockup(&Params::rest(dark, DialCorner::BottomRight));
        let pixels = scene.render().unwrap();
        let centre = scene.dial_centre();
        let disc = rgb(at(&pixels, centre.x + 30.5, centre.y + 0.5));
        assert_eq!(
            disc,
            colour(
                scene
                    .window
                    .global::<ChromePalette>()
                    .get_popup_background()
            ),
            "{}",
            theme(dark)
        );
        discs.push(disc);
    }
    assert_ne!(discs[0], discs[1], "light and dark have different discs");
}

#[test]
fn the_disc_is_not_clipped_in_an_area_of_exactly_148_by_148() {
    for dark in [false, true] {
        let small = Params {
            width: 148,
            height: 148,
            ..Params::rest(dark, DialCorner::BottomRight)
        };
        let scene = mockup(&small);
        let pixels = scene.render().unwrap();
        out(&format!("area-148-{}", theme(dark)), &pixels);
        assert_eq!((pixels.width(), pixels.height()), (148, 148));
        let centre = scene.dial_centre();
        assert_eq!(
            (centre.x, centre.y),
            (74.0, 74.0),
            "inset 74 meets half the area"
        );

        // Every point of the disc and its ring is what the big window draws around its dial.
        let big = mockup(&Params::rest(dark, DialCorner::BottomRight));
        let big_pixels = big.render().unwrap();
        let big_centre = big.dial_centre();
        let mut compared = 0;
        for dy in -38..38 {
            for dx in -38..38 {
                let (dx, dy) = (dx as f32 + 0.5, dy as f32 + 0.5);
                if dx.hypot(dy) > 37.0 {
                    continue;
                }
                let small = rgb(at(&pixels, centre.x + dx, centre.y + dy));
                let big = rgb(at(&big_pixels, big_centre.x + dx, big_centre.y + dy));
                assert!(
                    within(small, big, 2),
                    "{}: the dial at ({dx}, {dy}) from its centre: {small:?} against {big:?}",
                    theme(dark)
                );
                compared += 1;
            }
        }
        assert!(compared > 4000, "{compared} pixels compared");
        for deg in (0..360).step_by(90) {
            assert_eq!(
                ring_at(&pixels, &scene, deg as f32),
                track(&scene),
                "{}: the rim at {deg} degrees is whole",
                theme(dark)
            );
        }
        // Beside the dial, below the stand-in's toolbar, is the page: nothing else is drawn.
        assert_eq!(
            rgb(at(&pixels, 4.0, 120.0)),
            page(&scene),
            "{}",
            theme(dark)
        );
    }
}

#[test]
fn a_dragged_dial_leaves_its_corner_and_closes_the_fan() {
    let scene = mockup(&Params::fanned(false, DialCorner::BottomRight));
    scene.render().unwrap();
    scene.drag_to((1010.0, 590.0));
    let pixels = scene.render().unwrap();
    out("dragging-light", &pixels);
    let rest = scene.dial_centre();
    assert_eq!(
        rgb(at(&pixels, rest.x + 16.2, rest.y - 10.8)),
        page(&scene),
        "the corner is empty"
    );
    let active = colour(mockup_groups(false)[0].colour);
    assert!(
        within(rgb(at(&pixels, 1010.0 + 16.2, 590.0 - 10.8)), active, 6),
        "the planet is under the pointer"
    );
    // The planet is blue like the Research tabs, so the dial's own pixels are left out.
    let all = shades(&scene);
    let stray = (0..HEIGHT)
        .flat_map(|y| (0..WIDTH).map(move |x| (x as f32 + 0.5, y as f32 + 0.5)))
        .filter(|&(x, y)| (x - 1010.0).hypot(y - 590.0) > 45.0)
        .filter(|&(x, y)| all.contains(&rgb(at(&pixels, x, y))))
        .count();
    assert_eq!(stray, 0, "the fan is closed while the dial is dragged");
}
