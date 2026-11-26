// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! `RingBandsData`: everything one `RingBands` needs, as plain data (no Slint models, so it is
//! testable without a window). The field list and the signatures are the pinned contract of lane
//! ring-bands (brief §3.2); `drop_target` forwards to `drag`, the flags come from `flags`.

use std::collections::HashMap;

use slint::{Color, SharedString};

use crate::ring_geometry::hit::hit_test_polar;
use crate::ring_geometry::slint_models::{GlyphColours, RingModel, glyph_to_slint};
use crate::ring_geometry::{
    GlyphMetrics, Hit, LabelRequest, LabelStyle, Point, RingSpec, hit_test, label_glyphs,
};
use crate::{ArcGlyph, ArcSeg, FlagPin, GroupRow, RingDropTarget, RingHit, RingHitKind, TabRow};

use super::RingOptions;

/// The arrays one `RingBands` takes, built from the browser's rows (see the module documentation of
/// `ring_bands` for the rules).
#[derive(Clone, Debug, PartialEq)]
pub struct RingBandsData {
    /// The layout and the id of every position.
    pub model: RingModel,
    /// The groups that have a shown tab, in ring order (`label_bands[g]` belongs to `groups[g]`).
    pub groups: Vec<GroupRow>,
    /// The shown tabs, group by group, in `index-in-group` order (`segments[i]` belongs to `tabs[i]`).
    pub tabs: Vec<TabRow>,
    /// One shape per tab; `ArcSeg.tab` is the `TabRow.id`.
    pub segments: Vec<ArcSeg>,
    /// The hovered shape of each segment (same length and order as `segments`).
    pub lifted_segments: Vec<ArcSeg>,
    /// The accessible label of each segment (same length and order as `segments`).
    pub segment_labels: Vec<SharedString>,
    /// One band per group in `groups`; `ArcSeg.tab` is the `GroupRow.id`.
    pub label_bands: Vec<ArcSeg>,
    /// The curved labels of every group, colours resolved.
    pub label_glyphs: Vec<ArcGlyph>,
    /// The markers (spinner, speaker, dots, digits).
    pub flags: Vec<FlagPin>,
}

impl RingBandsData {
    /// Lays the ring out for `groups` and `tabs` (any order, any tabs: pinned tabs and tabs of
    /// unknown groups are dropped) and builds every array. `metrics` measures the label glyphs.
    pub fn new(
        spec: RingSpec,
        groups: &[GroupRow],
        tabs: &[TabRow],
        opts: &RingOptions,
        metrics: &dyn GlyphMetrics,
    ) -> Self {
        let (groups, tabs) = select(groups, tabs);
        let model = RingModel::new(spec, &groups, &tabs);
        debug_assert!(
            model.tab_rows.iter().copied().eq(0..tabs.len()),
            "the shown tabs are already in ring order"
        );

        let segments = model.segments_for_slint();
        let lift = model.layout.spec.lift;
        let lifted_segments = (0..tabs.len())
            .map(|flat| {
                model
                    .lifted_segment_for_slint(flat, lift)
                    .unwrap_or_else(|| {
                        debug_assert!(false, "every flat position has a lifted shape");
                        ArcSeg::default()
                    })
            })
            .collect();
        let label_bands = model.label_bands_for_slint();
        let segment_labels = segment_labels(&model, &groups, &tabs);
        let label_glyphs = group_glyphs(&model, &groups, opts, metrics);
        let flags = super::flags::flag_pins(&model, &tabs, opts);

        Self {
            model,
            groups,
            tabs,
            segments,
            lifted_segments,
            segment_labels,
            label_bands,
            label_glyphs,
            flags,
        }
    }

    /// What the point (`x`, `y`) (ring-local) is over; `hovered_tab` is the `TabRow.id` of the
    /// tab that is lifted now (-1 for none), so that its raised zone does not flicker.
    pub fn hit(&self, x: f32, y: f32, hovered_tab: i32) -> RingHit {
        if !x.is_finite() || !y.is_finite() {
            return ring_hit(&self.model, Hit::Outside);
        }
        let lifted = self.flat_of(hovered_tab);
        ring_hit(
            &self.model,
            hit_test(&self.model.layout, Point::new(x, y), lifted),
        )
    }

    /// The same for a radius and an angle (degrees clockwise from 12 o'clock), for keyboard
    /// callers and tests.
    pub fn hit_polar(&self, radius: f32, theta_deg: f32) -> RingHit {
        ring_hit(
            &self.model,
            hit_test_polar(&self.model.layout, radius, theta_deg, None),
        )
    }

