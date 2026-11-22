// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! The data of the dial's fan (lane corner-dial, item 1): `DialFan` over `RingBandsData` with the
//! corner's `RingSpec`, the box origin and the hover caption, on the owner-approved mockup rows
//! (`ring_bands::fixtures`, needs `--features test-support`).
//!
//! Most tests read the real arrays of `RingBandsData::new` (segments, mid-points, hit test); with
//! only the ring-bands scaffold (its `new` returning empty arrays) they fail, which says nothing
//! about the dial.

use mercury_settings::DialCorner;
use mercury_ui::corner_dial::{DialFan, DialGeometry, FAN_BOX, fan_start_deg};
use mercury_ui::ring_bands::{RingBandsData, RingOptions, fixtures};
use mercury_ui::ring_geometry::{ApproxMetrics, Point};
use mercury_ui::{DialCaption, RingHitKind};

const CORNERS: [DialCorner; 4] = [
    DialCorner::TopLeft,
    DialCorner::TopRight,
    DialCorner::BottomLeft,
    DialCorner::BottomRight,
];

const CAPTION_WIDTH: f32 = 186.0;
const CAPTION_HEIGHT: f32 = 30.0;

/// "Essay on maps" in the group "Reading" (ids 101 to 122 in ring order: Research 101-105, Lisbon
/// trip 106-109, Reading 110-115).
const ESSAY: i32 = 111;

fn area() -> DialGeometry {
    DialGeometry::new(1440.0, 900.0)
}

fn mockup_fan(corner: DialCorner) -> DialFan {
    DialFan::new(
        &area(),
        corner,
        &fixtures::mockup_groups(false),
        &fixtures::mockup_tabs(false),
        &RingOptions::default(),
        &ApproxMetrics,
    )
}

fn bearing(dx: f32, dy: f32) -> f32 {
    dx.atan2(-dy).to_degrees().rem_euclid(360.0)
}

fn assert_parallel(data: &RingBandsData, tabs: usize, groups: usize) {
    assert_eq!(data.tabs.len(), tabs, "tabs");
    assert_eq!(data.segments.len(), tabs, "segments");
    assert_eq!(data.lifted_segments.len(), tabs, "lifted segments");
    assert_eq!(data.segment_labels.len(), tabs, "segment labels");
    assert_eq!(data.groups.len(), groups, "groups");
    assert_eq!(data.label_bands.len(), groups, "label bands");
    for (segment, tab) in data.segments.iter().zip(&data.tabs) {
        assert_eq!(segment.tab, tab.id, "segments[i] belongs to tabs[i]");
    }
}

// ----- origin and orientation (no ring data needed) ------------------------------------------

#[test]
fn the_box_origin_is_the_dials_centre_minus_384_for_each_corner() {
    for (w, h) in [(1440.0, 900.0), (1000.0, 700.0), (140.0, 140.0)] {
        let g = DialGeometry::new(w, h);
        for corner in CORNERS {
            let fan = DialFan::new(
                &g,
                corner,
                &[],
                &[],
                &RingOptions::default(),
                &ApproxMetrics,
            );
            let c = g.centre(corner);
            assert_eq!(fan.corner, corner);
            assert_eq!(
                fan.origin,
                Point::new(c.x - 384.0, c.y - 384.0),
                "{w}x{h} {corner:?}"
            );
            assert_eq!(FAN_BOX / 2.0, 384.0);
        }
    }
    assert_eq!(
        mockup_fan(DialCorner::BottomRight).origin,
        Point::new(982.0, 442.0)
    );
    assert_eq!(
        mockup_fan(DialCorner::TopLeft).origin,
        Point::new(-310.0, -310.0)
    );
}

#[test]
fn an_empty_fan_has_no_arrays_and_no_caption() {
    for corner in CORNERS {
        let fan = DialFan::new(
            &area(),
            corner,
            &[],
            &[],
            &RingOptions::default(),
            &ApproxMetrics,
        );
        assert_parallel(&fan.data, 0, 0);
        assert_eq!(fan.caption(0), None);
        assert_eq!(fan.caption(ESSAY), None);
    }
}

#[test]
fn an_unknown_tab_has_no_caption() {
    for corner in CORNERS {
        let fan = mockup_fan(corner);
        for id in [-1, 0, 1, 100, 123, 999, i32::MIN, i32::MAX] {
            assert_eq!(fan.caption(id), None, "{corner:?} tab {id}");
        }
    }
}

// ----- the arrays (need the real ring-bands data) --------------------------------------------

