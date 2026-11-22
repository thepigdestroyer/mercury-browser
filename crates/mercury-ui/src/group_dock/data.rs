// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! The dock's data: pill widths from measured text, the fit to the container, the magnified frame
//! (from `ring_geometry::magnified_layout`, asked once per pill when the dock is built), the hit test
//! and the capsule.
//! The public signatures are the pinned contract of lane group-dock; only additions were made
//! ([`DockData::tooltip_for`], [`DockData::tier`], [`DockData::container_height`],
//! [`DockData::current_group`], [`DockData::pill_widths`]).
//!
//! # Geometry (brief D4)
//!
//! The resting layout is the pills followed by **one** trailing entry for the "+" and the x
//! (26 + 4 + 26 px), `gap` apart. `resting_left_edge` of that layout centres the whole bar at rest;
//! a pointer at window x is `x - resting_left_edge` in the resting coordinates and goes to **one**
//! [`magnified_layout`] call per pill (when the dock is built) over the pills alone: the pill whose
//! resting extent holds the pointer is magnified. The trailing block is not magnified, and a
//! pointer past the middle of the gap before it counts as off the pills.
//!
//! The scales of a frame are those of the pill under the pointer, blended with its neighbour's
//! within 24 px of a boundary (`DockData::blend_of`): every number of the frame is then continuous
//! in the pointer's x, and in the middle of a pill the scales are exactly 1.45 / 1.12 / 1.0.
//! The bar is drawn **anchored on the pointer** (brief D4, amended after the first renders): the
//! pill under the pointer is drawn so that the pointer stays at the same fraction of it as in the
//! resting layout, and the bar grows upward only. A bar drawn centred instead put the pointer up to
//! half the growth (about 80 px on the mockup row) away from the pill it magnified, so the
//! neighbour took the click. The anchored bar stays at least `HEADROOM` px from the container's
//! edges (the left limit wins where they cross); with no pointer (or the pointer past the pills) it
//! is centred. The trailing block sits `gap` after the last magnified pill. If the bar is wider than
//! the container less the headroom (only when even the last fit tier does not fit) it is centred
//! or, if wider than the container, anchored at its left edge.
//!
//! The bar is `2 * 10` px of padding wider than its entries. Every x in the frame is in the
//! container's coordinates (the window's content area, origin at its left edge).
//!
//! # The y axis of [`DockData::hit`]
//!
//! `y` is measured in `GroupDock`'s own box, whose height is [`DockData::container_height`]
//! (`BOTTOM_MARGIN + dock_height(1.45)` = 77.5: the 72 px band plus the 5.5 px overshoot). The
//! bar's bottom edge is `BOTTOM_MARGIN` above its bottom, the top edge `dock_height` above that.
//! Every entry of a pill (name, chips, slivers, the pill's "+") and of the trailing block
//! answers over the whole height of the bar: a pill is a column, so the answer does not depend on
//! whether the pills are centred or bottom-aligned inside the bar.
//!
//! The "+" inside an expanded pill is `26 * scale` wide and sits 4 * scale from the pill's right
//! edge; a hit there is `new-tab` with that pill's group. The trailing "+" targets the current
//! group; the trailing x is `shrink`. A click in the gap between two pills, in the bar's padding or
//! between the two buttons is `none`.
//!
//! # The tooltip
//!
//! `frame` only knows the pointer's x, and the tooltip depends on what is under it, so it leaves
//! `DockFrame::tooltip` empty. The host asks [`DockData::tooltip_for`] with the frame and the
//! pointer and stores the answer in the frame (or straight into the dock's `tooltip` and
//! `tooltip-x`): `"<group> · <title>"` over a sliver or chip, centred above that sliver or chip
//! (the returned x is its centre); over a folded pill that shows no name (tiers 3 and 4) the group's
//! name alone; `None` everywhere else.

use std::ops::Range;

use slint::{Model, ModelRc, SharedString, VecModel};

