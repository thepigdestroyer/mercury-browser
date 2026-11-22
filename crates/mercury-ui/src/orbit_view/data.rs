// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! From the layout and the rows to what `OrbitView` draws (brief §6 item 2): [`OrbitData`] is plain
//! data, [`OrbitModels`] keeps persistent Slint models and fills the `OrbitPlanet` rows (one
//! `ring_bands::RingBandsModels` per planet), the previews, chips, card and the other singles.
//!
//! # Conventions of the rows
//!
//! - Positions are stage coordinates. Planet, sun, "New group", preview and drag positions name a
//!   **centre**; the card, the neighbours, the chips, the tooltip and the popover name a **top-left**.
//! - A chip's `key-label` is the whole text, "⌥S" (empty without a key); a planet's tooltip and
//!   labels are the SPEC §5 wording.
//! - The ghost of a drag starts at the drop target (or at its source) and the host moves it with
//!   the pointer.
//! - The arrays of an `OrbitPlanet` come from the models ([`OrbitModels`]); in [`OrbitData`] they
//!   are plain vectors next to the row.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use mercury_appearance::group_colour::preset;
use slint::{Color, Image, Model, ModelRc, SharedString, VecModel};

use super::drag::{DropTarget, Source};
use super::layout::{CardCell, OrbitLayout, PlanetCell, PreviewCell, Rect, RingCell, tooltip_text};
use super::{Level, OrbitInput, Stage, Target, View};
use crate::ring_bands::{RingBandsData, RingBandsModels, RingColours, RingOptions};
use crate::ring_geometry::{GlyphMetrics, Point, RingLayout, RingSpec, point_on_circle};
use crate::{
    ArcSeg, FlagKind, FlagPin, GroupRow, OrbitCard, OrbitChip, OrbitDragView, OrbitNeighbour,
    OrbitNewGroup, OrbitPlanet, OrbitPopover, OrbitPreviewCard, OrbitSun, TabRow,
};

/// The preset names of the six colour dots of the new-group popover, in the mockup's order.
pub const POPOVER_COLOURS: [&str; 6] = ["blue", "amber", "violet", "rose", "green", "teal"];

/// The popover's size (mockup: 230 x 100).
const POPOVER_WIDTH: f32 = 230.0;
const POPOVER_HEIGHT: f32 = 100.0;
/// Size of a group badge at scale 1 (the unread dot is half of it).
const BADGE_SIZE: f32 = 18.0;
/// Angle of the first badge (clockwise from the top); the next ones are 20 degrees further round.
const BADGE_FIRST_DEG: f32 = 200.0;
const BADGE_STEP_DEG: f32 = 20.0;

/// What a build depends on besides the rows and the view.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Options {
    /// The colour scheme (the label colours of the rows are already resolved; the glyph colours
    /// are `colours`).
    pub dark: bool,
    /// The diameter of a state flag on a planet's tab band at scale 1 (Orbit passes less than the
    /// hub's 20).
    pub flag_size: f32,
    /// The glyph colours of the ring labels (`RingColours::from_palette` in the glue).
    pub colours: RingColours,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            dark: false,
            flag_size: 12.0,
            colours: RingColours::default(),
        }
    }
}

/// One planet or the centre ring: the Slint row without its model arrays, and the arrays as data.
#[derive(Clone, Debug, PartialEq)]
pub struct PlanetData {
    pub row: OrbitPlanet,
    pub ring: RingBandsData,
    pub badges: Vec<FlagPin>,
}

/// One preview card: the Slint row without its flags model, and the flags as data.
#[derive(Clone, Debug, PartialEq)]
pub struct PreviewData {
    pub row: OrbitPreviewCard,
    pub flags: Vec<FlagPin>,
}

/// The plain data of one frame.
#[derive(Clone, Debug, PartialEq)]
pub struct OrbitData {
    pub layout: OrbitLayout,
    pub planets: Vec<PlanetData>,
    pub centre: Option<PlanetData>,
    pub previews: Vec<PreviewData>,
    pub more_previews: usize,
    pub chips: Vec<OrbitChip>,
    pub neighbours: Vec<OrbitNeighbour>,
    pub card: OrbitCard,
    pub sun: OrbitSun,
    pub new_group: OrbitNewGroup,
    pub drag: OrbitDragView,
    /// Without its `colours` and `colour-names` (the models hold them).
    pub popover: OrbitPopover,
    pub crumbs: Vec<SharedString>,
}

