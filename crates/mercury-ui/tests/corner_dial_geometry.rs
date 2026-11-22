// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! The snapping maths of the corner dial (lane corner-dial, item 1): where the dial rests, which
//! corner a release snaps to, how a drag is kept inside the area, the fan's orientation and whether
//! it fits. No window and no ring data: `ring_geometry` (the real hit test) is the only other code.

use mercury_settings::DialCorner;
use mercury_ui::corner_dial::{
    DIAL_DIAMETER, DIAL_MARGIN, DialGeometry, FAN_BOX, FAN_REACH, fan_spec, fan_start_deg,
};
use mercury_ui::ring_geometry::{Hit, Point, RingLayout, hit_test, point_on_circle};

const CORNERS: [DialCorner; 4] = [
    DialCorner::TopLeft,
    DialCorner::TopRight,
    DialCorner::BottomLeft,
    DialCorner::BottomRight,
];

/// Bottom-right, bottom-left, top-left, top-right: the order of a quarter turn clockwise, so that
/// `FAN_ORDER[k]` is the bottom-right fan turned `k` quarter turns.
const FAN_ORDER: [DialCorner; 4] = [
    DialCorner::BottomRight,
    DialCorner::BottomLeft,
    DialCorner::TopLeft,
    DialCorner::TopRight,
];

/// The groups of the 22-tab mockup fixture: tabs per group.
const MOCKUP_GROUPS: [usize; 6] = [5, 4, 6, 3, 2, 2];

fn p(x: f32, y: f32) -> Point {
    Point::new(x, y)
}

/// The tie tolerance the contract names, in px.
const TIE: f64 = 1e-4;

/// The order in which a tie that does not include the current corner is settled (the contract).
const TIE_ORDER: [DialCorner; 4] = [
    DialCorner::TopLeft,
    DialCorner::TopRight,
    DialCorner::BottomLeft,
    DialCorner::BottomRight,
];

/// The corners whose resting centre is within `TIE` of the nearest one, from squared distances (not
/// the code under test's path: no `hypot`), in `TIE_ORDER`.
fn tied_corners(g: &DialGeometry, release: Point) -> Vec<DialCorner> {
    let squared: Vec<(DialCorner, f64)> = TIE_ORDER
        .iter()
        .map(|&c| {
            let centre = g.centre(c);
            let dx = f64::from(release.x) - f64::from(centre.x);
            let dy = f64::from(release.y) - f64::from(centre.y);
            (c, dx * dx + dy * dy)
        })
        .collect();
    let best = squared
        .iter()
        .map(|d| d.1)
        .fold(f64::INFINITY, f64::min)
        .sqrt();
    squared
        .iter()
        .filter(|d| d.1.sqrt() - best <= TIE)
        .map(|d| d.0)
        .collect()
}

/// The oracle: the tie set first, then the rule: the only corner of the set; else `current` if it is
/// in the set; else the first of the set in `TIE_ORDER`.
fn oracle(g: &DialGeometry, release: Point, current: DialCorner) -> DialCorner {
    let tied = tied_corners(g, release);
    if tied.contains(&current) {
        current
    } else {
        tied[0]
    }
}

/// `(i, j)` of an `(nx + 1) x (ny + 1)` grid over the area `[x0, x1] x [y0, y1]`.
fn grid(x0: f32, x1: f32, y0: f32, y1: f32, nx: usize, ny: usize) -> Vec<Point> {
    let mut points = Vec::new();
    for j in 0..=ny {
        for i in 0..=nx {
            let x = x0 + (x1 - x0) * i as f32 / nx as f32;
            let y = y0 + (y1 - y0) * j as f32 / ny as f32;
            points.push(p(x, y));
        }
    }
    points
}

const AREAS: [(f32, f32); 7] = [
    (1440.0, 900.0),
    (900.0, 1440.0),
    (1000.0, 1000.0),
    (200.0, 200.0),
    (140.0, 140.0),
    (60.0, 40.0),
    (2000.0, 90.0),
];

// ----- the resting centres ------------------------------------------------------------------

