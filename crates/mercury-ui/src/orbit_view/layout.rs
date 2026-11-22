// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! The stage layout of the three Orbit levels (SPEC §4.6, brief §4 and D12): where the sun, the
//! planets, the "New group" ring, the preview cards, the chips and the level-3 card sit for a window
//! size, and which of them a point is over. Pure functions of the rows, the machine's [`View`] and the
//! stage; positions come from `ring_geometry::orbit` (`planet_layout`, `preview_layout`).
//!
//! # Rules
//!
//! - **Level 1.** The planets sit on the mockup's 470 x 290 ellipse round the stage centre, scaled
//!   *down* (never up) by the largest factor at which every planet's halo is inside the stage (clear of
//!   the title bar, the footer and the sides) and clear of the "New group" ring; when no factor does,
//!   the ring is ignored, and at the floor (0.25) the planets are placed as they are. A planet's
//!   tab band has radius `40 + 2 x tabs` (at most [`MAX_PLANET_RADIUS`]) and width 16, times the
//!   planet's scale; the label band (gap 2, width 22) scales with it.
//! - **Level 2.** The group's ring (radius 92, band 30) in the middle; cards of 184 x 124 on the
//!   520 x 292 ellipse, all scaled by the same factor when the window is small; at most 24 cards, the
//!   inner ellipse's cards (13th on) at 0.58 and *first* in draw order.
//! - **Level 3.** A card 68 % of the window wide (980 x 660 at 1440 x 900), the previous and next
//!   tab's cards half off the stage, the group's ring huge behind (tab band outer radius 560).
//! - **Order.** Planets follow the groups; cards follow [`OrbitLayout::preview_order`].

use crate::ring_geometry::orbit::{
    MAX_PREVIEWS, PREVIEWS_PER_RING, PlanetLayout, planet_layout, preview_layout,
};
use crate::ring_geometry::{ApproxMetrics, Ellipse, GlyphMetrics, Point, RingSpec};

use super::{FOOTER_HEIGHT, Level, OrbitInput, Stage, TITLE_BAR_HEIGHT, Target, View};
use crate::GroupRow;

/// Radius of the sun.
pub const SUN_RADIUS: f32 = 92.0;
/// Radius of the sun's halo.
pub const SUN_GLOW_RADIUS: f32 = 133.4;
/// Radius of the dashed "New group" ring.
pub const NEW_GROUP_RADIUS: f32 = 50.0;
/// Width of a planet's tab band at scale 1.
pub const PLANET_BAND: f32 = 16.0;
/// The tab band's radius never exceeds this (a group of 14 tabs or more), so that big groups do not
/// grow into their neighbours.
pub const MAX_PLANET_RADIUS: f32 = 68.0;
/// The hover halo reaches this far beyond the label band (times the planet's scale).
pub const HALO_MARGIN: f32 = 11.0;
/// A level-2 preview card at scale 1.
pub const CARD_WIDTH: f32 = 184.0;
pub const CARD_HEIGHT: f32 = 124.0;
/// The level-1 ellipse's half-axes.
pub const LEVEL_1_ELLIPSE: Ellipse = Ellipse {
    rx: 470.0,
    ry: 290.0,
};
/// The level-2 ellipse's half-axes.
pub const LEVEL_2_ELLIPSE: Ellipse = Ellipse {
    rx: 520.0,
    ry: 292.0,
};
/// Where the footer starts, measured from the bottom of the stage (the key hints live there).
pub const FOOTER_BAND: f32 = 48.0;
/// Height of a chip.
pub const CHIP_HEIGHT: f32 = 24.0;
/// The chip row's top, measured from the bottom of the stage.
pub const CHIP_ROW_FROM_BOTTOM: f32 = 54.0;
/// Where the first chip starts (after "OTHER GROUPS" at [`OTHER_GROUPS_X`]).
pub const CHIPS_X: f32 = 128.0;
pub const OTHER_GROUPS_X: f32 = 30.0;
/// Space between two chips.
pub const CHIP_GAP: f32 = 28.0;
/// The key hints take this much of the chip row's right end.
const HINTS_WIDTH: f32 = 520.0;
/// The level-2 ring: tab band radius and width.
pub const CENTRE_RADIUS: f32 = 92.0;
pub const CENTRE_BAND: f32 = 30.0;
/// The level-3 ring: tab band radius (outer radius 560) and width.
pub const BIG_RADIUS: f32 = 546.0;
pub const BIG_BAND: f32 = 28.0;
/// The inner preview ellipse's cards are this much smaller.
const INNER_CARD_FACTOR: f32 = 0.58;
/// The level-3 card is this share of the window's width, at most.
const CARD_SHARE: f32 = 0.68;
/// Space kept between a thing and the window's edge.
const EDGE: f32 = 8.0;
/// Planets of the other groups slide this far out of the stage while a group is shown.
const SLIDE_OUT: f32 = 1.5;
/// A tooltip's height and the space between it and the halo.
const TOOLTIP_HEIGHT: f32 = 32.0;
const TOOLTIP_GAP: f32 = 8.0;