thread_local! {
    static METRICS: Box<dyn GlyphMetrics> = crate::ring_bands::metrics();
}

/// "1 tab" or "10 tabs".
pub fn count_text(count: usize) -> String {
    if count == 1 {
        "1 tab".to_owned()
    } else {
        format!("{count} tabs")
    }
}

/// The label of a planet or a chip (SPEC §5): "Research, tab group, 10 tabs, key Option A".
pub fn group_label(group: &GroupRow, tabs: usize) -> String {
    let mut label = format!("{}, tab group, {}", group.name, count_text(tabs));
    if !group.key_label.is_empty() {
        label.push_str(&format!(", key Option {}", group.key_label));
    }
    label
}

/// The favicon chip's letter: the first letter or digit of the title, else of the host.
pub fn letter(title: &str, host: &str) -> SharedString {
    [title, host]
        .iter()
        .find_map(|text| text.chars().find(|c| c.is_alphanumeric()))
        .map(|c| c.to_uppercase().collect::<String>())
        .unwrap_or_default()
        .into()
}

impl OrbitData {
    /// Builds the frame for `view` with the real glyph metrics (`ring_bands::metrics()`, made once
    /// per thread). `snapshots(tab)` gives a tab's picture (the glue caches them); `None` draws the
    /// placeholder. It is asked once per tab that is shown, and not at all at level 1.
    pub fn build(
        stage: Stage,
        input: &OrbitInput,
        view: &View,
        snapshots: &dyn Fn(i32) -> Option<Image>,
        options: &Options,
    ) -> Self {
        METRICS.with(|metrics| {
            Self::build_with(stage, input, view, snapshots, options, metrics.as_ref())
        })
    }

    /// [`OrbitData::build`] with the given glyph metrics (tests pass `ApproxMetrics`).
    pub fn build_with(
        stage: Stage,
        input: &OrbitInput,
        view: &View,
        snapshots: &dyn Fn(i32) -> Option<Image>,
        options: &Options,
        metrics: &dyn GlyphMetrics,
    ) -> Self {
        let layout = OrbitLayout::new(stage, input, view);
        let ctx = Ctx {
            input,
            view,
            layout: &layout,
            options,
            metrics,
        };
        let planets: Vec<PlanetData> = layout
            .planets
            .iter()
            .filter_map(|cell| input.group(cell.group).map(|group| ctx.planet(cell, group)))
            .collect();
        let centre = layout
            .centre
            .as_ref()
            .and_then(|ring| input.group(ring.group).map(|group| ctx.centre(ring, group)));
        let previews = layout
            .previews
            .iter()
            .filter_map(|cell| ctx.preview(cell, snapshots))
            .collect();
        let chips = layout
            .chips
            .iter()
            .filter_map(|cell| {
                let group = input.group(cell.group)?;
                Some(OrbitChip {
                    group: group.id,
                    name: group.name.clone(),
                    key_label: key_text(group),
                    colour: group.colour,
                    x: cell.rect.x,
                    y: cell.rect.y,
                    width: cell.rect.w,
                    height: cell.rect.h,
                    accessible_label: group_label(group, ctx.tab_count(group.id)).into(),
                })
            })
            .collect();
        let neighbours = layout
            .neighbours
            .iter()
            .filter_map(|cell| {
                let tab = input.tab(cell.tab)?;
                let picture = snapshots(tab.id);
                let side = match cell.side {
                    super::layout::Side::Left => "previous",
                    super::layout::Side::Right => "next",
                };
                Some(OrbitNeighbour {
                    tab: tab.id,
                    x: cell.rect.x,
                    y: cell.rect.y,
                    width: cell.rect.w,
                    height: cell.rect.h,
                    title: tab.title.clone(),
                    has_picture: picture.is_some(),
                    picture: picture.unwrap_or_default(),
                    letter: letter(&tab.title, &tab.host),
                    shade: tab.shade,
                    accessible_label: format!("{}, {side} tab", tab.title).into(),
                })
            })
            .collect();
        let card = layout
            .card
            .as_ref()
            .and_then(|cell| ctx.card(cell, snapshots))
            .unwrap_or_default();
        let total = input.tabs.len();
        let sun = OrbitSun {
            x: layout.sun.centre.x,
            y: layout.sun.centre.y,
            radius: layout.sun.radius,
            title: input.profile_name.clone(),
            subtitle: format!("{} · {}", input.window_label, count_text(total)).into(),
            opacity: layout.sun.opacity,
        };
        let new_group = OrbitNewGroup {
            x: layout.new_group.centre.x,
            y: layout.new_group.centre.y,
            radius: layout.new_group.radius,
            lit: layout.new_group.visible
                && view.drag.as_ref().is_some_and(|drag| {
                    matches!(drag.source, Source::Tab(_))
                        && matches!(drag.over, Some(DropTarget::Empty | DropTarget::NewGroup))
                }),
            visible: layout.new_group.visible,
        };
        let drag = ctx.drag();
        let popover = ctx.popover();
        let crumbs = ctx.crumbs();
        let more_previews = layout.more;
        Self {
            layout,
            planets,
            centre,
            previews,
            more_previews,
            chips,
            neighbours,
            card,
            sun,
            new_group,
            drag,
            popover,
            crumbs,
        }
    }
}