#[test]
fn the_four_centres_of_1440_by_900_are_the_specs() {
    let g = DialGeometry::new(1440.0, 900.0);
    assert_eq!(g.centre(DialCorner::TopLeft), p(74.0, 74.0));
    assert_eq!(g.centre(DialCorner::TopRight), p(1366.0, 74.0));
    assert_eq!(g.centre(DialCorner::BottomLeft), p(74.0, 826.0));
    assert_eq!(g.centre(DialCorner::BottomRight), p(1366.0, 826.0));
}

#[test]
fn the_inset_is_74_px_the_radius_plus_the_margin() {
    assert_eq!(DIAL_DIAMETER / 2.0 + DIAL_MARGIN, 74.0);
    assert_eq!(DialGeometry::new(1440.0, 900.0).inset(), 74.0);
    assert_eq!(DialGeometry::new(148.0, 148.0).inset(), 74.0);
    // 38 px between the dial's edge and the area's edge.
    let g = DialGeometry::new(800.0, 600.0);
    let c = g.centre(DialCorner::BottomRight);
    assert_eq!(800.0 - c.x - DIAL_DIAMETER / 2.0, 38.0);
    assert_eq!(600.0 - c.y - DIAL_DIAMETER / 2.0, 38.0);
}

#[test]
fn the_inset_never_exceeds_half_the_area_nor_74() {
    for (w, h) in AREAS {
        let g = DialGeometry::new(w, h);
        let inset = g.inset();
        assert!(inset >= 0.0, "{w}x{h}");
        assert!(
            inset <= w / 2.0 && inset <= h / 2.0 && inset <= 74.0,
            "{w}x{h}"
        );
    }
    assert_eq!(DialGeometry::new(140.0, 140.0).inset(), 70.0);
    assert_eq!(DialGeometry::new(2000.0, 90.0).inset(), 45.0);
    assert_eq!(DialGeometry::new(60.0, 40.0).inset(), 20.0);
}

#[test]
fn an_area_of_140_meets_the_centres_in_the_middle_and_none_crosses() {
    // 140 px is under 148: the inset is clamped to 70, which is half the area, so the left and right
    // centres meet at 70 and cannot cross.
    let g = DialGeometry::new(140.0, 140.0);
    for corner in CORNERS {
        assert_eq!(g.centre(corner), p(70.0, 70.0));
    }
    // Between the clamp and 74 px the centres are apart again and ordered.
    let g = DialGeometry::new(200.0, 160.0);
    let (tl, tr) = (
        g.centre(DialCorner::TopLeft),
        g.centre(DialCorner::TopRight),
    );
    let (bl, br) = (
        g.centre(DialCorner::BottomLeft),
        g.centre(DialCorner::BottomRight),
    );
    assert_eq!(
        (tl, tr, bl, br),
        (p(74.0, 74.0), p(126.0, 74.0), p(74.0, 86.0), p(126.0, 86.0))
    );
}

#[test]
fn centres_are_ordered_and_inside_the_area_for_every_area() {
    let hostile = [
        (0.0, 0.0),
        (1.0, 1.0),
        (-5.0, 300.0),
        (f32::NAN, 300.0),
        (300.0, f32::NAN),
        (f32::INFINITY, 900.0),
        (f32::MAX, f32::MAX),
        (1.0e9, 1.0e9),
    ];
    for (w, h) in AREAS.into_iter().chain(hostile) {
        let g = DialGeometry::new(w, h);
        let inset = g.inset();
        assert!(inset.is_finite() && inset >= 0.0, "{w}x{h}: inset {inset}");
        let tl = g.centre(DialCorner::TopLeft);
        let tr = g.centre(DialCorner::TopRight);
        let bl = g.centre(DialCorner::BottomLeft);
        let br = g.centre(DialCorner::BottomRight);
        for c in [tl, tr, bl, br] {
            assert!(c.x.is_finite() && c.y.is_finite(), "{w}x{h}: {c:?}");
            assert!(c.x >= 0.0 && c.y >= 0.0, "{w}x{h}: {c:?}");
        }
        assert!(tl.x <= tr.x && bl.x <= br.x, "{w}x{h}: left of right");
        assert!(tl.y <= bl.y && tr.y <= br.y, "{w}x{h}: top above bottom");
        assert_eq!((tl.x, tl.y), (bl.x, tr.y), "{w}x{h}: a rectangle");
    }
}