    /// Where `dragging_tab` (a `TabRow.id`, -1 for none) would land if released at (`x`, `y`).
    pub fn drop_target(&self, x: f32, y: f32, dragging_tab: i32) -> RingDropTarget {
        super::drag::drop_target_for(&self.model, x, y, dragging_tab)
    }

    /// The flat ring position of the tab with this id (`None` for -1 and for an unknown id).
    fn flat_of(&self, tab_id: i32) -> Option<usize> {
        if tab_id == -1 {
            return None;
        }
        self.tabs.iter().position(|tab| tab.id == tab_id)
    }
}

/// The groups that have a shown tab, in the input order, and their tabs in ring order: the
/// tabs of a group by `index_in_group` (stable). Pinned tabs and tabs of an unknown group are
/// dropped; of two groups with one id the first owns the tabs (as `RingModel` does).
fn select(groups: &[GroupRow], tabs: &[TabRow]) -> (Vec<GroupRow>, Vec<TabRow>) {
    let mut owner: HashMap<i32, usize> = HashMap::with_capacity(groups.len());
    for (index, group) in groups.iter().enumerate() {
        owner.entry(group.id).or_insert(index);
    }
    let mut own: Vec<Vec<&TabRow>> = vec![Vec::new(); groups.len()];
    for tab in tabs.iter().filter(|tab| !tab.pinned) {
        if let Some(&group) = owner.get(&tab.group) {
            own[group].push(tab);
        }
    }

    let mut shown_groups = Vec::new();
    let mut shown_tabs = Vec::new();
    for (group, mut rows) in own.into_iter().enumerate() {
        if rows.is_empty() {
            continue;
        }
        rows.sort_by_key(|tab| tab.index_in_group);
        shown_groups.push(groups[group].clone());
        shown_tabs.extend(rows.into_iter().cloned());
    }
    (shown_groups, shown_tabs)
}

/// The kind and id of what `hit` is over, with the row id of the tab or group.
fn ring_hit(model: &RingModel, hit: Hit) -> RingHit {
    let (kind, id) = match hit {
        Hit::Tab(flat) => (RingHitKind::Tab, model.tab_id(flat)),
        Hit::Group(group) => (RingHitKind::Group, model.group_id(group)),
        Hit::Centre => (RingHitKind::Centre, None),
        Hit::Reserve => (RingHitKind::Reserve, None),
        _ => (RingHitKind::Outside, None),
    };
    RingHit {
        kind,
        id: id.unwrap_or(-1),
    }
}

fn segment_labels(model: &RingModel, groups: &[GroupRow], tabs: &[TabRow]) -> Vec<SharedString> {
    let mut labels = Vec::with_capacity(tabs.len());
    for (group, arc) in groups.iter().zip(&model.layout.groups) {
        for (index, tab) in tabs
            .iter()
            .skip(arc.first_tab)
            .take(arc.tab_count)
            .enumerate()
        {
            labels.push(segment_label(tab, &group.name, index, arc.tab_count));
        }
    }
    labels
}

/// SPEC §5: "<title>, <group>, tab <i> of <n>", then the states that apply, in the pinned order.
fn segment_label(tab: &TabRow, group_name: &str, index: usize, in_group: usize) -> SharedString {
    let title = [tab.title.trim(), tab.host.trim()]
        .into_iter()
        .find(|text| !text.is_empty())
        .unwrap_or("Untitled");
    let mut label = format!("{title}, {group_name}, tab {} of {in_group}", index + 1);
    if tab.active {
        label.push_str(", current tab");
    }
    if tab.loading {
        if (0.0..=1.0).contains(&tab.progress) {
            let percent = (tab.progress * 100.0).round() as i32;
            label.push_str(&format!(", loading {percent} percent"));
        } else {
            label.push_str(", loading");
        }
    }
    if tab.audible && !tab.muted {
        label.push_str(", playing sound");
    }
    if tab.muted {
        label.push_str(", muted");
    }
    if tab.unread {
        label.push_str(", unread");
    }
    if tab.crashed {
        label.push_str(", crashed");
    }
    if tab.capture {
        label.push_str(", using camera or microphone");
    }
    label.into()
}

fn group_glyphs(
    model: &RingModel,
    groups: &[GroupRow],
    opts: &RingOptions,
    metrics: &dyn GlyphMetrics,
) -> Vec<ArcGlyph> {
    let style = if opts.held {
        LabelStyle::Held
    } else {
        LabelStyle::Rest
    };
    let mut glyphs = Vec::new();
    for (group, arc) in groups.iter().zip(&model.layout.groups) {
        let request = LabelRequest {
            name: group.name.as_str(),
            key: group
                .key_label
                .chars()
                .next()
                .and_then(|key| key.to_uppercase().next()),
            count: usize::try_from(group.count).unwrap_or(0),
            style,
        };
        let colours = glyph_colours(group, opts);
        let layout = label_glyphs(arc, &model.layout.spec, &request, metrics);
        glyphs.extend(
            layout
                .glyphs
                .iter()
                .map(|glyph| glyph_to_slint(glyph, &colours)),
        );
    }
    glyphs
}

