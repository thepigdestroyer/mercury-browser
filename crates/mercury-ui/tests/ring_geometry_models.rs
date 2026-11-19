// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! The ring geometry's glue to the Slint structs of `ui/groups/models.slint`, without a window:
//! the four SPEC §4.1 structs are built field by field, `RingModel` orders and filters the rows,
//! and the row ids reach `ArcSeg::tab`. The last tests cross the modules: on the 22-tab mockup
//! ring, `hit_test` of the middle of every drawn shape returns the tab or group it draws.

use mercury_ui::ring_geometry::hit::hit_test;
use mercury_ui::ring_geometry::paths;
use mercury_ui::ring_geometry::slint_models::{GlyphColours, RingModel, glyph_to_slint};
use mercury_ui::ring_geometry::{
    GlyphPlacement, GlyphRole, Hit, Point, RingLayout, RingSpec, point_on_circle,
};
use mercury_ui::{ArcGlyph, ArcSeg, GroupRow, TabRow};
use slint::Color;

fn group(id: i32) -> GroupRow {
    GroupRow {
        id,
        name: format!("Group {id}").into(),
        ..Default::default()
    }
}

fn tab(id: i32, group: i32, index_in_group: i32) -> TabRow {
    TabRow {
        id,
        group,
        index_in_group,
        ..Default::default()
    }
}

fn spec() -> RingSpec {
    RingSpec::default()
}

/// The ids of the tabs in ring order.
fn tab_ids(model: &RingModel) -> Vec<i32> {
    (0..model.layout.tabs.len())
        .map(|flat| model.tab_id(flat).expect("every ring position has a row"))
        .collect()
}

// ---- the four structs of SPEC 4.1

#[test]
fn the_four_slint_structs_are_built_with_every_field() {
    // No `..Default::default()` on purpose: a field added to or removed from `models.slint`
    // without the SPEC and these tests noticing fails to compile here.
    let colour = Color::from_rgb_u8(0x2e, 0x7d, 0x6b);
    let group = GroupRow {
        id: 3,
        name: "Research".into(),
        colour,
        tint: Color::from_rgb_u8(0xe3, 0xf1, 0xed),
        deep: Color::from_rgb_u8(0x1b, 0x5e, 0x4f),
        key_label: "R".into(),
        count: 5,
        last_used: "2 min ago".into(),
        active: true,
        any_loading: false,
        any_audible: true,
        any_unread: false,
    };
    assert_eq!(
        (group.id, group.count, group.key_label.as_str()),
        (3, 5, "R")
    );
    assert_eq!(group.name.as_str(), "Research");
    assert!(group.active && group.any_audible && !group.any_loading && !group.any_unread);
    assert_eq!(group.colour, colour);

    let tab = TabRow {
        id: 11,
        group: 3,
        index_in_group: 2,
        title: "Rust".into(),
        host: "rust-lang.org".into(),
        shade: colour,
        favicon: slint::Image::default(),
        active: true,
        pinned: false,
        loading: true,
        progress: 0.5,
        audible: false,
        muted: true,
        unread: false,
        crashed: false,
        capture: true,
        matches_query: true,
    };
    assert_eq!((tab.id, tab.group, tab.index_in_group), (11, 3, 2));
    assert_eq!(
        (tab.title.as_str(), tab.host.as_str()),
        ("Rust", "rust-lang.org")
    );
    assert_eq!(tab.progress, 0.5);
    assert!(tab.active && tab.loading && tab.muted && tab.capture && tab.matches_query);
    assert!(!tab.pinned && !tab.audible && !tab.unread && !tab.crashed);

    let seg = ArcSeg {
        tab: 11,
        commands: "M0 0 Z".into(),
        mid_x: 10.5,
        mid_y: 20.25,
    };
    assert_eq!((seg.tab, seg.commands.as_str()), (11, "M0 0 Z"));
    assert_eq!((seg.mid_x, seg.mid_y), (10.5, 20.25));

    let glyph = ArcGlyph {
        text: "R".into(),
        x: 100.0,
        y: 50.0,
        angle: 12.0,
        size: 12.0,
        weight: 600,
        colour,
    };
    assert_eq!(glyph.text.as_str(), "R");
    assert_eq!((glyph.x, glyph.y, glyph.angle), (100.0, 50.0, 12.0));
    assert_eq!(
        (glyph.size, glyph.weight, glyph.colour),
        (12.0, 600, colour)
    );
}