// ----- snapping ------------------------------------------------------------------------------

#[test]
fn snap_equals_the_brute_force_nearest_centre_except_on_a_tie() {
    for (w, h) in AREAS {
        let g = DialGeometry::new(w, h);
        for release in grid(0.0, w, 0.0, h, 60, 40) {
            for current in CORNERS {
                assert_eq!(
                    g.snap(release, current),
                    oracle(&g, release, current),
                    "{w}x{h} release {release:?} current {current:?}"
                );
            }
        }
    }
}

#[test]
fn snap_equals_the_oracle_far_outside_the_area_too() {
    for (w, h) in [(1440.0, 900.0), (300.0, 900.0)] {
        let g = DialGeometry::new(w, h);
        for release in grid(-2.0 * w, 3.0 * w, -2.0 * h, 3.0 * h, 61, 41) {
            for current in CORNERS {
                assert_eq!(g.snap(release, current), oracle(&g, release, current));
            }
        }
    }
}

#[test]
fn off_the_axes_the_nearest_corner_is_the_quadrant_of_the_areas_middle() {
    // Brief D2: the nearest resting centre is the quadrant of the middle whenever the four centres
    // form a rectangle (the area is at least 148 px both ways).
    for (w, h) in [
        (1440.0, 900.0),
        (900.0, 1440.0),
        (1000.0, 1000.0),
        (200.0, 200.0),
        (300.0, 700.0),
    ] {
        let g = DialGeometry::new(w, h);
        for release in grid(-100.0, w + 100.0, -100.0, h + 100.0, 83, 59) {
            if (release.x - w / 2.0).abs() < 0.01 || (release.y - h / 2.0).abs() < 0.01 {
                continue;
            }
            let expected = match (release.x < w / 2.0, release.y < h / 2.0) {
                (true, true) => DialCorner::TopLeft,
                (false, true) => DialCorner::TopRight,
                (true, false) => DialCorner::BottomLeft,
                (false, false) => DialCorner::BottomRight,
            };
            for current in CORNERS {
                assert_eq!(g.snap(release, current), expected, "{w}x{h} {release:?}");
            }
        }
    }
}

#[test]
fn a_release_on_an_axis_is_decided_between_the_two_tied_corners_only() {
    let g = DialGeometry::new(1440.0, 900.0);
    let (tl, tr, bl, br) = (
        DialCorner::TopLeft,
        DialCorner::TopRight,
        DialCorner::BottomLeft,
        DialCorner::BottomRight,
    );
    // (release, the tied corners): on the vertical axis the left and the right centre are equally
    // far, on the horizontal axis the top and the bottom one.
    let cases = [
        (p(720.0, 100.0), [tl, tr]),
        (p(720.0, 800.0), [bl, br]),
        (p(100.0, 450.0), [tl, bl]),
        (p(1300.0, 450.0), [tr, br]),
    ];
    for (release, pair) in cases {
        assert_eq!(tied_corners(&g, release), pair, "{release:?}");
        for current in TIE_ORDER {
            let got = g.snap(release, current);
            if pair.contains(&current) {
                assert_eq!(got, current, "{release:?}: a tied current corner is kept");
            } else {
                // A dial dropped at the top that came from the bottom goes to the top, not back.
                assert_eq!(got, pair[0], "{release:?} from {current:?}");
            }
        }
    }
    // The exact middle: all four are tied, so the current corner is kept.
    for current in TIE_ORDER {
        assert_eq!(g.snap(p(720.0, 450.0), current), current);
    }
    // A hair to either side of the axis is decided.
    assert_eq!(g.snap(p(719.0, 100.0), br), tl);
    assert_eq!(g.snap(p(721.0, 100.0), bl), tr);
}

