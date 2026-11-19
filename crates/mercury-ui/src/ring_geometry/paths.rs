// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! SVG path strings for the ring's tab segments and group label bands (SPEC §4.9).
//!
//! Slint has no clickable arcs, so Rust writes the shapes and the Slint `Path` element only fills
//! them. This file is the one place in the crate that writes path text: if an arc-heavy path ever
//! misrenders in a renderer, `annulus_sector` is the single function to change (for example to
//! split the arcs into straight chords).
//!
//! # The contract the Slint side relies on
//!
//! - Every shape is a **closed, filled** annulus sector (a slice of a ring between two radii):
//!   `M <outer start> A <outer arc> L <inner end> A <inner arc> Z`. It is filled, not stroked, so
//!   it needs no stroke width, grows cleanly when a hovered segment is lifted (the outer radius
//!   grows) and has exact radial edges.
//! - Coordinates are **ring-local**: the origin is the top-left corner of the ring's square box,
//!   y grows downwards, exactly like `RingSpec::centre`. The Slint `Path` must be as big as that
//!   box and set `viewbox-width` and `viewbox-height` to the box size (no `viewbox-x/y`), so that
//!   the commands are not rescaled.
//! - An arc of more than 180 degrees is written as two `A` commands (an SVG arc is fixed by its
//!   end points and two flags, which is fragile close to and beyond half a turn). Every `A` is
//!   therefore a small arc (`large-arc` 0): the outer arcs sweep clockwise (`sweep` 1), the inner
//!   arcs counter-clockwise (`sweep` 0).
//! - Numbers have at most two decimals, no trailing zeros and are never `-0`. The output is a
//!   pure function of its input, so two calls give the same text.
//!
//! Angles follow the ring conventions (degrees clockwise from 12 o'clock, see the root module).

use super::{ArcSegment, Point, RingLayout, point_on_circle};

/// A number the way the path text writes it: two decimals at most, no trailing zeros, never `-0`.
/// A value that is not finite is written as `0` (the callers reject non-finite input before).
fn num(value: f32) -> String {
    if !value.is_finite() {
        return "0".to_owned();
    }
    let mut text = format!("{value:.2}");
    while text.ends_with('0') {
        text.pop();
    }
    if text.ends_with('.') {
        text.pop();
    }
    if text == "-0" {
        text = "0".to_owned();
    }
    text
}

/// Appends `x y` (two numbers separated by a space).
fn push_point(out: &mut String, point: Point) {
    out.push_str(&num(point.x));
    out.push(' ');
    out.push_str(&num(point.y));
}

/// Appends an arc command of radius `radius` that ends at `to` (always a small arc).
fn push_arc(out: &mut String, radius: f32, clockwise: bool, to: Point) {
    let radius = num(radius);
    out.push_str(" A");
    out.push_str(&radius);
    out.push(' ');
    out.push_str(&radius);
    out.push_str(if clockwise { " 0 0 1 " } else { " 0 0 0 " });
    push_point(out, to);
}

/// The closed, filled outline of a ring slice: `M <outer start> A <outer arc(s)> L <inner end>
/// A <inner arc(s)> Z`, around `centre`, between `r_in` and `r_out`, from `start_deg` clockwise to
/// `end_deg` (degrees clockwise from 12 o'clock; the values may exceed 360).
///
/// The sweep is clamped to `0..=360` degrees (an `end_deg` before `start_deg` gives an empty
/// slice, a sweep of 360 the whole ring) and `r_in` to `0..=r_out`. An arc of more than 180
/// degrees is written as two `A` commands, so every `A` is a small arc. Returns an empty string if
/// any argument is NaN or infinite (a `Path` with no commands draws nothing). See the module
/// documentation for the coordinate and `viewbox` contract.
pub fn annulus_sector(
    centre: Point,
    r_in: f32,
    r_out: f32,
    start_deg: f32,
    end_deg: f32,
) -> String {
    let finite = [centre.x, centre.y, r_in, r_out, start_deg, end_deg]
        .iter()
        .all(|value| value.is_finite());
    if !finite {
        return String::new();
    }
    let r_out = r_out.max(0.0);
    let r_in = r_in.clamp(0.0, r_out);
    let sweep = (end_deg - start_deg).clamp(0.0, 360.0);
    let end_deg = start_deg + sweep;
    let mid_deg = start_deg + sweep / 2.0;
    let split = sweep > 180.0;

    let at = |radius: f32, deg: f32| point_on_circle(centre, radius, deg);
    let mut out = String::with_capacity(112);
    out.push('M');
    push_point(&mut out, at(r_out, start_deg));
    if split {
        push_arc(&mut out, r_out, true, at(r_out, mid_deg));
    }
    push_arc(&mut out, r_out, true, at(r_out, end_deg));
    out.push_str(" L");
    push_point(&mut out, at(r_in, end_deg));
    if split {
        push_arc(&mut out, r_in, false, at(r_in, mid_deg));
    }
    push_arc(&mut out, r_in, false, at(r_in, start_deg));
    out.push_str(" Z");
    out
}

