// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! The pinned beads (`hub_ring::beads`) and the hub button's donut (`hub_ring::hub_button`).
//! Pure Rust, no window. Needs `--features test-support` for the fixtures.
//!
//! The mockup (`ring-open`): ring centre (338, 342), radius 180, three beads of diameter 20 whose
//! centres are (317.9, 163.1), (338, 162) and (358.1, 163.1).

use mercury_ui::hub_ring::beads::{
    BEAD_PITCH, BEAD_SIZE, BeadLayout, BeadSpot, PinnedBead, layout, layout_for, rows,
};
use mercury_ui::hub_ring::fixtures;
use mercury_ui::hub_ring::hub_button::{BUTTON_SIZE, donut, label};
use mercury_ui::ring_bands::fixtures as ring_fixtures;
use mercury_ui::{GroupRow, TabRow};
use slint::{Color, Image, Rgba8Pixel, SharedPixelBuffer};

const CENTRE: (f32, f32) = (338.0, 342.0);
const RADIUS: f32 = 180.0;
const RESERVE: f32 = 24.0;

fn centres(layout: &BeadLayout) -> Vec<(f32, f32)> {
    layout.spots.iter().map(BeadSpot::centre).collect()
}

fn distance(a: (f32, f32), b: (f32, f32)) -> f32 {
    ((a.0 - b.0).powi(2) + (a.1 - b.1).powi(2)).sqrt()
}

fn pinned(count: usize) -> Vec<PinnedBead> {
    (0..count)
        .map(|i| PinnedBead {
            tab: 300 + i as i32,
            title: format!("Pinned {i}"),
            letter: char::from(b'A' + i as u8).to_string(),
            mark: Color::from_rgb_u8(10 * i as u8, 20, 30),
            favicon: Image::default(),
            has_favicon: false,
        })
        .collect()
}

// ---- capacity -----------------------------------------------------------------------------

#[test]
fn the_mockup_reserve_holds_three_beads_and_no_reserve_none() {
    assert_eq!(layout(0, RESERVE, RADIUS, CENTRE).capacity, 3);
    assert_eq!(layout(0, 0.0, RADIUS, CENTRE).capacity, 0);
}

#[test]
fn capacity_steps_at_the_documented_reserves() {
    // capacity = floor((reserve_rad * radius + BEAD_SIZE - BEAD_PITCH - 2) / BEAD_PITCH), which
    // steps at reserve_rad * radius = 20.1 n + 2.1: 7.07, 13.46, 19.86 and 26.26 degrees at 180.
    let capacity = |deg: f32| layout(0, deg, RADIUS, CENTRE).capacity;
    assert_eq!(capacity(7.0), 0);
    assert_eq!(capacity(7.1), 1);
    assert_eq!(capacity(13.4), 1);
    assert_eq!(capacity(13.5), 2);
    assert_eq!(capacity(19.8), 2, "boundary 2 to 3");
    assert_eq!(capacity(19.9), 3, "boundary 2 to 3");
    assert_eq!(capacity(26.2), 3, "boundary 3 to 4");
    assert_eq!(capacity(26.3), 4, "boundary 3 to 4");
}

#[test]
fn capacity_grows_with_the_reserve_and_the_radius() {
    let mut last = 0;
    for deg in (0..=120).map(|d| d as f32) {
        let capacity = layout(0, deg, RADIUS, CENTRE).capacity;
        assert!(capacity >= last, "never shrinks as the reserve widens");
        last = capacity;
    }
    assert!(
        layout(0, RESERVE, 90.0, CENTRE).capacity < layout(0, RESERVE, RADIUS, CENTRE).capacity,
        "the same angle is a shorter arc on a smaller circle"
    );
}

