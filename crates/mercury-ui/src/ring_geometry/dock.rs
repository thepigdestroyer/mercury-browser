// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! Magnification of the group dock (SPEC §4.4): the pill under the pointer grows to 1.45, its two
//! neighbours to 1.12, the others stay at 1.0, and the pills are laid out with their scaled
//! widths. Pure functions of the pointer position and the pill widths; no window, no colours.
//!
//! # The one rule that keeps the dock steady
//!
//! `dock_scales` must be given the centres of the **unscaled** layout (`unscaled_centres`), and
//! the pointer x in the same coordinates (origin = the left edge of the resting layout). The
//! magnified dock is wider, so its pills sit at other places than the resting ones; if their
//! centres were fed back, the pill nearest to a pointer that stands still would change from
//! frame to frame (the dock "grows away" from the pointer), and the same x would give different
//! answers depending on where the pointer came from. With the resting centres the scales are a
//! pure function of x: the magnified pill is always the one nearest in the resting layout and
//! moves one pill at a time as the pointer moves. `magnified_layout` does the whole frame in
//! the right order so that a caller cannot get this wrong; the tests pin both the rule and the
//! failure it avoids.
//!
//! A centred dock grows around its middle, so a caller that measures the pointer in the window
//! subtracts the resting layout's left edge, `(dock_width - rest_width) / 2` with
//! `rest_width = dock_layout(widths, &[], gap).total_width`, before asking.
//!
//! The dock grows upward only: its height is `base_height * max_scale`.

/// Scale of the pill under the pointer.
pub const UNDER_POINTER: f32 = 1.45;
/// Scale of the two pills next to the one under the pointer.
pub const NEIGHBOUR: f32 = 1.12;
/// Space between two pills. The gap itself is not scaled.
pub const PILL_GAP: f32 = 10.0;

/// The scale of every pill for a pointer at `x`, in the coordinates of the **unscaled** layout.
///
/// `centres[i]` is the centre of pill `i` with every scale at 1.0 (`unscaled_centres`). The pill
/// whose centre is nearest to `x` (ties go to the lower index) gets `UNDER_POINTER`, its two
/// neighbours by index get `NEIGHBOUR`, the rest 1.0. `None` (the pointer is not over the dock),
/// a NaN or infinite `x`, no pills, or no pill with a finite centre give 1.0 for every pill.
/// Pills with a NaN or infinite centre are never the nearest.
///
/// Passing the centres of an already magnified layout is a bug, see the module documentation.
pub fn dock_scales(x: Option<f32>, centres: &[f32]) -> Vec<f32> {
    let mut scales = vec![1.0; centres.len()];
    let Some(x) = x.filter(|x| x.is_finite()) else {
        return scales;
    };
    let mut nearest: Option<(usize, f32)> = None;
    for (i, &centre) in centres.iter().enumerate() {
        if !centre.is_finite() {
            continue;
        }
        let distance = (centre - x).abs();
        // Strictly nearer only: an equal distance keeps the lower index.
        if nearest.is_none_or(|(_, best)| distance < best) {
            nearest = Some((i, distance));
        }
    }
    let Some((under, _)) = nearest else {
        return scales;
    };
    if under > 0 {
        scales[under - 1] = NEIGHBOUR;
    }
    if under + 1 < scales.len() {
        scales[under + 1] = NEIGHBOUR;
    }
    scales[under] = UNDER_POINTER;
    scales
}

/// The centres of the pills laid out at scale 1.0 from x = 0, `gap` apart: what `dock_scales`
/// has to be given. A negative, NaN or infinite width counts as 0 and so does a bad `gap`.
pub fn unscaled_centres(widths: &[f32], gap: f32) -> Vec<f32> {
    let resting = dock_layout(widths, &[], gap);
    resting
        .xs
        .iter()
        .zip(&resting.widths)
        .map(|(x, width)| x + width / 2.0)
        .collect()
}

/// Where the pills of a magnified dock sit.
#[derive(Clone, Debug, PartialEq)]
pub struct DockLayout {
    /// Left edge of every pill, from 0.
    pub xs: Vec<f32>,
    /// The scaled width of every pill.
    pub widths: Vec<f32>,
    /// From the left edge of the first pill to the right edge of the last (no trailing gap);
    /// 0 for no pills.
    pub total_width: f32,
    /// The largest scale in use (1.0 for no pills): the dock grows upward to
    /// `base_height * max_scale`.
    pub max_scale: f32,
}