/// A rectangle by its top-left corner.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    pub const fn new(x: f32, y: f32, w: f32, h: f32) -> Self {
        Self { x, y, w, h }
    }

    pub fn right(&self) -> f32 {
        self.x + self.w
    }

    pub fn bottom(&self) -> f32 {
        self.y + self.h
    }

    pub fn centre(&self) -> Point {
        Point::new(self.x + self.w / 2.0, self.y + self.h / 2.0)
    }

    /// Whether the point is inside (left and top edges count, right and bottom edges do not).
    pub fn contains(&self, x: f32, y: f32) -> bool {
        x >= self.x && x < self.right() && y >= self.y && y < self.bottom()
    }
}

/// Which side of something a card sits on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    Left,
    Right,
}

/// One planet (level 1; kept at levels 2 and 3 so that the view can animate it away).
#[derive(Clone, Debug, PartialEq)]
pub struct PlanetCell {
    /// `GroupRow.id` (`LOOSE_GROUP_ID` for the pseudo planet).
    pub group: i32,
    pub tabs: usize,
    /// The level-1 centre, in stage coordinates.
    pub centre: Point,
    pub scale: f32,
    /// Radius of the tab band's centre line (`40 + 2 x tabs` times `scale`).
    pub radius: f32,
    /// Twice the label band's outer radius: the drawn disc.
    pub diameter: f32,
    /// For `RingBandsData::new`: ring-local, centred on (`ring_size / 2`, `ring_size / 2`).
    pub spec: RingSpec,
    /// The side of the square ring box, centred on the planet.
    pub ring_size: f32,
    /// The hover and drop halo (103 at scale 1 for 10 tabs).
    pub halo_radius: f32,
    pub tooltip: Rect,
    pub tooltip_side: Side,
    /// Where the planet is drawn at the current level (the centre at level 1).
    pub shown: Point,
    pub opacity: f32,
    pub dimmed: bool,
}

/// The sun.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SunCell {
    pub centre: Point,
    pub radius: f32,
    pub glow_radius: f32,
    pub opacity: f32,
}

/// The dashed "New group" ring.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct NewGroupCell {
    pub centre: Point,
    pub radius: f32,
    pub visible: bool,
}

/// The shown group's ring at levels 2 and 3.
#[derive(Clone, Debug, PartialEq)]
pub struct RingCell {
    pub group: i32,
    pub centre: Point,
    pub spec: RingSpec,
    pub ring_size: f32,
    pub scale: f32,
}

/// One preview card (level 2).
#[derive(Clone, Debug, PartialEq)]
pub struct PreviewCell {
    pub tab: i32,
    /// Position in the preview order (the active tab is 0).
    pub order: usize,
    /// 0 for the outer ellipse, 1 for the inner one.
    pub ring: usize,
    pub rect: Rect,
    pub spoke_from: Point,
    pub spoke_to: Point,
}

/// One "Other groups" chip (level 2).
#[derive(Clone, Debug, PartialEq)]
pub struct ChipCell {
    pub group: i32,
    pub rect: Rect,
}

