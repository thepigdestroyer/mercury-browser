// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! The group dock's data (SPEC §4.4, brief D2 D4 D8 D13): the resting layout, the fit tiers, the
//! magnified frame, the hit test, the tooltip and the persistent models. No window: rows come from
//! `ring_bands::fixtures`, text from `ApproxMetrics`.

use std::cell::{Cell, RefCell};
use std::pin::Pin;
use std::time::Instant;

use mercury_ui::group_dock::{
    BOTTOM_MARGIN, DockData, DockFrame, DockModels, DockOptions, dock_height,
};
use mercury_ui::ring_bands::fixtures::{generated, mockup_groups, mockup_tabs};
use mercury_ui::ring_geometry::{ApproxMetrics, magnified_layout};
use mercury_ui::{DockHit, DockHitKind, DockPill, GroupRow, TabRow};
use slint::Model;
use slint::private_unstable_api::re_exports::{ModelChangeListener, ModelChangeListenerContainer};

const CONTAINER: f32 = 1440.0;
const BUTTON: f32 = 26.0;
const TRAILING: f32 = 56.0;
const BAR_PAD: f32 = 10.0;

fn options(container: f32) -> DockOptions {
    DockOptions {
        container_width: container,
        ..DockOptions::default()
    }
}

fn mockup(container: f32) -> DockData {
    DockData::new(
        &mockup_groups(false),
        &mockup_tabs(false),
        &options(container),
        &ApproxMetrics,
    )
}

fn six_groups(container: f32, opts: DockOptions) -> DockData {
    let (groups, tabs) = generated(6, 12, false);
    DockData::new(
        &groups,
        &tabs,
        &DockOptions {
            container_width: container,
            ..opts
        },
        &ApproxMetrics,
    )
}

fn crowded(container: f32) -> DockData {
    let (groups, tabs) = generated(64, 192, false);
    DockData::new(&groups, &tabs, &options(container), &ApproxMetrics)
}

/// The vertical middle of the bar in `GroupDock`'s box.
fn mid_y(frame: &DockFrame) -> f32 {
    DockData::container_height() - BOTTOM_MARGIN - frame.dock_height / 2.0
}

fn close(a: f32, b: f32) -> bool {
    (a - b).abs() < 0.01
}

fn magnified(scales: &[f32]) -> Option<usize> {
    scales.iter().position(|scale| close(*scale, 1.45))
}

/// The index of the largest scale, if any is above 1.0 (the pill that grows most, also within the
/// 24 px around a boundary where two layouts are blended).
fn largest(scales: &[f32]) -> Option<usize> {
    let (index, scale) = scales
        .iter()
        .enumerate()
        .fold((0, 1.0_f32), |best, (i, scale)| {
            if *scale > best.1 + 0.01 {
                (i, *scale)
            } else {
                best
            }
        });
    (scale > 1.01).then_some(index)
}

fn chips(pill: &DockPill) -> Vec<(i32, f32, f32)> {
    pill.chips.iter().map(|c| (c.tab, c.x, c.width)).collect()
}

fn slivers(pill: &DockPill) -> Vec<(i32, f32, f32)> {
    pill.slivers.iter().map(|s| (s.tab, s.x, s.width)).collect()
}

/// Every number of a frame, to compare two frames (a `DockPill` compares its models by identity).
fn numbers(frame: &DockFrame) -> Vec<f32> {
    let mut all = vec![
        frame.dock_x,
        frame.dock_width,
        frame.dock_height,
        frame.max_scale,
        frame.new_tab_x,
        frame.shrink_x,
    ];
    all.extend(&frame.scales);
    for pill in &frame.pills {
        all.extend([pill.x, pill.width, pill.height, pill.scale]);
        for (tab, x, width) in chips(pill).into_iter().chain(slivers(pill)) {
            all.extend([tab as f32, x, width]);
        }
    }
    all
}

fn is_sane(frame: &DockFrame) -> bool {
    let finite = numbers(frame).iter().all(|n| n.is_finite());
    let widths = frame
        .pills
        .iter()
        .all(|p| p.width >= 0.0 && p.height >= 0.0);
    finite && widths && frame.dock_x >= 0.0 && frame.dock_width >= 0.0
}

// ---- the rows

#[test]
fn the_mockup_dock_has_six_pills_and_the_current_group_is_expanded() {
    let data = mockup(CONTAINER);
    let frame = data.frame(None);
    assert_eq!(frame.pills.len(), 6);
    assert_eq!(frame.scales, vec![1.0; 6]);
    let current = &frame.pills[0];
    assert!(current.expanded);
    assert_eq!(current.chips.row_count(), 5);
    assert_eq!(current.slivers.row_count(), 0);
    let widest_folded = frame.pills[1..].iter().map(|p| p.width).fold(0.0, f32::max);
    assert!(
        current.width >= widest_folded,
        "{} vs {widest_folded}",
        current.width
    );
    let tab_counts = [4, 6, 3, 2, 2];
    for (pill, count) in frame.pills[1..].iter().zip(tab_counts) {
        assert!(!pill.expanded);
        assert_eq!(pill.chips.row_count(), 0);
        assert_eq!(pill.slivers.row_count(), count, "{}", pill.group.name);
        assert!(pill.width > 0.0);
    }
    assert_eq!(data.current_group(), Some(1));
}

#[test]
fn the_resting_bar_is_centred_and_is_the_pills_the_gaps_and_the_trailing_block() {
    for container in [1440.0, 1000.0, 1700.0] {
        let data = mockup(container);
        let frame = data.frame(None);
        assert!(
            close(frame.dock_x * 2.0 + frame.dock_width, container),
            "{container}"
        );
        let gap = frame.pills[1].x - (frame.pills[0].x + frame.pills[0].width);
        let sum: f32 = frame.pills.iter().map(|p| p.width).sum();
        let expected = 2.0 * BAR_PAD + sum + gap * 6.0 + TRAILING;
        assert!(
            close(frame.dock_width, expected),
            "{} vs {expected}",
            frame.dock_width
        );
        assert!(close(frame.pills[0].x, frame.dock_x + BAR_PAD));
        assert!(close(
            frame.new_tab_x,
            frame.pills[5].x + frame.pills[5].width + gap
        ));
        assert!(close(frame.shrink_x, frame.new_tab_x + BUTTON + 4.0));
        assert!(close(
            frame.dock_x + frame.dock_width - BAR_PAD,
            frame.shrink_x + BUTTON
        ));
        assert_eq!(frame.dock_height, 52.0);
        assert_eq!(frame.max_scale, 1.0);
        assert!(frame.tooltip.is_none());
    }
}