#[test]
fn nonsense_input_gives_capacity_zero_and_no_panic() {
    for (reserve, radius) in [
        (f32::NAN, RADIUS),
        (RESERVE, f32::NAN),
        (f32::INFINITY, RADIUS),
        (RESERVE, f32::INFINITY),
        (-30.0, RADIUS),
        (RESERVE, -180.0),
        (RESERVE, 0.0),
    ] {
        let laid = layout(5, reserve, radius, CENTRE);
        assert_eq!(laid.capacity, 0, "reserve {reserve} radius {radius}");
        assert!(laid.spots.is_empty());
        assert_eq!(laid.hidden, 5);
    }
}

// ---- positions ----------------------------------------------------------------------------

#[test]
fn three_beads_sit_where_the_mockup_has_them() {
    let laid = layout(3, RESERVE, RADIUS, CENTRE);
    let want = [(317.9, 163.1), (338.0, 162.0), (358.1, 163.1)];
    let got = centres(&laid);
    assert_eq!(got.len(), 3);
    for (got, want) in got.iter().zip(want) {
        assert!(
            (got.0 - want.0).abs() < 0.05 && (got.1 - want.1).abs() < 0.05,
            "got {got:?}, mockup {want:?}"
        );
    }
    assert!(laid.spots.iter().all(|spot| spot.size == BEAD_SIZE));
    assert!(laid.spots.iter().all(|spot| spot.more == 0));
    assert_eq!(laid.hidden, 0);
}

#[test]
fn one_bead_is_at_12_oclock() {
    let laid = layout(1, RESERVE, RADIUS, CENTRE);
    let got = centres(&laid);
    assert_eq!(got.len(), 1);
    assert!((got[0].0 - 338.0).abs() < 0.05 && (got[0].1 - 162.0).abs() < 0.05);
}

#[test]
fn spot_position_is_the_top_left_of_its_box_and_centre_is_the_middle() {
    let spot = layout(1, RESERVE, RADIUS, CENTRE).spots[0];
    let (cx, cy) = spot.centre();
    assert!((spot.x - (cx - BEAD_SIZE / 2.0)).abs() < 0.001);
    assert!((spot.y - (cy - BEAD_SIZE / 2.0)).abs() < 0.001);
}

/// The geometry rules, for any count and reserve: on the circle, mirrored about 12 o'clock (y up
/// is smaller y), neighbours `BEAD_PITCH` apart, never overlapping.
fn check_geometry(count: usize, reserve: f32, radius: f32, centre: (f32, f32)) {
    let laid = layout(count, reserve, radius, centre);
    let got = centres(&laid);
    let n = got.len();
    let what = format!("count {count} reserve {reserve} radius {radius}");
    for &point in &got {
        assert!(
            (distance(point, centre) - radius).abs() < 0.05,
            "{what}: {point:?} is not on the circle"
        );
        assert!(point.1 < centre.1, "{what}: {point:?} is below the centre");
    }
    for i in 0..n {
        let (a, b) = (got[i], got[n - 1 - i]);
        assert!(
            ((a.0 - centre.0) + (b.0 - centre.0)).abs() < 0.05,
            "{what}: beads {i} and {} are not mirrored about 12 o'clock",
            n - 1 - i
        );
        assert!(
            (a.1 - b.1).abs() < 0.05,
            "{what}: mirrored beads differ in y"
        );
    }
    for pair in got.windows(2) {
        assert!(pair[1].0 > pair[0].0, "{what}: left to right");
        let gap = distance(pair[0], pair[1]);
        assert!(
            (gap - BEAD_PITCH).abs() < 0.01,
            "{what}: neighbours {gap} apart, want {BEAD_PITCH}"
        );
        assert!(gap >= BEAD_SIZE, "{what}: overlap");
    }
    for i in 0..n {
        for j in i + 1..n {
            assert!(distance(got[i], got[j]) >= BEAD_SIZE, "{what}: overlap");
        }
    }
}

#[test]
fn beads_are_on_the_circle_mirrored_and_apart_for_every_count_at_the_mockup_reserve() {
    for count in [1, 2, 3, 4, 8] {
        check_geometry(count, RESERVE, RADIUS, CENTRE);
    }
}