#[test]
fn a_glyph_placement_becomes_an_arc_glyph_with_the_colour_of_its_role() {
    let colours = GlyphColours {
        name: Color::from_rgb_u8(1, 2, 3),
        key: Color::from_rgb_u8(4, 5, 6),
        count: Color::from_rgb_u8(7, 8, 9),
    };
    let placement = GlyphPlacement {
        text: "⌥".to_owned(),
        centre: Point::new(300.25, 40.5),
        angle_deg: 181.5,
        size: 11.0,
        weight: 700,
        role: GlyphRole::Key,
    };
    assert_eq!(
        glyph_to_slint(&placement, &colours),
        ArcGlyph {
            text: "⌥".into(),
            x: 300.25,
            y: 40.5,
            angle: 181.5,
            size: 11.0,
            weight: 700,
            colour: colours.key,
        }
    );
}

// ---- RingModel

#[test]
fn ring_order_follows_the_groups_list_and_not_the_ids() {
    let groups = [group(30), group(10), group(20)];
    let tabs = [
        tab(1, 10, 0),
        tab(2, 30, 0),
        tab(3, 20, 0),
        tab(4, 10, 1),
        tab(5, 30, 1),
        tab(6, 30, 2),
    ];
    let model = RingModel::new(spec(), &groups, &tabs);
    // Group 30 first (tabs 2, 5, 6), then 10 (1, 4), then 20 (3).
    assert_eq!(tab_ids(&model), vec![2, 5, 6, 1, 4, 3]);
    assert_eq!(model.tab_rows, vec![1, 4, 5, 0, 3, 2]);
    let sizes: Vec<usize> = model.layout.groups.iter().map(|g| g.tab_count).collect();
    assert_eq!(sizes, vec![3, 2, 1]);
    let owners: Vec<usize> = model.layout.tabs.iter().map(|t| t.group).collect();
    assert_eq!(owners, vec![0, 0, 0, 1, 1, 2]);
    assert_eq!(model.group_id(0), Some(30));
    assert_eq!(model.group_id(2), Some(20));
}

#[test]
fn the_tabs_of_a_group_are_ordered_by_index_in_group_and_ties_keep_the_input_order() {
    let groups = [group(1)];
    let tabs = [
        tab(50, 1, 2),
        tab(51, 1, 0),
        tab(52, 1, 1),
        tab(53, 1, 1),
        tab(54, 1, -1),
    ];
    let model = RingModel::new(spec(), &groups, &tabs);
    assert_eq!(tab_ids(&model), vec![54, 51, 52, 53, 50]);
    assert_eq!(model.tab_rows, vec![4, 1, 2, 3, 0]);
    let slots: Vec<usize> = model.layout.tabs.iter().map(|t| t.index_in_group).collect();
    assert_eq!(
        slots,
        vec![0, 1, 2, 3, 4],
        "the layout counts positions, not row values"
    );
}

#[test]
fn a_tab_of_an_unknown_group_is_left_out() {
    let groups = [group(1), group(2)];
    let tabs = [tab(10, 1, 0), tab(11, 99, 0), tab(12, 2, 0), tab(13, -5, 1)];
    let model = RingModel::new(spec(), &groups, &tabs);
    assert_eq!(tab_ids(&model), vec![10, 12]);
    assert_eq!(model.tab_rows, vec![0, 2]);
    assert_eq!(model.layout.tabs.len(), 2);
    assert_eq!(model.segments_for_slint().len(), 2);
    assert_eq!(model.tab_id(2), None);
}

#[test]
fn tabs_with_no_group_at_all_leave_an_empty_ring() {
    let tabs = [tab(1, 1, 0), tab(2, 1, 1)];
    let model = RingModel::new(spec(), &[], &tabs);
    assert!(model.layout.tabs.is_empty() && model.layout.groups.is_empty());
    assert!(model.tab_rows.is_empty());
    assert!(model.segments_for_slint().is_empty());
    assert!(model.label_bands_for_slint().is_empty());
    assert!(model.lifted_segment_for_slint(0, 10.0).is_none());
}

#[test]
fn groups_without_tabs_stay_in_the_list_but_get_no_band() {
    let groups = [group(7), group(8), group(9)];
    let tabs = [tab(1, 7, 0), tab(2, 9, 0), tab(3, 9, 1)];
    let model = RingModel::new(spec(), &groups, &tabs);
    assert_eq!(model.layout.groups.len(), 3);
    assert_eq!(model.layout.groups[1].tab_count, 0);
    // The group index of the empty group still maps to its row id.
    assert_eq!(model.group_id(1), Some(8));
    let bands: Vec<i32> = model
        .label_bands_for_slint()
        .iter()
        .map(|b| b.tab)
        .collect();
    assert_eq!(
        bands,
        vec![7, 9],
        "bands carry group ids, and none for the empty group"
    );
}

