// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! The only file of the ring geometry that names the generated Slint structs of
//! `ui/groups/models.slint` (`ArcSeg`, `ArcGlyph`, `GroupRow`, `TabRow`). The pure modules return
//! plain Rust types and know no colours; this file converts them and resolves ids, so everything
//! else stays testable without the generated code.
//!
//! - `ArcSeg::from(&ArcSegment)` and `glyph_to_slint` convert one shape or glyph.
//! - [`RingModel`] is the glue: it counts the tabs per group from `GroupRow` and `TabRow`, lays
//!   the ring out once and hands out `ArcSeg`s whose `tab` is the row's **id**, so a click maps
//!   back to a tab or group without a second lookup.

use std::collections::HashMap;

use slint::Color;

use super::{ArcSegment, GlyphPlacement, GlyphRole, RingLayout, RingSpec, paths};

/// A count or index as a Slint `int`. Saturates (it cannot happen with at most a few hundred
/// tabs); never panics.
fn slint_int(value: usize) -> i32 {
    i32::try_from(value).unwrap_or(i32::MAX)
}

impl From<&ArcSegment> for crate::ArcSeg {
    /// `tab` as an `int` (the flat index or the group index the geometry used; [`RingModel`]
    /// replaces it with the row id), `mid` as the two lengths `mid-x` and `mid-y`. The path text
    /// is carried unchanged.
    fn from(segment: &ArcSegment) -> Self {
        Self {
            tab: slint_int(segment.tab),
            commands: segment.commands.as_str().into(),
            mid_x: segment.mid.x,
            mid_y: segment.mid.y,
        }
    }
}

/// The three colours of a curved label, resolved from the palette by the caller (Rust geometry
/// returns roles, not colours): the group's name, the "⌥K" key and the tab count.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct GlyphColours {
    pub name: Color,
    pub key: Color,
    pub count: Color,
}

/// One label glyph as a Slint `ArcGlyph`: the text centred on (`x`, `y`), rotated by `angle`
/// degrees clockwise, with the colour of the glyph's role.
pub fn glyph_to_slint(glyph: &GlyphPlacement, colours: &GlyphColours) -> crate::ArcGlyph {
    let colour = match glyph.role {
        GlyphRole::Name => colours.name,
        GlyphRole::Key => colours.key,
        GlyphRole::Count => colours.count,
    };
    crate::ArcGlyph {
        text: glyph.text.as_str().into(),
        x: glyph.centre.x,
        y: glyph.centre.y,
        angle: glyph.angle_deg,
        size: glyph.size,
        weight: i32::from(glyph.weight),
        colour,
    }
}

/// One ring ready for Slint: the layout of the tabs that belong to the groups, and where each
/// ring position came from.
///
/// Built by [`RingModel::new`] from the browser's `GroupRow` and `TabRow` lists:
///
/// - **Ring order** is the order of `groups`: layout group `g` is `groups[g]`, so a group index
///   from `paths::label_bands` or `Hit::Group` indexes `groups` directly. A group without tabs
///   stays in the layout with zero tabs (it takes no angle and gets no label band).
/// - **Tabs of a group** are ordered by `TabRow::index_in_group`; equal values keep their order in
///   `tabs` (a stable sort).
/// - **Unknown groups**: a tab whose `group` is no `GroupRow::id` is left out. If two `GroupRow`s
///   share an id, the first one owns the tabs.
/// - `tab_rows[flat]` is the index into `tabs` of the tab at ring position `flat`, which is the
///   number `Hit::Tab` and `paths::segments` use.
#[derive(Clone, Debug, PartialEq)]
pub struct RingModel {
    pub layout: RingLayout,
    /// For every flat tab position: the index of its `TabRow` in the `tabs` slice given to `new`.
    pub tab_rows: Vec<usize>,
    /// `TabRow::id` per flat tab position.
    tab_ids: Vec<i32>,
    /// `GroupRow::id` per group index.
    group_ids: Vec<i32>,
}

impl RingModel {
    /// Lays out `spec` for the tabs of `groups` (see the type's documentation for the order).
    pub fn new(spec: RingSpec, groups: &[crate::GroupRow], tabs: &[crate::TabRow]) -> Self {
        let mut group_of_id: HashMap<i32, usize> = HashMap::with_capacity(groups.len());
        for (index, group) in groups.iter().enumerate() {
            group_of_id.entry(group.id).or_insert(index);
        }

        let mut rows_of_group: Vec<Vec<usize>> = vec![Vec::new(); groups.len()];
        for (row, tab) in tabs.iter().enumerate() {
            if let Some(&group) = group_of_id.get(&tab.group) {
                rows_of_group[group].push(row);
            }
        }
        for rows in &mut rows_of_group {
            rows.sort_by_key(|&row| tabs[row].index_in_group);
        }

        let group_sizes: Vec<usize> = rows_of_group.iter().map(Vec::len).collect();
        let tab_rows: Vec<usize> = rows_of_group.into_iter().flatten().collect();
        Self {
            layout: RingLayout::new(spec, &group_sizes),
            tab_ids: tab_rows.iter().map(|&row| tabs[row].id).collect(),
            group_ids: groups.iter().map(|group| group.id).collect(),
            tab_rows,
        }
    }

    /// The `TabRow::id` of the tab at flat ring position `flat`.
    pub fn tab_id(&self, flat: usize) -> Option<i32> {
        self.tab_ids.get(flat).copied()
    }