/// One shape of the layout: the outline is drawn out to `r_drawn_out`, but `mid` stays on the
/// centre line of the band `r_in..r_out` (so an anchor does not move when a segment is lifted).
fn shape(
    centre: Point,
    id: usize,
    (r_in, r_out): (f32, f32),
    r_drawn_out: f32,
    (start_deg, end_deg): (f32, f32),
) -> ArcSegment {
    ArcSegment {
        tab: id,
        commands: annulus_sector(centre, r_in, r_drawn_out, start_deg, end_deg),
        mid: point_on_circle(centre, (r_in + r_out) / 2.0, (start_deg + end_deg) / 2.0),
    }
}

/// One shape per tab, in flat order: `ArcSegment::tab` is the flat tab index, the outline is the
/// tab's drawn extent (`TabArc::start_deg..end_deg`, which is the slot without the gap) between
/// the band's inner and outer radius, and `mid` is on the band's centre line at the mid angle.
pub fn segments(layout: &RingLayout) -> Vec<ArcSegment> {
    let spec = &layout.spec;
    let radii = (spec.inner_radius(), spec.outer_radius());
    layout
        .tabs
        .iter()
        .map(|tab| {
            shape(
                spec.centre,
                tab.tab,
                radii,
                radii.1,
                (tab.start_deg, tab.end_deg),
            )
        })
        .collect()
}

/// One shape per group that has tabs (a group with none takes no angle and gets no band):
/// `ArcSegment::tab` is the **group** index, the outline is the group's label extent
/// (`GroupArc::label_start_deg..label_end_deg`) between `label_inner_radius` and
/// `label_outer_radius`, and `mid` is on the label band's centre line, where the label text sits.
pub fn label_bands(layout: &RingLayout) -> Vec<ArcSegment> {
    let spec = &layout.spec;
    let radii = (spec.label_inner_radius(), spec.label_outer_radius());
    layout
        .groups
        .iter()
        .filter(|group| group.tab_count > 0)
        .map(|group| {
            shape(
                spec.centre,
                group.group,
                radii,
                radii.1,
                (group.label_start_deg, group.label_end_deg),
            )
        })
        .collect()
}