#[test]
fn a_repeated_group_id_gives_its_tabs_to_the_first_group_only() {
    let groups = [group(5), group(5), group(6)];
    let tabs = [tab(1, 5, 0), tab(2, 6, 0), tab(3, 5, 1)];
    let model = RingModel::new(spec(), &groups, &tabs);
    assert_eq!(tab_ids(&model), vec![1, 3, 2]);
    let sizes: Vec<usize> = model.layout.groups.iter().map(|g| g.tab_count).collect();
    assert_eq!(sizes, vec![2, 0, 1]);
}

#[test]
fn row_ids_are_carried_into_arc_seg_tab() {
    // Ids that look nothing like positions, so an index leaking through shows.
    let groups = [group(700), group(300), group(500)];
    let tabs = [
        tab(9001, 300, 0),
        tab(9002, 700, 1),
        tab(9003, 700, 0),
        tab(9004, 500, 0),
        tab(9005, 300, 1),
    ];
    let model = RingModel::new(spec(), &groups, &tabs);

    let segments = model.segments_for_slint();
    let plain = paths::segments(&model.layout);
    assert_eq!(segments.len(), 5);
    let ids: Vec<i32> = segments.iter().map(|s| s.tab).collect();
    assert_eq!(ids, vec![9003, 9002, 9001, 9005, 9004]);
    for (seg, geo) in segments.iter().zip(&plain) {
        assert_eq!(seg.commands.as_str(), geo.commands);
        assert_eq!((seg.mid_x, seg.mid_y), (geo.mid.x, geo.mid.y));
    }

    let bands = model.label_bands_for_slint();
    let band_ids: Vec<i32> = bands.iter().map(|b| b.tab).collect();
    assert_eq!(band_ids, vec![700, 300, 500]);
    let plain_bands = paths::label_bands(&model.layout);
    for (band, geo) in bands.iter().zip(&plain_bands) {
        assert_eq!(band.commands.as_str(), geo.commands);
        assert_eq!((band.mid_x, band.mid_y), (geo.mid.x, geo.mid.y));
    }

    // A hovered tab keeps its id and its anchor, and grows outward.
    let lifted = model.lifted_segment_for_slint(3, 10.0).expect("flat tab 3");
    assert_eq!(lifted.tab, 9005);
    assert_eq!(
        (lifted.mid_x, lifted.mid_y),
        (segments[3].mid_x, segments[3].mid_y)
    );
    assert_ne!(lifted.commands, segments[3].commands);
}

#[test]
fn sixty_four_groups_of_every_size_make_a_valid_model() {
    // Group ids run backwards so the ring order is not the id order.
    let groups: Vec<GroupRow> = (0..64).map(|i| group(1000 - i)).collect();
    let mut tabs = Vec::new();
    let mut next_id = 50_000;
    for (i, row) in groups.iter().enumerate() {
        for slot in 0..(i % 4 + 1) as i32 {
            tabs.push(tab(next_id, row.id, slot));
            next_id += 1;
        }
    }
    let model = RingModel::new(spec(), &groups, &tabs);
    assert_eq!(model.layout.groups.len(), 64);
    assert_eq!(model.layout.tabs.len(), tabs.len());
    assert_eq!(model.tab_rows.len(), tabs.len());

    let segments = model.segments_for_slint();
    assert_eq!(segments.len(), tabs.len());
    let mut ids: Vec<i32> = segments.iter().map(|s| s.tab).collect();
    ids.sort_unstable();
    ids.dedup();
    assert_eq!(ids.len(), tabs.len(), "every tab id appears once");
    for seg in &segments {
        assert!(seg.commands.as_str().starts_with('M') && seg.commands.as_str().ends_with('Z'));
        assert!(seg.mid_x.is_finite() && seg.mid_y.is_finite());
    }

    let bands = model.label_bands_for_slint();
    assert_eq!(bands.len(), 64);
    let band_ids: Vec<i32> = bands.iter().map(|b| b.tab).collect();
    let want: Vec<i32> = groups.iter().map(|g| g.id).collect();
    assert_eq!(band_ids, want);
}

// ---- cross-module golden test: what is drawn is what is hit