fn key_text(group: &GroupRow) -> SharedString {
    if group.key_label.is_empty() {
        SharedString::new()
    } else {
        format!("⌥{}", group.key_label).into()
    }
}

/// What every part of a build reads.
struct Ctx<'a> {
    input: &'a OrbitInput,
    view: &'a View,
    layout: &'a OrbitLayout,
    options: &'a Options,
    metrics: &'a dyn GlyphMetrics,
}

impl Ctx<'_> {
    fn tab_count(&self, group: i32) -> usize {
        self.input.tabs.iter().filter(|t| t.group == group).count()
    }

    fn ring(&self, group: &GroupRow, spec: RingSpec, flag_size: f32) -> RingBandsData {
        let tabs: Vec<TabRow> = self.input.tabs_of(group.id).into_iter().cloned().collect();
        let options = RingOptions {
            held: false,
            selected_group: None,
            flag_size,
            colours: self.options.colours,
        };
        let groups = std::slice::from_ref(group);
        let mut ring = RingBandsData::new(spec, groups, &tabs, &options, self.metrics);
        // A group that fills the whole ring has its label at the middle of its arc, which is the
        // bottom (180 degrees) when the segments start at 12 o'clock. The mockup has it centred
        // at the top, so the label glyphs come from a second build whose arc starts 180 degrees
        // later: same box, same centre, same colours, the label's middle at 0 degrees. Everything
        // else (segments, label bands, flags) is the build with the normal spec.
        let turned = RingSpec {
            start_deg: spec.start_deg + 180.0,
            ..spec
        };
        ring.label_glyphs =
            RingBandsData::new(turned, groups, &tabs, &options, self.metrics).label_glyphs;
        ring
    }

    fn is_lit(&self, target: Target) -> bool {
        self.view.hover == Some(target) || self.view.focus == Some(target)
    }

    /// The tab being dragged, if the drag is a tab's.
    fn dragged_tab(&self) -> Option<&TabRow> {
        match self.view.drag.as_ref()?.source {
            Source::Tab(id) => self.input.tab(id),
            Source::Group(_) => None,
        }
    }

    fn planet(&self, cell: &PlanetCell, group: &GroupRow) -> PlanetData {
        let level_one = self.layout.level == Level::Groups;
        let over = self.view.drag.as_ref().and_then(|d| d.over);
        let over_group = match over {
            Some(DropTarget::Group { group: g, slot }) if g == group.id => Some(slot),
            _ => None,
        };
        let drop_lit = level_one
            && over_group.is_some()
            && match self.view.drag.as_ref().map(|d| d.source) {
                Some(Source::Tab(t)) => {
                    !self.input.is_pinned(t)
                        && self.input.tab(t).is_some_and(|tab| tab.group != group.id)
                }
                Some(Source::Group(g)) => g != group.id,
                None => false,
            };
        let drop_bar = if level_one {
            over_group.and_then(|slot| self.drop_bar(group.id, &cell.spec, slot))
        } else {
            None
        };
        let row = OrbitPlanet {
            group: group.id,
            name: group.name.clone(),
            x: cell.shown.x,
            y: cell.shown.y,
            scale: cell.scale,
            ring_size: cell.ring_size,
            halo_radius: cell.halo_radius,
            lit: self.is_lit(Target::Planet(group.id)),
            drop_lit,
            dimmed: cell.dimmed,
            opacity: cell.opacity,
            hole_text: group.last_used.clone(),
            tooltip: tooltip_text(&group.name).into(),
            tooltip_x: cell.tooltip.x,
            tooltip_y: cell.tooltip.y,
            accessible_label: group_label(group, cell.tabs).into(),
            dragging_tab: if level_one {
                self.dragging_in(group.id)
            } else {
                -1
            },
            has_drop_bar: drop_bar.is_some(),
            drop_bar: drop_bar.unwrap_or_default(),
            ..OrbitPlanet::default()
        };
        PlanetData {
            ring: self.ring(group, cell.spec, self.options.flag_size * cell.scale),
            badges: badges(group, &cell.spec, cell.scale),
            row,
        }
    }

    fn centre(&self, ring: &RingCell, group: &GroupRow) -> PlanetData {
        let tabs = self.tab_count(group.id);
        let level_two = self.layout.level == Level::Group;
        let over_slot = match self.view.drag.as_ref().and_then(|d| d.over) {
            Some(DropTarget::Group { group: g, slot }) if g == group.id => Some(slot),
            _ => None,
        };
        let drop_bar = if level_two {
            over_slot.and_then(|slot| self.drop_bar(group.id, &ring.spec, slot))
        } else {
            None
        };
        let hole_text = if !level_two {
            String::new()
        } else if group.last_used.is_empty() {
            count_text(tabs)
        } else {
            format!("{} · {}", count_text(tabs), group.last_used)
        };
        let row = OrbitPlanet {
            group: group.id,
            name: group.name.clone(),
            x: ring.centre.x,
            y: ring.centre.y,
            scale: ring.scale,
            ring_size: ring.ring_size,
            opacity: 1.0,
            hole_text: hole_text.into(),
            accessible_label: group_label(group, tabs).into(),
            dragging_tab: self.dragging_in(group.id),
            has_drop_bar: drop_bar.is_some(),
            drop_bar: drop_bar.unwrap_or_default(),
            ..OrbitPlanet::default()
        };
        PlanetData {
            ring: self.ring(group, ring.spec, self.options.flag_size * 1.5 * ring.scale),
            badges: Vec::new(),
            row,
        }
    }

    /// The tab being dragged if it is in `group`, else -1.
    fn dragging_in(&self, group: i32) -> i32 {
        self.dragged_tab()
            .filter(|tab| tab.group == group)
            .map_or(-1, |tab| tab.id)
    }

    /// The accent bar across `group`'s tab band at the insertion slot (`None` = the end), unless the
    /// slot would leave the dragged tab where it is, or the source is not a movable tab.
    fn drop_bar(&self, group: i32, spec: &RingSpec, slot: Option<usize>) -> Option<ArcSeg> {
        let tab = self.dragged_tab()?;
        if self.input.is_pinned(tab.id) {
            return None;
        }
        let own = self.input.tabs_of(group);
        let n = own.len();
        if n == 0 {
            return None;
        }
        let slot = slot.unwrap_or(n).min(n);
        if tab.group == group {
            let at = own.iter().position(|t| t.id == tab.id)?;
            if slot == at || slot == at + 1 {
                return None;
            }
        }
        let rings = RingLayout::new(*spec, &[n]);
        let deg = if slot < n {
            rings.tabs.get(slot)?.slot_start_deg
        } else {
            rings.tabs.get(n - 1)?.slot_end_deg
        };
        let mid = point_on_circle(spec.centre, spec.radius, deg);
        let half_length = (spec.band_width / 2.0 + 4.0).max(6.0);
        let half_width = 1.5;
        let rad = deg.to_radians();
        let (rx, ry) = (rad.sin(), -rad.cos());
        let (tx, ty) = (rad.cos(), rad.sin());
        let corner = |along: f32, across: f32| {
            (
                mid.x + rx * along + tx * across,
                mid.y + ry * along + ty * across,
            )
        };
        let c = [
            corner(-half_length, -half_width),
            corner(half_length, -half_width),
            corner(half_length, half_width),
            corner(-half_length, half_width),
        ];
        Some(ArcSeg {
            tab: -1,
            commands: format!(
                "M{:.2} {:.2} L{:.2} {:.2} L{:.2} {:.2} L{:.2} {:.2} Z",
                c[0].0, c[0].1, c[1].0, c[1].1, c[2].0, c[2].1, c[3].0, c[3].1
            )
            .into(),
            mid_x: mid.x,
            mid_y: mid.y,
        })
    }

    fn preview(
        &self,
        cell: &PreviewCell,
        snapshots: &dyn Fn(i32) -> Option<Image>,
    ) -> Option<PreviewData> {
        let tab = self.input.tab(cell.tab)?;
        let group = self.input.group(tab.group)?;
        let own = self.input.tabs_of(group.id);
        let position = own.iter().position(|t| t.id == tab.id).map_or(1, |i| i + 1);
        let picture = snapshots(tab.id);
        let lit = self.is_lit(Target::Preview(tab.id));
        let centre = cell.rect.centre();
        let k = cell.rect.w / super::layout::CARD_WIDTH;
        let pinned = self.input.is_pinned(tab.id);
        let mut label = format!(
            "{}, {}, tab {} of {}",
            tab.title,
            group.name,
            position,
            own.len()
        );
        if tab.loading {
            if tab.progress > 0.0 {
                label.push_str(&format!(
                    ", loading {} percent",
                    (tab.progress * 100.0).round()
                ));
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
        if tab.crashed {
            label.push_str(", crashed");
        }
        if tab.unread {
            label.push_str(", unread");
        }
        if pinned {
            label.push_str(", pinned");
        }
        let row = OrbitPreviewCard {
            tab: tab.id,
            x: centre.x,
            y: centre.y,
            width: cell.rect.w,
            height: cell.rect.h,
            scale: if self.view.hover == Some(Target::Preview(tab.id)) {
                1.04
            } else {
                1.0
            },
            title: tab.title.clone(),
            host: tab.host.clone(),
            has_picture: picture.is_some(),
            picture: picture.unwrap_or_default(),
            letter: letter(&tab.title, &tab.host),
            shade: tab.shade,
            current: tab.id == self.input.active_tab,
            lit,
            draggable: !pinned,
            spoke_x1: cell.spoke_from.x,
            spoke_y1: cell.spoke_from.y,
            spoke_x2: cell.spoke_to.x,
            spoke_y2: cell.spoke_to.y,
            accessible_label: label.into(),
            ..OrbitPreviewCard::default()
        };
        Some(PreviewData {
            flags: card_flags(tab, k),
            row,
        })
    }

    fn card(&self, cell: &CardCell, snapshots: &dyn Fn(i32) -> Option<Image>) -> Option<OrbitCard> {
        let tab = self.input.tab(cell.tab)?;
        let group = self.input.group(tab.group)?;
        let own = self.input.tabs_of(group.id);
        let position = own.iter().position(|t| t.id == tab.id).map_or(1, |i| i + 1);
        let picture = snapshots(tab.id);
        Some(OrbitCard {
            tab: tab.id,
            visible: true,
            x: cell.rect.x,
            y: cell.rect.y,
            width: cell.rect.w,
            height: cell.rect.h,
            title: tab.title.clone(),
            host: tab.host.clone(),
            caption: format!("{} · tab {} of {}", group.name, position, own.len()).into(),
            has_picture: picture.is_some(),
            picture: picture.unwrap_or_default(),
            letter: letter(&tab.title, &tab.host),
            shade: tab.shade,
            group_colour: group.colour,
            accessible_label: format!("Open {}", tab.title).into(),
        })
    }

    fn crumbs(&self) -> Vec<SharedString> {
        let mut crumbs: Vec<SharedString> = vec!["Orbit".into()];
        if let Some(group) = self.layout.group.and_then(|g| self.input.group(g)) {
            crumbs.push(group.name.clone());
            if let Some(tab) = self.layout.tab.and_then(|t| self.input.tab(t)) {
                crumbs.push(tab.title.clone());
            }
        }
        crumbs
    }

    fn drag(&self) -> OrbitDragView {
        let Some(drag) = self.view.drag.as_ref() else {
            return OrbitDragView::default();
        };
        let (text, colour, from) = match drag.source {
            Source::Tab(id) => match self.input.tab(id) {
                Some(tab) => {
                    let colour = self
                        .input
                        .group(tab.group)
                        .map(|g| g.colour)
                        .unwrap_or_default();
                    (tab.title.clone(), colour, self.tab_slot_middle(tab))
                }
                None => (SharedString::new(), Color::default(), None),
            },
            Source::Group(id) => match self.input.group(id) {
                Some(group) => {
                    let from = self
                        .layout
                        .planets
                        .iter()
                        .find(|p| p.group == id && self.layout.level == Level::Groups)
                        .map(|p| p.centre);
                    (group.name.clone(), group.colour, from)
                }
                None => (SharedString::new(), Color::default(), None),
            },
        };
        let ghost = match drag.over {
            Some(DropTarget::NewGroup) if self.layout.new_group.visible => {
                Some(self.layout.new_group.centre)
            }
            Some(DropTarget::Group { group, .. }) => match self.layout.level {
                Level::Groups => self
                    .layout
                    .planets
                    .iter()
                    .find(|p| p.group == group)
                    .map(|p| p.centre),
                _ => self.layout.centre.as_ref().map(|r| r.centre),
            },
            _ => None,
        }
        .or(from)
        .unwrap_or_default();
        OrbitDragView {
            active: true,
            ghost_x: ghost.x,
            ghost_y: ghost.y,
            ghost_text: text,
            ghost_colour: colour,
            from_x: from.map_or(0.0, |p| p.x),
            from_y: from.map_or(0.0, |p| p.y),
            has_from: from.is_some(),
        }
    }

    /// The middle of a tab's slot: on its planet's tab band at level 1, its card at level 2.
    fn tab_slot_middle(&self, tab: &TabRow) -> Option<Point> {
        match self.layout.level {
            Level::Groups => {
                let cell = self.layout.planets.iter().find(|p| p.group == tab.group)?;
                let own = self.input.tabs_of(tab.group);
                let at = own.iter().position(|t| t.id == tab.id)?;
                let deg = (at as f32 + 0.5) * 360.0 / own.len() as f32;
                Some(point_on_circle(cell.centre, cell.radius, deg))
            }
            Level::Group => self
                .layout
                .previews
                .iter()
                .find(|p| p.tab == tab.id)
                .map(|p| p.rect.centre()),
            Level::Tab => None,
        }
    }

    fn popover(&self) -> OrbitPopover {
        let Some(popover) = self.view.popover.as_ref() else {
            return OrbitPopover::default();
        };
        let stage = self.layout.stage;
        let ring = self.layout.new_group.centre;
        let rect = Rect::new(
            (ring.x - 130.0).clamp(8.0, (stage.width - 8.0 - POPOVER_WIDTH).max(8.0)),
            (ring.y - 238.0).max(super::TITLE_BAR_HEIGHT + 8.0),
            POPOVER_WIDTH,
            POPOVER_HEIGHT,
        );
        OrbitPopover {
            visible: true,
            x: rect.x,
            y: rect.y,
            name: popover.name.as_str().into(),
            selected: POPOVER_COLOURS
                .iter()
                .position(|name| *name == popover.colour)
                .map_or(-1, |i| i as i32),
            ..OrbitPopover::default()
        }
    }
}

/// The group-level badges at the bottom of the label band: spinner, speaker and unread dot, from
/// 200 degrees on in steps of -20.
fn badges(group: &GroupRow, spec: &RingSpec, scale: f32) -> Vec<FlagPin> {
    [
        (group.any_loading, FlagKind::Spinner, BADGE_SIZE),
        (group.any_audible, FlagKind::Audible, BADGE_SIZE),
        (group.any_unread, FlagKind::Unread, BADGE_SIZE / 2.0),
    ]
    .into_iter()
    .filter(|(on, _, _)| *on)
    .enumerate()
    .map(|(i, (_, kind, size))| {
        let at = point_on_circle(
            spec.centre,
            spec.label_radius(),
            BADGE_FIRST_DEG - BADGE_STEP_DEG * i as f32,
        );
        FlagPin {
            tab: -1,
            kind,
            x: at.x,
            y: at.y,
            size: size * scale,
            progress: -1.0,
            label: SharedString::new(),
        }
    })
    .collect()
}

/// The flags of a preview card, card-local: from the right edge in, 20 px in and 8 px down (the
/// mockup's badge), at most three. `k` is the card's size relative to 184 x 124.
fn card_flags(tab: &TabRow, k: f32) -> Vec<FlagPin> {
    let width = super::layout::CARD_WIDTH * k;
    let kinds = [
        (tab.crashed, FlagKind::Crashed),
        (tab.loading, FlagKind::Spinner),
        (tab.audible && !tab.muted, FlagKind::Audible),
        (tab.muted, FlagKind::Muted),
        (tab.capture, FlagKind::Capture),
        (tab.unread, FlagKind::Unread),
    ];
    kinds
        .into_iter()
        .filter(|(on, _)| *on)
        .take(3)
        .enumerate()
        .map(|(i, (_, kind))| FlagPin {
            tab: tab.id,
            kind,
            x: width - (20.0 + 24.0 * i as f32) * k,
            y: 8.0 * k,
            size: 20.0 * k,
            progress: if kind == FlagKind::Spinner && tab.progress > 0.0 {
                tab.progress
            } else {
                -1.0
            },
            label: SharedString::new(),
        })
        .collect()
}

/// One ring's models and what they last held (an unchanged ring is not set again, so a hover does
/// not rebuild the rings' items).
struct RingSlot {
    ring: RingBandsModels,
    badges: Rc<VecModel<FlagPin>>,
    last: RefCell<Option<(RingBandsData, Vec<FlagPin>)>>,
}

impl RingSlot {
    fn new() -> Self {
        Self {
            ring: RingBandsModels::new(),
            badges: Rc::new(VecModel::default()),
            last: RefCell::new(None),
        }
    }

    /// The row of `data` with the arrays of this slot's models.
    fn row(&self, data: &PlanetData) -> OrbitPlanet {
        let changed = !self
            .last
            .borrow()
            .as_ref()
            .is_some_and(|(ring, badges)| *ring == data.ring && *badges == data.badges);
        if changed {
            self.ring.update(&data.ring);
            self.badges.set_vec(data.badges.clone());
            *self.last.borrow_mut() = Some((data.ring.clone(), data.badges.clone()));
        }
        OrbitPlanet {
            groups: self.ring.groups(),
            tabs: self.ring.tabs(),
            segments: self.ring.segments(),
            lifted_segments: self.ring.lifted_segments(),
            segment_labels: self.ring.segment_labels(),
            label_bands: self.ring.label_bands(),
            label_glyphs: self.ring.label_glyphs(),
            flags: self.ring.flags(),
            badges: ModelRc::from(Rc::clone(&self.badges)),
            ..data.row.clone()
        }
    }
}

/// Sets the rows of `model`: the rows that are there are replaced only when different, the extra
/// ones are removed or added (so the items keep their identity, and their animations).
fn sync_rows<T: Clone + PartialEq + 'static>(model: &VecModel<T>, rows: Vec<T>) {
    let old = model.row_count();
    let mut rows = rows.into_iter();
    for i in 0..old {
        match rows.next() {
            Some(row) => {
                if model.row_data(i).as_ref() != Some(&row) {
                    model.set_row_data(i, row);
                }
            }
            None => {
                while model.row_count() > i {
                    model.remove(model.row_count() - 1);
                }
                return;
            }
        }
    }
    for row in rows {
        model.push(row);
    }
}

