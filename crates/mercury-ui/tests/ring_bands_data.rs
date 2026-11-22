// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! The data builder of `RingBands` without a window: which rows are shown, the parallel arrays,
//! hit-testing in row ids, the accessible label of every segment, the glyph colours (rest, held,
//! theme), the persistent models and the cost of a build on crowded rings.
//! Needs `--features test-support` for the fixtures.

use std::any::Any;
use std::time::Instant;

use mercury_ui::ring_bands::{RingBandsData, RingBandsModels, RingColours, RingOptions, fixtures};
use mercury_ui::ring_geometry::{ApproxMetrics, Point, RingSpec, normalise_deg, point_on_circle};
use mercury_ui::{ArcGlyph, GroupRow, RingHit, RingHitKind, TabRow};
use slint::{Color, Model};

fn build(groups: &[GroupRow], tabs: &[TabRow]) -> RingBandsData {
    RingBandsData::new(
        RingSpec::default(),
        groups,
        tabs,
        &RingOptions::default(),
        &ApproxMetrics,
    )
}

fn mockup() -> RingBandsData {
    build(
        &fixtures::mockup_groups(false),
        &fixtures::mockup_tabs(false),
    )
}

fn assert_parallel(data: &RingBandsData) {
    let n = data.tabs.len();
    assert_eq!(data.segments.len(), n, "segments");
    assert_eq!(data.lifted_segments.len(), n, "lifted segments");
    assert_eq!(data.segment_labels.len(), n, "segment labels");
    assert_eq!(data.label_bands.len(), data.groups.len(), "label bands");
    assert_eq!(data.model.layout.tabs.len(), n, "layout tabs");
    assert_eq!(
        data.model.layout.groups.len(),
        data.groups.len(),
        "layout groups"
    );
    for (i, tab) in data.tabs.iter().enumerate() {
        assert_eq!(data.segments[i].tab, tab.id, "segments[{i}].tab");
        assert_eq!(data.lifted_segments[i].tab, tab.id, "lifted[{i}].tab");
        assert!(!data.segments[i].commands.is_empty(), "segments[{i}] path");
        assert!(
            !data.lifted_segments[i].commands.is_empty(),
            "lifted[{i}] path"
        );
    }
    for (g, group) in data.groups.iter().enumerate() {
        assert_eq!(data.label_bands[g].tab, group.id, "label_bands[{g}].tab");
        assert!(
            !data.label_bands[g].commands.is_empty(),
            "label band {g} path"
        );
    }
}

/// Everything the component reads is equal. `RingBandsData`'s own `==` is not usable on rows with
/// an empty favicon: Slint's `Image::default() == Image::default()` is false, so the rows are
/// compared by id.
fn assert_same_data(got: &RingBandsData, want: &RingBandsData) {
    let ids = |data: &RingBandsData| data.tabs.iter().map(|t| t.id).collect::<Vec<_>>();
    assert_eq!(ids(got), ids(want), "tabs");
    assert_eq!(got.model, want.model, "model");
    assert_eq!(got.groups, want.groups, "groups");
    assert_eq!(got.segments, want.segments, "segments");
    assert_eq!(got.lifted_segments, want.lifted_segments, "lifted segments");
    assert_eq!(got.segment_labels, want.segment_labels, "segment labels");
    assert_eq!(got.label_bands, want.label_bands, "label bands");
    assert_eq!(got.label_glyphs, want.label_glyphs, "label glyphs");
    assert_eq!(got.flags, want.flags, "flags");
}

fn label_of(data: &RingBandsData, tab_id: i32) -> String {
    let index = data
        .tabs
        .iter()
        .position(|tab| tab.id == tab_id)
        .expect("the tab is shown");
    data.segment_labels[index].to_string()
}

fn with_tab(tabs: &[TabRow], id: i32, change: impl FnOnce(&mut TabRow)) -> Vec<TabRow> {
    let mut tabs = tabs.to_vec();
    change(
        tabs.iter_mut()
            .find(|tab| tab.id == id)
            .expect("a fixture tab"),
    );
    tabs
}