/// The hub ring of the mockup `ring-open` (SVG centre (338, 342)): 22 tabs in groups of
/// 5, 4, 6, 3, 2 and 2.
fn mockup_spec() -> RingSpec {
    RingSpec {
        centre: Point::new(338.0, 342.0),
        ..RingSpec::default()
    }
}

const MOCKUP_GROUPS: [usize; 6] = [5, 4, 6, 3, 2, 2];

#[test]
fn the_middle_of_every_drawn_segment_and_label_band_hits_what_it_draws() {
    let layout = RingLayout::new(mockup_spec(), &MOCKUP_GROUPS);
    let segments = paths::segments(&layout);
    assert_eq!(segments.len(), 22);
    for (index, segment) in segments.iter().enumerate() {
        assert_eq!(segment.tab, index);
        assert_eq!(
            hit_test(&layout, segment.mid, None),
            Hit::Tab(index),
            "the middle of segment {index} at {:?}",
            segment.mid
        );
    }
    let bands = paths::label_bands(&layout);
    assert_eq!(bands.len(), 6);
    for band in &bands {
        assert_eq!(
            hit_test(&layout, band.mid, None),
            Hit::Group(band.tab),
            "the middle of the label band of group {} at {:?}",
            band.tab,
            band.mid
        );
    }
}

#[test]
fn the_inside_corners_of_every_drawn_segment_hit_its_tab_and_the_gaps_between_groups_hit_nothing() {
    let layout = RingLayout::new(mockup_spec(), &MOCKUP_GROUPS);
    let spec = layout.spec;
    let at = |radius: f32, deg: f32| point_on_circle(spec.centre, radius, deg);
    // A hair inside each of the four corners of the drawn shape (0.02 degrees, 0.2 px).
    for arc in &layout.tabs {
        for deg in [arc.start_deg + 0.02, arc.end_deg - 0.02] {
            for radius in [spec.inner_radius() + 0.2, spec.outer_radius() - 0.2] {
                assert_eq!(
                    hit_test(&layout, at(radius, deg), None),
                    Hit::Tab(arc.tab),
                    "corner of tab {} at {deg} degrees, radius {radius}",
                    arc.tab
                );
            }
        }
    }
    // The middle of the gap after each group (the last one is followed by the reserve) is
    // nothing, on the tab band and on the label band.
    for pair in layout.groups.windows(2) {
        let gap_mid = (pair[0].end_deg + pair[1].start_deg) / 2.0;
        assert_eq!(
            hit_test(&layout, at(spec.radius, gap_mid), None),
            Hit::Outside
        );
        assert_eq!(
            hit_test(&layout, at(spec.label_radius(), gap_mid), None),
            Hit::Outside
        );
    }
}

#[test]
fn the_ids_of_the_slint_shapes_lead_back_to_the_same_rows_through_the_hit_test() {
    // Six groups with ids that are not their positions, 22 tabs listed in a shuffled row order.
    let group_ids = [40, 7, 93, 12, 61, 5];
    let groups: Vec<GroupRow> = group_ids.iter().map(|&id| group(id)).collect();
    let mut tabs = Vec::new();
    let mut next_id = 700;
    for (position, &size) in MOCKUP_GROUPS.iter().enumerate() {
        for slot in 0..size as i32 {
            tabs.push(tab(next_id, group_ids[position], slot));
            next_id += 1;
        }
    }
    tabs.reverse();
    let model = RingModel::new(mockup_spec(), &groups, &tabs);
    assert_eq!(model.layout.tabs.len(), 22);

    let shapes = model.segments_for_slint();
    assert_eq!(shapes.len(), 22);
    for shape in &shapes {
        let Hit::Tab(flat) = hit_test(&model.layout, Point::new(shape.mid_x, shape.mid_y), None)
        else {
            panic!("the middle of tab {} hits a tab", shape.tab)
        };
        assert_eq!(
            model.tab_id(flat),
            Some(shape.tab),
            "tab at ring position {flat}"
        );
        // The row that the ring position came from carries the same id.
        assert_eq!(tabs[model.tab_rows[flat]].id, shape.tab);
    }
    for band in &model.label_bands_for_slint() {
        let Hit::Group(g) = hit_test(&model.layout, Point::new(band.mid_x, band.mid_y), None)
        else {
            panic!("the middle of the band of group {} hits a group", band.tab)
        };
        assert_eq!(model.group_id(g), Some(band.tab), "group at ring index {g}");
        assert_eq!(groups[g].id, band.tab);
    }
}