#[test]
fn the_geometry_holds_for_other_reserves_radii_and_centres() {
    check_geometry(7, 60.0, RADIUS, CENTRE);
    check_geometry(6, 60.0, RADIUS, CENTRE);
    check_geometry(2, 40.0, 100.0, (0.0, 0.0));
    check_geometry(9, 90.0, 258.0, (258.0, 258.0));
}

// ---- over capacity: the chip --------------------------------------------------------------

#[test]
fn no_pinned_tabs_no_spots() {
    let laid = layout(0, RESERVE, RADIUS, CENTRE);
    assert!(laid.spots.is_empty());
    assert_eq!(laid.hidden, 0);
}

#[test]
fn count_equal_to_capacity_needs_no_chip() {
    let laid = layout(3, RESERVE, RADIUS, CENTRE);
    assert_eq!(laid.spots.len(), 3);
    assert!(laid.spots.iter().all(|spot| spot.more == 0));
}

#[test]
fn four_pinned_tabs_in_a_reserve_for_three_make_two_beads_and_a_chip_for_two() {
    let laid = layout(4, RESERVE, RADIUS, CENTRE);
    assert_eq!(laid.capacity, 3);
    assert_eq!(laid.spots.len(), 3);
    assert_eq!(laid.spots[0].more, 0);
    assert_eq!(laid.spots[1].more, 0);
    assert_eq!(laid.spots[2].more, 2, "tabs 3 and 4 are behind the chip");
    assert_eq!(laid.hidden, 2);
    assert_eq!(
        laid.spots[2].tab, 2,
        "layout(count): the chip's tab is the index of the first hidden bead"
    );
}

#[test]
fn eight_pinned_tabs_make_two_beads_and_a_chip_for_six() {
    let laid = layout(8, RESERVE, RADIUS, CENTRE);
    assert_eq!(laid.spots.len(), 3);
    assert_eq!(laid.spots[2].more, 6);
    assert_eq!(laid.hidden, 6);
    let beads = laid.spots.iter().filter(|spot| spot.more == 0).count();
    assert_eq!(beads, 2);
    assert_eq!(
        beads + laid.hidden,
        8,
        "every pinned tab is a bead or hidden"
    );
}

#[test]
fn the_chip_sits_in_the_last_slot_with_the_same_spacing() {
    let chip = layout(8, RESERVE, RADIUS, CENTRE);
    let full = layout(3, RESERVE, RADIUS, CENTRE);
    for (a, b) in centres(&chip).iter().zip(centres(&full)) {
        assert!(distance(*a, b) < 0.01, "same three slots");
    }
}

#[test]
fn a_reserve_for_two_gives_one_bead_and_the_chip() {
    let laid = layout(3, 15.0, RADIUS, CENTRE);
    assert_eq!(laid.capacity, 2);
    assert_eq!(laid.spots.len(), 2);
    assert_eq!(laid.spots[0].more, 0);
    assert_eq!(laid.spots[1].more, 2);
    assert_eq!(laid.hidden, 2);
}

#[test]
fn a_reserve_for_one_gives_a_single_chip_for_many_and_a_bead_for_one() {
    let many = layout(4, 10.0, RADIUS, CENTRE);
    assert_eq!(many.capacity, 1);
    assert_eq!(many.spots.len(), 1);
    assert_eq!(many.spots[0].more, 4);
    assert_eq!(many.hidden, 4);
    assert!(
        (many.spots[0].centre().0 - CENTRE.0).abs() < 0.05,
        "at 12 o'clock"
    );

    let one = layout(1, 10.0, RADIUS, CENTRE);
    assert_eq!(one.spots.len(), 1);
    assert_eq!(
        one.spots[0].more, 0,
        "one pinned tab fits: a bead, not a chip"
    );
    assert_eq!(one.hidden, 0);
}