#[test]
fn coincident_centres_in_a_small_area_never_trap_the_dial() {
    // 2000 x 90: the top two centres are the bottom two (the inset is 45): every release is a tie of
    // two corners, and the dial still moves from the left to the right.
    let g = DialGeometry::new(2000.0, 90.0);
    assert_eq!(
        g.centre(DialCorner::TopLeft),
        g.centre(DialCorner::BottomLeft)
    );
    let right = g.snap(p(1900.0, 45.0), DialCorner::TopLeft);
    assert!(
        matches!(right, DialCorner::TopRight | DialCorner::BottomRight),
        "{right:?}"
    );
    let left = g.snap(p(100.0, 45.0), DialCorner::BottomRight);
    assert!(
        matches!(left, DialCorner::TopLeft | DialCorner::BottomLeft),
        "{left:?}"
    );
}

#[test]
fn a_release_at_the_dials_own_centre_never_changes_the_corner() {
    for (w, h) in AREAS {
        let g = DialGeometry::new(w, h);
        for corner in CORNERS {
            assert_eq!(
                g.snap(g.centre(corner), corner),
                corner,
                "{w}x{h} {corner:?}"
            );
        }
    }
    // Where the four centres are distinct the centre of a corner snaps to that corner whatever the
    // current one is.
    for (w, h) in [
        (1440.0, 900.0),
        (900.0, 1440.0),
        (1000.0, 1000.0),
        (200.0, 200.0),
    ] {
        let g = DialGeometry::new(w, h);
        for corner in CORNERS {
            for current in CORNERS {
                assert_eq!(g.snap(g.centre(corner), current), corner, "{w}x{h}");
            }
        }
    }
}

#[test]
fn a_release_that_is_not_a_number_keeps_the_current_corner() {
    let g = DialGeometry::new(1440.0, 900.0);
    for release in [
        p(f32::NAN, 100.0),
        p(100.0, f32::NAN),
        p(f32::NAN, f32::NAN),
        p(f32::INFINITY, 0.0),
        p(0.0, f32::NEG_INFINITY),
    ] {
        for current in CORNERS {
            assert_eq!(g.snap(release, current), current, "{release:?}");
        }
    }
}

#[test]
fn snap_is_total_on_hostile_areas() {
    for (w, h) in [
        (0.0, 0.0),
        (-3.0, 10.0),
        (f32::NAN, f32::NAN),
        (f32::INFINITY, 5.0),
        (f32::MAX, f32::MAX),
    ] {
        let g = DialGeometry::new(w, h);
        for release in [p(0.0, 0.0), p(10.0, 10.0), p(1.0e30, -1.0e30)] {
            for current in CORNERS {
                let _ = g.snap(release, current);
            }
        }
    }
}

// ----- dragging ------------------------------------------------------------------------------

#[test]
fn clamp_drag_keeps_the_disc_inside_the_area() {
    let g = DialGeometry::new(1440.0, 900.0);
    let half = DIAL_DIAMETER / 2.0;
    for q in grid(-5000.0, 6000.0, -5000.0, 6000.0, 40, 40) {
        let c = g.clamp_drag(q);
        assert!(c.x >= half && c.x <= 1440.0 - half, "{q:?} -> {c:?}");
        assert!(c.y >= half && c.y <= 900.0 - half, "{q:?} -> {c:?}");
    }
    // Inside the allowed rectangle nothing moves; outside the point goes to the nearest edge.
    assert_eq!(g.clamp_drag(p(700.0, 400.0)), p(700.0, 400.0));
    assert_eq!(g.clamp_drag(p(36.0, 864.0)), p(36.0, 864.0));
    assert_eq!(g.clamp_drag(p(-10.0, 400.0)), p(36.0, 400.0));
    assert_eq!(g.clamp_drag(p(700.0, 5000.0)), p(700.0, 864.0));
    assert_eq!(g.clamp_drag(p(9999.0, -9999.0)), p(1404.0, 36.0));
}