#[test]
fn a_pinned_tab_is_kept_and_a_group_without_a_tab_is_dropped() {
    let mut groups = mockup_groups(false);
    let mut tabs = mockup_tabs(false);
    tabs[15].pinned = true; // the first Shopping tab
    // A group with no tab, and a tab whose group is unknown.
    groups.push(GroupRow {
        id: 99,
        name: "Empty".into(),
        ..GroupRow::default()
    });
    tabs.push(TabRow {
        id: 500,
        group: 77,
        title: "Orphan".into(),
        ..TabRow::default()
    });
    let data = DockData::new(&groups, &tabs, &options(CONTAINER), &ApproxMetrics);
    let frame = data.frame(None);
    assert_eq!(frame.pills.len(), 6);
    assert!(frame.pills.iter().all(|p| p.group.id != 99));
    let shopping = &frame.pills[3];
    assert_eq!(shopping.group.name.as_str(), "Shopping");
    assert_eq!(shopping.slivers.row_count(), 3);
    assert!(
        slivers(shopping)
            .iter()
            .any(|(tab, ..)| *tab == tabs[15].id)
    );
    let all_tabs: usize = frame
        .pills
        .iter()
        .map(|p| p.chips.row_count() + p.slivers.row_count())
        .sum();
    assert_eq!(all_tabs, 22);
    assert_eq!(data.capsule().tab_count, 22);
    assert_eq!(data.capsule().group_count, 6);
}

#[test]
fn tabs_are_ordered_by_their_index_in_the_group() {
    let groups = mockup_groups(false);
    let mut tabs = mockup_tabs(false);
    tabs.reverse();
    let frame = DockData::new(&groups, &tabs, &options(CONTAINER), &ApproxMetrics).frame(None);
    let ids: Vec<i32> = chips(&frame.pills[0]).iter().map(|c| c.0).collect();
    assert_eq!(ids, vec![101, 102, 103, 104, 105]);
}

#[test]
fn the_current_group_is_the_option_else_the_active_tab_else_the_first() {
    let groups = mockup_groups(false);
    let tabs = mockup_tabs(false);
    let expanded = |opts: DockOptions, tabs: &[TabRow]| -> Vec<bool> {
        let data = DockData::new(&groups, tabs, &opts, &ApproxMetrics);
        data.frame(None).pills.iter().map(|p| p.expanded).collect()
    };
    let only = |index: usize| -> Vec<bool> { (0..6).map(|i| i == index).collect() };
    let with = |current_group| DockOptions {
        current_group,
        ..options(CONTAINER)
    };
    assert_eq!(expanded(with(Some(3)), &tabs), only(2));
    assert_eq!(expanded(with(None), &tabs), only(0));
    assert_eq!(
        expanded(with(Some(404)), &tabs),
        only(0),
        "an unknown id falls back"
    );
    let mut moved = tabs.clone();
    moved[0].active = false;
    moved[10].active = true; // a Reading tab
    assert_eq!(expanded(with(None), &moved), only(2));
    moved[10].active = false;
    assert_eq!(
        expanded(with(None), &moved),
        only(0),
        "no active tab: the first group"
    );
}

#[test]
fn no_rows_make_an_empty_but_valid_dock() {
    let data = DockData::new(&[], &[], &options(CONTAINER), &ApproxMetrics);
    let frame = data.frame(Some(700.0));
    assert!(frame.pills.is_empty() && frame.scales.is_empty());
    assert!(is_sane(&frame));
    assert_eq!(frame.max_scale, 1.0);
    assert_eq!(frame.dock_height, 52.0);
    assert!(close(frame.dock_width, 2.0 * BAR_PAD + TRAILING));
    assert_eq!(data.current_group(), None);
    let y = mid_y(&frame);
    assert_eq!(
        data.hit(&frame, frame.shrink_x + 5.0, y).kind,
        DockHitKind::Shrink
    );
    assert_eq!(data.hit(&frame, frame.new_tab_x + 5.0, y).group, -1);
}

// ---- the fit tiers (D8)

#[test]
fn six_groups_fit_at_tier_one_in_a_wide_window_and_a_higher_tier_in_a_narrow_one() {
    let wide = six_groups(1440.0, DockOptions::default());
    assert_eq!(wide.tier(), 1);
    let narrow = six_groups(700.0, DockOptions::default());
    assert!(narrow.tier() > 1, "tier {}", narrow.tier());
    for data in [&wide, &narrow] {
        let frame = data.frame(None);
        assert!(frame.dock_x >= 20.0 - 0.01);
    }
    // The 22-tab mockup has long titles: its chips are shrunk at 1440 (tier 2 with ApproxMetrics).
    assert!(mockup(1440.0).tier() <= 2);
}

#[test]
fn tier_one_has_natural_chips_and_tier_two_has_64_px_chips() {
    let tier1 = six_groups(1440.0, DockOptions::default()).frame(None);
    let chip_widths: Vec<f32> = chips(&tier1.pills[0]).iter().map(|c| c.2).collect();
    // The active chip (the first) may use up to 120 px, the others at most 88.
    assert!(chip_widths.iter().all(|w| (64.0..=120.0).contains(w)));
    assert!(
        chip_widths[1..].iter().all(|w| *w <= 88.0),
        "{chip_widths:?}"
    );
    assert!(chip_widths.iter().any(|w| *w > 64.0));
    let tier2 = (40..=144)
        .rev()
        .map(|tens| mockup(tens as f32 * 10.0))
        .find(|data| data.tier() == 2)
        .expect("some container width gives tier 2");
    let widths: Vec<f32> = chips(&tier2.frame(None).pills[0])
        .iter()
        .map(|c| c.2)
        .collect();
    // Every chip is 64 px but the active one (the first), which keeps room for its title.
    assert!(widths[1..].iter().all(|w| close(*w, 64.0)), "{widths:?}");
    assert!(widths[0] > 64.0 && widths[0] <= 104.0, "{widths:?}");
}