/// Lays the pills out left to right with their scaled widths: `x_i` is the sum of
/// `w_j * s_j + gap` for `j < i`.
///
/// `scales` shorter than `widths` leaves the missing pills at 1.0, extra scales are ignored. A
/// NaN, infinite or negative scale counts as 1.0, a NaN, infinite or negative width or gap as 0
/// (nothing here can produce NaN or a negative width).
pub fn dock_layout(widths: &[f32], scales: &[f32], gap: f32) -> DockLayout {
    let gap = non_negative(gap);
    let mut xs = Vec::with_capacity(widths.len());
    let mut scaled = Vec::with_capacity(widths.len());
    let mut max_scale = None::<f32>;
    let mut x = 0.0_f32;
    for (i, &width) in widths.iter().enumerate() {
        let scale = scales
            .get(i)
            .copied()
            .filter(|scale| scale.is_finite() && *scale >= 0.0)
            .unwrap_or(1.0);
        let width = non_negative(width) * scale;
        xs.push(x);
        scaled.push(width);
        x += width + gap;
        max_scale = Some(max_scale.map_or(scale, |largest| largest.max(scale)));
    }
    let total_width = match (xs.last(), scaled.last()) {
        (Some(x), Some(width)) => x + width,
        _ => 0.0,
    };
    DockLayout {
        xs,
        widths: scaled,
        total_width,
        max_scale: max_scale.unwrap_or(1.0),
    }
}

/// One whole frame in the right order: the scales for a pointer at `pointer_x` (in resting
/// coordinates, `None` when it is off the dock) and the layout drawn with them. The centres
/// come from the resting layout, so the result depends on the pointer and the widths only.
pub fn magnified_layout(
    pointer_x: Option<f32>,
    widths: &[f32],
    gap: f32,
) -> (Vec<f32>, DockLayout) {
    let scales = dock_scales(pointer_x, &unscaled_centres(widths, gap));
    let layout = dock_layout(widths, &scales, gap);
    (scales, layout)
}