    /// The `GroupRow::id` of the group at index `group` of the `groups` given to `new`.
    pub fn group_id(&self, group: usize) -> Option<i32> {
        self.group_ids.get(group).copied()
    }

    /// `ArcSeg` of `segment` with `tab` replaced by `row_id`, the id of the row it belongs to
    /// (kept as the geometry's own index if there is none, which cannot happen for a segment of
    /// this model's own layout).
    fn arc_seg(segment: &ArcSegment, row_id: Option<i32>) -> crate::ArcSeg {
        let mut seg = crate::ArcSeg::from(segment);
        if let Some(id) = row_id {
            seg.tab = id;
        }
        seg
    }

    /// One filled shape per tab in ring order; `ArcSeg::tab` is the `TabRow::id`.
    pub fn segments_for_slint(&self) -> Vec<crate::ArcSeg> {
        paths::segments(&self.layout)
            .iter()
            .map(|segment| Self::arc_seg(segment, self.tab_id(segment.tab)))
            .collect()
    }

    /// One label band per group that has tabs; `ArcSeg::tab` is the `GroupRow::id`.
    pub fn label_bands_for_slint(&self) -> Vec<crate::ArcSeg> {
        paths::label_bands(&self.layout)
            .iter()
            .map(|segment| Self::arc_seg(segment, self.group_id(segment.tab)))
            .collect()
    }

    /// The hovered tab at flat position `flat`, drawn `lift` pixels further out (see
    /// `paths::lifted_segment`); `ArcSeg::tab` is the `TabRow::id`. `None` for an unknown tab.
    pub fn lifted_segment_for_slint(&self, flat: usize, lift: f32) -> Option<crate::ArcSeg> {
        let segment = paths::lifted_segment(&self.layout, flat, lift)?;
        Some(Self::arc_seg(&segment, self.tab_id(flat)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ring_geometry::Point;

    #[test]
    fn a_shape_converts_with_its_index_path_and_anchor() {
        let segment = ArcSegment {
            tab: 7,
            commands: "M1 2 Z".to_owned(),
            mid: Point::new(12.5, -3.25),
        };
        let seg = crate::ArcSeg::from(&segment);
        assert_eq!(seg.tab, 7);
        assert_eq!(seg.commands.as_str(), "M1 2 Z");
        assert_eq!((seg.mid_x, seg.mid_y), (12.5, -3.25));
    }

    #[test]
    fn an_index_beyond_the_int_range_saturates_instead_of_panicking() {
        assert_eq!(slint_int(0), 0);
        assert_eq!(slint_int(64), 64);
        assert_eq!(slint_int(usize::MAX), i32::MAX);
        let segment = ArcSegment {
            tab: usize::MAX,
            commands: String::new(),
            mid: Point::default(),
        };
        assert_eq!(crate::ArcSeg::from(&segment).tab, i32::MAX);
    }

    fn glyph(role: GlyphRole) -> GlyphPlacement {
        GlyphPlacement {
            text: "K".to_owned(),
            centre: Point::new(120.5, 64.0),
            angle_deg: -37.5,
            size: 11.0,
            weight: 700,
            role,
        }
    }

    #[test]
    fn a_glyph_carries_its_place_and_takes_the_colour_of_its_role() {
        let colours = GlyphColours {
            name: Color::from_rgb_u8(10, 20, 30),
            key: Color::from_rgb_u8(40, 50, 60),
            count: Color::from_rgb_u8(70, 80, 90),
        };
        for (role, want) in [
            (GlyphRole::Name, colours.name),
            (GlyphRole::Key, colours.key),
            (GlyphRole::Count, colours.count),
        ] {
            let out = glyph_to_slint(&glyph(role), &colours);
            assert_eq!(out.colour, want, "colour of {role:?}");
            assert_eq!(out.text.as_str(), "K");
            assert_eq!((out.x, out.y), (120.5, 64.0));
            assert_eq!(out.angle, -37.5);
            assert_eq!(out.size, 11.0);
            assert_eq!(out.weight, 700);
        }
    }

    fn group(id: i32) -> crate::GroupRow {
        crate::GroupRow {
            id,
            ..Default::default()
        }
    }

    fn tab(id: i32, group: i32, index_in_group: i32) -> crate::TabRow {
        crate::TabRow {
            id,
            group,
            index_in_group,
            ..Default::default()
        }
    }

    #[test]
    fn the_model_hands_out_row_ids_and_a_lifted_shape_with_the_tab_id() {
        let groups = [group(40), group(41)];
        let tabs = [tab(900, 41, 0), tab(901, 40, 0), tab(902, 40, 1)];
        let model = RingModel::new(RingSpec::default(), &groups, &tabs);
        assert_eq!(model.tab_rows, vec![1, 2, 0]);
        assert_eq!(model.tab_id(0), Some(901));
        assert_eq!(model.tab_id(2), Some(900));
        assert_eq!(model.tab_id(3), None);
        assert_eq!(model.group_id(1), Some(41));
        assert_eq!(model.group_id(2), None);

        let lifted = model.lifted_segment_for_slint(2, 10.0).expect("tab 2");
        assert_eq!(lifted.tab, 900);
        let plain = &model.segments_for_slint()[2];
        assert_eq!((lifted.mid_x, lifted.mid_y), (plain.mid_x, plain.mid_y));
        assert_ne!(lifted.commands, plain.commands);
        assert!(model.lifted_segment_for_slint(3, 10.0).is_none());
    }
}