/// A neighbour card at level 3.
#[derive(Clone, Debug, PartialEq)]
pub struct NeighbourCell {
    pub tab: i32,
    pub rect: Rect,
    pub side: Side,
}

/// The level-3 card.
#[derive(Clone, Debug, PartialEq)]
pub struct CardCell {
    pub tab: i32,
    pub rect: Rect,
}

/// A stage point inside a ring's box, in the ring's own coordinates (what `RingBandsData::hit` takes).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RingPoint {
    pub group: i32,
    pub x: f32,
    pub y: f32,
    /// The level-2 centre ring (otherwise a level-1 planet).
    pub centre: bool,
}

/// Everything positioned for one frame.
#[derive(Clone, Debug, PartialEq)]
pub struct OrbitLayout {
    pub stage: Stage,
    /// The level that is laid out (the view's level, or a lower one when its group or tab is gone).
    pub level: Level,
    /// The shown group at levels 2 and 3.
    pub group: Option<i32>,
    /// The shown tab at level 3.
    pub tab: Option<i32>,
    pub title_bar: Rect,
    pub footer: Rect,
    /// The scale of the level-1 ellipse (1 unless the window is small).
    pub fit: f32,
    pub ellipse: Ellipse,
    pub sun: SunCell,
    pub new_group: NewGroupCell,
    pub planets: Vec<PlanetCell>,
    pub centre: Option<RingCell>,
    /// Inner ring first (drawn first), then the outer ring.
    pub previews: Vec<PreviewCell>,
    /// Tabs beyond the 24 that are shown ("+N more").
    pub more: usize,
    /// The left end and the middle height of the "OTHER GROUPS" caption.
    pub other_groups_label: Point,
    pub chips: Vec<ChipCell>,
    pub card: Option<CardCell>,
    pub neighbours: Vec<NeighbourCell>,
}

/// The radius of a planet's tab band at scale 1 for a group of `tabs` tabs.
pub fn planet_radius(tabs: usize) -> f32 {
    (40.0 + 2.0 * tabs as f32).min(MAX_PLANET_RADIUS)
}

/// The tooltip of a planet.
pub fn tooltip_text(name: &str) -> String {
    format!("{name} · click or pinch in to zoom")
}

/// The chip's width: dot, name and key (an estimate: the real text is set by Slint).
pub fn chip_width(group: &GroupRow) -> f32 {
    let mut width = 16.0 + 6.0 + text_width(&group.name, 12.0, 400);
    if !group.key_label.is_empty() {
        width += 8.0 + text_width(&format!("⌥{}", group.key_label), 11.0, 700);
    }
    width
}

fn text_width(text: &str, size: f32, weight: u16) -> f32 {
    text.chars()
        .map(|c| ApproxMetrics.advance(c, size, weight))
        .sum()
}

/// A scaled ring of the planet shape: (spec centred in its box, side of the box).
fn ring_spec(radius: f32, band: f32, scale: f32) -> (RingSpec, f32) {
    let mut spec = RingSpec::planet(Point::new(0.0, 0.0), radius * scale, band * scale);
    spec.label_gap *= scale;
    spec.label_width *= scale;
    let half = (spec.label_outer_radius() + spec.lift).ceil() + 2.0;
    spec.centre = Point::new(half, half);
    (spec, half * 2.0)
}

fn dist(a: Point, b: Point) -> f32 {
    (a.x - b.x).hypot(a.y - b.y)
}

fn halo_of(tabs: usize, scale: f32) -> f32 {
    (planet_radius(tabs) + 32.0 + HALO_MARGIN) * scale
}