#[test]
fn the_active_chip_gets_more_room_than_the_others_when_its_title_is_long() {
    let (mut groups, mut tabs) = generated(2, 6, false);
    groups[0].active = true;
    for tab in tabs.iter_mut().filter(|tab| tab.group == groups[0].id) {
        tab.title = "A fairly long page title that will not fit".into();
        tab.active = false;
    }
    let first = tabs
        .iter_mut()
        .find(|tab| tab.group == groups[0].id)
        .expect("a tab");
    first.active = true;
    let data = DockData::new(&groups, &tabs, &options(1600.0), &ApproxMetrics);
    assert_eq!(data.tier(), 1);
    let widths: Vec<f32> = chips(&data.frame(None).pills[0])
        .iter()
        .map(|c| c.2)
        .collect();
    assert!(close(widths[0], 120.0), "{widths:?}");
    assert!(widths[1..].iter().all(|w| close(*w, 88.0)), "{widths:?}");
}

#[test]
fn the_tier_never_gets_lower_as_the_window_gets_narrower() {
    let mut last = 0;
    for container in (300..=1800).rev().step_by(10) {
        let tier = mockup(container as f32).tier();
        assert!(tier >= last, "{container}: {tier} after {last}");
        last = tier;
    }
    assert_eq!(last, 5);
}

#[test]
fn the_dock_always_has_headroom_for_the_magnification_when_a_tier_fits() {
    for container in [1440.0, 1200.0, 900.0, 700.0] {
        for data in [
            mockup(container),
            six_groups(container, DockOptions::default()),
        ] {
            if data.tier() == 5 {
                continue;
            }
            let rest = data.frame(None);
            let mut x = 0.0;
            while x <= container {
                let frame = data.frame(Some(x));
                assert!(
                    frame.dock_width <= container - 40.0 + 0.01,
                    "{container} tier {} x {x}: {}",
                    data.tier(),
                    frame.dock_width
                );
                assert!(frame.dock_x >= 20.0 - 0.01);
                x += 0.5;
            }
            assert!(rest.dock_width <= container - 40.0);
        }
    }
}

#[test]
fn sixty_four_groups_of_three_tabs_fit_in_1440_with_the_magnification() {
    let data = crowded(CONTAINER);
    let frame = data.frame(None);
    assert_eq!(frame.pills.len(), 64);
    assert_eq!(data.pill_widths().len(), 64);
    let mut x = -50.0;
    while x <= CONTAINER + 50.0 {
        let frame = data.frame(Some(x));
        assert!(is_sane(&frame));
        assert!(
            frame.dock_width <= CONTAINER - 40.0 + 0.01,
            "x {x}: {}",
            frame.dock_width
        );
        assert!(frame.dock_x >= 20.0 - 0.01, "x {x}");
        let left = frame.dock_x;
        let right = frame.dock_x + frame.dock_width;
        for pill in &frame.pills {
            assert!(pill.x >= left && pill.x + pill.width <= right, "x {x}");
        }
        x += 0.5;
    }
}

#[test]
fn sixty_four_groups_at_360_px_do_not_panic_and_stay_finite_and_non_negative() {
    let data = crowded(360.0);
    assert_eq!(data.tier(), 5);
    let mut x = -20.0;
    while x <= 400.0 {
        let frame = data.frame(Some(x));
        assert!(is_sane(&frame), "x {x}");
        for pill in &frame.pills {
            for (_, cx, cw) in chips(pill).into_iter().chain(slivers(pill)) {
                assert!(cx >= 0.0 && cw >= 0.0 && cx + cw <= pill.width + 0.01);
            }
        }
        let hit = data.hit(&frame, x, mid_y(&frame));
        assert!(hit.group >= -1 && hit.tab >= -1);
        x += 1.0;
    }
}

#[test]
fn a_bad_container_width_gives_finite_output() {
    for container in [f32::NAN, 0.0, -100.0, f32::INFINITY, f32::NEG_INFINITY, 1.0] {
        for data in [mockup(container), crowded(container)] {
            for x in [
                None,
                Some(0.0),
                Some(300.0),
                Some(f32::NAN),
                Some(f32::INFINITY),
            ] {
                let frame = data.frame(x);
                assert!(is_sane(&frame), "{container} {x:?}");
            }
            let frame = data.frame(Some(10.0));
            let _ = data.hit(&frame, 10.0, 10.0);
            let _ = data.tooltip_for(&frame, 10.0, 10.0);
        }
    }
}

#[test]
fn a_bad_pointer_is_the_same_as_no_pointer() {
    let data = mockup(CONTAINER);
    let none = numbers(&data.frame(None));
    for x in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
        assert_eq!(numbers(&data.frame(Some(x))), none);
    }
}

// ---- key badges (D13)

#[test]
fn held_adds_key_badges_and_widens_the_pills_and_a_group_without_a_key_has_none() {
    let (groups, tabs) = generated(6, 12, false);
    let build = |held| {
        let opts = DockOptions {
            held,
            ..options(CONTAINER)
        };
        DockData::new(&groups, &tabs, &opts, &ApproxMetrics)
    };
    let (rest, held) = (build(false), build(true));
    assert!(held.tier() <= 2);
    let (rest, held) = (rest.frame(None), held.frame(None));
    assert!(rest.pills.iter().all(|p| !p.show_key));
    for (index, (a, b)) in rest.pills.iter().zip(&held.pills).enumerate() {
        let has_key = !a.group.key_label.is_empty();
        assert_eq!(b.show_key, has_key, "pill {index}");
        if has_key {
            assert!(b.width > a.width, "pill {index}");
        } else {
            assert!(close(b.width, a.width), "pill {index}");
        }
    }
    assert_eq!(held.pills.iter().filter(|p| p.show_key).count(), 6);
}

