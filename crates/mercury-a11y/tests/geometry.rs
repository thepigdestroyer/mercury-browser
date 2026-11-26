// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! Node bounds and scroll membership (`mercury_a11y::geometry`, EE10).

use accesskit::{Affine, Rect};
use mercury_a11y::geometry::{SpatialKind, aabb_of_transformed, scroll_applies, union_rects};

fn rect(x0: f64, y0: f64, x1: f64, y1: f64) -> Rect {
    Rect::new(x0, y0, x1, y1)
}

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-6
}

fn rotation(radians: f64) -> Affine {
    let (sin, cos) = radians.sin_cos();
    Affine::new([cos, sin, -sin, cos, 0.0, 0.0])
}

#[test]
fn union_of_two_rects() {
    let union = union_rects([rect(0.0, 0.0, 10.0, 10.0), rect(5.0, -5.0, 20.0, 8.0)]);
    assert_eq!(union, Some(rect(0.0, -5.0, 20.0, 10.0)));
}

#[test]
fn union_of_nothing_is_none() {
    assert_eq!(union_rects(std::iter::empty()), None);
}

#[test]
fn union_drops_nan_and_infinite() {
    let union = union_rects([
        rect(f64::NAN, 0.0, 10.0, 10.0),
        rect(0.0, 0.0, f64::INFINITY, 10.0),
        rect(2.0, 3.0, 4.0, 5.0),
    ]);
    assert_eq!(union, Some(rect(2.0, 3.0, 4.0, 5.0)));
    assert_eq!(union_rects([rect(f64::NAN, 0.0, 1.0, 1.0)]), None);
}

#[test]
fn aabb_of_a_90_degree_rotation_swaps_width_and_height() {
    let bounds = aabb_of_transformed(
        rect(0.0, 0.0, 10.0, 4.0),
        rotation(std::f64::consts::FRAC_PI_2),
    )
    .expect("finite rotation");
    assert!(
        close(bounds.x1 - bounds.x0, 4.0),
        "width {}",
        bounds.x1 - bounds.x0
    );
    assert!(
        close(bounds.y1 - bounds.y0, 10.0),
        "height {}",
        bounds.y1 - bounds.y0
    );
}

#[test]
fn aabb_of_a_45_degree_rotation_of_a_square_has_side_1_414() {
    let bounds = aabb_of_transformed(
        rect(0.0, 0.0, 1.0, 1.0),
        rotation(std::f64::consts::FRAC_PI_4),
    )
    .expect("finite rotation");
    let side = bounds.x1 - bounds.x0;
    assert!((side - 1.414).abs() < 1e-3, "side {side}");
    assert!(close(bounds.y1 - bounds.y0, side));
}

#[test]
fn aabb_with_translation() {
    let bounds = aabb_of_transformed(rect(0.0, 0.0, 10.0, 10.0), Affine::translate((5.0, 3.0)));
    assert_eq!(bounds, Some(rect(5.0, 3.0, 15.0, 13.0)));
}

#[test]
fn aabb_with_non_finite_transform_is_none() {
    let nan = Affine::new([f64::NAN, 0.0, 0.0, 1.0, 0.0, 0.0]);
    let infinite = Affine::new([1.0, 0.0, 0.0, 1.0, f64::INFINITY, 0.0]);
    assert_eq!(aabb_of_transformed(rect(0.0, 0.0, 1.0, 1.0), nan), None);
    assert_eq!(
        aabb_of_transformed(rect(0.0, 0.0, 1.0, 1.0), infinite),
        None
    );
}

#[test]
fn a_fixed_child_is_not_scrolled() {
    let chain = [SpatialKind::ReferenceFrame, SpatialKind::Other];
    assert!(!scroll_applies(&chain, 7));
}

#[test]
fn a_child_inside_the_scroller_is_scrolled() {
    let chain = [
        SpatialKind::Other,
        SpatialKind::Scroll(7),
        SpatialKind::Scroll(1),
    ];
    assert!(scroll_applies(&chain, 7));
    assert!(scroll_applies(&chain, 1));
}

#[test]
fn a_sticky_node_in_the_scroller_is_scrolled_by_its_scroller_only() {
    let chain = [SpatialKind::Sticky, SpatialKind::Scroll(7)];
    assert!(scroll_applies(&chain, 7));
    assert!(!scroll_applies(&chain, 1));
}