use crate::ring_geometry::GlyphMetrics;
use crate::ring_geometry::dock::{
    DockLayout, UNDER_POINTER, dock_layout, magnified_layout, resting_left_edge,
};
use crate::{DockChip, DockHit, DockHitKind, DockPill, DockSliver, GroupRow, TabRow};

use super::band::{BOTTOM_MARGIN, PILL_HEIGHT, dock_height};
use super::capsule::{CapsuleFrame, capsule_frame};
use super::fit::{
    BAR_PAD_X, BUTTON, BUTTON_GAP, CHIP_SIZE, CHIP_WEIGHT, COUNT_SIZE, COUNT_WEIGHT, Fit, HEADROOM,
    KEY_SIZE, KEY_WEIGHT, Measure, NAME_MAX, NAME_SIZE, NAME_WEIGHT, PLUS_RIGHT_INSET, Plan, Span,
    TRAILING_WIDTH, badge_width, chip_natural, choose,
};

/// The close target of a chip: its right end, `CHIP_CLOSE` wide (the x glyph is 16 px, centred in
/// it, so the target is 24 px as every other touch target of the dock). Scaled with the pill.
const CHIP_CLOSE: f32 = 24.0;
/// How far from a pill boundary (half the gap) the anchor of the bar starts to blend the two
/// layouts that meet there, at most (`DockData::inside`).
const ANCHOR_BLEND: f32 = 24.0;

/// What a dock is built for besides its rows.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DockOptions {
    /// The width of the area the dock is centred in (the window's content width). Never the
    /// width of the dock as drawn.
    pub container_width: f32,
    /// ⌥ is held: key badges after every name.
    pub held: bool,
    /// "Show keys" is on: badges at rest too, if there is room (D13).
    pub show_keys: bool,
    /// A `GroupRow.id`; `None` = the group of the active tab.
    pub current_group: Option<i32>,
}

impl Default for DockOptions {
    fn default() -> Self {
        Self {
            container_width: 1440.0,
            held: false,
            show_keys: false,
            current_group: None,
        }
    }
}

/// Everything `GroupDock` draws for one pointer position.
#[derive(Clone, Debug, Default)]
pub struct DockFrame {
    /// One per shown group, in dock order, with the drawn (magnified) numbers.
    pub pills: Vec<DockPill>,
    /// The scale of every pill (1.0, 1.12, 1.45).
    pub scales: Vec<f32>,
    /// The drawn bar, in the container's coordinates.
    pub dock_x: f32,
    pub dock_width: f32,
    pub dock_height: f32,
    /// The largest scale in use.
    pub max_scale: f32,
    /// The trailing "+" and x, drawn positions.
    pub new_tab_x: f32,
    pub shrink_x: f32,
    /// "<group> · <title>" and its x, for a hovered sliver or chip.
    pub tooltip: Option<(SharedString, f32)>,
    /// The tab whose chip or sliver is under the pointer of this frame, if any. A chip shows its x
    /// (and `hit` reports `ChipClose` over it) when it is the active one or this one.
    pub hovered_tab: Option<i32>,
}

/// One shown group's pill at rest: the numbers of its plan as Slint rows, and the two models of
/// the rows at scale 1.0, shared by every frame in which the pill is not magnified.
#[derive(Clone, Debug)]
struct Pill {
    width: f32,
    expanded: bool,
    show_key: bool,
    chips: Vec<DockChip>,
    slivers: Vec<DockSliver>,
    rest_chips: ModelRc<DockChip>,
    rest_slivers: ModelRc<DockSliver>,
}

/// What a point of a frame is over.
struct Located {
    kind: DockHitKind,
    group: i32,
    tab: i32,
    /// The x of the middle of the element, for the tooltip.
    centre: f32,
}