/// The largest ellipse factor (and its planets) at which every halo is inside `area` and, when
/// `avoid` is given, clear of that circle.
fn fit_planets(
    stage: Stage,
    counts: &[usize],
    avoid: Option<(Point, f32)>,
) -> Option<(f32, PlanetLayout)> {
    let (cx, cy) = stage.centre();
    let top = TITLE_BAR_HEIGHT + EDGE;
    let bottom = stage.height - FOOTER_BAND - EDGE;
    (25..=100_u32).rev().find_map(|percent| {
        let fit = percent as f32 / 100.0;
        let layout = planet_layout(
            counts.len(),
            Ellipse {
                rx: LEVEL_1_ELLIPSE.rx * fit,
                ry: LEVEL_1_ELLIPSE.ry * fit,
            },
        );
        let inside = layout.centres.iter().zip(counts).all(|(p, tabs)| {
            let (x, y) = (cx + p.x, cy + p.y);
            let halo = halo_of(*tabs, layout.scale);
            let clear = avoid.is_none_or(|(c, r)| dist(Point::new(x, y), c) >= halo + r + 4.0);
            x - halo >= EDGE
                && x + halo <= stage.width - EDGE
                && y - halo >= top
                && y + halo <= bottom
                && clear
        });
        inside.then_some((fit, layout))
    })
}

impl OrbitLayout {
    /// Lays out the level and focus in `view` for these rows on `stage`.
    pub fn new(stage: Stage, input: &OrbitInput, view: &View) -> Self {
        let (cx, cy) = stage.centre();
        let centre = Point::new(cx, cy);
        let group = view.group.filter(|g| input.group(*g).is_some());
        let tab = view
            .tab
            .filter(|t| input.tab(*t).is_some_and(|row| Some(row.group) == group));
        let level = match (view.level, group, tab) {
            (Level::Tab, Some(_), Some(_)) => Level::Tab,
            (Level::Tab | Level::Group, Some(_), _) => Level::Group,
            _ => Level::Groups,
        };
        let shown_group = group.filter(|_| level != Level::Groups);
        let shown_tab = tab.filter(|_| level == Level::Tab);

        let title_bar = Rect::new(0.0, 0.0, stage.width, TITLE_BAR_HEIGHT);
        let footer = Rect::new(
            0.0,
            stage.height - FOOTER_BAND,
            stage.width,
            FOOTER_BAND - FOOTER_HEIGHT,
        );
        let new_group = NewGroupCell {
            centre: Point::new(
                (stage.width - 190.0).clamp(
                    NEW_GROUP_RADIUS + EDGE,
                    (stage.width - NEW_GROUP_RADIUS - EDGE).max(NEW_GROUP_RADIUS + EDGE),
                ),
                (stage.height - 100.0)
                    .min(footer.y - NEW_GROUP_RADIUS)
                    .max(TITLE_BAR_HEIGHT + NEW_GROUP_RADIUS),
            ),
            radius: NEW_GROUP_RADIUS,
            visible: level == Level::Groups,
        };

        let counts: Vec<usize> = input
            .groups
            .iter()
            .map(|g| input.tabs.iter().filter(|t| t.group == g.id).count())
            .collect();
        let (fit, plan) = fit_planets(stage, &counts, Some((new_group.centre, NEW_GROUP_RADIUS)))
            .or_else(|| fit_planets(stage, &counts, None))
            .unwrap_or_else(|| {
                let fit = 0.25;
                let layout = planet_layout(
                    counts.len(),
                    Ellipse {
                        rx: LEVEL_1_ELLIPSE.rx * fit,
                        ry: LEVEL_1_ELLIPSE.ry * fit,
                    },
                );
                (fit, layout)
            });

        let planets = input
            .groups
            .iter()
            .zip(&counts)
            .zip(&plan.centres)
            .map(|((row, tabs), p)| {
                let home = Point::new(cx + p.x, cy + p.y);
                let (spec, ring_size) = ring_spec(planet_radius(*tabs), PLANET_BAND, plan.scale);
                let halo_radius = spec.label_outer_radius() + HALO_MARGIN * plan.scale;
                let (tooltip, tooltip_side) =
                    tooltip_for(&tooltip_text(&row.name), home, halo_radius, stage);
                let (shown, opacity, dimmed) = match shown_group {
                    None => (home, 1.0, false),
                    Some(g) if g == row.id => (centre, 0.0, false),
                    Some(_) => (
                        Point::new(
                            cx + (home.x - cx) * SLIDE_OUT,
                            cy + (home.y - cy) * SLIDE_OUT,
                        ),
                        0.0,
                        true,
                    ),
                };
                PlanetCell {
                    group: row.id,
                    tabs: *tabs,
                    centre: home,
                    scale: plan.scale,
                    radius: spec.radius,
                    diameter: 2.0 * spec.label_outer_radius(),
                    spec,
                    ring_size,
                    halo_radius,
                    tooltip,
                    tooltip_side,
                    shown,
                    opacity,
                    dimmed,
                }
            })
            .collect();

        let mut layout = Self {
            stage,
            level,
            group: shown_group,
            tab: shown_tab,
            title_bar,
            footer,
            fit,
            ellipse: Ellipse {
                rx: LEVEL_1_ELLIPSE.rx * fit,
                ry: LEVEL_1_ELLIPSE.ry * fit,
            },
            sun: SunCell {
                centre,
                radius: SUN_RADIUS,
                glow_radius: SUN_GLOW_RADIUS,
                opacity: if level == Level::Groups { 1.0 } else { 0.0 },
            },
            new_group,
            planets,
            centre: None,
            previews: Vec::new(),
            more: 0,
            other_groups_label: Point::new(
                OTHER_GROUPS_X,
                stage.height - CHIP_ROW_FROM_BOTTOM + CHIP_HEIGHT / 2.0,
            ),
            chips: Vec::new(),
            card: None,
            neighbours: Vec::new(),
        };
        match (level, shown_group, shown_tab) {
            (Level::Group, Some(g), _) => layout.level_two(input, g),
            (Level::Tab, Some(g), Some(t)) => layout.level_three(input, g, t),
            _ => {}
        }
        layout
    }