#[test]
fn clamp_drag_of_infinite_input_goes_to_the_edge_and_of_nan_to_the_middle() {
    let g = DialGeometry::new(1440.0, 900.0);
    assert_eq!(
        g.clamp_drag(p(f32::INFINITY, f32::NEG_INFINITY)),
        p(1404.0, 36.0)
    );
    assert_eq!(
        g.clamp_drag(p(f32::NEG_INFINITY, f32::INFINITY)),
        p(36.0, 864.0)
    );
    // NaN becomes the middle of the area, per coordinate.
    assert_eq!(g.clamp_drag(p(f32::NAN, f32::NAN)), p(720.0, 450.0));
    assert_eq!(g.clamp_drag(p(f32::NAN, 100.0)), p(720.0, 100.0));
    assert_eq!(g.clamp_drag(p(100.0, f32::NAN)), p(100.0, 450.0));
}

#[test]
fn clamp_drag_in_an_area_under_72_px_is_the_middle() {
    let g = DialGeometry::new(60.0, 40.0);
    for q in [
        p(0.0, 0.0),
        p(30.0, 20.0),
        p(500.0, 500.0),
        p(f32::NAN, 7.0),
    ] {
        assert_eq!(g.clamp_drag(q), p(30.0, 20.0), "{q:?}");
    }
    // Exactly 72 px: the allowed range is the single point 36.
    let g = DialGeometry::new(72.0, 72.0);
    assert_eq!(g.clamp_drag(p(0.0, 100.0)), p(36.0, 36.0));
    // A wide area that is too low: only the vertical middle.
    let g = DialGeometry::new(1000.0, 50.0);
    assert_eq!(g.clamp_drag(p(10.0, 10.0)), p(36.0, 25.0));
}

#[test]
fn clamp_drag_is_total_on_hostile_areas() {
    for (w, h) in [
        (0.0, 0.0),
        (-3.0, 10.0),
        (f32::NAN, f32::NAN),
        (f32::INFINITY, 5.0),
        (f32::MAX, f32::MAX),
    ] {
        let g = DialGeometry::new(w, h);
        for q in [p(0.0, 0.0), p(f32::NAN, 1.0), p(1.0e30, -1.0e30)] {
            let c = g.clamp_drag(q);
            assert!(!c.x.is_nan() && !c.y.is_nan(), "{w}x{h} {q:?} -> {c:?}");
        }
    }
}

#[test]
fn snap_after_clamp_drag_is_stable() {
    for (w, h) in [
        (1440.0, 900.0),
        (900.0, 1440.0),
        (1000.0, 1000.0),
        (200.0, 200.0),
        (150.0, 700.0),
    ] {
        let g = DialGeometry::new(w, h);
        for q in grid(-2.0 * w, 3.0 * w, -2.0 * h, 3.0 * h, 50, 50) {
            let once = g.clamp_drag(q);
            // Clamping twice changes nothing ...
            assert_eq!(g.clamp_drag(once), once, "{w}x{h} {q:?}");
            for current in CORNERS {
                // ... the clamp never moves a point to another quadrant, so the snap agrees with
                // the snap of the raw point ...
                let snapped = g.snap(once, current);
                assert_eq!(snapped, g.snap(q, current), "{w}x{h} {q:?}");
                // ... and snapping the resting centre of the answer gives the answer again.
                assert_eq!(g.snap(g.centre(snapped), snapped), snapped);
                assert_eq!(g.snap(g.clamp_drag(g.centre(snapped)), current), snapped);
            }
        }
    }
}

// ----- the fan -------------------------------------------------------------------------------

#[test]
fn the_start_angles_are_267_plus_a_quarter_turn_per_corner() {
    assert_eq!(fan_start_deg(DialCorner::BottomRight), 267.0);
    assert_eq!(fan_start_deg(DialCorner::BottomLeft), 357.0);
    assert_eq!(fan_start_deg(DialCorner::TopLeft), 87.0);
    assert_eq!(fan_start_deg(DialCorner::TopRight), 177.0);
    for (k, corner) in FAN_ORDER.into_iter().enumerate() {
        assert_eq!(fan_start_deg(corner), (267.0 + 90.0 * k as f32) % 360.0);
    }
}