/// The dock's rows and the resting layout; `frame` and `hit` are pure functions of it.
#[derive(Clone, Debug)]
pub struct DockData {
    /// The shown groups in dock order (every one has a tab).
    groups: Vec<GroupRow>,
    /// The shown tabs, group by group, in tab order.
    tabs: Vec<TabRow>,
    pills: Vec<Pill>,
    widths: Vec<f32>,
    /// A `GroupRow.id`, -1 without groups.
    current: i32,
    container: f32,
    gap: f32,
    tier: u8,
    /// Where the first pill's left edge is at rest, in the container.
    rest_left: f32,
    /// The resting layout (every scale 1.0), computed once.
    rest: DockLayout,
    /// For each pill, the scales and the layout drawn while the pointer is over it (the current
    /// pill pinned); the last entry is the resting layout (every scale 1.0).
    anchor_scales: Vec<Vec<f32>>,
    anchors: Vec<DockLayout>,
}

fn clean(value: f32) -> f32 {
    if value.is_finite() {
        value.max(0.0)
    } else {
        0.0
    }
}

fn text_width(metrics: &dyn GlyphMetrics, text: &str, size: f32, weight: u16) -> f32 {
    text.chars()
        .map(|ch| metrics.advance(ch, size, weight))
        .filter(|advance| advance.is_finite() && *advance > 0.0)
        .sum()
}

fn chip_row(tab: &TabRow, span: &Span) -> DockChip {
    DockChip {
        tab: tab.id,
        title: tab.title.clone(),
        shade: tab.shade,
        active: tab.active,
        unread: tab.unread,
        audible: tab.audible,
        muted: tab.muted,
        crashed: tab.crashed,
        capture: tab.capture,
        loading: tab.loading,
        x: span.x,
        width: span.width,
    }
}

fn sliver_row(tab: &TabRow, span: &Span) -> DockSliver {
    DockSliver {
        tab: tab.id,
        shade: tab.shade,
        active: tab.active,
        x: span.x,
        width: span.width,
    }
}

fn model<T: Clone + 'static>(rows: &[T]) -> ModelRc<T> {
    ModelRc::new(VecModel::from(rows.to_vec()))
}

fn pill_of(plan: &Plan, tabs: &[TabRow]) -> Pill {
    let chips: Vec<DockChip> = plan
        .chips
        .iter()
        .filter_map(|span| tabs.get(span.index).map(|tab| chip_row(tab, span)))
        .collect();
    let slivers: Vec<DockSliver> = plan
        .slivers
        .iter()
        .filter_map(|span| tabs.get(span.index).map(|tab| sliver_row(tab, span)))
        .collect();
    Pill {
        width: plan.width,
        expanded: plan.expanded,
        show_key: plan.show_key,
        rest_chips: model(&chips),
        rest_slivers: model(&slivers),
        chips,
        slivers,
    }
}

/// The index and middle of the span (id, x, width) nearest to `local`, if `local` is inside the run
/// from the first span's left edge to the last one's right edge (the gaps count as the nearer
/// span, the lower on a tie).
fn nearest_span(spans: &[(i32, f32, f32)], local: f32) -> Option<(i32, f32)> {
    let first = spans.first()?;
    let last = spans.last()?;
    if local < first.1 || local > last.1 + last.2 {
        return None;
    }
    let distance =
        |span: &&(i32, f32, f32)| (span.1 - local).max(local - (span.1 + span.2)).max(0.0);
    spans
        .iter()
        .min_by(|a, b| distance(a).total_cmp(&distance(b)))
        .map(|span| (span.0, span.1 + span.2 / 2.0))
}