#[test]
fn every_corner_keeps_22_segments_and_6_label_bands_in_ring_order() {
    for corner in CORNERS {
        let fan = mockup_fan(corner);
        assert_parallel(&fan.data, 22, 6);
        let ids: Vec<i32> = fan.data.tabs.iter().map(|t| t.id).collect();
        assert_eq!(
            ids,
            (101..=122).collect::<Vec<i32>>(),
            "{corner:?}: ring order"
        );
        let groups: Vec<i32> = fan.data.groups.iter().map(|g| g.id).collect();
        assert_eq!(groups, [1, 2, 3, 4, 5, 6], "{corner:?}");
    }
}

#[test]
fn every_segment_lies_in_the_span_at_radius_340() {
    for corner in CORNERS {
        let fan = mockup_fan(corner);
        let start = fan_start_deg(corner);
        assert_eq!(fan.data.segments.len(), 22);
        for segment in &fan.data.segments {
            let (dx, dy) = (segment.mid_x - 384.0, segment.mid_y - 384.0);
            let radius = dx.hypot(dy);
            assert!(
                (radius - 340.0).abs() <= 0.5,
                "{corner:?} tab {}: radius {radius}",
                segment.tab
            );
            let offset = (bearing(dx, dy) - start).rem_euclid(360.0);
            assert!(
                (0.0..=98.0).contains(&offset),
                "{corner:?} tab {}: {offset} degrees into the fan",
                segment.tab
            );
        }
        // The segments run clockwise: the offsets grow with the ring order.
        let offsets: Vec<f32> = fan
            .data
            .segments
            .iter()
            .map(|s| (bearing(s.mid_x - 384.0, s.mid_y - 384.0) - start).rem_euclid(360.0))
            .collect();
        assert!(
            offsets.windows(2).all(|w| w[0] < w[1]),
            "{corner:?}: {offsets:?}"
        );
    }
}

#[test]
fn the_hit_test_over_a_segments_mid_point_is_that_tab() {
    for corner in CORNERS {
        let fan = mockup_fan(corner);
        assert_eq!(fan.data.segments.len(), 22);
        for segment in &fan.data.segments {
            let hit = fan.data.hit(segment.mid_x, segment.mid_y, -1);
            assert_eq!(hit.kind, RingHitKind::Tab, "{corner:?} tab {}", segment.tab);
            assert_eq!(hit.id, segment.tab, "{corner:?}");
        }
    }
}

#[test]
fn the_dials_centre_is_the_centre_not_a_tab() {
    for corner in CORNERS {
        let fan = mockup_fan(corner);
        let hit = fan.data.hit(384.0, 384.0, -1);
        assert_eq!(hit.kind, RingHitKind::Centre, "{corner:?}");
        assert_eq!(hit.id, -1);
        // A point at the dial's own radius (r < 326) on the side away from the fan is the centre too.
        let away = fan_start_deg(corner) + 49.0 + 180.0;
        let (sin, cos) = away.to_radians().sin_cos();
        let hit = fan.data.hit(384.0 + 100.0 * sin, 384.0 - 100.0 * cos, -1);
        assert_eq!(hit.kind, RingHitKind::Centre, "{corner:?}");
    }
}

#[test]
fn sixty_four_groups_and_two_hundred_tabs_build_with_parallel_arrays() {
    let (groups, tabs) = fixtures::generated(64, 64, false);
    for corner in CORNERS {
        let fan = DialFan::new(
            &area(),
            corner,
            &groups,
            &tabs,
            &RingOptions::default(),
            &ApproxMetrics,
        );
        assert_parallel(&fan.data, 64, 64);
        for tab in [1000, 1031, 1063] {
            assert!(fan.caption(tab).is_some(), "{corner:?} tab {tab}");
        }
    }
    let (groups, tabs) = fixtures::generated(5, 200, true);
    for corner in CORNERS {
        let fan = DialFan::new(
            &area(),
            corner,
            &groups,
            &tabs,
            &RingOptions::default(),
            &ApproxMetrics,
        );
        assert_parallel(&fan.data, 200, 5);
    }
}

// ----- the caption (needs the real segments) ---------------------------------------------------

fn inside_the_box(caption: &DialCaption) -> bool {
    caption.x >= 0.0
        && caption.y >= 0.0
        && caption.x + CAPTION_WIDTH <= FAN_BOX
        && caption.y + CAPTION_HEIGHT <= FAN_BOX
}

#[test]
fn the_caption_of_the_mockups_hovered_tab_reads_group_dot_title() {
    for corner in CORNERS {
        let caption = mockup_fan(corner)
            .caption(ESSAY)
            .expect("Essay on maps is shown");
        assert_eq!(
            caption.text.as_str(),
            "Reading · Essay on maps",
            "{corner:?}"
        );
    }
}