    /// The scale of everything at level 2 that makes it fit the window (1 for 1440 x 900).
    fn detail_scale(stage: Stage) -> f32 {
        let (_, cy) = stage.centre();
        let top = TITLE_BAR_HEIGHT + EDGE;
        let bottom = stage.height - CHIP_ROW_FROM_BOTTOM - 6.0;
        let half_h = (cy - top).min(bottom - cy);
        let half_w = stage.width / 2.0 - EDGE;
        // Half a card and its outline stick out of the ellipse.
        let fit_h = half_h / (LEVEL_2_ELLIPSE.ry + CARD_HEIGHT / 2.0 + 2.0);
        let fit_w = half_w / (LEVEL_2_ELLIPSE.rx + CARD_WIDTH / 2.0 + 3.0);
        fit_h.min(fit_w).clamp(0.2, 1.0)
    }

    fn level_two(&mut self, input: &OrbitInput, group: i32) {
        let stage = self.stage;
        let (cx, cy) = stage.centre();
        let centre = Point::new(cx, cy);
        let e = Self::detail_scale(stage);
        let (spec, ring_size) = ring_spec(CENTRE_RADIUS, CENTRE_BAND, e);
        let outer = spec.label_outer_radius();
        self.centre = Some(RingCell {
            group,
            centre,
            spec,
            ring_size,
            scale: e,
        });

        let order = Self::preview_order(input, group);
        let shown = order.len().min(MAX_PREVIEWS);
        self.more = order.len() - shown;
        let points = preview_layout(
            shown,
            Ellipse {
                rx: LEVEL_2_ELLIPSE.rx * e,
                ry: LEVEL_2_ELLIPSE.ry * e,
            },
        );
        let mut cells: Vec<PreviewCell> = points
            .iter()
            .zip(&order)
            .enumerate()
            .map(|(i, (p, tab))| {
                let ring = usize::from(i >= PREVIEWS_PER_RING);
                let factor = if ring == 1 { INNER_CARD_FACTOR } else { 1.0 };
                let (w, h) = (CARD_WIDTH * e * factor, CARD_HEIGHT * e * factor);
                let middle = Point::new(cx + p.x, cy + p.y);
                let rect = Rect::new(middle.x - w / 2.0, middle.y - h / 2.0, w, h);
                let (spoke_from, spoke_to) = spoke(centre, outer, rect);
                PreviewCell {
                    tab: *tab,
                    order: i,
                    ring,
                    rect,
                    spoke_from,
                    spoke_to,
                }
            })
            .collect();
        // The inner ring is drawn first so that no outer card is hidden by an inner one.
        cells.sort_by_key(|cell| std::cmp::Reverse(cell.ring));
        self.previews = cells;

        let limit = stage.width - HINTS_WIDTH;
        let y = stage.height - CHIP_ROW_FROM_BOTTOM;
        let mut x = CHIPS_X;
        for row in input.groups.iter().filter(|g| g.id != group) {
            let w = chip_width(row);
            if x + w > limit {
                break;
            }
            self.chips.push(ChipCell {
                group: row.id,
                rect: Rect::new(x, y, w, CHIP_HEIGHT),
            });
            x += w + CHIP_GAP;
        }
    }