impl DockData {
    /// Builds the dock for these rows: pinned tabs are kept (D2), a group with no tab is dropped, a
    /// tab whose group is unknown is ignored.
    pub fn new(
        groups: &[GroupRow],
        tabs: &[TabRow],
        options: &DockOptions,
        metrics: &dyn GlyphMetrics,
    ) -> Self {
        let container = clean(options.container_width);
        let mut shown_groups: Vec<GroupRow> = Vec::new();
        let mut shown_tabs: Vec<TabRow> = Vec::new();
        let mut ranges: Vec<Range<usize>> = Vec::new();
        for group in groups {
            if shown_groups.iter().any(|shown| shown.id == group.id) {
                continue;
            }
            let mut own: Vec<&TabRow> = tabs.iter().filter(|tab| tab.group == group.id).collect();
            if own.is_empty() {
                continue;
            }
            own.sort_by_key(|tab| tab.index_in_group);
            let start = shown_tabs.len();
            shown_tabs.extend(own.into_iter().cloned());
            ranges.push(start..shown_tabs.len());
            shown_groups.push(group.clone());
        }

        let current = options
            .current_group
            .and_then(|id| shown_groups.iter().position(|group| group.id == id))
            .or_else(|| {
                let active = shown_tabs.iter().find(|tab| tab.active)?;
                shown_groups
                    .iter()
                    .position(|group| group.id == active.group)
            })
            .unwrap_or(0);

        let measures: Vec<Measure> = shown_groups
            .iter()
            .zip(&ranges)
            .map(|(group, range)| {
                let own = &shown_tabs[range.clone()];
                Measure {
                    name: text_width(metrics, &group.name, NAME_SIZE, NAME_WEIGHT).min(NAME_MAX),
                    badge: (!group.key_label.is_empty()).then(|| {
                        badge_width(text_width(metrics, &group.key_label, KEY_SIZE, KEY_WEIGHT))
                    }),
                    count: text_width(metrics, &own.len().to_string(), COUNT_SIZE, COUNT_WEIGHT),
                    chips: own
                        .iter()
                        .map(|tab| {
                            chip_natural(
                                text_width(metrics, &tab.title, CHIP_SIZE, CHIP_WEIGHT),
                                tab.active,
                            )
                        })
                        .collect(),
                    active_chip: own.iter().position(|tab| tab.active),
                }
            })
            .collect();
        let Fit { tier, gap, plans } = choose(
            &measures,
            current,
            container,
            options.held,
            options.show_keys,
        );

        let pills: Vec<Pill> = plans
            .iter()
            .zip(&ranges)
            .map(|(plan, range)| pill_of(plan, &shown_tabs[range.clone()]))
            .collect();
        let widths: Vec<f32> = pills.iter().map(|pill| pill.width).collect();
        let mut resting = widths.clone();
        resting.push(TRAILING_WIDTH);
        let rest_left = resting_left_edge(container, &resting, gap).max(BAR_PAD_X);

        let rest = dock_layout(&widths, &[], gap);
        let current_index = (!shown_groups.is_empty()).then_some(current);
        let mut anchor_scales: Vec<Vec<f32>> = (0..widths.len())
            .map(|index| {
                let middle = rest.xs[index] + rest.widths[index] / 2.0;
                let (mut scales, _) = magnified_layout(Some(middle), &widths, gap);
                pin_current(&mut scales, current_index);
                scales
            })
            .collect();
        anchor_scales.push(vec![1.0; widths.len()]);
        let anchors: Vec<DockLayout> = anchor_scales
            .iter()
            .map(|scales| dock_layout(&widths, scales, gap))
            .collect();
        Self {
            current: shown_groups.get(current).map_or(-1, |group| group.id),
            rest,
            anchor_scales,
            anchors,
            groups: shown_groups,
            tabs: shown_tabs,
            pills,
            widths,
            container,
            gap,
            tier,
            rest_left,
        }
    }

    /// The fit tier of D8 (1 = everything at its natural size, 5 = the last resort).
    pub fn tier(&self) -> u8 {
        self.tier
    }

    /// The id of the current (expanded) group; `None` without groups.
    pub fn current_group(&self) -> Option<i32> {
        (self.current >= 0).then_some(self.current)
    }

    /// The resting width of every shown pill, in dock order.
    pub fn pill_widths(&self) -> &[f32] {
        &self.widths
    }

    /// The height of `GroupDock`'s box, the y axis of [`DockData::hit`] (see the module docs).
    pub fn container_height() -> f32 {
        BOTTOM_MARGIN + dock_height(UNDER_POINTER)
    }