#[test]
fn no_reserve_draws_nothing_and_hides_every_pinned_tab() {
    for count in [0, 1, 3, 4, 8] {
        let laid = layout(count, 0.0, RADIUS, CENTRE);
        assert_eq!(laid.capacity, 0, "count {count}");
        assert!(laid.spots.is_empty(), "count {count}");
        assert_eq!(laid.hidden, count, "count {count}");
    }
}

#[test]
fn layout_for_uses_the_real_tab_ids_and_the_same_geometry() {
    for count in [0, 1, 3, 4, 8] {
        let pins = pinned(count);
        let with_ids = layout_for(&pins, RESERVE, RADIUS, CENTRE);
        let plain = layout(count, RESERVE, RADIUS, CENTRE);
        assert_eq!(with_ids.capacity, plain.capacity);
        assert_eq!(with_ids.hidden, plain.hidden);
        assert_eq!(with_ids.spots.len(), plain.spots.len());
        for (a, b) in with_ids.spots.iter().zip(&plain.spots) {
            assert_eq!((a.x, a.y, a.size, a.more), (b.x, b.y, b.size, b.more));
        }
    }
    let laid = layout_for(&pinned(8), RESERVE, RADIUS, CENTRE);
    assert_eq!(laid.spots[0].tab, 300);
    assert_eq!(laid.spots[1].tab, 301);
    assert_eq!(laid.spots[2].tab, 302, "the chip is the first hidden bead");
    assert_eq!(laid.spots[2].more, 6);
}

// ---- rows ---------------------------------------------------------------------------------

#[test]
fn rows_carry_the_letter_mark_title_and_place_of_each_bead() {
    let pins: Vec<PinnedBead> = fixtures::pinned_tabs()
        .iter()
        .map(PinnedBead::from_row)
        .collect();
    let laid = layout_for(&pins, RESERVE, RADIUS, CENTRE);
    let out = rows(&laid, &pins, None, &|_| false);
    assert_eq!(out.len(), 3);
    assert_eq!(
        out.iter().map(|r| r.tab).collect::<Vec<_>>(),
        fixtures::PINNED_IDS
    );
    assert_eq!(
        out.iter().map(|r| r.letter.as_str()).collect::<Vec<_>>(),
        ["M", "M", "C"]
    );
    assert_eq!(
        out.iter().map(|r| r.title.as_str()).collect::<Vec<_>>(),
        ["Mail", "Maps", "Calendar"]
    );
    for (row, pin) in out.iter().zip(&pins) {
        assert_eq!(row.mark, pin.mark);
        assert_eq!(row.size, BEAD_SIZE);
        assert_eq!(row.more, 0);
        assert!(!row.focused);
        assert!(!row.dimmed);
        assert!(!row.has_favicon);
    }
    for (row, spot) in out.iter().zip(&laid.spots) {
        assert_eq!((row.x, row.y), (spot.x, spot.y));
    }
}

#[test]
fn rows_mark_the_focused_bead_and_dim_the_ones_that_do_not_match() {
    let pins = pinned(3);
    let laid = layout_for(&pins, RESERVE, RADIUS, CENTRE);
    let out = rows(&laid, &pins, Some(301), &|tab| tab == 302);
    assert_eq!(
        out.iter().map(|r| r.focused).collect::<Vec<_>>(),
        [false, true, false]
    );
    assert_eq!(
        out.iter().map(|r| r.dimmed).collect::<Vec<_>>(),
        [false, false, true]
    );
}

#[test]
fn a_favicon_row_says_so() {
    let mut pins = pinned(1);
    pins[0].favicon = Image::from_rgba8(SharedPixelBuffer::<Rgba8Pixel>::new(4, 4));
    pins[0].has_favicon = true;
    let laid = layout_for(&pins, RESERVE, RADIUS, CENTRE);
    let out = rows(&laid, &pins, None, &|_| false);
    assert!(out[0].has_favicon);
    assert_eq!(out[0].favicon.size().width, 4);
}