/// The point at `radius` and the middle angle of the tab at flat position `flat`.
fn at_tab(data: &RingBandsData, flat: usize, radius: f32) -> Point {
    let arc = &data.model.layout.tabs[flat];
    point_on_circle(
        data.model.layout.spec.centre,
        radius,
        (arc.start_deg + arc.end_deg) / 2.0,
    )
}

// ---- which rows are shown

#[test]
fn the_mockup_gives_six_bands_and_twenty_two_segments_in_group_order() {
    let data = mockup();
    assert_eq!((data.groups.len(), data.tabs.len()), (6, 22));
    assert_parallel(&data);
    let group_ids: Vec<i32> = data.groups.iter().map(|g| g.id).collect();
    assert_eq!(group_ids, vec![1, 2, 3, 4, 5, 6]);
    let tab_ids: Vec<i32> = data.tabs.iter().map(|t| t.id).collect();
    assert_eq!(tab_ids, (101..=122).collect::<Vec<_>>());
    assert!(!data.label_glyphs.is_empty());
}

#[test]
fn a_shuffled_input_gives_the_same_data() {
    let groups = fixtures::mockup_groups(false);
    let tabs = fixtures::mockup_tabs(false);
    let want = build(&groups, &tabs);

    let mut reversed = tabs.clone();
    reversed.reverse();
    assert_same_data(&build(&groups, &reversed), &want);

    // A fixed permutation: stride 7 is coprime with 22.
    let strided: Vec<TabRow> = (0..tabs.len())
        .map(|i| tabs[(i * 7 + 3) % tabs.len()].clone())
        .collect();
    assert_ne!(
        strided.iter().map(|t| t.id).collect::<Vec<_>>(),
        tabs.iter().map(|t| t.id).collect::<Vec<_>>()
    );
    assert_same_data(&build(&groups, &strided), &want);
}

#[test]
fn the_group_order_of_the_input_is_the_ring_order() {
    let mut groups = fixtures::mockup_groups(false);
    groups.swap(0, 3);
    let data = build(&groups, &fixtures::mockup_tabs(false));
    let ids: Vec<i32> = data.groups.iter().map(|g| g.id).collect();
    assert_eq!(ids, vec![4, 2, 3, 1, 5, 6]);
    assert_parallel(&data);
    assert_eq!(data.tabs[0].id, 116, "Shopping's first tab leads the ring");
}

#[test]
fn a_pinned_tab_is_dropped_and_the_rest_renumber() {
    let groups = fixtures::mockup_groups(false);
    let tabs = with_tab(&fixtures::mockup_tabs(false), 101, |t| t.pinned = true);
    let data = build(&groups, &tabs);
    assert_eq!((data.groups.len(), data.tabs.len()), (6, 21));
    assert_parallel(&data);
    assert!(data.tabs.iter().all(|t| t.id != 101));
    assert_eq!(
        label_of(&data, 102),
        "WebRender overview, Research, tab 1 of 4, loading 64 percent"
    );
}

#[test]
fn a_group_whose_only_tab_is_pinned_loses_its_band() {
    let groups = fixtures::mockup_groups(false);
    let mut tabs = fixtures::mockup_tabs(false);
    for tab in tabs.iter_mut().filter(|t| t.group == 6) {
        tab.pinned = true;
    }
    let data = build(&groups, &tabs);
    assert_eq!((data.groups.len(), data.tabs.len()), (5, 20));
    assert_eq!(data.label_bands.len(), 5);
    assert_parallel(&data);
    assert!(data.groups.iter().all(|g| g.id != 6));

    // The group in the middle: the other groups' arrays stay parallel and their ids stay right.
    let mut tabs = fixtures::mockup_tabs(false);
    for tab in tabs.iter_mut().filter(|t| t.group == 2) {
        tab.pinned = true;
    }
    let data = build(&groups, &tabs);
    let ids: Vec<i32> = data.groups.iter().map(|g| g.id).collect();
    assert_eq!(ids, vec![1, 3, 4, 5, 6]);
    assert_parallel(&data);
    let band_mid = data.model.layout.groups[1];
    let probe = point_on_circle(
        data.model.layout.spec.centre,
        data.model.layout.spec.label_radius(),
        (band_mid.label_start_deg + band_mid.label_end_deg) / 2.0,
    );
    assert_eq!(
        data.hit(probe.x, probe.y, -1),
        RingHit {
            kind: RingHitKind::Group,
            id: 3
        }
    );
}