    /// The frame for a pointer at `pointer_window_x` (the window's x; `None` = off the dock). One
    /// `magnified_layout` call, the resting layout's left edge subtracted first (D4).
    pub fn frame(&self, pointer_window_x: Option<f32>) -> DockFrame {
        let count = self.pills.len();
        let raw = pointer_window_x
            .filter(|x| x.is_finite())
            .map(|x| x - self.rest_left);
        // The scales are those of the pill under the pointer (`ring_geometry::magnified_layout`,
        // asked once per pill when the dock is built, with the current pill pinned: brief D16),
        // blended with its neighbour's over `ANCHOR_BLEND` px on each side of a boundary, so that
        // every number of the frame (the scales, the widths, the bar's position and width) is
        // continuous in the pointer's x. In the middle of a pill they are exactly 1.45 / 1.12 / 1.0.
        let blend = raw.map(|x| self.blend_of(x));
        let scales: Vec<f32> = match blend {
            Some((a, b, weight)) => self.anchor_scales[a]
                .iter()
                .zip(&self.anchor_scales[b])
                .map(|(from, to)| from * (1.0 - weight) + to * weight)
                .collect(),
            None => vec![1.0; count],
        };
        let layout = dock_layout(&self.widths, &scales, self.gap);
        let trailing_gap = if count > 0 { self.gap } else { 0.0 };
        let entries = layout.total_width + trailing_gap + TRAILING_WIDTH;
        let dock_width = entries + 2.0 * BAR_PAD_X;
        let centred = ((self.container - dock_width) / 2.0).max(0.0);
        let dock_x = match (pointer_window_x, raw, blend) {
            (Some(window_x), Some(resting_x), Some((a, b, weight)))
                if dock_width <= self.container - 2.0 * HEADROOM =>
            {
                // Anchor the bar on the pointer (brief D4, amended): the point of the resting
                // layout under the pointer is drawn under it (`mapped`, blended like the scales).
                let inside =
                    self.mapped(a, resting_x) * (1.0 - weight) + self.mapped(b, resting_x) * weight;
                // Where the limits cross (a bar that fills the window) the left one wins.
                (window_x - inside - BAR_PAD_X)
                    .min(self.container - HEADROOM - dock_width)
                    .max(HEADROOM)
            }
            _ => centred,
        };
        let left = dock_x + BAR_PAD_X;
        let pills = self
            .pills
            .iter()
            .zip(&self.groups)
            .zip(scales.iter().zip(layout.xs.iter().zip(&layout.widths)))
            .map(|((pill, group), (scale, (x, width)))| {
                let (chips, slivers) = if (*scale - 1.0).abs() < f32::EPSILON {
                    (pill.rest_chips.clone(), pill.rest_slivers.clone())
                } else {
                    (
                        model(&scale_rows(&pill.chips, *scale, |chip, scale| DockChip {
                            x: chip.x * scale,
                            width: chip.width * scale,
                            ..chip.clone()
                        })),
                        model(&scale_rows(&pill.slivers, *scale, |sliver, scale| {
                            DockSliver {
                                x: sliver.x * scale,
                                width: sliver.width * scale,
                                ..sliver.clone()
                            }
                        })),
                    )
                };
                DockPill {
                    group: group.clone(),
                    x: left + x,
                    width: *width,
                    height: PILL_HEIGHT * scale,
                    scale: *scale,
                    expanded: pill.expanded,
                    show_key: pill.show_key,
                    chips,
                    slivers,
                }
            })
            .collect();
        let new_tab_x = left + layout.total_width + trailing_gap;
        let mut frame = DockFrame {
            pills,
            scales,
            dock_x,
            dock_width,
            dock_height: dock_height(layout.max_scale),
            max_scale: layout.max_scale,
            new_tab_x,
            shrink_x: new_tab_x + BUTTON + BUTTON_GAP,
            tooltip: None,
            hovered_tab: None,
        };
        if let Some(x) = pointer_window_x {
            let middle = Self::container_height() - BOTTOM_MARGIN - frame.dock_height / 2.0;
            frame.hovered_tab = self
                .locate(&frame, x, middle)
                .filter(|found| {
                    matches!(
                        found.kind,
                        DockHitKind::Chip | DockHitKind::ChipClose | DockHitKind::Sliver
                    )
                })
                .map(|found| found.tab);
        }
        frame
    }