#[test]
fn the_chip_row_says_plus_n_and_stands_for_the_first_hidden_bead() {
    let pins = pinned(8);
    let laid = layout_for(&pins, RESERVE, RADIUS, CENTRE);
    let out = rows(&laid, &pins, None, &|_| false);
    assert_eq!(out.len(), 3);
    assert_eq!(out[0].more, 0);
    assert_eq!(out[1].more, 0);
    assert_eq!(out[2].more, 6);
    assert_eq!(out[2].letter.as_str(), "+6");
    assert_eq!(
        out[2].tab, 302,
        "activating the chip opens the first hidden bead"
    );
    assert_eq!(out[1].letter.as_str(), "B");
}

#[test]
fn the_chip_row_is_right_with_the_index_layout_too() {
    let pins = pinned(4);
    let laid = layout(4, RESERVE, RADIUS, CENTRE);
    let out = rows(&laid, &pins, None, &|_| false);
    assert_eq!(out.len(), 3);
    assert_eq!(out[0].tab, 300);
    assert_eq!(out[1].tab, 301);
    assert_eq!(out[2].tab, 302);
    assert_eq!(out[2].more, 2);
    assert_eq!(out[2].letter.as_str(), "+2");
}

#[test]
fn the_chip_is_focused_when_the_focus_is_on_a_hidden_bead_and_dimmed_when_all_of_them_are() {
    let pins = pinned(8);
    let laid = layout_for(&pins, RESERVE, RADIUS, CENTRE);
    let on_hidden = rows(&laid, &pins, Some(306), &|_| false);
    assert!(
        on_hidden[2].focused,
        "the chip shows the focused hidden bead"
    );
    assert!(!on_hidden[0].focused && !on_hidden[1].focused);
    let on_shown = rows(&laid, &pins, Some(300), &|_| false);
    assert!(on_shown[0].focused && !on_shown[2].focused);

    let some_match = rows(&laid, &pins, None, &|tab| tab != 307);
    assert!(!some_match[2].dimmed, "one hidden bead still matches");
    let none_match = rows(&laid, &pins, None, &|_| true);
    assert!(none_match[2].dimmed);
}

#[test]
fn no_reserve_gives_no_rows() {
    let pins = pinned(5);
    let laid = layout_for(&pins, 0.0, RADIUS, CENTRE);
    assert!(rows(&laid, &pins, Some(300), &|_| false).is_empty());
}

// ---- PinnedBead::from_row -----------------------------------------------------------------

fn tab(host: &str, title: &str) -> TabRow {
    TabRow {
        id: 7,
        title: title.into(),
        host: host.into(),
        shade: Color::from_rgb_u8(1, 2, 3),
        pinned: true,
        ..TabRow::default()
    }
}

#[test]
fn the_letter_is_the_first_letter_of_the_host_upper_cased() {
    let pin = PinnedBead::from_row(&tab("mail.example.org", "Inbox"));
    assert_eq!(pin.letter, "M");
    assert_eq!(pin.tab, 7);
    assert_eq!(pin.title, "Inbox");
    assert_eq!(pin.mark, Color::from_rgb_u8(1, 2, 3));
    assert!(!pin.has_favicon);
}

#[test]
fn the_letter_skips_leading_punctuation_and_handles_digits_and_accents() {
    assert_eq!(
        PinnedBead::from_row(&tab("-.1password.com", "x")).letter,
        "1"
    );
    assert_eq!(PinnedBead::from_row(&tab("école.example", "x")).letter, "É");
}

#[test]
fn without_a_host_the_title_gives_the_letter_and_without_both_a_question_mark() {
    assert_eq!(PinnedBead::from_row(&tab("", "  calendar")).letter, "C");
    assert_eq!(PinnedBead::from_row(&tab("..", "(notes)")).letter, "N");
    assert_eq!(PinnedBead::from_row(&tab("", "")).letter, "?");
    assert_eq!(PinnedBead::from_row(&tab("--", "...")).letter, "?");
}