#[test]
fn a_tab_of_an_unknown_group_is_dropped() {
    let groups = fixtures::mockup_groups(false);
    let mut tabs = fixtures::mockup_tabs(false);
    let want = build(&groups, &tabs);
    tabs.push(TabRow {
        id: 900,
        group: 77,
        ..Default::default()
    });
    let data = build(&groups, &tabs);
    assert_same_data(&data, &want);
    assert!(data.tabs.iter().all(|t| t.id != 900));
}

#[test]
fn of_two_groups_with_one_id_the_first_owns_the_tabs() {
    let mut groups = fixtures::mockup_groups(false);
    let tabs = fixtures::mockup_tabs(false);
    let want = build(&groups, &tabs);
    let mut twin = groups[0].clone();
    twin.name = "Twin".into();
    groups.push(twin);
    let data = build(&groups, &tabs);
    assert_same_data(&data, &want);
    assert_eq!(data.groups[0].name.as_str(), "Research");
}

#[test]
fn no_groups_or_no_tabs_give_an_empty_ring_without_panicking() {
    let data = build(&[], &[]);
    assert!(data.groups.is_empty() && data.tabs.is_empty() && data.segments.is_empty());
    assert!(data.label_bands.is_empty() && data.label_glyphs.is_empty());
    assert_parallel(&data);
    assert_eq!(data.hit(258.0, 258.0, -1).kind, RingHitKind::Centre);

    let data = build(&fixtures::mockup_groups(false), &[]);
    assert!(
        data.groups.is_empty(),
        "groups without a shown tab are dropped"
    );
    assert_parallel(&data);

    let data = build(&[], &fixtures::mockup_tabs(false));
    assert!(data.tabs.is_empty(), "tabs of no group are dropped");
}

// ---- hit-testing in row ids

#[test]
fn hit_over_every_segment_mid_point_returns_that_tab() {
    let data = mockup();
    for (i, seg) in data.segments.iter().enumerate() {
        let hit = data.hit(seg.mid_x, seg.mid_y, -1);
        assert_eq!(
            hit,
            RingHit {
                kind: RingHitKind::Tab,
                id: data.tabs[i].id
            },
            "segment {i}"
        );
    }
}

#[test]
fn the_lifted_zone_hits_the_hovered_tab_only() {
    let data = mockup();
    let spec = data.model.layout.spec;
    for flat in 0..data.tabs.len() {
        let id = data.tabs[flat].id;
        let point = at_tab(&data, flat, spec.outer_radius() + spec.lift / 2.0 + 1.0);
        let lifted = data.hit(point.x, point.y, id);
        assert_eq!(
            lifted,
            RingHit {
                kind: RingHitKind::Tab,
                id
            },
            "hovered tab {id}"
        );
        let resting = data.hit(point.x, point.y, -1);
        assert_ne!(
            resting.kind,
            RingHitKind::Tab,
            "tab {id} is not hit when it is not lifted"
        );
        assert!(
            matches!(resting.kind, RingHitKind::Group | RingHitKind::Outside),
            "{resting:?}"
        );
        // Another tab being hovered does not lift this one.
        let other = data.tabs[(flat + 1) % data.tabs.len()].id;
        assert_ne!(data.hit(point.x, point.y, other).kind, RingHitKind::Tab);
    }
}

#[test]
fn an_unknown_hovered_tab_is_no_hovered_tab() {
    let data = mockup();
    let spec = data.model.layout.spec;
    let point = at_tab(&data, 4, spec.outer_radius() + spec.lift / 2.0 + 1.0);
    let rest = data.hit(point.x, point.y, -1);
    assert_eq!(data.hit(point.x, point.y, 99_999), rest);
    assert_eq!(data.hit(point.x, point.y, 0), rest);
}