#[test]
fn show_keys_shows_badges_at_rest_only_at_tier_one() {
    let with_keys = DockOptions {
        show_keys: true,
        ..DockOptions::default()
    };
    let wide = six_groups(1440.0, with_keys).frame(None);
    assert!(wide.pills.iter().all(|p| p.show_key));
    // The 22-tab mockup is at tier 2 in 1440 px: no badge.
    let data = DockData::new(
        &mockup_groups(false),
        &mockup_tabs(false),
        &DockOptions {
            show_keys: true,
            ..options(CONTAINER)
        },
        &ApproxMetrics,
    );
    assert!(data.tier() == 1 || data.frame(None).pills.iter().all(|p| !p.show_key));
    // Whatever the width: a badge only with tier 1.
    for container in (500..=1800).step_by(10) {
        let data = six_groups(container as f32, with_keys);
        let shown = data.frame(None).pills.iter().any(|p| p.show_key);
        assert!(
            !shown || data.tier() == 1,
            "{container}: tier {}",
            data.tier()
        );
    }
}

#[test]
fn held_at_a_higher_tier_has_no_badges_when_there_is_no_name() {
    let held = DockOptions {
        held: true,
        ..DockOptions::default()
    };
    let data = six_groups(700.0, held);
    assert!(data.tier() >= 3);
    assert!(data.frame(None).pills.iter().all(|p| !p.show_key));
}

// ---- magnification (D4)

#[test]
fn the_pointer_over_a_folded_pill_magnifies_it_and_its_folded_neighbours_and_never_the_current_one()
{
    let data = mockup(CONTAINER);
    let rest = data.frame(None);
    // The mockup's current group (Research) is the first pill and shows its tabs by name: it
    // does not grow, and nothing grows while the pointer is over it (brief D16).
    assert_eq!(data.current_group(), Some(rest.pills[0].group.id));
    for (index, pill) in rest.pills.iter().enumerate() {
        // (Within 24 px of a boundary two layouts are blended: see the test of the blend.)
        for fraction in [0.3, 0.5, 0.7] {
            let x = pill.x + pill.width * fraction;
            let frame = data.frame(Some(x));
            for (i, scale) in frame.scales.iter().enumerate() {
                let expected = if index == 0 || i == 0 {
                    1.0
                } else if i == index {
                    1.45
                } else if i + 1 == index || i == index + 1 {
                    1.12
                } else {
                    1.0
                };
                assert!(
                    close(*scale, expected),
                    "pill {index} x {x}: {:?}",
                    frame.scales
                );
                assert!(close(frame.pills[i].scale, expected));
                assert!(close(frame.pills[i].height, 30.0 * expected));
            }
            let largest = if index == 0 { 1.0 } else { 1.45 };
            assert!(close(frame.max_scale, largest), "pill {index}");
            assert!(
                close(frame.dock_height, dock_height(largest)),
                "pill {index}"
            );
        }
    }
}

#[test]
fn in_a_gap_the_nearer_pill_grows_most_and_past_the_pills_nothing_grows() {
    let data = mockup(CONTAINER);
    let rest = data.frame(None);
    for index in 0..5 {
        let right = rest.pills[index].x + rest.pills[index].width;
        let left = rest.pills[index + 1].x;
        // 1 px into the gap on either side of a pill: the nearer pill grows most (the current
        // group, pill 0, never grows, so after it the next pill does).
        assert_eq!(
            largest(&data.frame(Some(right + 1.0)).scales),
            Some(if index == 0 { 1 } else { index })
        );
        assert_eq!(
            largest(&data.frame(Some(left - 1.0)).scales),
            Some(index + 1)
        );
    }
    // On the trailing buttons, well past the last pill, nothing grows and the bar is at rest.
    let frame = data.frame(Some(rest.shrink_x + 13.0));
    assert_eq!(frame.scales, vec![1.0; 6]);
    assert_eq!(frame.dock_height, 52.0);
    assert!(close(frame.dock_x, rest.dock_x));
    // Between the last pill and the trailing block the last pill's growth fades out over the
    // 24 px after the middle of the gap: monotonic, and gone beyond them.
    let right = rest.pills[5].x + rest.pills[5].width;
    let mut previous = data.frame(Some(right - 30.0)).scales[5];
    assert!(close(previous, 1.45), "{previous}");
    let mut x = right - 29.0;
    while x < right + 35.0 {
        let scale = data.frame(Some(x)).scales[5];
        assert!(scale <= previous + 1e-4, "x {x}: {previous} -> {scale}");
        previous = scale;
        x += 1.0;
    }
    assert!(close(previous, 1.0), "{previous}");
}

/// `magnified_layout`'s answer with the first pill (the mockup's current group) kept at 1.0 and,
/// when the pointer is over it, nothing grown (brief D16).
fn without_the_first(
    mut scales: Vec<f32>,
    widths: &[f32],
    gap: f32,
    unchanged: mercury_ui::ring_geometry::DockLayout,
) -> (Vec<f32>, mercury_ui::ring_geometry::DockLayout) {
    if scales[0] > 1.0 {
        if scales[0] >= 1.44 {
            scales.iter_mut().for_each(|scale| *scale = 1.0);
        } else {
            scales[0] = 1.0;
        }
        let layout = mercury_ui::ring_geometry::dock_layout(widths, &scales, gap);
        (scales, layout)
    } else {
        (scales, unchanged)
    }
}

#[test]
fn in_the_middle_of_a_pill_the_frame_is_magnified_layouts_answer_from_the_resting_left_edge() {
    let data = mockup(CONTAINER);
    let rest = data.frame(None);
    let widths = data.pill_widths().to_vec();
    let gap = rest.pills[1].x - (rest.pills[0].x + rest.pills[0].width);
    let rest_left = rest.pills[0].x;
    for pill in &rest.pills {
        let x = pill.x + pill.width / 2.0;
        let frame = data.frame(Some(x));
        let (scales, layout) = magnified_layout(Some(x - rest_left), &widths, gap);
        // The one change to its answer: the current group's pill (the first) does not grow.
        let (scales, layout) = without_the_first(scales, &widths, gap, layout);
        assert_eq!(frame.scales, scales, "x {x}");
        let left = frame.pills[0].x;
        for (i, pill) in frame.pills.iter().enumerate() {
            assert!(close(pill.x, left + layout.xs[i]), "x {x} pill {i}");
            assert!(close(pill.width, layout.widths[i]), "x {x} pill {i}");
        }
    }
}