/// At rest: the group's `deep`, the key and count colours. Held: every glyph is `held_text`, at
/// `dimmed_alpha` of its alpha for the groups other than the selected one (when there is one).
fn glyph_colours(group: &GroupRow, opts: &RingOptions) -> GlyphColours {
    let colours = &opts.colours;
    if !opts.held {
        return GlyphColours {
            name: group.deep,
            key: colours.key,
            count: colours.count,
        };
    }
    let dimmed = opts
        .selected_group
        .is_some_and(|selected| selected != group.id);
    let text = if dimmed {
        dim(colours.held_text, colours.dimmed_alpha)
    } else {
        colours.held_text
    };
    GlyphColours {
        name: text,
        key: text,
        count: text,
    }
}

fn dim(colour: Color, factor: f32) -> Color {
    let factor = if factor.is_finite() {
        factor.clamp(0.0, 1.0)
    } else {
        1.0
    };
    let alpha = (f32::from(colour.alpha()) * factor)
        .round()
        .clamp(0.0, 255.0) as u8;
    Color::from_argb_u8(alpha, colour.red(), colour.green(), colour.blue())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tab() -> TabRow {
        TabRow {
            title: "Page".into(),
            host: "example.org".into(),
            ..Default::default()
        }
    }

    fn label(tab: &TabRow) -> String {
        segment_label(tab, "Group", 1, 4).to_string()
    }

    #[test]
    fn a_plain_tab_has_the_title_group_and_place() {
        assert_eq!(label(&tab()), "Page, Group, tab 2 of 4");
    }

    #[test]
    fn states_follow_in_the_pinned_order() {
        let all = TabRow {
            active: true,
            loading: true,
            progress: 0.5,
            audible: true,
            muted: true,
            unread: true,
            crashed: true,
            capture: true,
            ..tab()
        };
        assert_eq!(
            label(&all),
            "Page, Group, tab 2 of 4, current tab, loading 50 percent, muted, unread, crashed, \
             using camera or microphone"
        );
        let sound = TabRow {
            audible: true,
            ..tab()
        };
        assert!(label(&sound).ends_with(", playing sound"));
    }

    #[test]
    fn loading_without_a_usable_progress_says_only_loading() {
        for progress in [-1.0, 1.5, f32::NAN, f32::INFINITY] {
            let loading = TabRow {
                loading: true,
                progress,
                ..tab()
            };
            assert_eq!(label(&loading), "Page, Group, tab 2 of 4, loading");
        }
        let done = TabRow {
            loading: true,
            progress: 1.0,
            ..tab()
        };
        assert!(label(&done).ends_with("loading 100 percent"));
    }

    #[test]
    fn an_empty_title_uses_the_host_then_untitled() {
        let no_title = TabRow {
            title: "  ".into(),
            ..tab()
        };
        assert_eq!(label(&no_title), "example.org, Group, tab 2 of 4");
        let nothing = TabRow {
            title: "".into(),
            host: "\t".into(),
            ..tab()
        };
        assert_eq!(label(&nothing), "Untitled, Group, tab 2 of 4");
    }

    #[test]
    fn the_flags_are_what_flag_pins_says_for_the_shown_tabs() {
        use crate::ring_bands::fixtures;
        use crate::ring_geometry::ApproxMetrics;

        for held in [false, true] {
            let opts = RingOptions {
                held,
                selected_group: Some(1),
                ..RingOptions::default()
            };
            let mut tabs = fixtures::mockup_tabs(false);
            tabs[2].pinned = true;
            let data = RingBandsData::new(
                RingSpec::default(),
                &fixtures::mockup_groups(false),
                &tabs,
                &opts,
                &ApproxMetrics,
            );
            let want = crate::ring_bands::flags::flag_pins(&data.model, &data.tabs, &opts);
            assert_eq!(data.flags, want);
        }
    }

    #[test]
    fn dimming_multiplies_the_alpha_and_keeps_the_colour() {
        let white = Color::from_rgb_u8(255, 255, 255);
        let dimmed = dim(white, 0.45);
        assert_eq!(dimmed.alpha(), 115);
        assert_eq!(
            (dimmed.red(), dimmed.green(), dimmed.blue()),
            (255, 255, 255)
        );
        assert_eq!(dim(white, f32::NAN), white);
        assert_eq!(dim(white, 7.0), white);
        assert_eq!(dim(white, -1.0).alpha(), 0);
    }
}