#[test]
fn the_centre_the_reserve_a_label_and_the_corner_are_told_apart() {
    let data = mockup();
    let spec = data.model.layout.spec;
    let none = |kind| RingHit { kind, id: -1 };
    assert_eq!(
        data.hit(spec.centre.x, spec.centre.y, -1),
        none(RingHitKind::Centre)
    );
    assert_eq!(
        data.hit(spec.centre.x, spec.centre.y - spec.radius, -1),
        none(RingHitKind::Reserve)
    );
    assert_eq!(data.hit(0.0, 0.0, -1), none(RingHitKind::Outside));
    let arc = data.model.layout.groups[1];
    let label = point_on_circle(
        spec.centre,
        spec.label_radius(),
        (arc.label_start_deg + arc.label_end_deg) / 2.0,
    );
    assert_eq!(
        data.hit(label.x, label.y, -1),
        RingHit {
            kind: RingHitKind::Group,
            id: 2
        }
    );
}

#[test]
fn a_point_that_is_not_finite_is_outside() {
    let data = mockup();
    for (x, y) in [
        (f32::NAN, 100.0),
        (100.0, f32::NAN),
        (f32::INFINITY, 0.0),
        (0.0, f32::NEG_INFINITY),
    ] {
        assert_eq!(
            data.hit(x, y, 101),
            RingHit {
                kind: RingHitKind::Outside,
                id: -1
            }
        );
    }
}

#[test]
fn hit_polar_agrees_with_hit() {
    let data = mockup();
    let spec = data.model.layout.spec;
    for (flat, arc) in data.model.layout.tabs.iter().enumerate() {
        let theta = (arc.start_deg + arc.end_deg) / 2.0;
        assert_eq!(
            data.hit_polar(spec.radius, theta),
            RingHit {
                kind: RingHitKind::Tab,
                id: data.tabs[flat].id
            }
        );
    }
    assert_eq!(data.hit_polar(10.0, 45.0).kind, RingHitKind::Centre);
    assert_eq!(data.hit_polar(spec.radius, 0.0).kind, RingHitKind::Reserve);
    assert_eq!(data.hit_polar(-1.0, 45.0).kind, RingHitKind::Outside);
    assert_eq!(data.hit_polar(f32::NAN, 45.0).kind, RingHitKind::Outside);
    assert_eq!(
        data.hit_polar(spec.radius, f32::NAN).kind,
        RingHitKind::Outside
    );
    assert_eq!(data.hit_polar(1000.0, 45.0).id, -1);
}

// ---- segment labels

#[test]
fn the_segment_labels_of_the_mockup_follow_the_spec_wording() {
    let data = mockup();
    assert_eq!(
        label_of(&data, 101),
        "Servo layout notes, Research, tab 1 of 5, current tab"
    );
    assert_eq!(
        label_of(&data, 102),
        "WebRender overview, Research, tab 2 of 5, loading 64 percent"
    );
    assert_eq!(
        label_of(&data, 103),
        "Slint rendering, Research, tab 3 of 5"
    );
    assert_eq!(
        label_of(&data, 115),
        "Interview: Servo, Reading, tab 6 of 6, playing sound"
    );
    assert_eq!(
        label_of(&data, 107),
        "Pasteis de Belem, Lisbon trip, tab 2 of 4, unread"
    );
    let unique: std::collections::HashSet<_> = data.segment_labels.iter().collect();
    assert_eq!(unique.len(), 22, "no two labels are equal");
}

#[test]
fn muted_crashed_capture_and_the_empty_title_have_their_words() {
    let groups = fixtures::mockup_groups(false);
    let base = fixtures::mockup_tabs(false);

    let tabs = with_tab(&base, 115, |t| t.muted = true);
    assert_eq!(
        label_of(&build(&groups, &tabs), 115),
        "Interview: Servo, Reading, tab 6 of 6, muted"
    );
    let tabs = with_tab(&base, 103, |t| t.crashed = true);
    assert_eq!(
        label_of(&build(&groups, &tabs), 103),
        "Slint rendering, Research, tab 3 of 5, crashed"
    );
    let tabs = with_tab(&base, 103, |t| t.capture = true);
    assert_eq!(
        label_of(&build(&groups, &tabs), 103),
        "Slint rendering, Research, tab 3 of 5, using camera or microphone"
    );
    let tabs = with_tab(&base, 119, |t| t.title = "   ".into());
    assert_eq!(
        label_of(&build(&groups, &tabs), 119),
        "work.example.com, Work, tab 1 of 2, unread"
    );
    let tabs = with_tab(&base, 119, |t| {
        t.title = "".into();
        t.host = "".into();
    });
    assert_eq!(
        label_of(&build(&groups, &tabs), 119),
        "Untitled, Work, tab 1 of 2, unread"
    );
}