/// The persistent models of `OrbitView`: `update` keeps every `ModelRc` identity.
pub struct OrbitModels {
    planets: Rc<VecModel<OrbitPlanet>>,
    previews: Rc<VecModel<OrbitPreviewCard>>,
    chips: Rc<VecModel<OrbitChip>>,
    neighbours: Rc<VecModel<OrbitNeighbour>>,
    crumbs: Rc<VecModel<SharedString>>,
    rings: RefCell<Vec<RingSlot>>,
    centre_ring: RingSlot,
    preview_flags: RefCell<Vec<Rc<VecModel<FlagPin>>>>,
    centre: RefCell<OrbitPlanet>,
    has_centre: Cell<bool>,
    card: RefCell<OrbitCard>,
    sun: RefCell<OrbitSun>,
    new_group: RefCell<OrbitNewGroup>,
    drag: RefCell<OrbitDragView>,
    popover: RefCell<OrbitPopover>,
    more_previews: Cell<i32>,
    popover_colours: ModelRc<Color>,
    popover_names: ModelRc<SharedString>,
}

impl Default for OrbitModels {
    fn default() -> Self {
        Self::new()
    }
}

impl OrbitModels {
    /// Empty models.
    pub fn new() -> Self {
        let colours: Vec<Color> = POPOVER_COLOURS
            .iter()
            .map(|name| {
                preset(name).map_or_else(Color::default, |p| {
                    Color::from_rgb_u8(p.base.r, p.base.g, p.base.b)
                })
            })
            .collect();
        let names: Vec<SharedString> = POPOVER_COLOURS.iter().map(|n| (*n).into()).collect();
        let popover_colours = ModelRc::new(VecModel::from(colours));
        let popover_names = ModelRc::new(VecModel::from(names));
        Self {
            planets: Rc::new(VecModel::default()),
            previews: Rc::new(VecModel::default()),
            chips: Rc::new(VecModel::default()),
            neighbours: Rc::new(VecModel::default()),
            crumbs: Rc::new(VecModel::default()),
            rings: RefCell::new(Vec::new()),
            centre_ring: RingSlot::new(),
            preview_flags: RefCell::new(Vec::new()),
            centre: RefCell::new(OrbitPlanet::default()),
            has_centre: Cell::new(false),
            card: RefCell::new(OrbitCard::default()),
            sun: RefCell::new(OrbitSun::default()),
            new_group: RefCell::new(OrbitNewGroup::default()),
            drag: RefCell::new(OrbitDragView::default()),
            popover: RefCell::new(OrbitPopover {
                colours: popover_colours.clone(),
                colour_names: popover_names.clone(),
                ..OrbitPopover::default()
            }),
            more_previews: Cell::new(0),
            popover_colours,
            popover_names,
        }
    }