#[test]
fn the_caption_of_every_tab_names_its_group_and_title() {
    let groups = fixtures::mockup_groups(false);
    let tabs = fixtures::mockup_tabs(false);
    let fan = mockup_fan(DialCorner::BottomRight);
    for tab in &tabs {
        let group = groups.iter().find(|g| g.id == tab.group).expect("group");
        let caption = fan.caption(tab.id).expect("shown");
        assert_eq!(
            caption.text.as_str(),
            format!("{} · {}", group.name, tab.title)
        );
    }
}

#[test]
fn the_caption_sits_24_px_toward_the_dial_in_the_mockups_corner() {
    // gen.swift `dialFinal`: tip(hp.0 + 24, hp.1 + 24, ..) for the bottom-right dial: the top-left
    // corner of the box is 24 px right of and below the segment's mid-point.
    let fan = mockup_fan(DialCorner::BottomRight);
    let segment = fan
        .data
        .segments
        .iter()
        .find(|s| s.tab == ESSAY)
        .expect("segment");
    let caption = fan.caption(ESSAY).expect("caption");
    assert_eq!(caption.x, segment.mid_x + 24.0);
    assert_eq!(caption.y, segment.mid_y + 24.0);
}

#[test]
fn in_every_corner_the_box_lies_on_the_dials_side_of_the_mid_point() {
    // The box's nearest corner is 24 px from the mid-point in both axes, toward the dial (the centre
    // of the fan's box), unless the clamp into the fan's box had to move it.
    for corner in CORNERS {
        let fan = mockup_fan(corner);
        for segment in &fan.data.segments {
            let caption = fan.caption(segment.tab).expect("caption");
            assert!(
                inside_the_box(&caption),
                "{corner:?} tab {}: {caption:?}",
                segment.tab
            );
            let toward_x = if segment.mid_x < 384.0 { 1.0 } else { -1.0 };
            let toward_y = if segment.mid_y < 384.0 { 1.0 } else { -1.0 };
            let unclamped_x = caption.x > 0.0 && caption.x + CAPTION_WIDTH < FAN_BOX;
            let unclamped_y = caption.y > 0.0 && caption.y + CAPTION_HEIGHT < FAN_BOX;
            if unclamped_x {
                let near = if toward_x > 0.0 {
                    caption.x
                } else {
                    caption.x + CAPTION_WIDTH
                };
                assert_eq!(
                    near - segment.mid_x,
                    24.0 * toward_x,
                    "{corner:?} tab {}",
                    segment.tab
                );
            }
            if unclamped_y {
                let near = if toward_y > 0.0 {
                    caption.y
                } else {
                    caption.y + CAPTION_HEIGHT
                };
                assert_eq!(
                    near - segment.mid_y,
                    24.0 * toward_y,
                    "{corner:?} tab {}",
                    segment.tab
                );
            }
        }
    }
}

#[test]
fn the_caption_box_is_inside_the_fan_box_for_the_first_middle_and_last_tab() {
    for corner in CORNERS {
        let fan = mockup_fan(corner);
        for tab in [101, ESSAY, 112, 122] {
            let caption = fan.caption(tab).expect("caption");
            assert!(
                inside_the_box(&caption),
                "{corner:?} tab {tab}: {caption:?}"
            );
        }
    }
}

#[test]
fn many_tabs_keep_every_caption_inside_the_box() {
    let (groups, tabs) = fixtures::generated(64, 64, false);
    for corner in CORNERS {
        let fan = DialFan::new(
            &area(),
            corner,
            &groups,
            &tabs,
            &RingOptions::default(),
            &ApproxMetrics,
        );
        for tab in &tabs {
            let caption = fan.caption(tab.id).expect("caption");
            assert!(
                inside_the_box(&caption),
                "{corner:?} tab {}: {caption:?}",
                tab.id
            );
        }
    }
}

#[test]
fn a_tab_without_a_title_is_named_by_its_host_and_without_either_untitled() {
    let groups = fixtures::mockup_groups(false);
    let mut tabs = fixtures::mockup_tabs(false);
    let essay = tabs.iter().position(|t| t.id == ESSAY).expect("tab");
    tabs[essay].title = "".into();
    let road = tabs.iter().position(|t| t.id == 110).expect("tab");
    tabs[road].title = "".into();
    tabs[road].host = "".into();
    let fan = DialFan::new(
        &area(),
        DialCorner::BottomRight,
        &groups,
        &tabs,
        &RingOptions::default(),
        &ApproxMetrics,
    );
    assert_eq!(
        fan.caption(ESSAY).expect("caption").text.as_str(),
        "Reading · essays.example.net"
    );
    assert_eq!(
        fan.caption(110).expect("caption").text.as_str(),
        "Reading · Untitled"
    );
}