#[test]
fn a_one_character_letter_even_for_characters_that_upper_case_to_two() {
    assert_eq!(
        PinnedBead::from_row(&tab("ßeta.example", "x"))
            .letter
            .chars()
            .count(),
        1
    );
}

#[test]
fn has_favicon_follows_the_image_width() {
    let mut row = tab("a.example", "A");
    assert!(
        !PinnedBead::from_row(&row).has_favicon,
        "the default image is empty"
    );
    row.favicon = Image::from_rgba8(SharedPixelBuffer::<Rgba8Pixel>::new(16, 16));
    let pin = PinnedBead::from_row(&row);
    assert!(pin.has_favicon);
    assert_eq!(pin.favicon.size().width, 16);
}

// ---- hub button: label --------------------------------------------------------------------

fn mockup_groups() -> Vec<GroupRow> {
    ring_fixtures::mockup_groups(false)
}

#[test]
fn the_label_names_the_groups_and_tabs() {
    assert_eq!(label(&mockup_groups()), "Tab groups, 6 groups, 22 tabs");
}

#[test]
fn the_label_is_singular_for_one_group_and_one_tab() {
    let mut groups = mockup_groups();
    groups.truncate(1);
    assert_eq!(label(&groups), "Tab groups, 1 group, 5 tabs");
    groups[0].count = 1;
    assert_eq!(label(&groups), "Tab groups, 1 group, 1 tab");
}

#[test]
fn the_label_with_nothing_says_no_tabs() {
    assert_eq!(label(&[]), "Tab groups, no tabs");
    let mut groups = mockup_groups();
    for group in &mut groups {
        group.count = 0;
    }
    assert_eq!(label(&groups), "Tab groups, no tabs");
}

// ---- hub button: donut --------------------------------------------------------------------

const FALLBACK: Color = Color::from_rgb_u8(0x77, 0x77, 0x77);
const CENTRE_PX: (f32, f32) = (15.0, 15.0);
const DONUT_RADIUS_OUT: f32 = 14.0;

/// The numbers of a path text, in order.
fn numbers(text: &str) -> Vec<f32> {
    let mut out = Vec::new();
    let mut current = String::new();
    for c in text.chars() {
        let part_of_number = c.is_ascii_digit() || c == '.' || (c == '-' && current.is_empty());
        if part_of_number {
            current.push(c);
        } else if !current.is_empty() {
            out.push(current.parse().unwrap());
            current.clear();
        }
    }
    if !current.is_empty() {
        out.push(current.parse().unwrap());
    }
    out
}

/// Degrees clockwise from 12 o'clock of a point of the 30 px box.
fn angle_of(x: f32, y: f32) -> f32 {
    let deg = (x - CENTRE_PX.0).atan2(-(y - CENTRE_PX.1)).to_degrees();
    deg.rem_euclid(360.0)
}

/// (start, end) of the outer edge of an arc path, in degrees clockwise from 12 o'clock, `end`
/// unwrapped to be after `start`, plus the number of outer `A` commands.
fn arc_span(commands: &str) -> (f32, f32, usize) {
    let outer = commands.split('L').next().unwrap();
    let nums = numbers(outer);
    // M x y, then per A: rx ry large sweep x y.
    let start = angle_of(nums[0], nums[1]);
    let n = nums.len();
    let end_raw = angle_of(nums[n - 2], nums[n - 1]);
    let arcs = outer.matches('A').count();
    let mut end = end_raw;
    if end <= start + 1e-3 {
        end += 360.0;
    }
    (start, end, arcs)
}

fn group_with(count: i32, colour: Color, id: i32) -> GroupRow {
    GroupRow {
        id,
        count,
        colour,
        ..GroupRow::default()
    }
}

#[test]
fn the_button_is_30_px() {
    assert_eq!(BUTTON_SIZE, 30.0);
}