    fn level_three(&mut self, input: &OrbitInput, group: i32, tab: i32) {
        let stage = self.stage;
        let (cx, cy) = stage.centre();
        let scale = (stage.width / 1440.0).min(stage.height / 900.0).min(1.0);
        let (spec, ring_size) = ring_spec(BIG_RADIUS, BIG_BAND, scale);
        self.centre = Some(RingCell {
            group,
            centre: Point::new(cx, cy),
            spec,
            ring_size,
            scale,
        });

        let h = stage.height * (660.0 / 900.0);
        let w = (CARD_SHARE * stage.width).min(h * (980.0 / 660.0));
        let free = stage.height - TITLE_BAR_HEIGHT - FOOTER_HEIGHT - h;
        let rect = Rect::new((stage.width - w) / 2.0, TITLE_BAR_HEIGHT + free / 3.0, w, h);
        self.card = Some(CardCell { tab, rect });

        let nw = stage.width * (420.0 / 1440.0);
        let nh = nw * (300.0 / 420.0);
        let ny = rect.centre().y - 40.0 * (stage.height / 900.0) - nh / 2.0;
        let offset = stage.width * (740.0 / 1440.0);
        let own = input.tabs_of(group);
        if let Some(at) = own.iter().position(|t| t.id == tab) {
            if let Some(previous) = at.checked_sub(1).and_then(|i| own.get(i)) {
                self.neighbours.push(NeighbourCell {
                    tab: previous.id,
                    rect: Rect::new(cx - offset - nw / 2.0, ny, nw, nh),
                    side: Side::Left,
                });
            }
            if let Some(next) = own.get(at + 1) {
                self.neighbours.push(NeighbourCell {
                    tab: next.id,
                    rect: Rect::new(cx + offset - nw / 2.0, ny, nw, nh),
                    side: Side::Right,
                });
            }
        }
    }

    /// The tabs of a group in preview order: the active tab first, then strip order (D5). All of
    /// them; only the first 24 get a card.
    pub fn preview_order(input: &OrbitInput, group: i32) -> Vec<i32> {
        let mut tabs: Vec<i32> = input.tabs_of(group).iter().map(|t| t.id).collect();
        if let Some(at) = tabs.iter().position(|id| *id == input.active_tab) {
            let active = tabs.remove(at);
            tabs.insert(0, active);
        }
        tabs
    }