    /// Where the point of the resting layout at `resting_x` (from the first pill's left edge) is in
    /// the pills of the layout drawn while the pointer is over pill `index`, continued linearly past
    /// the pill; `index == count` is the resting layout.
    fn mapped(&self, index: usize, resting_x: f32) -> f32 {
        match (self.anchors.get(index), self.rest.xs.get(index)) {
            (Some(layout), Some(rest_x)) => {
                let rest_width = self.rest.widths[index].max(f32::EPSILON);
                layout.xs[index] + (resting_x - rest_x) / rest_width * layout.widths[index]
            }
            _ => resting_x,
        }
    }

    /// The two layouts (pill indices, `count` = the resting layout) that the frame at `resting_x`
    /// is made of and the weight of the second.
    ///
    /// The resting rule chooses pill `u` at `resting_x` (steady: it does not depend on what is
    /// drawn). Inside `u` the frame is `u`'s layout. Within `ANCHOR_BLEND` px of a boundary it is a
    /// blend with the neighbour's: with each pill's layout on its own, the bar would jump by tens of
    /// pixels when the pointer crosses a boundary (the old pill shrinks, the new one grows, and the
    /// bar is anchored on the pointer), and so would its width, which the window's edges limit.
    /// The weight is 0.5 at the boundary, so both sides give the same blend there. Past the last
    /// pill the other layout is the resting one.
    fn blend_of(&self, resting_x: f32) -> (usize, usize, f32) {
        let count = self.widths.len();
        // The boundary after pill `k`: half the gap after its right edge.
        let boundary = |k: usize| self.rest.xs[k] + self.rest.widths[k] + self.gap / 2.0;
        let band = |k: usize| -> f32 {
            let next = self.rest.widths.get(k + 1).copied().unwrap_or(f32::MAX);
            ANCHOR_BLEND
                .min(self.rest.widths[k].min(next) / 2.0)
                .max(f32::EPSILON)
        };
        // The region: the first pill whose boundary is not behind the pointer; `count` = past all.
        let region = (0..count)
            .find(|k| resting_x <= boundary(*k))
            .unwrap_or(count);
        let weight =
            |k: usize| ((resting_x - (boundary(k) - band(k))) / (2.0 * band(k))).clamp(0.0, 1.0);
        if region < count {
            let w = weight(region);
            if w > 0.0 {
                return (region, region + 1, w);
            }
        }
        if region > 0 {
            let w = weight(region - 1);
            if w < 1.0 {
                return (region - 1, region, w);
            }
        }
        (region, region, 0.0)
    }

    /// What is at (`x`, `y`) in the container's coordinates in `frame` (the y axis is described in
    /// the module docs).
    pub fn hit(&self, frame: &DockFrame, x: f32, y: f32) -> DockHit {
        match self.locate(frame, x, y) {
            Some(found) => DockHit {
                kind: found.kind,
                group: found.group,
                tab: found.tab,
            },
            None => DockHit {
                kind: DockHitKind::None,
                group: -1,
                tab: -1,
            },
        }
    }

    /// The tooltip for a pointer at (`x`, `y`) in `frame`, and the x of its middle: see the module
    /// docs. The host stores it into `DockFrame::tooltip`.
    pub fn tooltip_for(&self, frame: &DockFrame, x: f32, y: f32) -> Option<(SharedString, f32)> {
        let found = self.locate(frame, x, y)?;
        let pill = frame
            .pills
            .iter()
            .find(|pill| pill.group.id == found.group)?;
        match found.kind {
            DockHitKind::Sliver | DockHitKind::Chip => {
                let tab = self.tabs.iter().find(|tab| tab.id == found.tab)?;
                let text = format!("{} · {}", pill.group.name, tab.title);
                Some((text.into(), found.centre))
            }
            DockHitKind::Pill if !pill.expanded && self.tier >= 3 => {
                Some((pill.group.name.clone(), found.centre))
            }
            _ => None,
        }
    }