#[test]
fn the_spec_is_the_dial_fan_preset_centred_in_the_box() {
    assert_eq!(FAN_BOX, 768.0);
    for corner in CORNERS {
        let spec = fan_spec(corner);
        assert_eq!(spec.centre, p(384.0, 384.0));
        assert_eq!(spec.start_deg, fan_start_deg(corner));
        assert_eq!(spec.span_deg, 98.0);
        assert_eq!(spec.radius, 340.0);
        assert_eq!(spec.band_width, 28.0);
        assert_eq!(spec.reserve_deg, 0.0);
        // 340 + 14 + 2 + 24: the reach the dial's constants name.
        assert_eq!(spec.label_outer_radius(), FAN_REACH);
    }
}

/// `v` turned `quarters` quarter turns clockwise (on screen, y down) about the origin.
fn turn(mut v: (f32, f32), quarters: usize) -> (f32, f32) {
    for _ in 0..quarters {
        v = (-v.1, v.0);
    }
    v
}

#[test]
fn the_four_fans_are_the_bottom_right_fan_turned_by_quarter_turns() {
    // Every point of the box, turned about the dial by k quarter turns, has the same `Hit` in fan k
    // as the original has in the bottom-right fan: the same tabs, in the same clockwise order.
    let layouts: Vec<RingLayout> = FAN_ORDER
        .iter()
        .map(|&c| RingLayout::new(fan_spec(c), &MOCKUP_GROUPS))
        .collect();
    let centre = layouts[0].spec.centre;
    let mut seen_tabs = std::collections::BTreeSet::new();
    let mut seen_groups = std::collections::BTreeSet::new();
    let (mut centres, mut outsides) = (0, 0);
    let step = 5.3;
    let mut dy = -399.9;
    while dy < 400.0 {
        let mut dx = -399.9;
        while dx < 400.0 {
            let base = hit_test(&layouts[0], p(centre.x + dx, centre.y + dy), None);
            match base {
                Hit::Tab(t) => {
                    seen_tabs.insert(t);
                }
                Hit::Group(g) => {
                    seen_groups.insert(g);
                }
                Hit::Centre => centres += 1,
                Hit::Outside => outsides += 1,
                other => panic!("a fan has no {other:?}"),
            }
            for (k, layout) in layouts.iter().enumerate().skip(1) {
                let (tx, ty) = turn((dx, dy), k);
                let turned = hit_test(layout, p(centre.x + tx, centre.y + ty), None);
                assert_eq!(turned, base, "fan {k}: ({dx}, {dy}) turned to ({tx}, {ty})");
            }
            dx += step;
        }
        dy += step;
    }
    // The sweep reached everything there is: all 22 tabs, all 6 labels, the centre and the outside.
    assert_eq!(seen_tabs.len(), 22);
    assert_eq!(seen_groups.len(), 6);
    assert!(centres > 100 && outsides > 100);
}

/// The angle of the direction (`dx`, `dy`) in degrees clockwise from 12 o'clock, in `[0, 360)`.
fn bearing(dx: f32, dy: f32) -> f32 {
    dx.atan2(-dy).to_degrees().rem_euclid(360.0)
}

/// The smallest angle between two bearings.
fn angle_between(a: f32, b: f32) -> f32 {
    let d = (a - b).rem_euclid(360.0);
    d.min(360.0 - d)
}

#[test]
fn each_fan_opens_toward_the_middle_of_a_square_area_within_2_degrees() {
    for side in [600.0, 900.0, 1200.0, 2000.0] {
        let g = DialGeometry::new(side, side);
        for corner in CORNERS {
            let c = g.centre(corner);
            let toward_middle = bearing(side / 2.0 - c.x, side / 2.0 - c.y);
            let fan_middle = fan_start_deg(corner) + 98.0 / 2.0;
            let off = angle_between(fan_middle, toward_middle);
            assert!(
                off <= 2.0,
                "{side} {corner:?}: fan {fan_middle}, middle {toward_middle}"
            );
        }
    }
}