#[test]
fn every_state_comes_in_the_pinned_order() {
    let groups = fixtures::mockup_groups(false);
    let tabs = with_tab(&fixtures::mockup_tabs(false), 101, |t| {
        t.loading = true;
        t.progress = 0.5;
        t.audible = true;
        t.unread = true;
        t.crashed = true;
        t.capture = true;
    });
    assert_eq!(
        label_of(&build(&groups, &tabs), 101),
        "Servo layout notes, Research, tab 1 of 5, current tab, loading 50 percent, \
         playing sound, unread, crashed, using camera or microphone"
    );
}

// ---- label glyphs

fn glyph_group(data: &RingBandsData, glyph: &ArcGlyph) -> usize {
    let centre = data.model.layout.spec.centre;
    let deg = normalise_deg((glyph.x - centre.x).atan2(centre.y - glyph.y).to_degrees());
    data.model
        .layout
        .groups
        .iter()
        .position(|g| g.label_start_deg <= deg && deg <= g.label_end_deg)
        .expect("a glyph lies on its group's label band")
}

fn glyphs_of(data: &RingBandsData, group: usize) -> Vec<&ArcGlyph> {
    data.label_glyphs
        .iter()
        .filter(|glyph| glyph_group(data, glyph) == group)
        .collect()
}

fn held(selected: Option<i32>) -> RingOptions {
    RingOptions {
        held: true,
        selected_group: selected,
        ..RingOptions::default()
    }
}

#[test]
fn a_label_at_rest_uses_the_group_key_and_count_colours() {
    let data = mockup();
    let colours = RingColours::default();
    let glyphs = glyphs_of(&data, 0);
    assert!(!glyphs.is_empty());
    for glyph in &glyphs {
        let want = match glyph.text.as_str() {
            "⌥" | "A" if glyph.weight == 700 => colours.key,
            "5" => colours.count,
            _ => data.groups[0].deep,
        };
        assert_eq!(glyph.colour, want, "glyph {:?}", glyph.text);
    }
    assert!(glyphs.iter().any(|g| g.text == "⌥"));
    assert!(glyphs.iter().any(|g| g.text == "5"));
    assert!(
        data.label_glyphs.iter().all(|g| g.size < 15.0),
        "no big key letters at rest"
    );
}

#[test]
fn a_held_rebuild_is_white_and_dims_the_unselected_groups() {
    let groups = fixtures::mockup_groups(false);
    let tabs = fixtures::mockup_tabs(false);
    let opts = held(Some(1));
    let data = RingBandsData::new(RingSpec::default(), &groups, &tabs, &opts, &ApproxMetrics);
    let white = opts.colours.held_text;
    let dimmed = (f32::from(white.alpha()) * 0.45).round() as u8;
    assert_eq!(dimmed, 115);

    for glyph in glyphs_of(&data, 0) {
        assert_eq!(glyph.colour, white, "selected group, {:?}", glyph.text);
    }
    for group in 1..data.groups.len() {
        let glyphs = glyphs_of(&data, group);
        assert!(!glyphs.is_empty(), "group {group} has a label");
        for glyph in glyphs {
            assert_eq!(
                glyph.colour.alpha(),
                dimmed,
                "group {group}, {:?}",
                glyph.text
            );
            assert_eq!(
                (
                    glyph.colour.red(),
                    glyph.colour.green(),
                    glyph.colour.blue()
                ),
                (white.red(), white.green(), white.blue())
            );
        }
    }

    // The key letter is big: 15 px, weight 800 (Research's key is A); nothing is big at rest.
    let research = glyphs_of(&data, 0);
    let key = research
        .iter()
        .find(|g| g.text == "A")
        .expect("the held key letter");
    assert_eq!((key.size, key.weight), (15.0, 800));
}