    fn locate(&self, frame: &DockFrame, x: f32, y: f32) -> Option<Located> {
        if !x.is_finite() || !y.is_finite() {
            return None;
        }
        let bottom = Self::container_height() - BOTTOM_MARGIN;
        let top = bottom - frame.dock_height;
        if y < top || y > bottom {
            return None;
        }
        if x < frame.dock_x || x > frame.dock_x + frame.dock_width {
            return None;
        }
        let inside = |left: f32, width: f32| x >= left && x <= left + width;
        if inside(frame.new_tab_x, BUTTON) {
            return Some(Located {
                kind: DockHitKind::NewTab,
                group: self.current,
                tab: -1,
                centre: frame.new_tab_x + BUTTON / 2.0,
            });
        }
        if inside(frame.shrink_x, BUTTON) {
            return Some(Located {
                kind: DockHitKind::Shrink,
                group: -1,
                tab: -1,
                centre: frame.shrink_x + BUTTON / 2.0,
            });
        }
        let pill = frame.pills.iter().find(|pill| inside(pill.x, pill.width))?;
        let local = x - pill.x;
        let group = pill.group.id;
        if pill.expanded {
            let plus = pill.width - (PLUS_RIGHT_INSET + BUTTON) * pill.scale;
            if local >= plus && local <= plus + BUTTON * pill.scale {
                return Some(Located {
                    kind: DockHitKind::NewTab,
                    group,
                    tab: -1,
                    centre: pill.x + plus + BUTTON * pill.scale / 2.0,
                });
            }
        }
        let chips: Vec<(i32, f32, f32)> = pill
            .chips
            .iter()
            .map(|chip| (chip.tab, chip.x, chip.width))
            .collect();
        if let Some((tab, centre)) = nearest_span(&chips, local) {
            // The chip's x is drawn on the active chip and on the one under the pointer, and a
            // close target is only there when it is drawn.
            let on_close = chips.iter().find(|chip| chip.0 == tab).is_some_and(|chip| {
                let shown = self.tabs.iter().any(|row| row.id == tab && row.active)
                    || frame.hovered_tab == Some(tab);
                let right = chip.1 + chip.2;
                shown && local >= right - CHIP_CLOSE * pill.scale && local <= right
            });
            return Some(Located {
                kind: if on_close {
                    DockHitKind::ChipClose
                } else {
                    DockHitKind::Chip
                },
                group,
                tab,
                centre: pill.x + centre,
            });
        }
        let slivers: Vec<(i32, f32, f32)> = pill
            .slivers
            .iter()
            .map(|sliver| (sliver.tab, sliver.x, sliver.width))
            .collect();
        if let Some((tab, centre)) = nearest_span(&slivers, local) {
            return Some(Located {
                kind: DockHitKind::Sliver,
                group,
                tab,
                centre: pill.x + centre,
            });
        }
        Some(Located {
            kind: DockHitKind::Pill,
            group,
            tab: -1,
            centre: pill.x + pill.width / 2.0,
        })
    }

    /// The shrunk dock's contents for the same rows.
    pub fn capsule(&self) -> CapsuleFrame {
        capsule_frame(&self.groups, &self.tabs)
    }
}

/// Keeps the current group's pill at 1.0: over it nothing grows, over a folded pill it is not one of
/// the neighbours that grow.
fn pin_current(scales: &mut [f32], current: Option<usize>) {
    let Some(current) = current else { return };
    let on_current = scales
        .get(current)
        .is_some_and(|scale| *scale >= UNDER_POINTER - 1e-3);
    if on_current {
        scales.iter_mut().for_each(|scale| *scale = 1.0);
    } else if let Some(scale) = scales.get_mut(current) {
        *scale = 1.0;
    }
}

fn scale_rows<T>(rows: &[T], scale: f32, scaled: impl Fn(&T, f32) -> T) -> Vec<T> {
    rows.iter().map(|row| scaled(row, scale)).collect()
}