#[test]
fn at_a_boundary_two_layouts_are_blended_half_and_half_from_both_sides() {
    let data = mockup(CONTAINER);
    let rest = data.frame(None);
    // The boundary between pills 2 and 3: half the gap after pill 2.
    let boundary = rest.pills[2].x + rest.pills[2].width + 5.0;
    let before = data.frame(Some(boundary - 0.01));
    let after = data.frame(Some(boundary + 0.01));
    for (a, b) in before.scales.iter().zip(&after.scales) {
        assert!(
            (a - b).abs() < 0.01,
            "{:?} / {:?}",
            before.scales,
            after.scales
        );
    }
    // Each of the two pills is at the mean of 1.45 and 1.12 there.
    assert!((before.scales[2] - 1.285).abs() < 0.01 && (before.scales[3] - 1.285).abs() < 0.01);
    assert!((before.dock_x - after.dock_x).abs() < 0.1);
    assert!((before.dock_width - after.dock_width).abs() < 0.1);
}

#[test]
fn the_frame_is_a_pure_function_of_the_pointer() {
    let data = mockup(CONTAINER);
    for x in [None, Some(300.0), Some(640.0), Some(1000.0)] {
        assert_eq!(numbers(&data.frame(x)), numbers(&data.frame(x)));
    }
    // And another call in between does not change the answer.
    let before = numbers(&data.frame(Some(500.0)));
    let _ = data.frame(Some(900.0));
    assert_eq!(numbers(&data.frame(Some(500.0))), before);
}

#[test]
fn sweeping_across_the_bar_magnifies_one_pill_at_a_time_and_never_flips_back() {
    for data in [
        mockup(CONTAINER),
        six_groups(CONTAINER, DockOptions::default()),
        crowded(CONTAINER),
    ] {
        let rest = data.frame(None);
        let mut last = 0_usize;
        let mut x = rest.dock_x;
        while x <= rest.dock_x + rest.dock_width {
            let frame = data.frame(Some(x));
            let now = magnified(&frame.scales).unwrap_or(last);
            assert!(now >= last && now - last <= 1, "x {x}: {last} -> {now}");
            last = now;
            x += 0.5;
        }
        assert_eq!(last, rest.pills.len() - 1);
    }
}

#[test]
fn the_magnified_bar_is_anchored_on_the_pointer_and_its_height_follows_the_scale() {
    let data = mockup(CONTAINER);
    let rest = data.frame(None);
    let pointer = rest.pills[2].x + rest.pills[2].width / 2.0;
    let on_third = data.frame(Some(pointer));
    assert!(on_third.dock_width > rest.dock_width);
    // The pill that grew is drawn under the pointer, at the same fraction of it.
    let drawn = &on_third.pills[2];
    assert!(
        drawn.x <= pointer && pointer <= drawn.x + drawn.width,
        "pointer {pointer} outside the magnified pill {}..{}",
        drawn.x,
        drawn.x + drawn.width
    );
    assert!(
        close(pointer, drawn.x + drawn.width / 2.0),
        "the middle stays the middle"
    );
    // Every pill of the sweep is drawn under the pointer that magnified it.
    let mut x = rest.dock_x + 20.0;
    while x < rest.dock_x + rest.dock_width - 80.0 {
        let frame = data.frame(Some(x));
        if let Some(under) = magnified(&frame.scales) {
            let pill = &frame.pills[under];
            // Near a boundary the bar's position blends two layouts (see `DockData::inside`), so
            // the pointer may be a few pixels outside the pill that grew, never a pill away.
            assert!(
                pill.x - 12.0 <= x && x <= pill.x + pill.width + 12.0,
                "x {x}: pill {under} drawn at {}..{}",
                pill.x,
                pill.x + pill.width
            );
        }
        x += 3.0;
    }
    assert!(close(on_third.dock_height, dock_height(1.45)));
    assert!(close(on_third.dock_height, 65.5));
    assert_eq!(rest.dock_height, 52.0);
    // Pills touch the gap and keep the order without overlap.
    for pair in on_third.pills.windows(2) {
        assert!(pair[1].x >= pair[0].x + pair[0].width + 9.99);
    }
    let right = on_third.pills[5].x + on_third.pills[5].width;
    assert!(on_third.new_tab_x >= right);
    assert!(close(
        on_third.dock_x + on_third.dock_width - BAR_PAD,
        on_third.shrink_x + BUTTON
    ));
    // Neighbours at 1.12 are not as tall as the one under the pointer.
    assert!(close(on_third.pills[1].height, 33.6));
}

// ---- hit test

fn assert_hit(hit: DockHit, kind: DockHitKind, group: i32, tab: i32, what: &str) {
    assert_eq!((hit.kind, hit.group, hit.tab), (kind, group, tab), "{what}");
}

#[test]
fn hit_finds_every_pill_chip_sliver_and_button_at_rest_and_magnified() {
    let data = mockup(CONTAINER);
    let rest = data.frame(None);
    let frames = [
        data.frame(None),
        data.frame(Some(rest.pills[2].x + 40.0)),
        data.frame(Some(rest.pills[0].x + 100.0)),
    ];
    for frame in &frames {
        let y = mid_y(frame);
        for pill in &frame.pills {
            let group = pill.group.id;
            assert_hit(
                data.hit(frame, pill.x + 3.0, y),
                DockHitKind::Pill,
                group,
                -1,
                "pill",
            );
            for (tab, x, width) in chips(pill) {
                let at = pill.x + x + width / 2.0;
                assert_hit(
                    data.hit(frame, at, y),
                    DockHitKind::Chip,
                    group,
                    tab,
                    "chip",
                );
            }
            for (tab, x, width) in slivers(pill) {
                let at = pill.x + x + width / 2.0;
                assert_hit(
                    data.hit(frame, at, y),
                    DockHitKind::Sliver,
                    group,
                    tab,
                    "sliver",
                );
            }
            if pill.expanded {
                let at = pill.x + pill.width - (4.0 + BUTTON / 2.0) * pill.scale;
                assert_hit(
                    data.hit(frame, at, y),
                    DockHitKind::NewTab,
                    group,
                    -1,
                    "pill plus",
                );
            }
        }
        assert_hit(
            data.hit(frame, frame.new_tab_x + BUTTON / 2.0, y),
            DockHitKind::NewTab,
            1,
            -1,
            "plus",
        );
        assert_hit(
            data.hit(frame, frame.shrink_x + BUTTON / 2.0, y),
            DockHitKind::Shrink,
            -1,
            -1,
            "x",
        );
        // The targets are at least 24 px wide.
        assert_hit(
            data.hit(frame, frame.new_tab_x + 1.0, y),
            DockHitKind::NewTab,
            1,
            -1,
            "plus left",
        );
        assert_hit(
            data.hit(frame, frame.new_tab_x + 25.0, y),
            DockHitKind::NewTab,
            1,
            -1,
            "plus right",
        );
        assert_hit(
            data.hit(frame, frame.shrink_x + 1.0, y),
            DockHitKind::Shrink,
            -1,
            -1,
            "x left",
        );
        assert_hit(
            data.hit(frame, frame.shrink_x + 25.0, y),
            DockHitKind::Shrink,
            -1,
            -1,
            "x right",
        );
    }
}