#[test]
fn held_without_a_selected_group_dims_nobody() {
    let groups = fixtures::mockup_groups(false);
    let tabs = fixtures::mockup_tabs(false);
    let opts = held(None);
    let data = RingBandsData::new(RingSpec::default(), &groups, &tabs, &opts, &ApproxMetrics);
    assert!(!data.label_glyphs.is_empty());
    assert!(
        data.label_glyphs
            .iter()
            .all(|g| g.colour == opts.colours.held_text)
    );
}

#[test]
fn a_theme_change_changes_the_glyph_colours() {
    let tabs = fixtures::mockup_tabs(false);
    let light = build(&fixtures::mockup_groups(false), &tabs);
    let dark = build(&fixtures::mockup_groups(true), &tabs);
    assert_eq!(light.label_glyphs.len(), dark.label_glyphs.len());
    assert_ne!(
        light
            .label_glyphs
            .iter()
            .map(|g| g.colour)
            .collect::<Vec<_>>(),
        dark.label_glyphs
            .iter()
            .map(|g| g.colour)
            .collect::<Vec<_>>(),
        "the name glyphs follow the group's deep colour"
    );

    let groups = fixtures::mockup_groups(false);
    let opts = RingOptions {
        colours: RingColours {
            key: Color::from_rgb_u8(1, 2, 3),
            count: Color::from_rgb_u8(4, 5, 6),
            ..RingColours::default()
        },
        ..RingOptions::default()
    };
    let themed = RingBandsData::new(RingSpec::default(), &groups, &tabs, &opts, &ApproxMetrics);
    assert!(
        themed
            .label_glyphs
            .iter()
            .any(|g| g.colour == opts.colours.key)
    );
    assert!(
        themed
            .label_glyphs
            .iter()
            .any(|g| g.colour == opts.colours.count)
    );
    assert_ne!(themed.label_glyphs, light.label_glyphs);
}

// ---- crowded rings

fn timed(n_groups: usize, n_tabs: usize) -> (RingBandsData, f64) {
    let (groups, tabs) = fixtures::generated(n_groups, n_tabs, false);
    let started = Instant::now();
    let data = build(&groups, &tabs);
    let ms = started.elapsed().as_secs_f64() * 1000.0;
    (data, ms)
}

#[test]
fn crowded_rings_stay_parallel_and_build_fast() {
    let mockup_started = Instant::now();
    let data = mockup();
    let mockup_ms = mockup_started.elapsed().as_secs_f64() * 1000.0;
    assert_parallel(&data);
    println!("build time, 22 tabs in 6 groups: {mockup_ms:.2} ms");
    assert!(mockup_ms < 200.0, "{mockup_ms} ms");

    let (data, ms) = timed(5, 60);
    assert_eq!((data.groups.len(), data.tabs.len()), (5, 60));
    assert_parallel(&data);
    println!("build time, 60 tabs in 5 groups: {ms:.2} ms");
    assert!(ms < 200.0, "{ms} ms");

    let (data, ms) = timed(64, 64);
    assert_eq!((data.groups.len(), data.tabs.len()), (64, 64));
    assert_parallel(&data);
    println!("build time, 64 groups of 1 tab: {ms:.2} ms");
    assert!(ms < 200.0, "{ms} ms");
}

// ---- the persistent models