fn non_negative(value: f32) -> f32 {
    if value.is_finite() {
        value.max(0.0)
    } else {
        0.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const EPS: f32 = 1e-3;

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() <= EPS
    }

    fn assert_scales(actual: &[f32], expected: &[f32]) {
        assert_eq!(actual.len(), expected.len(), "{actual:?} vs {expected:?}");
        for (a, e) in actual.iter().zip(expected) {
            assert!(close(*a, *e), "{actual:?} vs {expected:?}");
        }
    }

    /// The index of the largest scale (the first one on a tie).
    fn magnified(scales: &[f32]) -> usize {
        let mut best = 0;
        for (i, scale) in scales.iter().enumerate() {
            if *scale > scales[best] {
                best = i;
            }
        }
        best
    }

    /// Index of the pill whose centre is nearest to `x` (the first one on a tie).
    fn nearest(centres: &[f32], x: f32) -> usize {
        let mut best = 0;
        for (i, centre) in centres.iter().enumerate() {
            if (centre - x).abs() < (centres[best] - x).abs() {
                best = i;
            }
        }
        best
    }

    /// The centres of the dock as it is drawn now, with its scales applied.
    fn drawn_centres(widths: &[f32], scales: &[f32]) -> Vec<f32> {
        let drawn = dock_layout(widths, scales, PILL_GAP);
        drawn
            .xs
            .iter()
            .zip(&drawn.widths)
            .map(|(x, width)| x + width / 2.0)
            .collect()
    }

    /// Mockup row: five pills of different widths.
    const MOCKUP_WIDTHS: [f32; 5] = [96.0, 110.0, 88.0, 104.0, 92.0];

    // ---- dock_scales ----

    #[test]
    fn the_constants_are_the_mockups() {
        assert_eq!(UNDER_POINTER, 1.45);
        assert_eq!(NEIGHBOUR, 1.12);
        assert_eq!(PILL_GAP, 10.0);
    }

    #[test]
    fn the_mockup_row_with_the_pointer_on_the_third_pill() {
        let centres = unscaled_centres(&MOCKUP_WIDTHS, PILL_GAP);
        let scales = dock_scales(Some(centres[2]), &centres);
        assert_scales(&scales, &[1.0, 1.12, 1.45, 1.12, 1.0]);
    }

    #[test]
    fn no_pills_give_no_scales() {
        assert!(dock_scales(Some(10.0), &[]).is_empty());
        assert!(dock_scales(None, &[]).is_empty());
    }

    #[test]
    fn one_pill_is_under_the_pointer_wherever_it_is() {
        for x in [-500.0, 0.0, 50.0, 1.0e6] {
            assert_scales(&dock_scales(Some(x), &[50.0]), &[UNDER_POINTER]);
        }
    }

    #[test]
    fn two_pills_the_nearer_one_is_magnified_and_the_other_is_its_neighbour() {
        let centres = [50.0, 160.0];
        assert_scales(&dock_scales(Some(60.0), &centres), &[1.45, 1.12]);
        assert_scales(&dock_scales(Some(150.0), &centres), &[1.12, 1.45]);
    }

    #[test]
    fn a_tie_goes_to_the_lower_index() {
        assert_scales(&dock_scales(Some(50.0), &[0.0, 100.0]), &[1.45, 1.12]);
        // Just past the middle the other pill wins.
        assert_scales(&dock_scales(Some(50.001), &[0.0, 100.0]), &[1.12, 1.45]);
        // Three pills, pointer exactly between the second and the third.
        assert_scales(
            &dock_scales(Some(150.0), &[0.0, 100.0, 200.0]),
            &[1.12, 1.45, 1.12],
        );
    }

    #[test]
    fn the_ends_have_one_neighbour() {
        let centres = [50.0, 160.0, 270.0];
        assert_scales(&dock_scales(Some(-40.0), &centres), &[1.45, 1.12, 1.0]);
        assert_scales(&dock_scales(Some(900.0), &centres), &[1.0, 1.12, 1.45]);
    }

    #[test]
    fn no_pointer_or_a_bad_pointer_leaves_every_pill_at_one() {
        let centres = unscaled_centres(&MOCKUP_WIDTHS, PILL_GAP);
        for x in [
            None,
            Some(f32::NAN),
            Some(f32::INFINITY),
            Some(f32::NEG_INFINITY),
        ] {
            assert_scales(&dock_scales(x, &centres), &[1.0; 5]);
        }
    }

    #[test]
    fn pills_without_a_finite_centre_are_never_magnified() {
        assert_scales(&dock_scales(Some(0.0), &[f32::NAN, 100.0]), &[1.12, 1.45]);
        assert_scales(
            &dock_scales(Some(0.0), &[f32::INFINITY, f32::NAN]),
            &[1.0, 1.0],
        );
    }

    #[test]
    fn sixty_four_pills_magnify_exactly_three() {
        let widths = [80.0; 64];
        let centres = unscaled_centres(&widths, PILL_GAP);
        assert_eq!(centres.len(), 64);
        let scales = dock_scales(Some(centres[31] + 3.0), &centres);
        assert_eq!(scales.len(), 64);
        for (i, scale) in scales.iter().enumerate() {
            let expected = match i {
                31 => UNDER_POINTER,
                30 | 32 => NEIGHBOUR,
                _ => 1.0,
            };
            assert!(close(*scale, expected), "pill {i}: {scale}");
        }
    }

    // ---- unscaled_centres ----

    #[test]
    fn unscaled_centres_follow_the_widths_and_the_gap() {
        assert_scales(
            &unscaled_centres(&[100.0, 50.0, 80.0], 10.0),
            &[50.0, 135.0, 210.0],
        );
        assert!(unscaled_centres(&[], 10.0).is_empty());
        assert_scales(&unscaled_centres(&[40.0], 10.0), &[20.0]);
    }

    #[test]
    fn unscaled_centres_cope_with_bad_widths() {
        let centres = unscaled_centres(&[f32::NAN, -5.0, 20.0], 10.0);
        assert!(centres.iter().all(|c| c.is_finite()));
        assert_scales(&centres, &[0.0, 10.0, 30.0]);
    }

    // ---- dock_layout ----

    #[test]
    fn an_empty_dock_is_empty() {
        let layout = dock_layout(&[], &[], PILL_GAP);
        assert!(layout.xs.is_empty());
        assert!(layout.widths.is_empty());
        assert_eq!(layout.total_width, 0.0);
        assert_eq!(layout.max_scale, 1.0);
    }

    #[test]
    fn one_pill_has_no_trailing_gap() {
        let layout = dock_layout(&[100.0], &[1.45], PILL_GAP);
        assert_eq!(layout.xs, vec![0.0]);
        assert!(close(layout.widths[0], 145.0));
        assert!(close(layout.total_width, 145.0));
        assert!(close(layout.max_scale, 1.45));
    }

    #[test]
    fn two_pills_the_second_starts_after_the_scaled_first_and_the_gap() {
        let layout = dock_layout(&[100.0, 50.0], &[1.0, 2.0], 10.0);
        assert_scales(&layout.xs, &[0.0, 110.0]);
        assert_scales(&layout.widths, &[100.0, 100.0]);
        assert!(close(layout.total_width, 210.0));
        assert!(close(layout.max_scale, 2.0));
    }

    #[test]
    fn the_mockup_row_laid_out_with_the_pointer_on_the_third_pill() {
        let scales = [1.0, 1.12, 1.45, 1.12, 1.0];
        let layout = dock_layout(&MOCKUP_WIDTHS, &scales, PILL_GAP);
        let mut x = 0.0;
        for i in 0..5 {
            assert!(close(layout.xs[i], x), "pill {i}");
            assert!(
                close(layout.widths[i], MOCKUP_WIDTHS[i] * scales[i]),
                "pill {i}"
            );
            x += MOCKUP_WIDTHS[i] * scales[i] + PILL_GAP;
        }
        assert!(close(layout.total_width, x - PILL_GAP));
        assert!(close(layout.max_scale, 1.45));
        // The middle pill grew, so the dock is wider than at rest.
        let resting = dock_layout(&MOCKUP_WIDTHS, &[], PILL_GAP);
        assert!(layout.total_width > resting.total_width);
        assert_eq!(resting.max_scale, 1.0);
    }

    #[test]
    fn six_pills_never_overlap_and_keep_the_gap() {
        let widths = [90.0, 120.0, 70.0, 100.0, 110.0, 85.0];
        let scales = [1.0, 1.0, 1.12, 1.45, 1.12, 1.0];
        let layout = dock_layout(&widths, &scales, PILL_GAP);
        for i in 1..6 {
            assert!(close(
                layout.xs[i],
                layout.xs[i - 1] + layout.widths[i - 1] + PILL_GAP
            ));
        }
        assert!(close(layout.total_width, layout.xs[5] + layout.widths[5]));
    }

    #[test]
    fn sixty_four_pills_lay_out_in_order() {
        let widths: Vec<f32> = (0..64).map(|i| 60.0 + (i % 7) as f32 * 5.0).collect();
        let centres = unscaled_centres(&widths, PILL_GAP);
        let scales = dock_scales(Some(centres[40]), &centres);
        let layout = dock_layout(&widths, &scales, PILL_GAP);
        assert_eq!(layout.xs.len(), 64);
        assert_eq!(layout.widths.len(), 64);
        for i in 1..64 {
            assert!(layout.xs[i] > layout.xs[i - 1]);
            assert!(close(
                layout.xs[i],
                layout.xs[i - 1] + layout.widths[i - 1] + PILL_GAP
            ));
        }
        let sum: f32 = layout.widths.iter().sum();
        assert!((layout.total_width - (sum + 63.0 * PILL_GAP)).abs() < 0.05);
        assert!(close(layout.max_scale, UNDER_POINTER));
        assert!(layout.widths.iter().all(|w| w.is_finite() && *w > 0.0));
    }

    #[test]
    fn missing_scales_count_as_one_and_extra_scales_are_ignored() {
        let short = dock_layout(&[100.0, 100.0, 100.0], &[2.0], 10.0);
        assert_scales(&short.widths, &[200.0, 100.0, 100.0]);
        assert_scales(&short.xs, &[0.0, 210.0, 320.0]);
        let none = dock_layout(&[100.0, 100.0], &[], 10.0);
        assert_scales(&none.widths, &[100.0, 100.0]);
        let long = dock_layout(&[100.0], &[1.0, 9.0, 9.0], 10.0);
        assert_scales(&long.widths, &[100.0]);
        assert!(
            close(long.max_scale, 1.0),
            "scales of pills that do not exist are not used"
        );
    }

    #[test]
    fn bad_numbers_never_make_nan_or_negative_widths() {
        let layout = dock_layout(
            &[f32::NAN, -20.0, f32::INFINITY, 40.0],
            &[f32::NAN, 2.0, 1.0, -1.0],
            f32::NAN,
        );
        for value in layout.xs.iter().chain(&layout.widths) {
            assert!(value.is_finite() && *value >= 0.0, "{layout:?}");
        }
        assert!(layout.total_width.is_finite() && layout.total_width >= 0.0);
        assert!(layout.max_scale.is_finite());
        // The negative scale counted as 1.0.
        assert!(close(layout.widths[3], 40.0));
    }

    // ---- why dock_scales wants the unscaled centres ----

    /// Six pills of 100 px: resting centres at 50, 160, 270, 380, 490, 600.
    const SIX: [f32; 6] = [100.0; 6];

    /// The wrong loop: each frame asks `dock_scales` about the centres of the dock as it was
    /// drawn in the frame before.
    fn settle_with_scaled_centres(x: f32, before: Vec<f32>, frames: usize) -> Vec<f32> {
        let mut scales = before;
        for _ in 0..frames {
            scales = dock_scales(Some(x), &drawn_centres(&SIX, &scales));
        }
        scales
    }

    /// The right loop: the centres are always the resting ones.
    fn settle_with_unscaled_centres(x: f32, before: Vec<f32>, frames: usize) -> Vec<f32> {
        let rest = unscaled_centres(&SIX, PILL_GAP);
        let mut scales = before;
        for _ in 0..frames {
            scales = dock_scales(Some(x), &rest);
        }
        scales
    }

    #[test]
    fn a_still_pointer_gets_the_same_answer_whatever_the_dock_looked_like_before() {
        // x = 250 is nearer to the resting centre of pill 2 (270) than of pill 1 (160).
        let x = 250.0;
        let rest = unscaled_centres(&SIX, PILL_GAP);
        let from_rest = vec![1.0; 6];
        let from_pill_one = dock_scales(Some(200.0), &rest);
        let from_pill_three = dock_scales(Some(380.0), &rest);
        assert_eq!(magnified(&from_pill_one), 1);
        assert_eq!(magnified(&from_pill_three), 3);
        for before in [from_rest, from_pill_one, from_pill_three] {
            let settled = settle_with_unscaled_centres(x, before, 8);
            assert_eq!(magnified(&settled), 2);
        }
    }

    #[test]
    fn feeding_scaled_centres_back_makes_the_answer_depend_on_the_history() {
        // The same pointer x, two different pills: this is what the unscaled rule prevents.
        let x = 250.0;
        let rest = unscaled_centres(&SIX, PILL_GAP);
        let coming_from_the_left = dock_scales(Some(200.0), &rest);
        let coming_from_the_right = dock_scales(Some(300.0), &rest);
        let left = settle_with_scaled_centres(x, coming_from_the_left, 8);
        let right = settle_with_scaled_centres(x, coming_from_the_right, 8);
        assert_eq!(magnified(&left), 1);
        assert_eq!(magnified(&right), 2);
        assert_ne!(left, right);
    }

    #[test]
    fn feeding_scaled_centres_back_moves_the_magnified_pill_with_the_pointer_standing_still() {
        // At x = 220 the resting layout says pill 2 (270 is 50 away, 160 is 60 away). The dock
        // grows, pill 1's drawn centre moves to 166 and pill 2's to 304.5, and the pointer is
        // now "over" pill 1 without having moved.
        let x = 220.0;
        let rest = unscaled_centres(&SIX, PILL_GAP);
        let first = dock_scales(Some(x), &rest);
        assert_eq!(magnified(&first), 2);
        let second = dock_scales(Some(x), &drawn_centres(&SIX, &first));
        assert_eq!(magnified(&second), 1, "the loop moved by itself");
    }

    #[test]
    fn magnified_layout_always_picks_the_pill_nearest_in_the_resting_layout() {
        // This is the test that fails if `magnified_layout` ever asks about scaled centres.
        let rest = unscaled_centres(&SIX, PILL_GAP);
        let mut x = -20.0_f32;
        while x <= 680.0 {
            let (scales, layout) = magnified_layout(Some(x), &SIX, PILL_GAP);
            assert_eq!(magnified(&scales), nearest(&rest, x), "x = {x}");
            assert_eq!(layout, dock_layout(&SIX, &scales, PILL_GAP));
            x += 0.5;
        }
    }

    #[test]
    fn a_still_pointer_gives_identical_frames() {
        let mut previous = None;
        for _frame in 0..10 {
            let frame = magnified_layout(Some(250.0), &SIX, PILL_GAP);
            if let Some(before) = &previous {
                assert_eq!(&frame, before);
            }
            previous = Some(frame);
        }
    }

    #[test]
    fn moving_the_pointer_one_pixel_moves_the_magnified_pill_by_at_most_one() {
        let mut last = 0_usize;
        let mut x = -10.0_f32;
        while x <= 650.0 {
            let (scales, _) = magnified_layout(Some(x), &SIX, PILL_GAP);
            let now = magnified(&scales);
            assert!(now >= last && now - last <= 1, "x = {x}: {last} -> {now}");
            last = now;
            x += 1.0;
        }
        assert_eq!(last, 5);
    }

    #[test]
    fn magnified_layout_without_a_pointer_is_the_resting_dock() {
        let (scales, layout) = magnified_layout(None, &MOCKUP_WIDTHS, PILL_GAP);
        assert_scales(&scales, &[1.0; 5]);
        assert_eq!(layout, dock_layout(&MOCKUP_WIDTHS, &[], PILL_GAP));
        let (scales, layout) = magnified_layout(Some(1.0), &[], PILL_GAP);
        assert!(scales.is_empty());
        assert_eq!(layout.total_width, 0.0);
    }
}