    /// Fills the models from a frame. The planets' ring models grow and shrink with the planets; the
    /// centre ring keeps its last content while it is not shown, so that a zoom out can animate.
    pub fn update(&self, data: &OrbitData) {
        {
            let mut rings = self.rings.borrow_mut();
            while rings.len() < data.planets.len() {
                rings.push(RingSlot::new());
            }
            rings.truncate(data.planets.len());
            let rows: Vec<OrbitPlanet> = data
                .planets
                .iter()
                .zip(rings.iter())
                .map(|(planet, slot)| slot.row(planet))
                .collect();
            sync_rows(&self.planets, rows);
        }
        if let Some(centre) = &data.centre {
            *self.centre.borrow_mut() = self.centre_ring.row(centre);
        }
        self.has_centre.set(data.centre.is_some());

        {
            let mut flags = self.preview_flags.borrow_mut();
            while flags.len() < data.previews.len() {
                flags.push(Rc::new(VecModel::default()));
            }
            let rows: Vec<OrbitPreviewCard> = data
                .previews
                .iter()
                .zip(flags.iter())
                .map(|(preview, model)| {
                    if model.iter().ne(preview.flags.iter().cloned()) {
                        model.set_vec(preview.flags.clone());
                    }
                    OrbitPreviewCard {
                        flags: ModelRc::from(Rc::clone(model)),
                        ..preview.row.clone()
                    }
                })
                .collect();
            sync_rows(&self.previews, rows);
        }
        self.more_previews.set(data.more_previews as i32);
        sync_rows(&self.chips, data.chips.clone());
        sync_rows(&self.neighbours, data.neighbours.clone());
        sync_rows(&self.crumbs, data.crumbs.clone());
        *self.card.borrow_mut() = data.card.clone();
        *self.sun.borrow_mut() = data.sun.clone();
        *self.new_group.borrow_mut() = data.new_group.clone();
        *self.drag.borrow_mut() = data.drag.clone();
        *self.popover.borrow_mut() = OrbitPopover {
            colours: self.popover_colours.clone(),
            colour_names: self.popover_names.clone(),
            ..data.popover.clone()
        };
    }