#[test]
fn the_models_keep_their_identity_and_follow_the_rows() {
    let models = RingBandsModels::new();
    let first = models.groups();
    let first_tabs = models.tabs();
    assert_eq!(first.row_count(), 0);

    models.update(&mockup());
    assert_eq!(
        first.row_count(),
        6,
        "seen through the handle taken before the update"
    );
    assert_eq!(first_tabs.row_count(), 22);
    assert_eq!(models.segments().row_count(), 22);
    assert_eq!(models.lifted_segments().row_count(), 22);
    assert_eq!(models.segment_labels().row_count(), 22);
    assert_eq!(models.label_bands().row_count(), 6);
    assert_eq!(
        models.label_glyphs().row_count(),
        mockup().label_glyphs.len()
    );
    assert_eq!(first.row_data(2).map(|g| g.id), Some(3));
    assert_eq!(
        models.segment_labels().row_data(1).map(|s| s.to_string()),
        Some("WebRender overview, Research, tab 2 of 5, loading 64 percent".to_owned())
    );

    let (groups, tabs) = fixtures::generated(2, 5, false);
    models.update(&build(&groups, &tabs));
    assert_eq!(first.row_count(), 2);
    assert_eq!(first_tabs.row_count(), 5);
    assert_eq!(models.segments().row_count(), 5);
    assert_eq!(first.row_data(1).map(|g| g.id), Some(2));

    let second = models.groups();
    let same = |a: &dyn Any, b: &dyn Any| {
        std::ptr::eq(
            a as *const dyn Any as *const (),
            b as *const dyn Any as *const (),
        )
    };
    assert!(
        same(first.as_any(), second.as_any()),
        "the same model behind every handle"
    );
    assert_eq!(
        (0..2)
            .map(|i| second.row_data(i).map(|g| g.id))
            .collect::<Vec<_>>(),
        vec![Some(1), Some(2)]
    );

    models.update(&build(&[], &[]));
    assert_eq!((first.row_count(), first_tabs.row_count()), (0, 0));
}

// ---- no panics

#[test]
fn building_and_hitting_never_panic_over_specs_and_sizes() {
    let nan = RingSpec {
        radius: f32::NAN,
        band_width: f32::INFINITY,
        span_deg: f32::NAN,
        reserve_deg: -5.0,
        ..RingSpec::default()
    };
    let zero = RingSpec {
        radius: 0.0,
        band_width: 0.0,
        label_width: 0.0,
        ..RingSpec::default()
    };
    let specs = [
        ("hub", RingSpec::hub(Point::new(258.0, 258.0))),
        (
            "dial_fan",
            RingSpec::dial_fan(Point::new(40.0, 600.0), 270.0),
        ),
        (
            "planet",
            RingSpec::planet(Point::new(120.0, 120.0), 60.0, 16.0),
        ),
        ("default", RingSpec::default()),
        ("zero radius", zero),
        ("nan", nan),
    ];
    let mockup_groups = fixtures::mockup_groups(false);
    let mockup_tabs = fixtures::mockup_tabs(false);
    let sizes: [(&str, Vec<GroupRow>, Vec<TabRow>); 4] = [
        ("0 tabs", Vec::new(), Vec::new()),
        (
            "1 tab",
            fixtures::generated(1, 1, false).0,
            fixtures::generated(1, 1, false).1,
        ),
        ("22 tabs", mockup_groups, mockup_tabs),
        (
            "60 tabs",
            fixtures::generated(5, 60, false).0,
            fixtures::generated(5, 60, false).1,
        ),
    ];
    for (spec_name, spec) in specs {
        for (size_name, groups, tabs) in &sizes {
            let what = format!("{spec_name}, {size_name}");
            let data =
                RingBandsData::new(spec, groups, tabs, &RingOptions::default(), &ApproxMetrics);
            assert_parallel(&data);
            let while_held = RingBandsData::new(spec, groups, tabs, &held(Some(1)), &ApproxMetrics);
            assert_parallel(&while_held);
            let hovered = tabs.first().map_or(-1, |t| t.id);
            for step in 0..=24 {
                for other in 0..=24 {
                    let (x, y) = (step as f32 * 30.0 - 60.0, other as f32 * 30.0 - 60.0);
                    for hover in [-1, hovered, 99_999] {
                        data.hit(x, y, hover);
                    }
                    data.drop_target(x, y, hovered);
                }
            }
            for radius in [0.0, 1.0, 180.0, 400.0, f32::MAX] {
                for theta in [0.0, 90.0, 359.9, -45.0, 720.5, f32::MAX] {
                    data.hit_polar(radius, theta);
                }
            }
            println!("ok: {what}");
        }
    }
}