#[test]
fn a_gap_between_slivers_goes_to_the_nearer_sliver() {
    let data = mockup(CONTAINER);
    let frame = data.frame(None);
    let pill = &frame.pills[2];
    let list = slivers(pill);
    for pair in list.windows(2) {
        let between = pill.x + (pair[0].1 + pair[0].2 + pair[1].1) / 2.0 - 0.01;
        let hit = data.hit(&frame, between, mid_y(&frame));
        assert_eq!(hit.kind, DockHitKind::Sliver);
        assert_eq!(hit.tab, pair[0].0);
    }
}

#[test]
fn outside_the_bar_and_in_its_gaps_hit_is_none_with_minus_ones() {
    let data = mockup(CONTAINER);
    let frame = data.frame(None);
    let y = mid_y(&frame);
    let none = |x: f32, y: f32, what: &str| {
        assert_hit(data.hit(&frame, x, y), DockHitKind::None, -1, -1, what);
    };
    none(frame.dock_x - 5.0, y, "left of the bar");
    none(frame.dock_x + frame.dock_width + 5.0, y, "right of the bar");
    none(frame.dock_x + 3.0, y, "the bar's padding");
    none(frame.pills[1].x + 1.0, -3.0, "above");
    none(
        frame.pills[1].x + 1.0,
        DockData::container_height() + 3.0,
        "below",
    );
    none(
        frame.pills[1].x + 1.0,
        DockData::container_height() - BOTTOM_MARGIN + 1.0,
        "the bottom margin",
    );
    none(f32::NAN, y, "NaN");
    let gap_x = frame.pills[1].x - 5.0;
    none(gap_x, y, "between two pills");
    none(frame.new_tab_x + BUTTON + 2.0, y, "between the buttons");
}

#[test]
fn every_hit_lies_inside_the_drawn_bar() {
    let data = mockup(CONTAINER);
    let rest = data.frame(None);
    for frame in [data.frame(None), data.frame(Some(rest.pills[3].x + 20.0))] {
        let bottom = DockData::container_height() - BOTTOM_MARGIN;
        let top = bottom - frame.dock_height;
        let mut hits = 0;
        let mut y = -5.0;
        while y <= 80.0 {
            let mut x = 0.0;
            while x <= CONTAINER {
                if data.hit(&frame, x, y).kind != DockHitKind::None {
                    hits += 1;
                    assert!(
                        x >= frame.dock_x && x <= frame.dock_x + frame.dock_width,
                        "x {x}"
                    );
                    assert!(y >= top && y <= bottom, "y {y}");
                }
                x += 1.0;
            }
            y += 1.0;
        }
        assert!(hits > 1000);
    }
}

#[test]
fn chips_and_slivers_of_a_pill_do_not_overlap_and_lie_inside_it() {
    let rest = mockup(CONTAINER);
    for data in [
        &rest,
        &six_groups(1440.0, DockOptions::default()),
        &crowded(CONTAINER),
        &crowded(360.0),
    ] {
        let base = data.frame(None);
        let frames = [
            data.frame(None),
            data.frame(Some(base.pills[0].x + 30.0)),
            data.frame(Some(base.pills[base.pills.len() / 2].x + 3.0)),
        ];
        for frame in &frames {
            for pill in &frame.pills {
                for list in [chips(pill), slivers(pill)] {
                    for (_, x, width) in &list {
                        assert!(*x >= -0.01 && *width > 0.0, "{x} {width}");
                        assert!(
                            x + width <= pill.width + 0.01,
                            "{x} + {width} vs {}",
                            pill.width
                        );
                    }
                    for pair in list.windows(2) {
                        assert!(pair[1].1 >= pair[0].1 + pair[0].2 - 0.01, "{pair:?}");
                    }
                }
            }
        }
    }
}

#[test]
fn chips_and_slivers_are_scaled_with_the_pill() {
    let data = mockup(CONTAINER);
    let rest = data.frame(None);
    let on_third = data.frame(Some(rest.pills[2].x + 40.0));
    for (a, b) in slivers(&rest.pills[2])
        .iter()
        .zip(slivers(&on_third.pills[2]))
    {
        assert!(close(b.1, a.1 * 1.45) && close(b.2, a.2 * 1.45));
    }
    for (a, b) in slivers(&rest.pills[3])
        .iter()
        .zip(slivers(&on_third.pills[3]))
    {
        assert!(close(b.1, a.1 * 1.12) && close(b.2, a.2 * 1.12));
    }
    // A pill at 1.0 shares its models with the resting frame (the current group's chips too).
    assert!(rest.pills[5].slivers == on_third.pills[5].slivers);
    assert!(rest.pills[0].chips == on_third.pills[0].chips);
    assert!(rest.pills[3].slivers != on_third.pills[3].slivers);
}

// ---- tooltip

#[test]
fn the_tooltip_over_a_sliver_is_the_group_and_the_title() {
    let data = mockup(CONTAINER);
    let rest = data.frame(None);
    let frame = data.frame(Some(rest.pills[2].x + 40.0));
    let pill = &frame.pills[2];
    let y = mid_y(&frame);
    let (tab, x, width) = slivers(pill)[1];
    let at = pill.x + x + width / 2.0;
    let (text, tip_x) = data
        .tooltip_for(&frame, at, y)
        .expect("a tooltip over a sliver");
    assert_eq!(text.as_str(), "Reading · Essay on maps");
    assert!(close(tip_x, at));
    assert_eq!(tab, 111, "the second Reading tab");
    // A chip of the current group.
    let frame = data.frame(Some(rest.pills[0].x + 5.0));
    let pill = &frame.pills[0];
    let (_, x, width) = chips(pill)[0];
    let (text, tip_x) = data
        .tooltip_for(&frame, pill.x + x + width / 2.0, mid_y(&frame))
        .expect("a tooltip over a chip");
    assert_eq!(text.as_str(), "Research · Servo layout notes");
    assert!(close(tip_x, pill.x + x + width / 2.0));
}