#[test]
fn no_groups_is_one_grey_ring() {
    let arcs = donut(&[], FALLBACK);
    assert_eq!(arcs.len(), 1);
    assert_eq!(arcs[0].colour, FALLBACK);
    assert!(!arcs[0].commands.is_empty());
    let (_, _, outer_arcs) = arc_span(arcs[0].commands.as_str());
    assert_eq!(outer_arcs, 2, "a full ring is written as two half arcs");
}

#[test]
fn groups_without_tabs_count_as_none() {
    let groups = [group_with(0, Color::from_rgb_u8(9, 9, 9), 1)];
    let arcs = donut(&groups, FALLBACK);
    assert_eq!(arcs.len(), 1);
    assert_eq!(arcs[0].colour, FALLBACK);
}

#[test]
fn one_group_is_a_full_ring_in_its_colour() {
    let colour = Color::from_rgb_u8(0x26, 0x55, 0xA0);
    let arcs = donut(&[group_with(5, colour, 1)], FALLBACK);
    assert_eq!(arcs.len(), 1);
    assert_eq!(arcs[0].colour, colour);
    let commands = arcs[0].commands.as_str();
    let (start, _, outer_arcs) = arc_span(commands);
    assert!(
        start < 0.5 || start > 359.5,
        "starts at 12 o'clock, was {start}"
    );
    assert_eq!(outer_arcs, 2, "no gap, two half arcs: {commands}");
    assert!(commands.ends_with('Z'));
}

#[test]
fn the_ring_has_the_buttons_radii() {
    // Centre (15, 15), ring radius 11.5, width 5: outer 14, inner 9, inside the 30 px box.
    let arcs = donut(&[group_with(3, Color::from_rgb_u8(1, 1, 1), 1)], FALLBACK);
    let nums = numbers(arcs[0].commands.as_str());
    let (x, y) = (nums[0], nums[1]);
    assert!(
        (distance((x, y), CENTRE_PX) - DONUT_RADIUS_OUT).abs() < 0.05,
        "outer edge at 14 px, start point {x} {y}"
    );
    let inner_start = arcs[0].commands.split('L').nth(1).unwrap();
    let inner = numbers(inner_start);
    assert!(
        (distance((inner[0], inner[1]), CENTRE_PX) - 9.0).abs() < 0.05,
        "inner edge at 9 px"
    );
}

#[test]
fn the_mockup_groups_are_arcs_sized_by_tab_count_with_4_degree_gaps() {
    let groups = mockup_groups();
    let arcs = donut(&groups, FALLBACK);
    assert_eq!(arcs.len(), 6);
    let counts: Vec<f32> = groups.iter().map(|g| g.count as f32).collect();
    let total: f32 = counts.iter().sum();
    let spans: Vec<(f32, f32, usize)> = arcs
        .iter()
        .map(|arc| arc_span(arc.commands.as_str()))
        .collect();

    let share = 360.0 - 6.0 * 4.0;
    let mut sweep_sum = 0.0;
    for (i, &(start, end, _)) in spans.iter().enumerate() {
        let sweep = end - start;
        sweep_sum += sweep;
        let want = share * counts[i] / total;
        assert!(
            (sweep - want).abs() < 0.5,
            "group {i}: sweep {sweep}, proportional share {want}"
        );
        assert_eq!(
            arcs[i].colour, groups[i].colour,
            "group {i} keeps its colour"
        );
    }
    assert!(
        (sweep_sum - share).abs() < 0.5,
        "arcs sum to 360 minus 6 gaps of 4: {sweep_sum}"
    );
    assert!(spans[0].0 < 0.5, "the first arc starts at 12 o'clock");
    for pair in spans.windows(2) {
        let gap = pair[1].0 - pair[0].1;
        assert!((gap - 4.0).abs() < 0.5, "gap between arcs {gap}");
    }
    let closing_gap = 360.0 - spans[5].1 + spans[0].0;
    assert!(
        (closing_gap - 4.0).abs() < 0.5,
        "gap after the last arc {closing_gap}"
    );
}