#[test]
fn on_a_wide_area_each_fan_still_opens_into_the_quadrant_of_the_middle() {
    // The diagonal is only exact on a square area; on 1440 x 900 the bearing of the middle is up to
    // 16 degrees off the fan's axis, but the whole fan's axis stays on the screen side of the dial.
    let g = DialGeometry::new(1440.0, 900.0);
    for corner in CORNERS {
        let c = g.centre(corner);
        let toward_middle = bearing(720.0 - c.x, 450.0 - c.y);
        let fan_middle = fan_start_deg(corner) + 49.0;
        assert!(
            angle_between(fan_middle, toward_middle) < 20.0,
            "{corner:?}"
        );
        let axis = (
            fan_middle.to_radians().sin(),
            -fan_middle.to_radians().cos(),
        );
        assert_eq!(axis.0 > 0.0, 720.0 > c.x, "{corner:?} horizontal");
        assert_eq!(axis.1 > 0.0, 450.0 > c.y, "{corner:?} vertical");
    }
}

#[test]
fn the_fan_fits_a_normal_window_and_not_a_small_one() {
    let big = DialGeometry::new(1440.0, 900.0);
    let small = DialGeometry::new(400.0, 300.0);
    for corner in CORNERS {
        assert!(big.fan_fits(corner), "{corner:?} on 1440 x 900");
        assert!(!small.fan_fits(corner), "{corner:?} on 400 x 300");
    }
}

#[test]
fn the_fan_needs_454_px_both_ways_exactly() {
    // The arc's extreme points sit on the axes: 74 + 380 = 454 from the dial's near edge.
    for corner in CORNERS {
        assert!(
            DialGeometry::new(454.0, 454.0).fan_fits(corner),
            "{corner:?} 454"
        );
        assert!(
            !DialGeometry::new(453.9, 454.0).fan_fits(corner),
            "{corner:?} 453.9 wide"
        );
        assert!(
            !DialGeometry::new(454.0, 453.9).fan_fits(corner),
            "{corner:?} 453.9 high"
        );
        assert!(DialGeometry::new(5000.0, 5000.0).fan_fits(corner));
        // Wide but low, and tall but narrow.
        assert!(!DialGeometry::new(1440.0, 300.0).fan_fits(corner));
        assert!(!DialGeometry::new(300.0, 900.0).fan_fits(corner));
        assert!(!DialGeometry::new(0.0, 0.0).fan_fits(corner));
        assert!(!DialGeometry::new(f32::NAN, 900.0).fan_fits(corner));
    }
}

#[test]
fn the_axis_crossings_decide_not_only_the_end_points() {
    // The bottom-right fan spans 267 to 5 degrees. In a 453.9 px square its end points are inside
    // (the start is 0.4 px from the left edge) but the arc passes through 270 degrees, 0.1 px beyond
    // the edge: only the axis crossing finds that.
    let g = DialGeometry::new(453.9, 453.9);
    let centre = g.centre(DialCorner::BottomRight);
    let start = fan_start_deg(DialCorner::BottomRight);
    for end in [start, start + 98.0] {
        let e = point_on_circle(centre, FAN_REACH, end);
        assert!(
            e.x >= 0.0 && e.y >= 0.0 && e.x <= 453.9 && e.y <= 453.9,
            "{e:?}"
        );
    }
    assert!(!g.fan_fits(DialCorner::BottomRight));
    // The same for the others: the arc's extreme is on an axis, never on an end point.
    assert!(!g.fan_fits(DialCorner::TopLeft));
    assert!(!g.fan_fits(DialCorner::BottomLeft));
    assert!(!g.fan_fits(DialCorner::TopRight));
}

#[test]
fn only_the_axes_inside_the_span_count() {
    // The bottom-right fan does not reach 90 or 180 degrees, so the dial's room to the right and
    // below is just its own 74 px: a tall, wide area is enough, nothing else is asked.
    let g = DialGeometry::new(454.0, 454.0);
    let centre = g.centre(DialCorner::BottomRight);
    assert_eq!(centre, p(380.0, 380.0));
    assert!(g.fan_fits(DialCorner::BottomRight));
    // The fan of the top-left corner is the mirror image: it needs the same 454 px.
    assert!(g.fan_fits(DialCorner::TopLeft));
}
