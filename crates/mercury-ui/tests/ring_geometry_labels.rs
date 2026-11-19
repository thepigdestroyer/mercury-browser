// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! The label path across the ring geometry modules, without a window: a real `RingLayout` feeds
//! `label_glyphs`, the placed glyphs are hit-tested and converted to the Slint `ArcGlyph`, and the
//! hand-derived label extents of the label unit tests are checked against what the layout computes.

use mercury_ui::ring_geometry::hit::hit_test;
use mercury_ui::ring_geometry::labels::{
    ApproxMetrics, LabelFit, LabelRequest, LabelStyle, is_flipped, label_glyphs,
};
use mercury_ui::ring_geometry::slint_models::{GlyphColours, glyph_to_slint};
use mercury_ui::ring_geometry::{GlyphRole, Hit, Point, RingLayout, RingSpec, normalise_deg};
use slint::Color;

/// The 22-tab mockup ring (`ring-open-light`): six groups of 5, 4, 6, 3, 2 and 2 tabs.
const MOCKUP_GROUPS: [usize; 6] = [5, 4, 6, 3, 2, 2];
const MOCKUP_NAMES: [&str; 6] = [
    "Research",
    "Lisbon trip",
    "Reading",
    "Shopping",
    "News",
    "Music",
];
const MOCKUP_KEYS: [Option<char>; 6] =
    [Some('R'), Some('L'), Some('E'), Some('S'), None, Some('M')];

fn mockup() -> (RingSpec, RingLayout) {
    let spec = RingSpec::hub(Point::new(338.0, 342.0));
    let layout = RingLayout::new(spec, &MOCKUP_GROUPS);
    (spec, layout)
}

/// The ring angle of a ring-local point, degrees clockwise from 12 o'clock in `[0, 360)`.
fn angle_of(spec: &RingSpec, p: Point) -> f32 {
    normalise_deg(
        (p.x - spec.centre.x)
            .atan2(spec.centre.y - p.y)
            .to_degrees(),
    )
}

fn radius_of(spec: &RingSpec, p: Point) -> f32 {
    (p.x - spec.centre.x).hypot(p.y - spec.centre.y)
}

fn request(name: &str, key: Option<char>, count: usize) -> LabelRequest<'_> {
    LabelRequest {
        name,
        key,
        count,
        style: LabelStyle::Rest,
    }
}

#[test]
fn the_label_extents_of_the_label_unit_tests_are_what_the_layout_computes() {
    // `labels.rs` derived these three by hand from the mockup rules; the layout must agree.
    let (_, layout) = mockup();
    let expected = [
        (1, 89.445, 144.282),
        (2, 150.082, 232.736),
        (3, 238.536, 279.464),
    ];
    for (g, start, end) in expected {
        let arc = &layout.groups[g];
        assert!(
            (arc.label_start_deg - start).abs() < 0.01,
            "group {g} label start {} vs {start}",
            arc.label_start_deg
        );
        assert!(
            (arc.label_end_deg - end).abs() < 0.01,
            "group {g} label end {} vs {end}",
            arc.label_end_deg
        );
    }
}

#[test]
fn the_mockup_labels_flip_where_the_mockup_flips_them() {
    // Lisbon trip (mid 116.9) and Reading (mid 191.5) read counter-clockwise, Shopping (259) not.
    let (spec, layout) = mockup();
    let flipped: Vec<bool> = layout
        .groups
        .iter()
        .enumerate()
        .map(|(g, arc)| {
            label_glyphs(
                arc,
                &spec,
                &request(MOCKUP_NAMES[g], MOCKUP_KEYS[g], MOCKUP_GROUPS[g]),
                &ApproxMetrics,
            )
            .flipped
        })
        .collect();
    assert!(flipped[1], "Lisbon trip is flipped");
    assert!(flipped[2], "Reading is flipped");
    assert!(!flipped[3], "Shopping is not flipped");
    for (g, arc) in layout.groups.iter().enumerate() {
        let mid = (arc.label_start_deg + arc.label_end_deg) / 2.0;
        assert_eq!(flipped[g], is_flipped(mid), "group {g}");
    }
}

#[test]
fn every_glyph_of_the_mockup_sits_in_its_own_label_band_and_hits_its_group() {
    let (spec, layout) = mockup();
    let mut total = 0;
    for (g, arc) in layout.groups.iter().enumerate() {
        let label = label_glyphs(
            arc,
            &spec,
            &request(MOCKUP_NAMES[g], MOCKUP_KEYS[g], MOCKUP_GROUPS[g]),
            &ApproxMetrics,
        );
        assert_ne!(
            label.fit,
            LabelFit::Hidden,
            "group {g} has room for a label"
        );
        assert!(!label.glyphs.is_empty(), "group {g}");
        let start = arc.label_start_deg;
        let end = arc.label_end_deg;
        for glyph in &label.glyphs {
            total += 1;
            assert!(
                (radius_of(&spec, glyph.centre) - spec.label_radius()).abs() < 0.01,
                "group {g} glyph {:?} is on the label radius",
                glyph.text
            );
            // Compare as an offset from the label start so that wrap-around needs no case.
            let offset = normalise_deg(angle_of(&spec, glyph.centre) - start);
            assert!(
                offset >= -0.01 && offset <= end - start + 0.01,
                "group {g} glyph {:?} at offset {offset} lies inside the label extent {}",
                glyph.text,
                end - start
            );
            assert_eq!(
                hit_test(&layout, glyph.centre, None),
                Hit::Group(g),
                "group {g} glyph {:?} hits its own group",
                glyph.text
            );
        }
    }
    assert!(
        total > 40,
        "the mockup writes more than 40 glyphs, got {total}"
    );
}