#[test]
fn there_is_no_tooltip_over_names_gaps_buttons_or_outside() {
    let data = mockup(CONTAINER);
    let frame = data.frame(None);
    let y = mid_y(&frame);
    assert!(data.tier() <= 2);
    for x in [
        frame.pills[1].x + 3.0,
        frame.pills[1].x - 5.0,
        frame.new_tab_x + 5.0,
        frame.shrink_x + 5.0,
        frame.dock_x - 20.0,
    ] {
        assert!(data.tooltip_for(&frame, x, y).is_none(), "x {x}");
    }
    assert!(
        data.tooltip_for(&frame, frame.pills[1].x + 3.0, -10.0)
            .is_none()
    );
    assert!(frame.tooltip.is_none());
}

#[test]
fn a_folded_pill_without_a_name_has_its_group_name_as_tooltip() {
    let data = crowded(CONTAINER);
    assert!(data.tier() >= 4);
    let frame = data.frame(None);
    let pill = &frame.pills[10];
    let (text, x) = data
        .tooltip_for(&frame, pill.x + pill.width / 2.0, mid_y(&frame))
        .expect("the name of the group");
    assert_eq!(text.as_str(), "Group 11");
    assert!(close(x, pill.x + pill.width / 2.0));
}

// ---- the models

#[derive(Default)]
struct Counts {
    changed: RefCell<Vec<usize>>,
    added: Cell<u32>,
    removed: Cell<u32>,
    reset: Cell<u32>,
}

impl ModelChangeListener for Counts {
    fn row_changed(self: Pin<&Self>, row: usize) {
        self.changed.borrow_mut().push(row);
    }
    fn row_added(self: Pin<&Self>, _index: usize, _count: usize) {
        self.added.set(self.added.get() + 1);
    }
    fn row_removed(self: Pin<&Self>, _index: usize, _count: usize) {
        self.removed.set(self.removed.get() + 1);
    }
    fn reset(self: Pin<&Self>) {
        self.reset.set(self.reset.get() + 1);
    }
}

impl Counts {
    fn clear(&self) {
        self.changed.borrow_mut().clear();
        self.added.set(0);
        self.removed.set(0);
        self.reset.set(0);
    }

    fn structural(&self) -> u32 {
        self.added.get() + self.removed.get() + self.reset.get()
    }
}

#[test]
fn update_keeps_the_model_identity_and_patches_rows_in_place_while_the_count_is_unchanged() {
    let data = mockup(CONTAINER);
    let models = DockModels::new();
    let (pills, capsule_slivers) = (models.pills(), models.slivers());
    let listener = Box::pin(ModelChangeListenerContainer::<Counts>::default());
    pills
        .model_tracker()
        .attach_peer(Pin::as_ref(&listener).model_peer());
    let sliver_listener = Box::pin(ModelChangeListenerContainer::<Counts>::default());
    capsule_slivers
        .model_tracker()
        .attach_peer(Pin::as_ref(&sliver_listener).model_peer());

    let rest = data.frame(None);
    models.update(&rest, &data.capsule());
    assert_eq!(pills.row_count(), 6);
    assert!(pills == models.pills() && capsule_slivers == models.slivers());
    assert!(listener.structural() >= 1, "the first fill adds the rows");
    for (index, pill) in rest.pills.iter().enumerate() {
        assert_eq!(
            pills.row_data(index).map(|p| p.group.id),
            Some(pill.group.id)
        );
    }

    // The pointer moves over the third pill: same count, rows patched in place.
    listener.clear();
    sliver_listener.clear();
    let magnified_frame = data.frame(Some(rest.pills[2].x + 40.0));
    models.update(&magnified_frame, &data.capsule());
    assert_eq!(listener.structural(), 0, "no row added, removed or reset");
    let changed = listener.changed.borrow().clone();
    assert!(!changed.is_empty());
    assert!(changed.iter().all(|row| *row < 6));
    // Pills far from the pointer (rows 4 and 5 at scale 1.0 both times) are not touched... except
    // that their x moved when the bar grew: here only the bar's centred growth moves them.
    for index in 0..6 {
        let row = pills.row_data(index).expect("row");
        assert!(
            close(row.scale, magnified_frame.scales[index]),
            "row {index}"
        );
        assert!(close(row.x, magnified_frame.pills[index].x), "row {index}");
    }
    assert!(pills == models.pills(), "same ModelRc");
    assert_eq!(sliver_listener.structural(), 0);

    // The same frame again changes nothing.
    listener.clear();
    models.update(&magnified_frame, &data.capsule());
    assert!(listener.changed.borrow().is_empty());
    assert_eq!(listener.structural(), 0);

    // Back to rest: in place again.
    models.update(&rest, &data.capsule());
    assert_eq!(listener.structural(), 0);
    assert_eq!(pills.row_data(2).map(|p| p.scale), Some(1.0));

    // A different number of pills replaces the content.
    let (groups, tabs) = generated(4, 8, false);
    let smaller = DockData::new(&groups, &tabs, &options(CONTAINER), &ApproxMetrics);
    listener.clear();
    models.update(&smaller.frame(None), &smaller.capsule());
    assert!(listener.structural() >= 1);
    assert_eq!(pills.row_count(), 4);
    assert!(pills == models.pills());
}

// ---- speed