/// The segment of `tab` (flat index) as it is drawn while hovered: the same shape with the outer
/// radius `lift` pixels larger (`mid` and the angles stay as they are). A negative or NaN `lift`
/// counts as 0. `None` for a tab the layout does not have.
pub fn lifted_segment(layout: &RingLayout, tab: usize, lift: f32) -> Option<ArcSegment> {
    let spec = &layout.spec;
    let arc = layout.tabs.get(tab)?;
    let radii = (spec.inner_radius(), spec.outer_radius());
    let lift = if lift.is_finite() { lift.max(0.0) } else { 0.0 };
    Some(shape(
        spec.centre,
        tab,
        radii,
        radii.1 + lift,
        (arc.start_deg, arc.end_deg),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ring_geometry::RingSpec;

    // ---- a small path parser, so the tests read the numbers back instead of trusting the text

    #[derive(Debug, PartialEq)]
    enum Cmd {
        Move(f64, f64),
        Arc {
            rx: f64,
            ry: f64,
            rotation: f64,
            large: u8,
            sweep: u8,
            x: f64,
            y: f64,
        },
        Line(f64, f64),
        Close,
    }

    fn number(tokens: &mut std::str::SplitWhitespace) -> f64 {
        tokens.next().unwrap().parse().unwrap()
    }

    fn parse(path: &str) -> Vec<Cmd> {
        // Put a space around every command letter so "M200 70A130" and "M200 70 A 130" both read.
        let spaced: String = path
            .chars()
            .flat_map(|c| {
                if c.is_ascii_alphabetic() {
                    vec![' ', c, ' ']
                } else {
                    vec![c]
                }
            })
            .collect();
        let mut tokens = spaced.split_whitespace();
        let mut cmds = Vec::new();
        while let Some(token) = tokens.next() {
            match token {
                "M" => cmds.push(Cmd::Move(number(&mut tokens), number(&mut tokens))),
                "L" => cmds.push(Cmd::Line(number(&mut tokens), number(&mut tokens))),
                "A" => cmds.push(Cmd::Arc {
                    rx: number(&mut tokens),
                    ry: number(&mut tokens),
                    rotation: number(&mut tokens),
                    large: number(&mut tokens) as u8,
                    sweep: number(&mut tokens) as u8,
                    x: number(&mut tokens),
                    y: number(&mut tokens),
                }),
                "Z" => cmds.push(Cmd::Close),
                other => panic!("unexpected token {other:?} in {path:?}"),
            }
        }
        cmds
    }

    /// Where `radius` and `deg` are around `centre`, computed in f64 on purpose (the code under
    /// test uses f32).
    fn expected(centre: Point, radius: f64, deg: f64) -> (f64, f64) {
        let rad = deg.to_radians();
        (
            f64::from(centre.x) + radius * rad.sin(),
            f64::from(centre.y) - radius * rad.cos(),
        )
    }

    fn assert_at(actual: (f64, f64), want: (f64, f64), what: &str) {
        assert!(
            (actual.0 - want.0).abs() < 0.02 && (actual.1 - want.1).abs() < 0.02,
            "{what}: got {actual:?}, want {want:?}"
        );
    }

    /// Checks the shape of the path text for one sector and that every end point lies on its
    /// circle at its angle (within 0.02 px).
    fn assert_sector(path: &str, centre: Point, r_in: f32, r_out: f32, start: f32, end: f32) {
        let cmds = parse(path);
        // The sweep is decided in f32, like the code under test: a sweep of "exactly" 180 degrees
        // must not depend on how the two f64 angles happen to round. A backwards extent (end
        // before start) is drawn as an empty slice at `start`, as `annulus_sector` documents.
        let sweep = (end - start).clamp(0.0, 360.0);
        let arcs = if sweep > 180.0 { 2 } else { 1 };
        let end = start + sweep;
        let (r_in, r_out) = (f64::from(r_in), f64::from(r_out));
        let (start, end) = (f64::from(start), f64::from(end));
        assert_eq!(cmds.len(), 2 * arcs + 3, "command count of {path}");

        let Cmd::Move(x, y) = cmds[0] else {
            panic!("a path starts with M: {path}")
        };
        assert_at((x, y), expected(centre, r_out, start), "outer start");
        for k in 0..arcs {
            let Cmd::Arc {
                rx,
                ry,
                rotation,
                large,
                sweep,
                x,
                y,
            } = cmds[1 + k]
            else {
                panic!("outer arc {k} is an A: {path}")
            };
            assert_eq!((rx, ry, rotation, large, sweep), (r_out, r_out, 0.0, 0, 1));
            let deg = start + (end - start) * (k + 1) as f64 / arcs as f64;
            assert_at((x, y), expected(centre, r_out, deg), "outer arc end");
        }
        let Cmd::Line(x, y) = cmds[1 + arcs] else {
            panic!("an L joins the arcs: {path}")
        };
        assert_at((x, y), expected(centre, r_in, end), "inner end");
        for k in 0..arcs {
            let Cmd::Arc {
                rx,
                ry,
                rotation,
                large,
                sweep,
                x,
                y,
            } = cmds[2 + arcs + k]
            else {
                panic!("inner arc {k} is an A: {path}")
            };
            assert_eq!((rx, ry, rotation, large, sweep), (r_in, r_in, 0.0, 0, 0));
            let deg = end - (end - start) * (k + 1) as f64 / arcs as f64;
            assert_at((x, y), expected(centre, r_in, deg), "inner arc end");
        }
        assert_eq!(cmds[2 + 2 * arcs], Cmd::Close);
    }

    // ---- annulus_sector

    #[test]
    fn a_quarter_ring_is_written_exactly() {
        // Hand-computed: theta 0 is straight up, theta 90 is to the right; radii 100 and 130
        // around (200, 200): out-start (200, 70), out-end (330, 200), in-end (300, 200), in-start
        // (200, 100).
        let path = annulus_sector(Point::new(200.0, 200.0), 100.0, 130.0, 0.0, 90.0);
        assert_eq!(
            path,
            "M200 70 A130 130 0 0 1 330 200 L300 200 A100 100 0 0 0 200 100 Z"
        );
    }

    #[test]
    fn a_sector_that_is_not_on_the_axes_is_written_exactly() {
        // theta 45..135 around (0, 0), radii 10 and 20: sin and cos of 45 degrees are 0.7071.
        let path = annulus_sector(Point::new(0.0, 0.0), 10.0, 20.0, 45.0, 135.0);
        assert_eq!(
            path,
            "M14.14 -14.14 A20 20 0 0 1 14.14 14.14 L7.07 7.07 A10 10 0 0 0 7.07 -7.07 Z"
        );
    }

    #[test]
    fn every_end_point_lies_on_its_circle_at_its_angle() {
        let centres = [Point::new(200.0, 200.0), Point::new(338.0, 342.0)];
        let radii = [(165.0, 195.0), (100.0, 130.0), (0.0, 50.0), (326.0, 354.0)];
        let starts = [0.0, 15.1, 89.645, 350.0, 400.5, -20.0];
        let sweeps = [
            0.0, 0.4, 12.709, 90.0, 179.9, 180.0, 180.1, 270.0, 355.0, 360.0,
        ];
        let mut checked = 0;
        for &centre in &centres {
            for &(r_in, r_out) in &radii {
                for &start in &starts {
                    for &sweep in &sweeps {
                        let end = start + sweep;
                        let path = annulus_sector(centre, r_in, r_out, start, end);
                        assert_sector(&path, centre, r_in, r_out, start, end);
                        checked += 1;
                    }
                }
            }
        }
        assert_eq!(checked, 2 * 4 * 6 * 10);
    }

    #[test]
    fn an_arc_of_more_than_half_a_turn_is_split_into_two() {
        let centre = Point::new(300.0, 300.0);
        let wide = annulus_sector(centre, 100.0, 130.0, 10.0, 200.0);
        assert_eq!(wide.matches('A').count(), 4, "two outer and two inner arcs");
        let half = annulus_sector(centre, 100.0, 130.0, 10.0, 190.0);
        assert_eq!(
            half.matches('A').count(),
            2,
            "180 degrees stay one arc per side"
        );
        let narrow = annulus_sector(centre, 100.0, 130.0, 10.0, 100.0);
        assert_eq!(narrow.matches('A').count(), 2);
    }

    #[test]
    fn a_355_degree_single_tab_ring_has_two_arcs_per_side() {
        // One tab, no reserve, one group gap of 3 and a tab gap of 2: the slot is 357 degrees and
        // the drawn segment 355.
        let spec = RingSpec {
            reserve_deg: 0.0,
            group_gap_deg: 3.0,
            tab_gap_deg: 2.0,
            ..RingSpec::default()
        };
        let layout = RingLayout::new(spec, &[1]);
        let tab = layout.tabs[0];
        assert!((tab.end_deg - tab.start_deg - 355.0).abs() < 0.01);
        let shapes = segments(&layout);
        assert_eq!(shapes.len(), 1);
        let path = &shapes[0].commands;
        assert_eq!(path.matches('A').count(), 4, "two arcs per side: {path}");
        assert_sector(
            path,
            spec.centre,
            spec.inner_radius(),
            spec.outer_radius(),
            tab.start_deg,
            tab.end_deg,
        );
        // The label band of the one group is nearly as long.
        let bands = label_bands(&layout);
        assert_eq!(bands.len(), 1);
        assert_eq!(bands[0].commands.matches('A').count(), 4);
    }

    #[test]
    fn a_full_turn_closes_on_its_start_point() {
        let centre = Point::new(100.0, 100.0);
        let path = annulus_sector(centre, 50.0, 80.0, 30.0, 390.0);
        assert_sector(&path, centre, 50.0, 80.0, 30.0, 390.0);
        let cmds = parse(&path);
        let (Cmd::Move(sx, sy), Cmd::Arc { x, y, .. }) = (&cmds[0], &cmds[2]) else {
            panic!("M then two A: {path}")
        };
        assert!((sx - x).abs() < 0.02 && (sy - y).abs() < 0.02);
    }

    #[test]
    fn odd_input_is_clamped_or_gives_nothing() {
        let centre = Point::new(50.0, 50.0);
        // A sweep that runs backwards is an empty slice, not a huge one.
        let backwards = annulus_sector(centre, 10.0, 20.0, 90.0, 10.0);
        assert_eq!(backwards.matches('A').count(), 2);
        let cmds = parse(&backwards);
        let (Cmd::Move(mx, my), Cmd::Arc { x, y, .. }) = (&cmds[0], &cmds[1]) else {
            panic!("M then A")
        };
        assert!((mx - x).abs() < 0.02 && (my - y).abs() < 0.02, "zero sweep");
        // A sweep over 360 is one turn.
        assert_eq!(
            annulus_sector(centre, 10.0, 20.0, 0.0, 720.0),
            annulus_sector(centre, 10.0, 20.0, 0.0, 360.0)
        );
        // An inner radius over the outer one, and a negative one, are clamped.
        assert_eq!(
            annulus_sector(centre, 30.0, 20.0, 0.0, 90.0),
            annulus_sector(centre, 20.0, 20.0, 0.0, 90.0)
        );
        assert_eq!(
            annulus_sector(centre, -5.0, 20.0, 0.0, 90.0),
            annulus_sector(centre, 0.0, 20.0, 0.0, 90.0)
        );
        // Non-finite input draws nothing.
        for bad in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            assert_eq!(annulus_sector(centre, 10.0, 20.0, bad, 90.0), "");
            assert_eq!(annulus_sector(centre, 10.0, 20.0, 0.0, bad), "");
            assert_eq!(annulus_sector(centre, bad, 20.0, 0.0, 90.0), "");
            assert_eq!(annulus_sector(centre, 10.0, bad, 0.0, 90.0), "");
            assert_eq!(
                annulus_sector(Point::new(bad, 0.0), 10.0, 20.0, 0.0, 90.0),
                ""
            );
            assert_eq!(
                annulus_sector(Point::new(0.0, bad), 10.0, 20.0, 0.0, 90.0),
                ""
            );
        }
    }

    #[test]
    fn the_same_input_gives_the_same_text() {
        let a = annulus_sector(Point::new(258.0, 258.0), 165.0, 195.0, 15.1, 27.8);
        let b = annulus_sector(Point::new(258.0, 258.0), 165.0, 195.0, 15.1, 27.8);
        assert_eq!(a, b);
    }

    // ---- number formatting

    #[test]
    fn numbers_have_at_most_two_decimals_and_no_trailing_zeros() {
        assert_eq!(num(0.0), "0");
        assert_eq!(num(1.0), "1");
        assert_eq!(num(100.0), "100");
        assert_eq!(num(1.5), "1.5");
        assert_eq!(num(5.1), "5.1");
        assert_eq!(num(7.14159), "7.14");
        assert_eq!(num(12.709), "12.71");
        assert_eq!(num(384.89), "384.89");
        assert_eq!(num(10.0 / 3.0), "3.33");
        assert_eq!(num(2.0 / 3.0), "0.67");
        assert_eq!(num(-3.0), "-3");
        assert_eq!(num(-12.349), "-12.35");
        assert_eq!(num(10_000_000.0), "10000000");
        assert_eq!(num(0.101), "0.1");
        assert_eq!(num(99.999), "100");
    }

    #[test]
    fn a_negative_zero_is_written_as_zero() {
        assert_eq!(num(-0.0), "0");
        assert_eq!(num(-0.004), "0");
        assert_eq!(num(-0.000_001), "0");
        assert_eq!(num(0.004), "0");
        // No coordinate of a whole path is "-0", whichever side of an axis a point lands on.
        for (start, end) in [(180.0, 360.0), (270.0, 450.0), (90.0, 270.0), (0.0, 180.0)] {
            let path = annulus_sector(Point::new(0.0, 0.0), 10.0, 20.0, start, end);
            let has_negative_zero = path
                .split(' ')
                .any(|token| token.trim_start_matches(['M', 'A', 'L']) == "-0");
            assert!(!has_negative_zero, "no -0 in {path}");
        }
    }

    #[test]
    fn non_finite_numbers_are_written_as_zero() {
        assert_eq!(num(f32::NAN), "0");
        assert_eq!(num(f32::INFINITY), "0");
        assert_eq!(num(f32::NEG_INFINITY), "0");
    }

    // ---- segments, label_bands, lifted_segment

    /// The hub ring of the mockup `ring-open`: 22 tabs in groups of 5, 4, 6, 3, 2 and 2, centred
    /// at (338, 342) in the SVG.
    fn mockup() -> RingLayout {
        let spec = RingSpec {
            centre: Point::new(338.0, 342.0),
            ..RingSpec::default()
        };
        RingLayout::new(spec, &[5, 4, 6, 3, 2, 2])
    }

    #[test]
    fn there_is_one_segment_per_tab_in_flat_order() {
        let layout = mockup();
        let shapes = segments(&layout);
        assert_eq!(shapes.len(), 22);
        let spec = layout.spec;
        for (i, (shape, arc)) in shapes.iter().zip(&layout.tabs).enumerate() {
            assert_eq!(shape.tab, i, "tab is the flat index");
            assert_sector(
                &shape.commands,
                spec.centre,
                spec.inner_radius(),
                spec.outer_radius(),
                arc.start_deg,
                arc.end_deg,
            );
            // `mid` is on the centre line of the band at the mid angle.
            let want = expected(
                spec.centre,
                f64::from(spec.radius),
                f64::from(arc.start_deg + arc.end_deg) / 2.0,
            );
            assert_at(
                (shape.mid.x.into(), shape.mid.y.into()),
                want,
                "segment mid",
            );
        }
    }

    #[test]
    fn the_first_mockup_segment_starts_at_the_published_angle() {
        // Tab 0 starts at theta 15.1 (see the layout test): its outer start point is on the
        // radius 195 circle at that angle.
        let layout = mockup();
        let shapes = segments(&layout);
        let Cmd::Move(x, y) = parse(&shapes[0].commands)[0] else {
            panic!("a path starts with M")
        };
        let want = expected(Point::new(338.0, 342.0), 195.0, 15.1);
        assert_at((x, y), want, "tab 0 start");
    }

    #[test]
    fn label_bands_are_one_per_group_with_the_group_index() {
        let layout = mockup();
        let bands = label_bands(&layout);
        assert_eq!(bands.len(), 6);
        let spec = layout.spec;
        for (g, (band, arc)) in bands.iter().zip(&layout.groups).enumerate() {
            assert_eq!(band.tab, g, "tab is the group index");
            assert_sector(
                &band.commands,
                spec.centre,
                spec.label_inner_radius(),
                spec.label_outer_radius(),
                arc.label_start_deg,
                arc.label_end_deg,
            );
            let want = expected(
                spec.centre,
                f64::from(spec.label_radius()),
                f64::from(arc.label_start_deg + arc.label_end_deg) / 2.0,
            );
            assert_at((band.mid.x.into(), band.mid.y.into()), want, "band mid");
        }
    }

    #[test]
    fn a_group_with_no_tabs_gets_no_band_but_keeps_its_index() {
        let layout = RingLayout::new(RingSpec::default(), &[3, 0, 2, 0]);
        let bands = label_bands(&layout);
        assert_eq!(
            bands.iter().map(|band| band.tab).collect::<Vec<_>>(),
            vec![0, 2]
        );
        assert_eq!(segments(&layout).len(), 5);
    }

    #[test]
    fn no_tabs_give_no_shapes() {
        for sizes in [&[][..], &[0][..], &[0, 0, 0][..]] {
            let layout = RingLayout::new(RingSpec::default(), sizes);
            assert!(segments(&layout).is_empty(), "segments of {sizes:?}");
            assert!(label_bands(&layout).is_empty(), "bands of {sizes:?}");
            assert!(lifted_segment(&layout, 0, 10.0).is_none());
        }
    }

    #[test]
    fn sixty_four_groups_give_valid_shapes_for_each() {
        for tabs_per_group in [1, 3] {
            let sizes = vec![tabs_per_group; 64];
            let layout = RingLayout::new(RingSpec::default(), &sizes);
            let spec = layout.spec;
            let shapes = segments(&layout);
            let bands = label_bands(&layout);
            assert_eq!(shapes.len(), 64 * tabs_per_group);
            assert_eq!(bands.len(), 64);
            for (shape, arc) in shapes.iter().zip(&layout.tabs) {
                assert!(shape.mid.x.is_finite() && shape.mid.y.is_finite());
                assert_sector(
                    &shape.commands,
                    spec.centre,
                    spec.inner_radius(),
                    spec.outer_radius(),
                    arc.start_deg,
                    arc.end_deg,
                );
            }
            for (band, arc) in bands.iter().zip(&layout.groups) {
                assert_sector(
                    &band.commands,
                    spec.centre,
                    spec.label_inner_radius(),
                    spec.label_outer_radius(),
                    arc.label_start_deg,
                    arc.label_end_deg,
                );
            }
            // Deterministic: a second layout and a second call give the same text.
            let again = segments(&RingLayout::new(RingSpec::default(), &sizes));
            assert_eq!(shapes, again);
        }
    }

    #[test]
    fn a_lifted_segment_grows_outward_only() {
        let layout = mockup();
        let spec = layout.spec;
        let plain = &segments(&layout)[7];
        let lifted = lifted_segment(&layout, 7, 10.0).expect("tab 7 exists");
        assert_eq!(lifted.tab, 7);
        assert_ne!(lifted.commands, plain.commands);
        // The outer radius is 10 px larger; the inner one and the angles are unchanged.
        let arc = layout.tabs[7];
        assert_sector(
            &lifted.commands,
            spec.centre,
            spec.inner_radius(),
            spec.outer_radius() + 10.0,
            arc.start_deg,
            arc.end_deg,
        );
        // The anchor does not move when the segment is lifted.
        assert_eq!(lifted.mid, plain.mid);
    }

    #[test]
    fn a_lift_of_zero_or_less_changes_nothing_and_an_unknown_tab_has_no_shape() {
        let layout = mockup();
        let plain = segments(&layout)[3].clone();
        assert_eq!(lifted_segment(&layout, 3, 0.0), Some(plain.clone()));
        assert_eq!(lifted_segment(&layout, 3, -4.0), Some(plain.clone()));
        assert_eq!(lifted_segment(&layout, 3, f32::NAN), Some(plain));
        assert!(lifted_segment(&layout, 22, 10.0).is_none());
        assert!(lifted_segment(&layout, usize::MAX, 10.0).is_none());
    }
}