    pub fn planets(&self) -> ModelRc<OrbitPlanet> {
        ModelRc::from(Rc::clone(&self.planets))
    }

    pub fn previews(&self) -> ModelRc<OrbitPreviewCard> {
        ModelRc::from(Rc::clone(&self.previews))
    }

    pub fn chips(&self) -> ModelRc<OrbitChip> {
        ModelRc::from(Rc::clone(&self.chips))
    }

    pub fn neighbours(&self) -> ModelRc<OrbitNeighbour> {
        ModelRc::from(Rc::clone(&self.neighbours))
    }

    pub fn crumbs(&self) -> ModelRc<SharedString> {
        ModelRc::from(Rc::clone(&self.crumbs))
    }

    /// The single-valued rows of the last `update`.
    pub fn centre(&self) -> OrbitPlanet {
        self.centre.borrow().clone()
    }

    pub fn has_centre(&self) -> bool {
        self.has_centre.get()
    }

    pub fn card(&self) -> OrbitCard {
        self.card.borrow().clone()
    }

    pub fn sun(&self) -> OrbitSun {
        self.sun.borrow().clone()
    }

    pub fn new_group(&self) -> OrbitNewGroup {
        self.new_group.borrow().clone()
    }

    pub fn drag(&self) -> OrbitDragView {
        self.drag.borrow().clone()
    }

    pub fn popover(&self) -> OrbitPopover {
        self.popover.borrow().clone()
    }

    pub fn more_previews(&self) -> i32 {
        self.more_previews.get()
    }
}