#[test]
fn sixty_four_groups_and_two_hundred_tabs_build_and_run_a_hundred_frames_quickly() {
    let (groups, tabs) = generated(64, 200, false);
    let started = Instant::now();
    let data = DockData::new(&groups, &tabs, &options(CONTAINER), &ApproxMetrics);
    let models = DockModels::new();
    let capsule = data.capsule();
    for step in 0..100 {
        let x = 20.0 + step as f32 * 13.9;
        let frame = data.frame(Some(x));
        let _ = data.hit(&frame, x, 40.0);
        let _ = data.tooltip_for(&frame, x, 40.0);
        models.update(&frame, &capsule);
    }
    let elapsed = started.elapsed();
    assert!(elapsed.as_secs_f32() < 2.0, "{elapsed:?}");
    println!("new + 100 frames, 64 groups, 200 tabs: {elapsed:?}");
}

#[test]
fn a_chips_x_is_a_24_px_target_that_exists_only_where_the_x_is_drawn() {
    let data = mockup(CONTAINER);
    let rest = data.frame(None);
    let pill = &rest.pills[0];
    let group = pill.group.id;
    let list = chips(pill);
    assert!(list.len() >= 3 && pill.expanded);
    let (active_tab, active_x, active_width) = list[0];
    assert_eq!(
        mockup_tabs(false)
            .iter()
            .find(|tab| tab.active)
            .map(|tab| tab.id),
        Some(active_tab),
        "the fixture's first chip is the active one"
    );

    // The active chip shows its x at rest: the whole 24 px at the right end is the close target
    // (the glyph is 16 px, centred in it), and 25 px in is the chip.
    let right = pill.x + active_x + active_width;
    let y = mid_y(&rest);
    for (offset, expected, what) in [
        (1.0, DockHitKind::ChipClose, "the edge pixel"),
        (12.0, DockHitKind::ChipClose, "the middle of the target"),
        (23.5, DockHitKind::ChipClose, "the inner edge of the target"),
        (25.0, DockHitKind::Chip, "just outside the target"),
    ] {
        assert_hit(
            data.hit(&rest, right - offset, y),
            expected,
            group,
            active_tab,
            what,
        );
    }

    // An inactive chip has no x until the pointer is on it: at rest (or with the pointer on
    // another chip) its right end is the chip; with the pointer on it, it is the close target.
    let (tab, x, width) = list[1];
    let end = pill.x + x + width - 12.0;
    assert_hit(
        data.hit(&rest, end, y),
        DockHitKind::Chip,
        group,
        tab,
        "no x drawn at rest",
    );
    let elsewhere = data.frame(Some(pill.x + list[2].1 + 20.0));
    assert_eq!(elsewhere.hovered_tab, Some(list[2].0));
    assert_hit(
        data.hit(&elsewhere, end, y),
        DockHitKind::Chip,
        group,
        tab,
        "the pointer is on another chip",
    );
    let on_it = data.frame(Some(end));
    assert_eq!(on_it.hovered_tab, Some(tab));
    assert_hit(
        data.hit(&on_it, end, y),
        DockHitKind::ChipClose,
        group,
        tab,
        "the pointer is on it: its x is drawn",
    );
    // The left end of any chip is the chip.
    assert_hit(
        data.hit(&on_it, pill.x + x + 10.0, y),
        DockHitKind::Chip,
        group,
        tab,
        "the chip's left end",
    );
}

#[test]
fn the_frame_says_which_tab_is_under_the_pointer() {
    let data = mockup(CONTAINER);
    let rest = data.frame(None);
    assert_eq!(rest.hovered_tab, None);
    let sliver = slivers(&rest.pills[1])[1];
    let frame = data.frame(Some(rest.pills[1].x + sliver.1 + sliver.2 / 2.0));
    // The pointer is on pill 1 of the resting layout, so that pill is the magnified one; the
    // sliver under the pointer of the *magnified* frame is the one reported.
    let magnified = &frame.pills[1];
    let hovered = frame.hovered_tab.expect("a tab is hovered");
    assert!(slivers(magnified).iter().any(|s| s.0 == hovered));
    assert_eq!(data.frame(Some(rest.dock_x - 30.0)).hovered_tab, None);
    assert_eq!(data.frame(Some(f32::NAN)).hovered_tab, None);
}

#[test]
fn the_bar_follows_the_pointer_smoothly_at_every_pill_boundary() {
    // A 1 px sweep over the whole bar and beyond it: the bar's left edge moves by a few pixels per
    // pixel of pointer travel, never by the tens of pixels a switch of the magnified pill would
    // give if each pill were anchored on its own (about 0.12 w + 0.33 w - gap on the mockup row).
    let rest_data = mockup(CONTAINER);
    let six = six_groups(1440.0, DockOptions::default());
    let many = crowded(CONTAINER);
    for (name, data) in [("mockup", &rest_data), ("six", &six), ("crowded", &many)] {
        let rest = data.frame(None);
        let mut x = rest.dock_x - 60.0;
        let mut previous = data.frame(Some(x));
        let mut worst: Option<(f32, f32, String)> = None;
        while x < rest.dock_x + rest.dock_width + 60.0 {
            x += 1.0;
            let frame = data.frame(Some(x));
            let step = (frame.dock_x - previous.dock_x).abs();
            if worst.as_ref().is_none_or(|(w, ..)| step > *w) {
                worst = Some((
                    step,
                    x,
                    format!(
                        "dock_x {} -> {}, width {} -> {}, scales {:?} -> {:?}, tier {}",
                        previous.dock_x,
                        frame.dock_x,
                        previous.dock_width,
                        frame.dock_width,
                        previous.scales,
                        frame.scales,
                        data.tier()
                    ),
                ));
            }
            previous = frame;
        }
        let (largest, at, what) = worst.expect("a sweep");
        assert!(
            largest <= 3.0,
            "{name}: the bar moved by {largest} px for 1 px of pointer at x {at}: {what}"
        );
    }
}

#[test]
fn the_bar_is_where_the_pointer_is_when_the_pointer_is_between_two_boundaries() {
    // In the middle of a folded pill (far from both boundaries) the point of the resting pill
    // under the pointer is drawn under it exactly.
    let data = mockup(CONTAINER);
    let rest = data.frame(None);
    for index in 1..6 {
        let pill = &rest.pills[index];
        let x = pill.x + pill.width / 2.0;
        let frame = data.frame(Some(x));
        let drawn = &frame.pills[index];
        assert!(
            close(x, drawn.x + drawn.width / 2.0),
            "pill {index}: pointer {x}, drawn middle {}",
            drawn.x + drawn.width / 2.0
        );
    }
}