#[test]
fn glyphs_read_in_order_along_the_ring_and_the_flipped_ones_run_the_other_way() {
    let (spec, layout) = mockup();
    for (g, arc) in layout.groups.iter().enumerate() {
        let label = label_glyphs(
            arc,
            &spec,
            &request(MOCKUP_NAMES[g], MOCKUP_KEYS[g], MOCKUP_GROUPS[g]),
            &ApproxMetrics,
        );
        let start = arc.label_start_deg;
        let offsets: Vec<f32> = label
            .glyphs
            .iter()
            .map(|glyph| normalise_deg(angle_of(&spec, glyph.centre) - start))
            .collect();
        for pair in offsets.windows(2) {
            if label.flipped {
                assert!(
                    pair[0] > pair[1],
                    "group {g}: flipped glyphs run counter-clockwise"
                );
            } else {
                assert!(pair[0] < pair[1], "group {g}: glyphs run clockwise");
            }
        }
        // Upright on the lower half: the rotation is the glyph's angle, minus 180 when flipped.
        for glyph in &label.glyphs {
            let angle = angle_of(&spec, glyph.centre);
            let expected = if label.flipped { angle - 180.0 } else { angle };
            let diff = normalise_deg(glyph.angle_deg - expected);
            assert!(
                diff < 0.01 || diff > 359.99,
                "group {g} glyph {:?}: rotation {} vs {expected}",
                glyph.text,
                glyph.angle_deg
            );
        }
    }
}

#[test]
fn the_slint_glyphs_carry_the_role_colours_and_the_placement() {
    let (spec, layout) = mockup();
    let colours = GlyphColours {
        name: Color::from_rgb_u8(10, 20, 30),
        key: Color::from_rgb_u8(40, 50, 60),
        count: Color::from_rgb_u8(70, 80, 90),
    };
    let label = label_glyphs(
        &layout.groups[0],
        &spec,
        &request("Research", Some('R'), 5),
        &ApproxMetrics,
    );
    assert_eq!(
        label.fit,
        LabelFit::Full,
        "Research shows name, key and count"
    );
    let mut roles = [0usize; 3];
    for glyph in &label.glyphs {
        let slint = glyph_to_slint(glyph, &colours);
        assert_eq!(slint.text.as_str(), glyph.text);
        assert_eq!((slint.x, slint.y), (glyph.centre.x, glyph.centre.y));
        assert_eq!(slint.angle, glyph.angle_deg);
        assert_eq!(slint.size, glyph.size);
        assert_eq!(slint.weight, i32::from(glyph.weight));
        let (slot, colour) = match glyph.role {
            GlyphRole::Name => (0, colours.name),
            GlyphRole::Key => (1, colours.key),
            GlyphRole::Count => (2, colours.count),
        };
        roles[slot] += 1;
        assert_eq!(slint.colour, colour);
    }
    assert_eq!(roles[0], "Research".chars().count(), "one glyph per letter");
    assert_eq!(roles[1], 2, "the option symbol and the key letter");
    assert_eq!(roles[2], 1, "one digit for a count of 5");
}

#[test]
fn a_crowded_ring_hides_labels_without_panicking_and_every_drawn_glyph_still_hits_its_group() {
    // 64 groups of one tab: the narrowest bands the product allows.
    let spec = RingSpec::hub(Point::new(258.0, 258.0));
    let layout = RingLayout::new(spec, &[1; 64]);
    assert_eq!(layout.groups.len(), 64);
    let mut hidden = 0;
    let mut shown = 0;
    for (g, arc) in layout.groups.iter().enumerate() {
        let name = format!("Group number {g}");
        let label = label_glyphs(
            arc,
            &spec,
            &request(&name, char::from_digit(g as u32 % 10, 10), 1),
            &ApproxMetrics,
        );
        if label.fit == LabelFit::Hidden {
            hidden += 1;
            assert!(label.glyphs.is_empty(), "a hidden label has no glyphs");
            continue;
        }
        shown += 1;
        for glyph in &label.glyphs {
            assert!(glyph.centre.x.is_finite() && glyph.centre.y.is_finite());
            assert_eq!(
                hit_test(&layout, glyph.centre, None),
                Hit::Group(g),
                "group {g} glyph {:?}",
                glyph.text
            );
        }
    }
    assert_eq!(hidden + shown, 64);
    assert!(
        hidden > 0,
        "64 groups on one ring do not all have room for a label"
    );
}

#[test]
fn the_held_label_on_the_real_layout_shows_the_key_letter_large() {
    let (spec, layout) = mockup();
    let held = LabelRequest {
        name: "Lisbon trip",
        key: Some('L'),
        count: 4,
        style: LabelStyle::Held,
    };
    let label = label_glyphs(&layout.groups[1], &spec, &held, &ApproxMetrics);
    assert_ne!(label.fit, LabelFit::Hidden);
    let key: Vec<_> = label
        .glyphs
        .iter()
        .filter(|glyph| glyph.role == GlyphRole::Key)
        .collect();
    assert_eq!(key.len(), 1, "the held label writes the key letter alone");
    assert_eq!(key[0].text, "L");
    assert_eq!(key[0].size, 15.0);
    assert!(
        label
            .glyphs
            .iter()
            .all(|glyph| glyph.role != GlyphRole::Count),
        "the held label has no count"
    );
}