#[test]
fn two_groups_get_two_gaps() {
    let groups = [
        group_with(3, Color::from_rgb_u8(255, 0, 0), 1),
        group_with(1, Color::from_rgb_u8(0, 0, 255), 2),
    ];
    let arcs = donut(&groups, FALLBACK);
    assert_eq!(arcs.len(), 2);
    let first = arc_span(arcs[0].commands.as_str());
    let second = arc_span(arcs[1].commands.as_str());
    assert!((first.1 - first.0 - 352.0 * 0.75).abs() < 0.5);
    assert!((second.1 - second.0 - 352.0 * 0.25).abs() < 0.5);
    assert!((first.1 - first.0) + (second.1 - second.0) > 351.0);
}

#[test]
fn groups_without_tabs_are_left_out_of_the_donut() {
    let groups = [
        group_with(2, Color::from_rgb_u8(255, 0, 0), 1),
        group_with(0, Color::from_rgb_u8(0, 255, 0), 2),
        group_with(2, Color::from_rgb_u8(0, 0, 255), 3),
    ];
    let arcs = donut(&groups, FALLBACK);
    assert_eq!(arcs.len(), 2);
    assert_eq!(arcs[0].colour, Color::from_rgb_u8(255, 0, 0));
    assert_eq!(arcs[1].colour, Color::from_rgb_u8(0, 0, 255));
    let (s, e, _) = arc_span(arcs[0].commands.as_str());
    assert!(
        (e - s - 176.0).abs() < 0.5,
        "two groups: (360 - 8) / 2 each"
    );
}

#[test]
fn sixty_four_groups_make_sixty_four_thin_arcs_that_still_add_up() {
    let (groups, _) = ring_fixtures::generated(64, 130, false);
    let arcs = donut(&groups, FALLBACK);
    assert_eq!(arcs.len(), 64);
    let total: f32 = groups.iter().map(|g| g.count as f32).sum();
    let share = 360.0 - 64.0 * 4.0;
    let mut sweep_sum = 0.0;
    let mut previous_end = None;
    for (i, arc) in arcs.iter().enumerate() {
        let (start, end, _) = arc_span(arc.commands.as_str());
        let sweep = end - start;
        let want = share * groups[i].count as f32 / total;
        assert!((sweep - want).abs() < 0.5, "arc {i}: {sweep} vs {want}");
        assert!(sweep > 0.0);
        if let Some(prev) = previous_end {
            let gap: f32 = start - prev;
            assert!((gap - 4.0).abs() < 0.5, "arc {i}: gap {gap}");
        }
        previous_end = Some(end);
        sweep_sum += sweep;
    }
    assert!(
        (sweep_sum - share).abs() < 0.5,
        "sum {sweep_sum}, want {share}"
    );
}

#[test]
fn a_crowd_of_groups_never_overlaps_or_goes_negative() {
    let groups: Vec<GroupRow> = (0..200)
        .map(|i| group_with(1 + i % 3, Color::from_rgb_u8(1, 2, 3), i))
        .collect();
    let arcs = donut(&groups, FALLBACK);
    assert_eq!(arcs.len(), 200);
    let mut cursor = -0.5;
    for arc in &arcs {
        let (start, end, _) = arc_span(arc.commands.as_str());
        assert!(end > start, "positive sweep");
        assert!(start >= cursor - 0.5, "arcs in order, no overlap");
        cursor = end;
    }
    assert!(cursor <= 360.5, "everything inside one turn");
}

#[test]
fn the_donut_is_deterministic() {
    let groups = mockup_groups();
    let a = donut(&groups, FALLBACK);
    let b = donut(&groups, FALLBACK);
    assert_eq!(
        a.iter().map(|x| x.commands.to_string()).collect::<Vec<_>>(),
        b.iter().map(|x| x.commands.to_string()).collect::<Vec<_>>()
    );
}