    /// What the stage point (`x`, `y`) is over, by Orbit's own geometry (halo circle, card rectangle,
    /// chip rectangle, sun, "New group" ring). A planet's segments are `ring_bands`' to resolve: see
    /// [`OrbitLayout::ring_at`].
    pub fn hit(&self, x: f32, y: f32) -> Target {
        let at = Point::new(x, y);
        match self.level {
            Level::Groups => {
                if self.new_group.visible
                    && dist(at, self.new_group.centre) <= self.new_group.radius
                {
                    return Target::NewGroup;
                }
                if dist(at, self.sun.centre) <= self.sun.radius {
                    return Target::Sun;
                }
                self.planets
                    .iter()
                    .filter(|p| dist(at, p.centre) <= p.halo_radius)
                    .min_by(|a, b| dist(at, a.centre).total_cmp(&dist(at, b.centre)))
                    .map_or(Target::Backdrop, |p| Target::Planet(p.group))
            }
            Level::Group => {
                if let Some(chip) = self.chips.iter().find(|c| c.rect.contains(x, y)) {
                    return Target::Chip(chip.group);
                }
                // The last card drawn is on top.
                self.previews
                    .iter()
                    .rev()
                    .find(|c| c.rect.contains(x, y))
                    .map_or(Target::Backdrop, |c| Target::Preview(c.tab))
            }
            Level::Tab => {
                if self.card.as_ref().is_some_and(|c| c.rect.contains(x, y)) {
                    return Target::Card;
                }
                self.neighbours
                    .iter()
                    .find(|n| n.rect.contains(x, y))
                    .map_or(Target::Backdrop, |n| Target::Neighbour(n.tab))
            }
        }
    }

    /// The ring that contains the stage point and the point in that ring's own coordinates, for
    /// `RingBandsData::hit`: a planet at level 1, the centre ring at level 2, none at level 3.
    pub fn ring_at(&self, x: f32, y: f32) -> Option<RingPoint> {
        let at = Point::new(x, y);
        match self.level {
            Level::Groups => self
                .planets
                .iter()
                .filter(|p| dist(at, p.centre) <= p.spec.label_outer_radius() + p.spec.lift)
                .min_by(|a, b| dist(at, a.centre).total_cmp(&dist(at, b.centre)))
                .map(|p| {
                    let half = p.ring_size / 2.0;
                    RingPoint {
                        group: p.group,
                        x: x - p.centre.x + half,
                        y: y - p.centre.y + half,
                        centre: false,
                    }
                }),
            Level::Group => self.centre.as_ref().and_then(|ring| {
                (dist(at, ring.centre) <= ring.spec.label_outer_radius() + ring.spec.lift).then(
                    || {
                        let half = ring.ring_size / 2.0;
                        RingPoint {
                            group: ring.group,
                            x: x - ring.centre.x + half,
                            y: y - ring.centre.y + half,
                            centre: true,
                        }
                    },
                )
            }),
            Level::Tab => None,
        }
    }
}

/// A tooltip card right of the halo, or left of it when it would leave the window.
fn tooltip_for(text: &str, centre: Point, halo: f32, stage: Stage) -> (Rect, Side) {
    let w = text_width(text, 12.5, 600) + 24.0;
    let mut side = Side::Right;
    let mut x = centre.x + halo + TOOLTIP_GAP;
    if x + w > stage.width - EDGE {
        side = Side::Left;
        x = centre.x - halo - TOOLTIP_GAP - w;
    }
    x = x.min(stage.width - EDGE - w).max(EDGE);
    let top = TITLE_BAR_HEIGHT + EDGE;
    let y = (centre.y - TOOLTIP_HEIGHT / 2.0)
        .min(stage.height - EDGE - TOOLTIP_HEIGHT)
        .max(top);
    (Rect::new(x, y, w, TOOLTIP_HEIGHT), side)
}

/// The spoke from a ring's outer edge toward a card, ending where it meets the card's edge.
fn spoke(ring: Point, outer: f32, card: Rect) -> (Point, Point) {
    let middle = card.centre();
    let d = dist(ring, middle);
    if d < 1e-3 {
        return (ring, ring);
    }
    let (ux, uy) = ((middle.x - ring.x) / d, (middle.y - ring.y) / d);
    let from = Point::new(ring.x + ux * outer, ring.y + uy * outer);
    // Walk back from the card's middle toward the ring until the card's edge.
    let reach = |half: f32, u: f32| {
        if u.abs() < 1e-6 {
            f32::INFINITY
        } else {
            half / u.abs()
        }
    };
    let s = reach(card.w / 2.0, ux).min(reach(card.h / 2.0, uy));
    let to = if s >= d {
        middle
    } else {
        Point::new(middle.x - ux * s, middle.y - uy * s)
    };
    (from, to)
}
